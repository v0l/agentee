use agentee_sim::fdtd::engine::{Debye, Edge, Grid, Materials, Media, PortDef, Sim};
use agentee_sim::fdtd::model::{Copper, Dielectric, ModelPort, PcbModel, Sheet};
use agentee_sim::fdtd::run::{Extras, Pulse, run};
use agentee_sim::fdtd::surface::Surface;
use agentee_sim::fdtd::{execute, plan, touchstone};
use serde_json::Value;

/// Free-space throughput run, the grid of openEMS's FreeSpace_Benchmark.py: `n`^3 cells of 1 mm,
/// PML_8 on all sides (`pml` 8) or PEC walls (`pml` 0), one 50 ohm port edge in the centre as the
/// source, no end criterion. Each step count runs `reps` times; the difference between two step
/// counts gives the per-step cost without the setup of a run.
fn throughput(n: usize, pml: usize, steps: &[usize], reps: usize) {
    let t = std::time::Instant::now();
    let dev = agentee_sim::gpu::gpu().map(|g| g.name.clone()).unwrap_or_default();
    eprintln!("device {dev}: {:.2}s", t.elapsed().as_secs_f64());
    let line: Vec<f64> = (0..=n).map(|i| i as f64 * 1e-3).collect();
    let grid = Grid { x: line.clone(), y: line.clone(), z: line, pml };
    let t = std::time::Instant::now();
    let mats = Materials::new(&grid);
    let c = n / 2;
    let port = PortDef {
        name: "src".into(),
        columns: vec![vec![Edge { comp: 2, at: [c, c, c] }]],
        r: 50.0,
    };
    let w0 = 2.0 * std::f64::consts::PI * 5e9;
    let media =
        Media { surface: Surface { scale: w0, ..Default::default() }, debye: Debye::default() };
    let sim = Sim::new(grid, &mats, &|_, _| false, &[], vec![port], &[], media);
    let nodes = sim.dims().iter().product::<usize>();
    eprintln!("setup (Sim::new) {:?} nodes {nodes}: {:.2}s", sim.dims(), t.elapsed().as_secs_f64());
    let pulse = Pulse { f0: 5e9, fc: 4e9 };
    let mut best = Vec::new();
    for &s in steps {
        let extras = Extras { max_steps: s, decay_db: f64::INFINITY, ..Default::default() };
        let mut times = Vec::new();
        for _ in 0..reps {
            let t = std::time::Instant::now();
            let r = run(&sim, 0, &pulse, &extras, &mut |_, _| {}).unwrap();
            assert_eq!(r.steps, s);
            times.push(t.elapsed().as_secs_f64());
        }
        let lo = times.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = times.iter().cloned().fold(0.0, f64::max);
        println!(
            "throughput n={n} pml={pml} steps={s}: best {lo:.3}s spread {:.3}s runs {times:.3?}, {:.0} MCells/s (nodes x steps / best run time)",
            hi - lo,
            nodes as f64 * s as f64 / lo / 1e6
        );
        best.push((s, lo));
    }
    for w in best.windows(2) {
        let ((s0, t0), (s1, t1)) = (w[0], w[1]);
        let per_step = (t1 - t0) / (s1 - s0) as f64;
        println!(
            "throughput n={n} pml={pml}: {:.3} ms/step, {:.0} MCells/s from steps {s0}..{s1}, overhead per run {:.3}s",
            per_step * 1e3,
            nodes as f64 / per_step / 1e6,
            t0 - per_step * s0 as f64
        );
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("throughput") {
        let arg =
            |i: usize, d: usize| std::env::args().nth(i).and_then(|v| v.parse().ok()).unwrap_or(d);
        let (n, pml, reps) = (arg(2, 300), arg(3, 8), arg(4, 3));
        let steps: Vec<usize> = std::env::args().skip(5).filter_map(|v| v.parse().ok()).collect();
        let steps = if steps.is_empty() { vec![200, 800] } else { steps };
        throughput(n, pml, &steps, reps);
        return;
    }
    let path = std::env::args().nth(1).expect("cases.json");
    let out = std::env::args().nth(2).unwrap_or_else(|| ".".into());
    let only = std::env::args().nth(3);
    let cases: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for c in cases {
        let name = c["name"].as_str().unwrap();
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let g = |k: &str| c[k].as_f64().unwrap();
        let (len, w, h) = (g("len"), g("w"), g("h"));
        let (y0, y1) = (c["y"][0].as_f64().unwrap(), c["y"][1].as_f64().unwrap());
        let copper = g("copper");
        let board = vec![[0.0, y0], [len, y0], [len, y1], [0.0, y1]];
        let port = |x: f64| {
            vec![
                [x - 0.05, -w / 2.0],
                [x + 0.05, -w / 2.0],
                [x + 0.05, w / 2.0],
                [x - 0.05, w / 2.0],
            ]
        };
        let mut cu = vec![
            (0, Copper::Seg([0.5, 0.0], [len - 0.5, 0.0], w)),
            (1, Copper::Poly(board.clone())),
        ];
        let mut features_x = vec![];
        let mut features_y_extra: Vec<f64> = vec![];
        if let Some(stub) = c["stub"].as_f64() {
            let x = len / 2.0;
            cu.push((
                0,
                Copper::Poly(vec![
                    [x - w / 2.0, 0.0],
                    [x + w / 2.0, 0.0],
                    [x + w / 2.0, stub],
                    [x - w / 2.0, stub],
                ]),
            ));
            features_x.extend([x - w / 2.0, x + w / 2.0]);
            features_y_extra.push(stub);
        }
        let m = PcbModel {
            outline: board,
            sheets: vec![
                Sheet { name: "F.Cu".into(), z: 0.0, thickness: copper },
                Sheet { name: "B.Cu".into(), z: -h, thickness: copper },
            ],
            dielectrics: vec![Dielectric {
                z0: -h,
                z1: 0.0,
                er: g("er"),
                tan: g("tan"),
                pinned: true,
            }],
            copper: cu,
            vias: vec![],
            ports: [0.5, len - 0.5]
                .iter()
                .enumerate()
                .map(|(i, x)| ModelPort {
                    name: format!("P{}", i + 1),
                    at: [*x, 0.0],
                    area: port(*x),
                    sheet: 0,
                    reference: 1,
                    r: 50.0,
                })
                .collect(),
            elements: vec![],
            features_x,
            features_y: [vec![-w / 2.0, w / 2.0], features_y_extra].concat(),
            region: None,
            roughness: Default::default(),
            plating: None,
        };
        let f = &c["f"];
        let t_plan = std::time::Instant::now();
        let p = plan(
            &m,
            f[0].as_f64().unwrap(),
            f[1].as_f64().unwrap(),
            f[2].as_u64().unwrap() as usize,
            g("cell"),
            vec![0],
            200_000,
        )
        .unwrap();
        let t_plan = t_plan.elapsed().as_secs_f64();
        let mut p = p;
        if let Some(db) = std::env::var("END_DB").ok().and_then(|v| v.parse().ok()) {
            p.end_db = db;
        }
        let t = std::time::Instant::now();
        let r = execute(&p, name, 0, &mut |_, _, _| {}).unwrap();
        let secs = t.elapsed().as_secs_f64();
        let nodes = p.sim.grid.dims().iter().product::<usize>();
        let steps: usize = r.steps.iter().sum();
        eprintln!(
            "{name}: {:?} cells {secs:.2}s, plan {t_plan:.2}s, {steps} steps of {:.3e} s, {:.0} MCells/s (nodes x steps / run time)",
            p.sim.grid.dims(),
            p.sim.dt,
            nodes as f64 * steps as f64 / secs / 1e6
        );
        std::fs::write(format!("{out}/{name}.agentee.s2p"), touchstone(&r)).unwrap();
    }
}
