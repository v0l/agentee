use agentee_core::calc;
use agentee_sim::xsection::{Line, Resolution, Stack, Trace, build, solve};

fn microstrip(h: f64, er: f64) -> Stack {
    Stack {
        above: vec![],
        below: vec![(h, er, 0.0)],
        plane_above: false,
        plane_below: true,
        copper: 0.0,
        fill_er: er,
        mask: None,
    }
}

fn run(s: &Stack, w: f64) -> Line {
    let (g, d) =
        build(s, &Trace { width: w, diff_gap: None, coplanar_gap: None }, &Resolution::FINE)
            .unwrap();
    solve(g, d, agentee_sim::gpu::gpu(), 1e-9)
}

#[test]
fn zero_thickness_microstrip_matches_hammerstad_jensen() {
    for (w, h, er) in
        [(0.1, 0.2, 1.0), (0.2, 0.2, 1.0), (0.6, 0.2, 1.0), (0.37, 0.21, 4.4), (2.9, 1.51, 4.5)]
    {
        let r = run(&microstrip(h, er), w);
        let hj = calc::microstrip_z0(w, 0.0, h, er);
        let err = (r.z0 - hj) / hj;
        eprintln!(
            "microstrip w {w} h {h} er {er}: {:.3} vs {hj:.3} ({:+.3}%) eeff {:.4} {}x{}",
            r.z0,
            err * 100.0,
            r.eeff,
            r.nx,
            r.ny
        );
        assert!(err.abs() < 0.005, "{err}");
    }
}
