use agentee_sim::fdtd::model::{Copper, Dielectric, ModelPort, PcbModel, Sheet};
use agentee_sim::fdtd::{execute, plan, touchstone};
use serde_json::Value;

fn main() {
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
        let mut p = p;
        if let Some(db) = std::env::var("END_DB").ok().and_then(|v| v.parse().ok()) {
            p.end_db = db;
        }
        let t = std::time::Instant::now();
        let r = execute(&p, name, 0, &mut |_, _, _| {}).unwrap();
        eprintln!("{name}: {:?} cells {:.1}s", p.sim.grid.dims(), t.elapsed().as_secs_f64());
        std::fs::write(format!("{out}/{name}.agentee.s2p"), touchstone(&r)).unwrap();
    }
}
