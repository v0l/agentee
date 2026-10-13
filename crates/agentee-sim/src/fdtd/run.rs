use super::engine::Sim;
use crate::gpu::gpu;

pub use fieldgrid::fdtd::{Extras, Pulse, Record};

pub fn fused_update() -> bool {
    std::env::var("AGENTEE_FDTD_FUSED").is_ok_and(|v| v != "0")
}

fn profiled() -> bool {
    std::env::var("AGENTEE_FDTD_PROFILE").is_ok_and(|v| v != "0")
}

pub fn run(
    sim: &Sim,
    driven: usize,
    pulse: &Pulse,
    extras: &Extras,
    progress: &mut dyn FnMut(usize, f64),
) -> Result<Record, String> {
    let g = gpu().ok_or("no GPU adapter for the FDTD run")?;
    let extras = Extras { profile: extras.profile || profiled(), ..extras.clone() };
    fieldgrid::fdtd::run_blocking(g, sim, driven, pulse, &extras, &mut |n, db| {
        progress(n, db);
        !crate::runner::cancelled()
    })
    .map_err(|e| if e == "cancelled" { "stopped".into() } else { e })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fdtd::engine::{Edge, Element, Grid, Materials, Media, PortDef};

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
        let media = Media::default();
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
