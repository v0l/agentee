pub mod engine;
pub mod model;
pub mod ntff;
pub mod run;

use agentee_core::sim::SimResult;
use engine::Sim;
use model::{Meshing, PcbModel};

pub struct Plan {
    pub sim: Sim,
    pub fields: Vec<f64>,
    pub far_field: bool,
    pub plane_k: Option<usize>,
    pub map_bounds: Option<([f64; 2], [f64; 2])>,
    pub f_start: f64,
    pub f_stop: f64,
    pub points: usize,
    pub excite: Vec<usize>,
    pub max_steps: usize,
}

pub fn plan(
    model: &PcbModel,
    f_start: f64,
    f_stop: f64,
    points: usize,
    cell: f64,
    excite: Vec<usize>,
    max_steps: usize,
) -> Result<Plan, String> {
    let opt = Meshing { cell, f_max: f_stop, margin: 1.5, pml: 8, f0: 0.5 * (f_start + f_stop) };
    let sim = model.build(&opt)?;
    let plane_k = (model.sheets.len() >= 2).then(|| {
        let a = sim.grid.nearest(2, model.sheets[0].z * 1e-3);
        let b = sim.grid.nearest(2, model.sheets[1].z * 1e-3);
        (a + b) / 2
    });
    let mut bb = agentee_core::graphic::Bounds::EMPTY;
    model.outline.iter().for_each(|p| bb.add(*p));
    let map_bounds = (!bb.is_empty()).then_some((bb.min, bb.max));
    Ok(Plan {
        sim,
        f_start,
        f_stop,
        points,
        excite,
        max_steps,
        fields: Vec::new(),
        far_field: false,
        plane_k,
        map_bounds,
    })
}

fn dft(
    series: &[f32],
    ports: usize,
    port: usize,
    which: usize,
    f: f64,
    dt: f64,
    half: bool,
) -> (f64, f64) {
    let w = 2.0 * std::f64::consts::PI * f * dt;
    let (rs, rc) = (-w).sin_cos();
    let start = if half { -0.5 * w } else { -w };
    let (mut pr, mut pi) = (start.cos() * dt, start.sin() * dt);
    let (mut re, mut im) = (0.0, 0.0);
    let steps = series.len() / (ports * 2);
    for n in 0..steps {
        let v = series[(n * ports + port) * 2 + which] as f64;
        re += v * pr;
        im += v * pi;
        let (a, b) = (pr * rc - pi * rs, pr * rs + pi * rc);
        pr = a;
        pi = b;
    }
    (re, im)
}

fn cdiv(a: (f64, f64), b: (f64, f64)) -> [f64; 2] {
    let d = b.0 * b.0 + b.1 * b.1;
    [(a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d]
}

pub fn execute(
    plan: &Plan,
    name: &str,
    spec_hash: u64,
    progress: &mut dyn FnMut(&str, usize, f64),
) -> Result<SimResult, String> {
    let t0 = std::time::Instant::now();
    let sim = &plan.sim;
    let np = sim.ports.len();
    let freqs: Vec<f64> = (0..plan.points)
        .map(|i| {
            plan.f_start + (plan.f_stop - plan.f_start) * i as f64 / (plan.points - 1).max(1) as f64
        })
        .collect();
    let mut s = vec![vec![vec![[0.0, 0.0]; freqs.len()]; np]; np];
    let mut excited = vec![false; np];
    let mut maps = Vec::new();
    let mut readings = Vec::new();
    let mut steps = Vec::new();
    let pulse = run::Pulse {
        f0: 0.5 * (plan.f_start + plan.f_stop),
        fc: 0.5 * (plan.f_stop - plan.f_start) * 1.1,
    };
    let min_steps = ((1.5 / plan.f_start) / sim.dt) as usize;
    for &j in &plan.excite {
        let label = sim.ports[j].name.clone();
        let patches = if plan.far_field && !plan.fields.is_empty() {
            Some(ntff::patches(sim, 3))
        } else {
            None
        };
        let extras = run::Extras {
            min_steps: min_steps.min(plan.max_steps),
            max_steps: plan.max_steps,
            decay_db: 40.0,
            freqs: plan.fields.clone(),
            plane_k: if plan.fields.is_empty() { None } else { plan.plane_k },
            ntff: patches.as_ref().map(|p| p.gpu.clone()),
        };
        let rec = run::run(sim, j, &pulse, &extras, &mut |n, db| progress(&label, n, db))?;
        extra_outputs(plan, &rec, j, &label, &patches, &mut maps, &mut readings);
        steps.push(rec.steps);
        excited[j] = true;
        for (fi, f) in freqs.iter().enumerate() {
            let z0 = |k: usize| sim.ports[k].r;
            let wave = |k: usize| {
                let v = dft(&rec.series, np, k, 0, *f, sim.dt, false);
                let i = dft(&rec.series, np, k, 1, *f, sim.dt, true);
                let r = z0(k);
                let a = ((v.0 + r * i.0) / (2.0 * r.sqrt()), (v.1 + r * i.1) / (2.0 * r.sqrt()));
                let b = ((v.0 - r * i.0) / (2.0 * r.sqrt()), (v.1 - r * i.1) / (2.0 * r.sqrt()));
                (a, b)
            };
            let (aj, _) = wave(j);
            for (i, row) in s.iter_mut().enumerate() {
                let (_, bi) = wave(i);
                row[j][fi] = cdiv(bi, aj);
            }
        }
    }
    let n = sim.dims();
    Ok(SimResult {
        name: name.to_string(),
        freqs,
        ports: sim.ports.iter().map(|p| p.name.clone()).collect(),
        s,
        excited,
        cells: sim.grid.cells(),
        grid: n,
        steps,
        dt: sim.dt,
        seconds: t0.elapsed().as_secs_f64(),
        device: crate::gpu::gpu().map(|g| g.name.clone()).unwrap_or_default(),
        spec_hash,
        maps,
        readings,
    })
}

fn tag(plan: &Plan, f: f64, port: &str) -> String {
    if plan.excite.len() > 1 { format!("{} {port}", freq_label(f)) } else { freq_label(f) }
}

fn freq_label(f: f64) -> String {
    if f >= 1e9 {
        format!("{} GHz", agentee_core::units::trim(f / 1e9, 3))
    } else {
        format!("{} MHz", agentee_core::units::trim(f / 1e6, 1))
    }
}

fn extra_outputs(
    plan: &Plan,
    rec: &run::Record,
    j: usize,
    label: &str,
    patches: &Option<ntff::Patches>,
    maps: &mut Vec<agentee_core::sim::LayerMap>,
    readings: &mut Vec<agentee_core::sim::Reading>,
) {
    let sim = &plan.sim;
    let np = sim.ports.len();
    let incident = |f: f64| -> (f64, f64) {
        let v = dft(&rec.series, np, j, 0, f, sim.dt, false);
        let i = dft(&rec.series, np, j, 1, f, sim.dt, true);
        let r = sim.ports[j].r;
        ((v.0 + r * i.0) / (2.0 * r.sqrt()), (v.1 + r * i.1) / (2.0 * r.sqrt()))
    };
    let nf = plan.fields.len().min(4);
    if !rec.plane.is_empty()
        && let Some((lo, hi)) = plan.map_bounds
    {
        let n0 = sim.dims();
        let cell = 0.05f64.max((hi[0] - lo[0]).max(hi[1] - lo[1]) / 600.0);
        let (w, h) = (
            ((hi[0] - lo[0]) / cell).ceil() as usize + 1,
            ((hi[1] - lo[1]) / cell).ceil() as usize + 1,
        );
        for (k, f) in plan.fields.iter().take(nf).enumerate() {
            let a = incident(*f);
            let scale = (2e-3f64).sqrt() / (a.0 * a.0 + a.1 * a.1).sqrt().max(1e-30);
            let mut e = vec![f32::NAN; w * h];
            let mut hm = vec![f32::NAN; w * h];
            for yy in 0..h {
                for xx in 0..w {
                    let (x, y) =
                        ((lo[0] + xx as f64 * cell) * 1e-3, (lo[1] + yy as f64 * cell) * 1e-3);
                    let (i, jj) = (sim.grid.nearest(0, x), sim.grid.nearest(1, y));
                    let b = ((i * n0[1] + jj) * nf + k) * 10;
                    let mag = |c: usize| {
                        (rec.plane[b + 2 * c] as f64).powi(2)
                            + (rec.plane[b + 2 * c + 1] as f64).powi(2)
                    };
                    e[yy * w + xx] = (20.0
                        * ((mag(0) + mag(1) + mag(2)).sqrt() * scale).max(1e-9).log10())
                        as f32;
                    hm[yy * w + xx] =
                        (20.0 * ((mag(3) + mag(4)).sqrt() * scale).max(1e-9).log10()) as f32;
                }
            }
            for v in [&mut e, &mut hm] {
                let top = v.iter().cloned().fold(f32::MIN, f32::max);
                v.iter_mut().for_each(|x| *x = x.max(top - 60.0));
            }
            let grid = agentee_core::sim::MapGrid { origin: lo, cell, width: w, height: h };
            let layer = tag(plan, *f, label);
            maps.push(agentee_core::sim::LayerMap::encode(&layer, "E at 1 mW", "dBV/m", grid, &e));
            maps.push(agentee_core::sim::LayerMap::encode(&layer, "H at 1 mW", "dBA/m", grid, &hm));
        }
    }
    let Some(p) = patches else { return };
    if rec.ntff.is_empty() {
        return;
    }
    for (k, f) in plan.fields.iter().take(nf).enumerate() {
        let a = incident(*f);
        let norm = std::f64::consts::SQRT_2 / (a.0 * a.0 + a.1 * a.1).max(1e-60);
        let (ar, ai) = (a.0 * norm, -a.1 * norm);
        let n_p = p.pos.len();
        let mut jv = Vec::with_capacity(n_p);
        let mut mv = Vec::with_capacity(n_p);
        for q in 0..n_p {
            let o = (q * nf + k) * 12;
            let c = |x: usize| {
                let (re, im) = (rec.ntff[o + x] as f64, rec.ntff[o + x + 1] as f64);
                (re * ar - im * ai, re * ai + im * ar)
            };
            jv.push([c(0), c(2), c(4)]);
            mv.push([c(6), c(8), c(10)]);
        }
        let far = ntff::Far { pos: &p.pos, area: &p.area, j: jv, m: mv, freq: *f };
        let (prad, umax) = far.radiated(18);
        let e3 = (engine::ETA0 * umax * 1e-3).sqrt() / 3.0;
        let dbuv = 20.0 * (e3 * 1e6).max(1e-30).log10();
        let limit = ntff::fcc_class_b_dbuv(*f);
        readings.push(agentee_core::sim::Reading {
            label: format!("radiated {}", tag(plan, *f, label)),
            value: prad * 100.0,
            unit: "%".into(),
            detail: format!(
                "directivity {:.1} dBi",
                10.0 * (4.0 * std::f64::consts::PI * umax / prad.max(1e-30)).log10()
            ),
        });
        readings.push(agentee_core::sim::Reading {
            label: format!("E at 3 m {}", tag(plan, *f, label)),
            value: dbuv,
            unit: "dBuV/m".into(),
            detail: format!("1 mW in, FCC B {limit}, margin {:.1} dB", limit - dbuv),
        });
    }
}

pub fn touchstone(r: &SimResult) -> String {
    let np = r.ports.len();
    let mut out = format!("! agentee {}\n! ports {}\n# Hz S RI R 50\n", r.name, r.ports.join(" "));
    for (fi, f) in r.freqs.iter().enumerate() {
        out += &format!("{f:.6e}");
        let order: Vec<(usize, usize)> = if np == 2 {
            vec![(0, 0), (1, 0), (0, 1), (1, 1)]
        } else {
            (0..np).flat_map(|i| (0..np).map(move |j| (i, j))).collect()
        };
        for (k, (i, j)) in order.iter().enumerate() {
            let c = r.s[*i][*j][fi];
            if np > 2 && k > 0 && k % np == 0 {
                out += "\n";
            }
            out += &format!(" {:.6e} {:.6e}", c[0], c[1]);
        }
        out += "\n";
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Copper, Dielectric, ModelPort, Sheet};

    fn line(len: f64) -> SimResult {
        let (w, h, er) = (2.9, 1.51, 4.5);
        let strip = |x: f64| {
            vec![
                [x - 0.05, -w / 2.0],
                [x + 0.05, -w / 2.0],
                [x + 0.05, w / 2.0],
                [x - 0.05, w / 2.0],
            ]
        };
        let m = PcbModel {
            outline: vec![[0.0, -5.0], [len, -5.0], [len, 5.0], [0.0, 5.0]],
            sheets: vec![
                Sheet { name: "F.Cu".into(), z: 0.0 },
                Sheet { name: "B.Cu".into(), z: -h },
            ],
            dielectrics: vec![Dielectric { z0: -h, z1: 0.0, er, tan: 0.0, pinned: true }],
            copper: vec![
                (0, Copper::Seg([0.5, 0.0], [len - 0.5, 0.0], w)),
                (1, Copper::Poly(vec![[0.0, -5.0], [len, -5.0], [len, 5.0], [0.0, 5.0]])),
            ],
            vias: vec![],
            ports: vec![
                ModelPort {
                    name: "P1".into(),
                    at: [0.5, 0.0],
                    area: strip(0.5),
                    sheet: 0,
                    reference: 1,
                    r: 50.0,
                },
                ModelPort {
                    name: "P2".into(),
                    at: [len - 0.5, 0.0],
                    area: strip(len - 0.5),
                    sheet: 0,
                    reference: 1,
                    r: 50.0,
                },
            ],
            elements: vec![],
            features_x: vec![],
            features_y: vec![-w / 2.0, w / 2.0],
            region: None,
        };
        let mut p = plan(&m, 0.5e9, 4e9, 36, 0.12, vec![0], 80_000).unwrap();
        if len < 30.0 {
            p.fields = vec![2e9, 3.5e9];
            p.far_field = true;
        }
        execute(&p, "line", 0, &mut |_, _, _| {}).unwrap()
    }

    fn delay(r: &SimResult, k: usize) -> f64 {
        let mut unwrapped = 0.0f64;
        let mut last = 0.0f64;
        for i in 0..=k {
            let ph = r.s[1][0][i][1].atan2(r.s[1][0][i][0]);
            let mut d = ph - last;
            while d > std::f64::consts::PI {
                d -= 2.0 * std::f64::consts::PI;
            }
            while d < -std::f64::consts::PI {
                d += 2.0 * std::f64::consts::PI;
            }
            unwrapped += d;
            last = ph;
        }
        -unwrapped / (2.0 * std::f64::consts::PI * r.freqs[k])
    }

    #[test]
    fn a_fifty_ohm_microstrip_is_matched_and_travels_at_the_right_speed() {
        if crate::gpu::gpu().is_none() {
            return;
        }
        let (a, b) = (line(20.0), line(40.0));
        for r in [&a, &b] {
            assert!(r.db(1, 0).iter().all(|v| *v > -0.2 && *v < 0.1), "{:?}", r.db(1, 0));
            assert!(r.db(0, 0).iter().all(|v| *v < -25.0), "{:?}", r.db(0, 0));
        }
        let k = a.freqs.iter().position(|f| *f >= 2e9).unwrap();
        for f in [2e9, 3.5e9] {
            let i = a.freqs.iter().position(|x| *x >= f - 1.0).unwrap();
            let lost = 1.0 - 10f64.powf(a.db(0, 0)[i] / 10.0) - 10f64.powf(a.db(1, 0)[i] / 10.0);
            let rad = a
                .readings
                .iter()
                .find(|r| {
                    r.label.contains("radiated")
                        && r.label.contains(if f < 3e9 { "2 GHz" } else { "3.5 GHz" })
                })
                .unwrap()
                .value
                / 100.0;
            eprintln!(
                "{} GHz: radiated {:.3}%, lost from S {:.3}%",
                f / 1e9,
                rad * 100.0,
                lost * 100.0
            );
            assert!(rad >= 0.0 && rad <= lost.max(0.0) + 0.01, "{rad} {lost}");
        }
        assert_eq!(a.maps.len(), 4);
        let per_mm = (delay(&b, k) - delay(&a, k)) / 20.0;
        let reference = 3.4388f64.sqrt() / engine::C0 * 1e-3;
        eprintln!("{:.3} ps/mm, Kirschning-Jansen {:.3} ps/mm", per_mm * 1e12, reference * 1e12);
        assert!((per_mm - reference).abs() / reference < 0.03);
    }
}
