pub mod engine;
pub mod model;
pub mod run;

use agentee_core::sim::SimResult;
use engine::Sim;
use model::{Meshing, PcbModel};

pub struct Plan {
    pub sim: Sim,
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
    Ok(Plan { sim, f_start, f_stop, points, excite, max_steps })
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
    let mut s = vec![vec![vec![[f64::NAN, f64::NAN]; freqs.len()]; np]; np];
    let mut excited = vec![false; np];
    let mut steps = Vec::new();
    let pulse = run::Pulse {
        f0: 0.5 * (plan.f_start + plan.f_stop),
        fc: 0.5 * (plan.f_stop - plan.f_start) * 1.1,
    };
    let min_steps = ((1.5 / plan.f_start) / sim.dt) as usize;
    for &j in &plan.excite {
        let label = sim.ports[j].name.clone();
        let rec = run::run(
            sim,
            j,
            &pulse,
            min_steps.min(plan.max_steps),
            plan.max_steps,
            40.0,
            &mut |n, db| progress(&label, n, db),
        )?;
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
    })
}

pub fn touchstone(r: &SimResult) -> String {
    let np = r.ports.len();
    let mut out =
        format!("! agentee FDTD {}\n! ports {}\n# Hz S RI R 50\n", r.name, r.ports.join(" "));
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
        let p = plan(&m, 0.5e9, 4e9, 36, 0.12, vec![0], 80_000).unwrap();
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
        let per_mm = (delay(&b, k) - delay(&a, k)) / 20.0;
        let reference = 3.4388f64.sqrt() / engine::C0 * 1e-3;
        eprintln!("{:.3} ps/mm, Kirschning-Jansen {:.3} ps/mm", per_mm * 1e12, reference * 1e12);
        assert!((per_mm - reference).abs() / reference < 0.03);
    }
}
