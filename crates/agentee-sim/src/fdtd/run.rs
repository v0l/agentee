use super::engine::{self, EPS0, MU0, PortDef, Sim, idx, pulse_shape};
use crate::gpu::{Gpu, gpu};
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    n: [u32; 4],
    pml: u32,
    sources: u32,
    inductors: u32,
    ports: u32,
    psi: [u32; 20],
    dt: f32,
    k_mu: f32,
    t0: f32,
    tau: f32,
    w0: f32,
    cap: u32,
    probes: u32,
    sheets: u32,
    base: [u32; 4],
    omega: [f32; 4],
    nf: u32,
    plane_k: u32,
    plane_n: u32,
    patches: u32,
    debye: u32,
    wide: u32,
    pad: [u32; 2],
}

#[derive(Clone, Default)]
pub struct Extras {
    pub min_steps: usize,
    pub max_steps: usize,
    pub decay_db: f64,
    pub freqs: Vec<f64>,
    pub plane_k: Option<usize>,
    pub ntff: Option<Vec<u32>>,
    pub fused_e: bool,
}

pub struct Pulse {
    pub f0: f64,
    pub fc: f64,
}

pub struct Record {
    pub steps: usize,
    pub ports: usize,
    pub series: Vec<f32>,
    pub decay_db: f64,
    pub plane: Vec<f32>,
    pub ntff: Vec<f32>,
}

fn psi_len(n: [usize; 3], axis: usize, pml: usize) -> usize {
    super::engine::psi_len(n, axis, pml)
}

struct CoefSets {
    index: Vec<u32>,
    sets: Vec<[f32; 2]>,
    wide: bool,
}

impl CoefSets {
    fn new(sim: &Sim) -> CoefSets {
        CoefSets::build(sim, 1 << 16)
    }

    fn build(sim: &Sim, narrow: usize) -> CoefSets {
        let nn = sim.ca[0].len();
        let mut lookup: std::collections::HashMap<(u32, u32), u32> = Default::default();
        let mut sets = vec![[0f32; 2]];
        lookup.insert((0, 0), 0);
        let mut flat = Vec::with_capacity(3 * nn);
        let mut last = ((0u32, 0u32), 0u32);
        for c in 0..3 {
            for (a, b) in sim.ca[c].iter().zip(&sim.cb[c]) {
                let key = if *b == 0.0 { (0, 0) } else { (a.to_bits(), b.to_bits()) };
                if key != last.0 {
                    let id = *lookup.entry(key).or_insert_with(|| {
                        sets.push([*a, *b]);
                        (sets.len() - 1) as u32
                    });
                    last = (key, id);
                }
                flat.push(last.1);
            }
        }
        let wide = sets.len() > narrow;
        let index = if wide {
            flat
        } else {
            flat.chunks(2).map(|p| p[0] | p.get(1).map_or(0, |v| v << 16)).collect()
        };
        CoefSets { index, sets, wide }
    }
}

fn port_loop(sim: &Sim, p: &PortDef, axis: usize, k: usize) -> Vec<(usize, usize, f64)> {
    let n = sim.dims();
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let at = |a: usize, b: usize| {
        let mut q = [0usize; 3];
        q[u] = a;
        q[v] = b;
        q[axis] = k;
        idx(n, q[0], q[1], q[2])
    };
    let mut turns: std::collections::BTreeMap<(usize, usize), i32> = Default::default();
    let nodes: std::collections::BTreeSet<(usize, usize)> =
        p.columns.iter().map(|c| (c[0].at[u], c[0].at[v])).collect();
    for (a, b) in &nodes {
        let (a, b) = (*a, *b);
        *turns.entry((at(a, b), v)).or_default() += 1;
        *turns.entry((at(a - 1, b), v)).or_default() -= 1;
        *turns.entry((at(a, b), u)).or_default() -= 1;
        *turns.entry((at(a, b - 1), u)).or_default() += 1;
    }
    turns
        .into_iter()
        .filter(|(_, t)| *t != 0)
        .map(|((id, comp), t)| (id, comp, t as f64 * sim.ax[comp].dd[engine::unidx(n, id)[comp]]))
        .collect()
}

fn fused_tile(depth: usize) -> (u32, u32) {
    if depth > 64 { (32, 8) } else { (64, 4) }
}

pub fn fused_update() -> bool {
    std::env::var("AGENTEE_FDTD_FUSED").is_ok_and(|v| v != "0")
}

const TIMESTAMPS: u32 = 4096;

fn kernel_timer(g: &Gpu) -> Option<(wgpu::QuerySet, wgpu::Buffer)> {
    let wanted = std::env::var("AGENTEE_FDTD_PROFILE").is_ok_and(|v| v != "0");
    if !wanted || !g.device.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES) {
        return None;
    }
    let set = g.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("kernel times"),
        ty: wgpu::QueryType::Timestamp,
        count: TIMESTAMPS,
    });
    let resolved = g.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("kernel times"),
        size: TIMESTAMPS as u64 * 8,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    Some((set, resolved))
}

fn report_kernel_times(g: &Gpu, resolved: &wgpu::Buffer, timed: &[usize], pipes: &[(usize, &str)]) {
    let ticks: Vec<u64> = g.read(resolved, 2 * timed.len());
    let ns = g.queue.get_timestamp_period() as f64;
    let mut sums = vec![(0.0f64, 0usize); pipes.len()];
    for (w, p) in ticks.chunks(2).zip(timed) {
        sums[*p].0 += w[1].saturating_sub(w[0]) as f64 * ns / 1e3;
        sums[*p].1 += 1;
    }
    let mut step = 0.0;
    for ((sum, count), (tag, name)) in sums.iter().zip(pipes).filter(|(s, _)| s.1 > 0) {
        let mean = sum / *count as f64;
        step += if *tag == 2 { mean } else { mean / 2.0 };
        let steps = ["even steps", "odd steps", "every step"][*tag];
        eprintln!("fdtd kernel {name} ({steps}): {mean:.1} us over {count} dispatches");
    }
    eprintln!("fdtd kernel time per step: {step:.1} us");
}

pub fn run(
    sim: &Sim,
    driven: usize,
    pulse: &Pulse,
    extras: &Extras,
    progress: &mut dyn FnMut(usize, f64),
) -> Result<Record, String> {
    let (min_steps, max_steps, decay_db) = (extras.min_steps, extras.max_steps, extras.decay_db);
    let g: &Gpu = gpu().ok_or("no GPU adapter for the FDTD run")?;
    let n = sim.dims();
    let nn = n[0] * n[1] * n[2];
    if nn >= (1 << 24) * 4 {
        return Err(format!("{} cells is too many for one run", sim.grid.cells()));
    }
    let mut offsets = [0u32; 20];
    let mut psi_total = 0usize;
    for e in 0..2 {
        for c in 0..3 {
            for a in 0..3 {
                offsets[e * 9 + c * 3 + a] = psi_total as u32;
                if a != c {
                    psi_total += psi_len(n, a, sim.pml);
                }
            }
        }
    }
    let mut axes = Vec::new();
    for a in &sim.ax {
        for i in 0..a.n {
            axes.extend_from_slice(&[
                a.inv_d[i],
                a.inv_dd[i],
                a.raw_inv_d[i],
                a.raw_inv_dd[i],
                a.be[i],
                a.ce[i],
                a.bh[i],
                a.ch[i],
                a.slot_e[i] as f32,
                a.slot_h[i] as f32,
            ]);
        }
    }
    let mut source = Vec::new();
    for (comp, id, s) in &sim.port_src[driven] {
        source.extend_from_slice(&[*id as f32, *s, *comp as f32]);
    }
    let mut inductor = Vec::new();
    for (comp, id, ke, ki) in &sim.inductors {
        inductor.extend_from_slice(&[*comp as f32, *id as f32, *ke, *ki, 0.0]);
    }
    let mut sheets = Vec::new();
    let branches = sim.surface.sheet_table(sim.dt);
    let states = sim.surface.real.len() + 2 * sim.surface.pairs.len();
    for s in &sim.sheets {
        let (hc, ok) = match s.comp {
            0 => (1usize, true),
            1 => (0usize, true),
            _ => (0, false),
        };
        if !ok || s.id % n[2] == 0 {
            continue;
        }
        let bits = |v: usize| f32::from_bits(v as u32);
        sheets.extend_from_slice(&[
            bits(s.comp * nn + s.id),
            bits(hc * nn + s.id),
            bits(hc * nn + s.id - 1),
            s.len as f32,
            s.c as f32,
            s.r as f32,
            if s.adaptive { 1.0 } else { 0.0 },
            0.0,
            0.0,
            0.0,
            0.5,
            0.5,
            0.0,
            0.0,
            bits(s.faces[0]),
            bits(s.faces[1]),
        ]);
    }
    let sheet_count = sheets.len() / 16;
    let mut debye_edges = Vec::with_capacity(sim.debye_edges.len() * 3);
    for (c, id, dd) in &sim.debye_edges {
        debye_edges.extend_from_slice(&[f32::from_bits((c * nn + id) as u32), *dd as f32, 0.0]);
    }
    let debye_count = sim.debye_edges.len();
    let fused = extras.fused_e;
    let coef = CoefSets::new(sim);
    let mut debye_table = vec![sim.debye.poles.len() as f32];
    for (x, a) in &sim.debye.poles {
        let d = 1.0 + 0.5 * x * sim.dt;
        debye_table
            .extend_from_slice(&[((1.0 - 0.5 * x * sim.dt) / d) as f32, (EPS0 * a * x / d) as f32]);
    }
    let mut probe_def = Vec::new();
    let mut port_edges = Vec::new();
    let mut loops = Vec::new();
    for p in &sim.ports {
        let axis = p.columns[0][0].comp;
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let mid = &p.columns[p.columns.len() / 2];
        let first = port_edges.len() / 2;
        let polarity = if p.reference_above { -1.0 } else { 1.0 };
        let weight = polarity / p.columns.len() as f64;
        for col in &p.columns {
            for e in col {
                port_edges.extend_from_slice(&[
                    idx(n, e.at[0], e.at[1], e.at[2]) as f32,
                    (sim.ax[axis].d[e.at[axis]] * weight) as f32,
                ]);
            }
        }
        let probed = port_edges.len() / 2 - first;
        let loop_first = loops.len() / 3;
        let height: f64 = mid.iter().map(|e| sim.ax[axis].d[e.at[axis]]).sum();
        for e in mid {
            let k = e.at[axis];
            let share = polarity * sim.ax[axis].d[k] / height;
            for (id, comp, w) in port_loop(sim, p, axis, k) {
                loops.extend_from_slice(&[id as f32, (3 + comp) as f32, (w * share) as f32]);
            }
        }
        probe_def.extend_from_slice(&[
            axis as f32,
            first as f32,
            probed as f32,
            loop_first as f32,
            ((loops.len() / 3) - loop_first) as f32,
            u as f32,
            v as f32,
            0.0,
        ]);
    }
    if port_edges.iter().step_by(2).any(|v| *v >= (1u32 << 24) as f32) {
        return Err("grid too large for f32 port indices".into());
    }
    let (t0, tau) = pulse_shape(pulse.fc);
    let cap = max_steps;
    let ports = sim.ports.len();
    let params = Params {
        n: [n[0] as u32, n[1] as u32, n[2] as u32, nn as u32],
        pml: sim.pml as u32,
        sources: sim.port_src[driven].len() as u32,
        inductors: sim.inductors.len() as u32,
        ports: ports as u32,
        psi: offsets,
        dt: sim.dt as f32,
        k_mu: (sim.dt / MU0) as f32,
        t0: t0 as f32,
        tau: tau as f32,
        w0: (2.0 * std::f64::consts::PI * pulse.f0) as f32,
        cap: cap as u32,
        probes: ports as u32,
        sheets: sheet_count as u32,
        base: [0, n[0] as u32, (n[0] + n[1]) as u32, 0],
        omega: {
            let mut w = [0f32; 4];
            for (k, f) in extras.freqs.iter().take(4).enumerate() {
                w[k] = (2.0 * std::f64::consts::PI * f) as f32;
            }
            w
        },
        nf: extras.freqs.len().min(4) as u32,
        plane_k: extras.plane_k.unwrap_or(0) as u32,
        plane_n: if extras.plane_k.is_some() { (n[0] * n[1]) as u32 } else { 0 },
        patches: extras.ntff.as_ref().map(|v| v.len() / 13).unwrap_or(0) as u32,
        debye: debye_count as u32,
        wide: coef.wide as u32,
        pad: [0; 2],
    };
    let nf = extras.freqs.len().min(4);
    let plane_len = if extras.plane_k.is_some() { n[0] * n[1] * nf * 10 } else { 0 };
    let patches = extras.ntff.as_ref().map(|v| v.len() / 13).unwrap_or(0);
    let nonempty = |v: Vec<f32>| if v.is_empty() { vec![0.0f32; 4] } else { v };
    let b_e = g.zeroed("e", (3 * nn * 4) as u64);
    let b_h = g.zeroed("h", (3 * nn * 4) as u64);
    let b_coef = g.storage("coef_idx", &coef.index);
    let b_coef_set = g.storage("coef_set", &coef.sets);
    let b_psi = g.zeroed("psi", (psi_total.max(1) * 4) as u64);
    let b_axes = g.storage("axes", &axes);
    let b_params = g.uniform("params", &params);
    let b_state = g.storage("state", &[0u32; 4]);
    let b_series = g.zeroed("series", (cap * ports * 2 * 4) as u64);
    let b_source = g.storage("source", &nonempty(source));
    let b_inductor = g.storage("inductor", &nonempty(inductor));
    let b_probe = g.storage("probe", &nonempty(probe_def));
    let b_partial = g.zeroed("partial", 1024 * 4);
    let b_edges = g.storage("edges", &nonempty(port_edges));
    let b_loops = g.storage("loops", &nonempty(loops));
    let b_plane = g.zeroed("plane", (plane_len.max(1) * 4) as u64);
    let b_patch = g.storage(
        "patch_idx",
        &extras.ntff.clone().filter(|v| !v.is_empty()).unwrap_or(vec![0u32; 13]),
    );
    let b_ntff = g.zeroed("ntff", ((patches * nf * 12).max(1) * 4) as u64);
    let b_sheets = g.storage("sheets", &nonempty(sheets));
    let b_branches = g.storage("branches", &branches);
    let b_debye_edges = g.storage("debye_edges", &nonempty(debye_edges));
    let b_debye_table = g.storage("debye_table", &debye_table);
    let b_debye_state =
        g.zeroed("debye_state", ((debye_count * sim.debye.poles.len()).max(1) * 4) as u64);
    let b_currents = g.zeroed("currents", ((sheet_count * states).max(1) * 4) as u64);
    let (tk, tj) = fused_tile(n[2]);
    let module = unsafe {
        g.device.create_shader_module_trusted(
            wgpu::ShaderModuleDescriptor {
                label: Some("fdtd"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("fdtd.wgsl")
                        .replace("const TK: u32 = 64u;", &format!("const TK: u32 = {tk}u;"))
                        .replace("const TJ: u32 = 4u;", &format!("const TJ: u32 = {tj}u;"))
                        .into(),
                ),
            },
            wgpu::ShaderRuntimeChecks::unchecked(),
        )
    };
    let twin = |b: &wgpu::Buffer, label: &str| {
        if fused { g.zeroed(label, b.size()) } else { g.zeroed(label, 4) }
    };
    let (b_e2, b_h2, b_psi2) = (twin(&b_e, "e2"), twin(&b_h, "h2"), twin(&b_psi, "psi2"));
    let fields = |e_in, e_out, h_in, h_out, psi_in, psi_out| -> [(u32, &wgpu::Buffer); 13] {
        [
            (0, e_out),
            (1, &b_coef),
            (2, psi_out),
            (3, &b_axes),
            (4, &b_params),
            (5, &b_state),
            (6, &b_series),
            (7, &b_source),
            (8, e_in),
            (9, h_out),
            (10, h_in),
            (11, &b_coef_set),
            (12, psi_in),
        ]
    };
    let group0 = fields(&b_e, &b_e, &b_h, &b_h, &b_psi, &b_psi);
    let stepped = [
        fields(&b_e, &b_e2, &b_h, &b_h2, &b_psi, &b_psi2),
        fields(&b_e2, &b_e, &b_h2, &b_h, &b_psi2, &b_psi),
    ];
    let fixing = [
        fields(&b_e, &b_e2, &b_h2, &b_h2, &b_psi2, &b_psi2),
        fields(&b_e2, &b_e, &b_h, &b_h, &b_psi, &b_psi),
    ];
    let latest = [
        fields(&b_e2, &b_e2, &b_h2, &b_h2, &b_psi2, &b_psi2),
        fields(&b_e, &b_e, &b_h, &b_h, &b_psi, &b_psi),
    ];
    let group1: [(u32, &wgpu::Buffer); 14] = [
        (0, &b_inductor),
        (1, &b_probe),
        (2, &b_partial),
        (3, &b_edges),
        (4, &b_loops),
        (5, &b_plane),
        (6, &b_patch),
        (7, &b_ntff),
        (8, &b_branches),
        (9, &b_sheets),
        (10, &b_currents),
        (11, &b_debye_edges),
        (12, &b_debye_table),
        (13, &b_debye_state),
    ];
    let make_with = |entry: &str, uses0: &[u32], uses1: &[u32], set0: &[(u32, &wgpu::Buffer)]| {
        let pipe = g.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        });
        let mut groups = Vec::new();
        for (gi, (uses, set)) in [(uses0, set0), (uses1, &group1[..])].iter().enumerate() {
            if uses.is_empty() {
                continue;
            }
            let entries: Vec<wgpu::BindGroupEntry> = set
                .iter()
                .filter(|(b, _)| uses.contains(b))
                .map(|(b, buf)| wgpu::BindGroupEntry {
                    binding: *b,
                    resource: buf.as_entire_binding(),
                })
                .collect();
            groups.push((
                gi as u32,
                g.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(entry),
                    layout: &pipe.get_bind_group_layout(gi as u32),
                    entries: &entries,
                }),
            ));
        }
        (pipe, groups, entry.to_string())
    };
    let make = |entry: &str, uses0: &[u32], uses1: &[u32]| make_with(entry, uses0, uses1, &group0);
    let grid = [(n[2] as u32).div_ceil(64), (n[1] as u32).div_ceil(4), n[0] as u32];
    let lumped_n = (sim.port_src[driven].len().max(sim.inductors.len()) as u32).div_ceil(64).max(1);
    let mut pipes = Vec::new();
    let after = |parity: usize, entry: &str, uses0: &[u32], uses1: &[u32]| {
        if fused {
            make_with(entry, uses0, uses1, &latest[parity])
        } else {
            make(entry, uses0, uses1)
        }
    };
    let sheet_groups = [(sheet_count as u32).div_ceil(64), 1, 1];
    let debye_groups = [(debye_count as u32).div_ceil(64).max(1), 1, 1];
    let debye_groups = if debye_groups[0] > 65535 {
        [65535, debye_groups[0].div_ceil(65535), 1]
    } else {
        debye_groups
    };
    if fused {
        let blocks =
            [(n[2] as u32).div_ceil(tk), (n[1] as u32).div_ceil(tj), (n[0] as u32).div_ceil(4)];
        for (parity, entry) in ["step_even", "step_odd"].iter().enumerate() {
            let uses0 = [0, 1, 2, 3, 4, 5, 8, 9, 10, 11, 12];
            pipes.push((parity, make_with(entry, &uses0, &[], &stepped[parity]), blocks));
            let fix = &fixing[parity];
            if sheet_count > 0 {
                let pipe = make_with("sheet_fix", &[0, 4, 8, 10], &[8, 9, 10], fix);
                pipes.push((parity, pipe, sheet_groups));
            }
            if debye_count > 0 {
                let pipe = make_with("debye_fix", &[0, 1, 4, 8, 11], &[11, 12, 13], fix);
                pipes.push((parity, pipe, debye_groups));
            }
            let pipe = make_with("lumped", &[0, 4, 5, 7], &[0], fix);
            pipes.push((parity, pipe, [lumped_n, 1, 1]));
        }
    } else {
        pipes.push((2, make("update_h", &[2, 3, 4, 5, 8, 9], &[]), grid));
        if sheet_count > 0 {
            pipes.push((2, make("sheet_pre", &[0, 4, 10], &[9]), sheet_groups));
        }
        if debye_count > 0 {
            pipes.push((2, make("debye_pre", &[0, 4], &[11]), debye_groups));
        }
        pipes.push((2, make("update_e", &[0, 1, 2, 3, 4, 10, 11], &[]), grid));
        if sheet_count > 0 {
            pipes.push((2, make("sheet_post", &[0, 4], &[8, 9, 10]), sheet_groups));
        }
        if debye_count > 0 {
            pipes.push((2, make("debye_post", &[0, 1, 4, 11], &[11, 12, 13]), debye_groups));
        }
        pipes.push((2, make("lumped", &[0, 4, 5, 7], &[0]), [lumped_n, 1, 1]));
    }
    let parities: &[usize] = if fused { &[0, 1] } else { &[2] };
    for &parity in parities {
        pipes.push((
            parity,
            after(parity, "probe", &[4, 5, 6, 8, 10], &[1, 3, 4]),
            [ports as u32, 1, 1],
        ));
        if plane_len > 0 {
            pipes.push((
                parity,
                after(parity, "field_dft", &[4, 5, 8, 10], &[5]),
                [((n[0] * n[1]) as u32).div_ceil(64), 1, 1],
            ));
        }
        if patches > 0 && nf > 0 {
            pipes.push((
                parity,
                after(parity, "ntff", &[4, 5, 8, 10], &[6, 7]),
                [(patches as u32).div_ceil(64), 1, 1],
            ));
        }
    }
    let energy = [after(0, "energy", &[4, 8, 10], &[2]), after(1, "energy", &[4, 8, 10], &[2])];
    let chunk = 1000;
    let mut steps = 0;
    let mut peak: f64 = 0.0;
    let mut decay = 0.0;
    let pulse_end = (2.0 * t0 / sim.dt) as usize;
    let timer = kernel_timer(g);
    let mut timed_pipes = Vec::new();
    while steps < max_steps {
        let k = chunk.min(max_steps - steps);
        let mut enc = g.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            for s in 0..k {
                let parity = (steps + s) % 2;
                for (p, (tag, (pipe, groups, _), d)) in pipes.iter().enumerate() {
                    if *tag != 2 && *tag != parity {
                        continue;
                    }
                    pass.set_pipeline(pipe);
                    for (gi, bg) in groups {
                        pass.set_bind_group(*gi, bg, &[]);
                    }
                    let q = 2 * timed_pipes.len() as u32;
                    let timed = timer.as_ref().filter(|_| steps == 0 && s >= 200 && q < TIMESTAMPS);
                    if let Some((set, _)) = timed {
                        pass.write_timestamp(set, q);
                    }
                    pass.dispatch_workgroups(d[0], d[1], d[2]);
                    if let Some((set, _)) = timed {
                        pass.write_timestamp(set, q + 1);
                        timed_pipes.push(p);
                    }
                }
            }
            let energy = &energy[(steps + k - 1) % 2];
            pass.set_pipeline(&energy.0);
            for (gi, bg) in &energy.1 {
                pass.set_bind_group(*gi, bg, &[]);
            }
            pass.dispatch_workgroups(1024, 1, 1);
        }
        let timed = timer.as_ref().filter(|_| steps == 0 && !timed_pipes.is_empty());
        if let Some((set, resolved)) = timed {
            enc.resolve_query_set(set, 0..2 * timed_pipes.len() as u32, resolved, 0);
        }
        g.queue.submit([enc.finish()]);
        if let Some((_, resolved)) = timed {
            let names: Vec<(usize, &str)> =
                pipes.iter().map(|(tag, (_, _, name), _)| (*tag, name.as_str())).collect();
            report_kernel_times(g, resolved, &timed_pipes, &names);
        }
        steps += k;
        let part: Vec<f32> = g.read(&b_partial, 1024);
        let e: f64 = part.iter().map(|v| *v as f64).sum();
        if !e.is_finite() {
            return Err("the FDTD run went unstable".into());
        }
        peak = peak.max(e);
        decay = if e > 0.0 && peak > 0.0 { 10.0 * (peak / e).log10() } else { 0.0 };
        progress(steps, decay);
        if crate::runner::cancelled() {
            return Err("stopped".into());
        }
        if steps > pulse_end.max(min_steps) && decay >= decay_db {
            break;
        }
    }
    let series: Vec<f32> = g.read(&b_series, steps * ports * 2);
    let plane = if plane_len > 0 { g.read(&b_plane, plane_len) } else { Vec::new() };
    let ntff = if patches > 0 && nf > 0 { g.read(&b_ntff, patches * nf * 12) } else { Vec::new() };
    Ok(Record { steps, ports, series, decay_db: decay, plane, ntff })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fdtd::engine::{Debye, Edge, Element, Grid, Materials, Media, PortDef};
    use crate::fdtd::surface::Surface;

    fn at(sets: &CoefSets, f: usize) -> [f32; 2] {
        let i =
            if sets.wide { sets.index[f] } else { (sets.index[f / 2] >> (16 * (f % 2))) & 0xffff };
        sets.sets[i as usize]
    }

    #[test]
    fn a_port_loop_circles_only_its_own_columns() {
        let line = |n: usize| (0..n).map(|i| i as f64 * 1e-3 * (1.0 + 0.1 * i as f64)).collect();
        let grid = Grid { x: line(9), y: line(8), z: line(7), pml: 0 };
        let mats = Materials::new(&grid);
        let media = || Media {
            surface: Surface { scale: 1e9, ..Default::default() },
            debye: Debye::default(),
        };
        let column = |i: usize, j: usize| vec![Edge { comp: 2, at: [i, j, 3] }];
        let ring: Vec<Vec<Edge>> = (2..5)
            .flat_map(|i| (2..5).map(move |j| (i, j)))
            .filter(|p| *p != (3, 3))
            .map(|(i, j)| column(i, j))
            .collect();
        let port =
            PortDef { name: "p".into(), columns: ring.clone(), r: 50.0, reference_above: false };
        let sim = Sim::new(grid.clone(), &mats, &|_, _| false, &[], vec![port], &[], media());
        let n = sim.dims();
        let got = port_loop(&sim, &sim.ports[0], 2, 3);
        let single = |i: usize, j: usize| {
            let one = PortDef {
                name: "q".into(),
                columns: vec![column(i, j)],
                r: 50.0,
                reference_above: false,
            };
            port_loop(&sim, &one, 2, 3)
        };
        let mut want: std::collections::BTreeMap<(usize, usize), f64> = Default::default();
        for c in &ring {
            for (id, comp, w) in single(c[0].at[0], c[0].at[1]) {
                *want.entry((id, comp)).or_default() += w;
            }
        }
        want.retain(|_, w| w.abs() > 1e-12);
        assert_eq!(got.len(), 16);
        assert_eq!(want.len(), 16);
        for (id, comp, w) in &got {
            assert!((want[&(*id, *comp)] - w).abs() < 1e-15);
        }
        for (id, comp, w) in single(3, 3) {
            let &(_, _, back) = got.iter().find(|g| g.0 == id && g.1 == comp).unwrap();
            assert_eq!(back, -w);
        }
        let square = |i: usize, j: usize| -> usize { idx(n, i, j, 3) };
        let single_x = single(3, 3);
        assert_eq!(single_x.len(), 4);
        assert!(single_x.iter().any(|s| s.0 == square(3, 3) && s.1 == 1 && s.2 > 0.0));
        assert!(single_x.iter().any(|s| s.0 == square(2, 3) && s.1 == 1 && s.2 < 0.0));
        assert!(single_x.iter().any(|s| s.0 == square(3, 3) && s.1 == 0 && s.2 < 0.0));
        assert!(single_x.iter().any(|s| s.0 == square(3, 2) && s.1 == 0 && s.2 > 0.0));
    }

    #[test]
    fn coefficient_sets_give_back_every_edge() {
        let line = |n: usize| (0..n).map(|i| i as f64 * 1e-3 * (1.0 + 0.1 * i as f64)).collect();
        let grid = Grid { x: line(9), y: line(8), z: line(7), pml: 0 };
        let mut mats = Materials::new(&grid);
        for (i, e) in mats.eps.iter_mut().enumerate() {
            *e = 1.0 + (i % 5) as f32;
        }
        for (i, s) in mats.sigma.iter_mut().enumerate() {
            *s = (i % 3) as f32 * 0.01;
        }
        let port = PortDef {
            name: "p".into(),
            columns: vec![vec![Edge { comp: 2, at: [4, 4, 3] }]],
            r: 50.0,
            reference_above: false,
        };
        let media = Media {
            surface: Surface { scale: 1e9, ..Default::default() },
            debye: Debye::default(),
        };
        let sim = Sim::new(grid, &mats, &|c, p| c == 0 && p[2] == 2, &[], vec![port], &[], media);
        let nn = sim.ca[0].len();
        for narrow in [1 << 16, 1] {
            let sets = CoefSets::build(&sim, narrow);
            assert_eq!(sets.wide, narrow == 1);
            assert!(sets.sets.len() > 2);
            for c in 0..3 {
                for id in 0..nn {
                    let [a, b] = at(&sets, c * nn + id);
                    assert_eq!(b, sim.cb[c][id]);
                    if b != 0.0 {
                        assert_eq!(a, sim.ca[c][id]);
                    }
                }
            }
        }
    }

    #[test]
    fn the_fused_step_matches_the_separate_kernels() {
        use crate::fdtd::model::Sheet;
        use crate::fdtd::model::{Copper, Dielectric, Meshing, ModelElement, ModelPort, PcbModel};
        if gpu().is_none() {
            return;
        }
        let board = vec![[0.0, -2.0], [8.0, -2.0], [8.0, 2.0], [0.0, 2.0]];
        let m = PcbModel {
            outline: board.clone(),
            sheets: vec![
                Sheet { name: "F.Cu".into(), z: 0.0, thickness: 0.035 },
                Sheet { name: "B.Cu".into(), z: -0.4, thickness: 0.035 },
            ],
            dielectrics: vec![Dielectric { z0: -0.4, z1: 0.0, er: 4.0, tan: 0.02, pinned: true }],
            copper: vec![
                (0, Copper::Seg([0.5, 0.0], [3.5, 0.0], 0.6)),
                (0, Copper::Seg([4.5, 0.0], [7.5, 0.0], 0.6)),
                (1, Copper::Poly(board)),
            ],
            ports: vec![ModelPort {
                name: "P1".into(),
                at: [0.5, 0.0],
                area: vec![],
                sheet: 0,
                reference: 1,
                r: 50.0,
            }],
            elements: vec![ModelElement {
                name: "L1".into(),
                a: [3.5, 0.0],
                b: [4.5, 0.0],
                sheet: 0,
                element: Element::Inductor(1e-9),
            }],
            ..Default::default()
        };
        let opt = Meshing { cell: 0.2, f_max: 6e9, margin: 1.0, pml: 4, f0: 3e9 };
        let sim = m.build(&opt).unwrap();
        assert!(!sim.sheets.is_empty() && !sim.debye_edges.is_empty() && !sim.inductors.is_empty());
        assert_fused_matches(&sim, &Pulse { f0: 3e9, fc: 3e9 }, 3000);
        let line = |n: usize| (0..n).map(|i| i as f64 * 1e-3 * (1.0 + 0.01 * i as f64)).collect();
        let grid = Grid { x: line(23), y: line(19), z: line(150), pml: 6 };
        let mats = Materials::new(&grid);
        let port = PortDef {
            name: "p".into(),
            columns: vec![vec![Edge { comp: 2, at: [11, 9, 75] }]],
            r: 50.0,
            reference_above: false,
        };
        let media = Media {
            surface: Surface { scale: 1e10, ..Default::default() },
            debye: Debye::default(),
        };
        let open = Sim::new(grid, &mats, &|_, _| false, &[], vec![port], &[], media);
        assert_fused_matches(&open, &Pulse { f0: 20e9, fc: 20e9 }, 801);
    }

    fn assert_fused_matches(sim: &Sim, pulse: &Pulse, steps: usize) {
        let record = |fused_e: bool| {
            let extras =
                Extras { max_steps: steps, decay_db: f64::INFINITY, fused_e, ..Default::default() };
            run(sim, 0, pulse, &extras, &mut |_, _| {}).unwrap().series
        };
        let (apart, fused) = (record(false), record(true));
        assert!(apart.iter().any(|v| *v != 0.0));
        let worst = apart.iter().zip(&fused).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        let scale = apart.iter().map(|v| v.abs()).fold(0.0f32, f32::max);
        assert!(worst <= 1e-6 * scale, "{worst} of {scale}");
    }
}
