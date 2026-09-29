use super::engine::{MU0, Sim, idx, pulse_shape};
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
    pad: u32,
    base: [u32; 4],
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
}

fn psi_len(n: [usize; 3], axis: usize, pml: usize) -> usize {
    super::engine::psi_len(n, axis, pml)
}

pub fn run(
    sim: &Sim,
    driven: usize,
    pulse: &Pulse,
    min_steps: usize,
    max_steps: usize,
    decay_db: f64,
    progress: &mut dyn FnMut(usize, f64),
) -> Result<Record, String> {
    let g: &Gpu = gpu().ok_or("no GPU adapter for the FDTD run")?;
    let n = sim.dims();
    let nn = n[0] * n[1] * n[2];
    if nn >= (1 << 24) * 4 {
        return Err(format!("{} cells is too many for one run", sim.grid.cells()));
    }
    let mut coef = Vec::with_capacity(6 * nn);
    for c in 0..3 {
        coef.extend_from_slice(&sim.ca[c]);
    }
    for c in 0..3 {
        coef.extend_from_slice(&sim.cb[c]);
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
    let mut probe_def = Vec::new();
    let mut port_edges = Vec::new();
    let mut loops = Vec::new();
    for p in &sim.ports {
        let axis = p.columns[0][0].comp;
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let mid = &p.columns[p.columns.len() / 2];
        let first = port_edges.len() / 2;
        for e in mid {
            port_edges.extend_from_slice(&[
                idx(n, e.at[0], e.at[1], e.at[2]) as f32,
                sim.ax[axis].d[e.at[axis]] as f32,
            ]);
        }
        let loop_first = loops.len() / 3;
        let k = mid[mid.len() / 2].at[axis];
        let (i0, i1) = (
            p.columns.iter().map(|c| c[0].at[u]).min().unwrap(),
            p.columns.iter().map(|c| c[0].at[u]).max().unwrap(),
        );
        let (j0, j1) = (
            p.columns.iter().map(|c| c[0].at[v]).min().unwrap(),
            p.columns.iter().map(|c| c[0].at[v]).max().unwrap(),
        );
        let at = |a: usize, b: usize| {
            let mut q = [0usize; 3];
            q[u] = a;
            q[v] = b;
            q[axis] = k;
            idx(n, q[0], q[1], q[2]) as f32
        };
        let (hu, hv) = ((3 + u) as f32, (3 + v) as f32);
        for b in j0..=j1 {
            let w = sim.ax[v].dd[b] as f32;
            loops.extend_from_slice(&[at(i1, b), hv, w, at(i0 - 1, b), hv, -w]);
        }
        for a in i0..=i1 {
            let w = sim.ax[u].dd[a] as f32;
            loops.extend_from_slice(&[at(a, j1), hu, -w, at(a, j0 - 1), hu, w]);
        }
        probe_def.extend_from_slice(&[
            axis as f32,
            first as f32,
            mid.len() as f32,
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
        pad: 0,
        base: [0, n[0] as u32, (n[0] + n[1]) as u32, 0],
    };
    let nonempty = |v: Vec<f32>| if v.is_empty() { vec![0.0f32; 4] } else { v };
    let b_e = g.zeroed("e", (3 * nn * 4) as u64);
    let b_h = g.zeroed("h", (3 * nn * 4) as u64);
    let b_coef = g.storage("coef", &coef);
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
    let module = unsafe {
        g.device.create_shader_module_trusted(
            wgpu::ShaderModuleDescriptor {
                label: Some("fdtd"),
                source: wgpu::ShaderSource::Wgsl(include_str!("fdtd.wgsl").into()),
            },
            wgpu::ShaderRuntimeChecks::unchecked(),
        )
    };
    let group0: [(u32, &wgpu::Buffer); 11] = [
        (0, &b_e),
        (1, &b_coef),
        (2, &b_psi),
        (3, &b_axes),
        (4, &b_params),
        (5, &b_state),
        (6, &b_series),
        (7, &b_source),
        (8, &b_e),
        (9, &b_h),
        (10, &b_h),
    ];
    let group1: [(u32, &wgpu::Buffer); 5] =
        [(0, &b_inductor), (1, &b_probe), (2, &b_partial), (3, &b_edges), (4, &b_loops)];
    let make = |entry: &str, uses0: &[u32], uses1: &[u32]| {
        let pipe = g.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        });
        let mut groups = Vec::new();
        for (gi, (uses, set)) in [(uses0, &group0[..]), (uses1, &group1[..])].iter().enumerate() {
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
        (pipe, groups)
    };
    let grid = [(n[2] as u32).div_ceil(64), (n[1] as u32).div_ceil(4), n[0] as u32];
    let lumped_n = (sim.port_src[driven].len().max(sim.inductors.len()) as u32).div_ceil(64).max(1);
    let pipes = vec![
        (make("update_h", &[2, 3, 4, 8, 9], &[]), grid),
        (make("update_e", &[0, 1, 2, 3, 4, 10], &[]), grid),
        (make("lumped", &[0, 4, 5, 7], &[0]), [lumped_n, 1, 1]),
        (make("probe", &[4, 5, 6, 8, 10], &[1, 3, 4]), [(ports as u32).div_ceil(64), 1, 1]),
        (make("tick", &[5], &[]), [1, 1, 1]),
    ];
    let energy = make("energy", &[4, 8], &[2]);
    let chunk = 1000;
    let mut steps = 0;
    let mut peak: f64 = 0.0;
    let mut decay = 0.0;
    let pulse_end = (2.0 * t0 / sim.dt) as usize;
    while steps < max_steps {
        let k = chunk.min(max_steps - steps);
        let mut enc = g.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            for _ in 0..k {
                for ((pipe, groups), d) in &pipes {
                    pass.set_pipeline(pipe);
                    for (gi, bg) in groups {
                        pass.set_bind_group(*gi, bg, &[]);
                    }
                    pass.dispatch_workgroups(d[0], d[1], d[2]);
                }
            }
            pass.set_pipeline(&energy.0);
            for (gi, bg) in &energy.1 {
                pass.set_bind_group(*gi, bg, &[]);
            }
            pass.dispatch_workgroups(1024, 1, 1);
        }
        g.queue.submit([enc.finish()]);
        steps += k;
        let part: Vec<f32> = g.read(&b_partial, 1024);
        let e: f64 = part.iter().map(|v| *v as f64).sum();
        if !e.is_finite() {
            return Err("the FDTD run went unstable".into());
        }
        peak = peak.max(e);
        decay = if e > 0.0 && peak > 0.0 { 10.0 * (peak / e).log10() } else { 0.0 };
        progress(steps, decay);
        if steps > pulse_end.max(min_steps) && decay >= decay_db {
            break;
        }
    }
    let series: Vec<f32> = g.read(&b_series, steps * ports * 2);
    Ok(Record { steps, ports, series, decay_db: decay })
}
