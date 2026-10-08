use agentee_core::Severity;
use agentee_core::geom::dist;
use agentee_core::layout::Layout;
use agentee_core::project::Project;
use std::path::{Path, PathBuf};

fn fresh_lna() -> (PathBuf, Project) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-rf-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    for sub in ["", "symbols", "footprints"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
        for e in std::fs::read_dir(lna.join(sub)).unwrap() {
            let path = e.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let wanted = if sub.is_empty() {
                name == "lna.board.toml" || name == "lna.sch.toml"
            } else {
                name.ends_with(".toml")
            };
            if wanted {
                std::fs::copy(&path, dir.join(sub).join(name)).unwrap();
            }
        }
    }
    let p = Project::load(&dir).unwrap();
    let board = &p.boards[0].item;
    let sch = p.schematics.iter().find(|s| s.name == "lna").unwrap();
    let text = agentee_layout::start::starter("fresh", board, &sch.item);
    std::fs::write(dir.join("fresh.pcb.toml"), text).unwrap();
    let p = Project::load(&dir).unwrap();
    (dir, p)
}

fn placed(dir: &Path, p: &Project) -> (String, Layout) {
    let path = dir.join("fresh.pcb.toml");
    let inputs = p.inputs_for(&path, "lna", "lna").unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let opts = agentee_core::place::PlaceOptions::default();
    let (text, _) = agentee_layout::start::place_text(&inputs, &text, &opts).unwrap();
    let layout = inputs.resolve(&text).unwrap().item;
    (text, layout)
}

fn pad(l: &Layout, r: &str, n: &str) -> [f64; 2] {
    let p = l.parts.iter().find(|p| p.reference == r).unwrap();
    let q = p.pads.iter().find(|q| q.number == n).unwrap();
    let mut b = agentee_core::graphic::Bounds::EMPTY;
    q.outlines.iter().flatten().for_each(|x| b.add(*x));
    b.center()
}

#[test]
fn a_new_layout_starts_with_a_ground_pour_and_rf_stitching() {
    let (_, p) = fresh_lna();
    let text = std::fs::read_to_string(&p.layouts[0].path).unwrap();
    assert!(
        text.contains(
            "[[zones]]\nnet = \"GND\"\nlayers = [\"F.Cu\", \"In1.Cu\", \"In2.Cu\", \"B.Cu\"]"
        ),
        "{text}"
    );
    assert!(
        text.contains("fence = [\"RF_IN\", \"RF_AMP_IN\", \"RF_AMP_OUT\", \"RF_OUT\"]"),
        "{text}"
    );
}

#[test]
fn the_rf_path_is_laid_in_one_line_between_opposite_edges() {
    let (dir, p) = fresh_lna();
    let (_, l) = placed(&dir, &p);
    let line = [
        pad(&l, "J1", "1"),
        pad(&l, "C1", "1"),
        pad(&l, "C1", "2"),
        pad(&l, "U1", "1"),
        pad(&l, "U1", "3"),
        pad(&l, "C2", "1"),
        pad(&l, "C2", "2"),
        pad(&l, "J2", "1"),
    ];
    for q in &line {
        assert!((q[1] - line[0][1]).abs() < 0.01, "{line:?}");
    }
    assert!(line.windows(2).all(|w| w[1][0] > w[0][0]), "in order along the line: {line:?}");
    let mut ob = agentee_core::graphic::Bounds::EMPTY;
    l.outline.iter().for_each(|q| ob.add(*q));
    assert!(
        line[0][0] - ob.min[0] < 4.0 && ob.max[0] - line[7][0] < 4.0,
        "connectors on the short edges"
    );
    for (r, n) in [("D3", "1"), ("L1", "1"), ("L3", "1")] {
        let q = pad(&l, r, n);
        assert!((q[1] - line[0][1]).abs() < 1.5, "{r} hangs off the line: {q:?}");
    }
}

#[test]
fn ground_pads_get_a_via_and_the_router_leaves_the_plane_alone() {
    let (dir, p) = fresh_lna();
    let (text, l) = placed(&dir, &p);
    let board = &p.boards[0].item;
    let t = agentee_core::tie::tie(&l, board, &[]).unwrap();
    assert!(t.tied >= 6, "the RF part and the decaps get a via beside each ground pad: {t:?}");
    let mut with = text.clone();
    for (tr, v) in t.tracks.iter().zip(&t.vias) {
        assert!(dist(tr.points[1], v.at) < 1e-6);
        with += &format!(
            "\n[[tracks]]\nnet = \"{}\"\nlayer = \"{}\"\npoints = [[{}, {}], [{}, {}]]\n\n[[vias]]\nnet = \"{}\"\nat = [{}, {}]\nvia = \"{}\"\n",
            tr.net,
            tr.layer,
            tr.points[0][0],
            tr.points[0][1],
            v.at[0],
            v.at[1],
            v.net,
            v.at[0],
            v.at[1],
            v.via
        );
    }
    let path = dir.join("fresh.pcb.toml");
    let inputs = p.inputs_for(&path, "lna", "lna").unwrap();
    let e = inputs.resolve(&with).unwrap();
    let bad: Vec<_> = e
        .diags
        .iter()
        .filter(|d| d.severity == Severity::Error && d.rule.as_deref() != Some("unrouted"))
        .filter(|d| !d.at.starts_with("silk"))
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
    let opts = agentee_core::route::RouteOptions { nets: vec!["*".into()], ..Default::default() };
    let r = agentee_core::route::route(&e.item, board, &opts).unwrap();
    assert!(r.tracks.iter().all(|t| t.net != "GND"), "a glob never routes a plane net");
}
