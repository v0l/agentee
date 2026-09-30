use agentee_core::Project;
use agentee_core::diag::Severity;
use agentee_core::layout::LayoutFile;
use agentee_core::place::{self, PlaceInput, PlaceOptions, Sides};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn temp_dir(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-place-{tag}-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    dir
}

fn lna() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna")
}

fn small_project(drc: &str, pcb: &str) -> Project {
    let dir = temp_dir("drc");
    for s in ["C", "SPF5189Z", "Conn_Coaxial"] {
        let f = format!("symbols/{s}.sym.toml");
        std::fs::copy(lna().join(&f), dir.join(&f)).unwrap();
    }
    for s in ["C_0402_1005Metric", "SOT-89-3", "SMA_Amphenol_132289_EdgeMount"] {
        let f = format!("footprints/{s}.fp.toml");
        std::fs::copy(lna().join(&f), dir.join(&f)).unwrap();
    }
    std::fs::write(
        dir.join("t.board.toml"),
        format!(
            r#"name = "t"
fab = "jlcpcb"
[outline]
size = [40, 30]
[stackup]
preset = "jlcpcb-4l-1.6mm-7628"
[[vias]]
name = "std"
drill = "0.3mm"
diameter = "0.6mm"
[[netclasses]]
name = "Default"
track_width = "0.2mm"
clearance = "0.15mm"
via = "std"
{drc}
"#
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("t.sch.toml"),
        r#"name = "t"
board = "t"

[[parts]]
ref = "U1"
symbol = "SPF5189Z"
value = "SPF5189Z"
at = [20.32, 20.32]

[[parts]]
ref = "C1"
symbol = "C"
value = "100n"
at = [30.48, 20.32]

[[parts]]
ref = "J1"
symbol = "Conn_Coaxial"
value = "IN"
at = [10.16, 20.32]

[[nets]]
name = "VCC"
pins = ["U1.3", "C1.1"]

[[nets]]
name = "GND"
pins = ["U1.2", "C1.2", "J1.2"]

[[nets]]
name = "IN"
pins = ["U1.1", "J1.1"]
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("t.pcb.toml"),
        format!("name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n{pcb}"),
    )
    .unwrap();
    Project::load(&dir).unwrap()
}

fn placed(u1: [f64; 2], c1: [f64; 2], j1: [f64; 2], j1_rot: f64) -> String {
    format!(
        "[[footprints]]\nref = \"U1\"\nat = [{}, {}]\n\n[[footprints]]\nref = \"C1\"\nat = [{}, {}]\n\n[[footprints]]\nref = \"J1\"\nat = [{}, {}]\nrotation = {j1_rot}\n",
        u1[0], u1[1], c1[0], c1[1], j1[0], j1[1]
    )
}

fn rule(p: &Project, id: &str) -> Vec<(Severity, String)> {
    p.layouts[0]
        .diags
        .iter()
        .filter(|d| d.rule.as_deref() == Some(id))
        .map(|d| (d.severity, d.message.clone()))
        .collect()
}

#[test]
fn far_decap_is_an_info_notice() {
    let p = small_project("", &placed([15.0, 10.0], [26.0, 16.0], [2.54, 10.0], 180.0));
    let found = rule(&p, "placement-decoupling-distance");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, Severity::Info);
    assert!(found[0].1.contains("C1 decouples VCC"), "{}", found[0].1);
}

#[test]
fn decoupling_threshold_comes_from_drc_placement() {
    let p = small_project(
        "[drc.placement]\ndecoupling_distance = \"20mm\"\n",
        &placed([15.0, 10.0], [26.0, 16.0], [2.54, 10.0], 180.0),
    );
    assert!(rule(&p, "placement-decoupling-distance").is_empty());
}

#[test]
fn placement_severity_can_be_raised() {
    let p = small_project(
        "[drc.severity]\n\"placement-decoupling-distance\" = \"warning\"\n",
        &placed([15.0, 10.0], [26.0, 16.0], [2.54, 10.0], 180.0),
    );
    let found = rule(&p, "placement-decoupling-distance");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, Severity::Warning);
}

#[test]
fn connector_off_the_edge_is_a_notice() {
    let inside = small_project("", &placed([30.0, 20.0], [32.0, 24.0], [10.0, 15.0], 0.0));
    let found = rule(&inside, "placement-connector-not-at-edge");
    assert_eq!(found.len(), 1, "{found:?}");
    let edge = small_project("", &placed([20.0, 10.0], [22.0, 14.0], [2.54, 10.0], 180.0));
    assert!(rule(&edge, "placement-connector-not-at-edge").is_empty());
}

fn copy_dir(from: &Path, to: &Path, all: bool) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        if e.file_type().unwrap().is_dir() {
            if name == "symbols" || name == "footprints" {
                copy_dir(&e.path(), &to.join(&name), true);
            }
        } else if all || name.ends_with(".board.toml") || name.ends_with(".sch.toml") {
            std::fs::copy(e.path(), to.join(&name)).unwrap();
        }
    }
}

fn run_place(dir: &Path, opts: &PlaceOptions) -> place::PlaceResult {
    let p = Project::load(dir).unwrap();
    let entry = &p.layouts[0];
    let layout = &entry.item;
    let board = &p.boards[0].item;
    let sch = &p.schematics.iter().find(|s| s.name == layout.schematic).unwrap().item;
    let text = std::fs::read_to_string(&entry.path).unwrap();
    let file: LayoutFile = agentee_core::project::parse(&text).unwrap();
    let footprints: HashMap<&str, &agentee_core::footprint::Footprint> =
        p.footprints.iter().map(|e| (e.name.as_str(), &e.item)).collect();
    let spec = file.place.clone().unwrap_or_default();
    let input = PlaceInput {
        board,
        outline: &layout.outline,
        schematic: sch,
        footprints: &footprints,
        placements: &file.footprints,
        spec: &spec,
        fast_nets: Vec::new(),
        heat: vec![("U1".into(), 0.45)],
    };
    place::place(&input, opts).unwrap()
}

fn write_placements(dir: &Path, extra: &str, r: &place::PlaceResult) {
    let mut text = format!("name = \"lna\"\nboard = \"lna\"\nschematic = \"lna\"\n{extra}");
    for pm in &r.placements {
        text += &format!(
            "\n[[footprints]]\nref = \"{}\"\nat = [{}, {}]\nrotation = {}\n{}",
            pm.reference,
            pm.at[0],
            pm.at[1],
            pm.rotation,
            if pm.bottom { "side = \"bottom\"\n" } else { "" }
        );
    }
    std::fs::write(dir.join("lna.pcb.toml"), text).unwrap();
}

#[test]
fn place_lna_is_legal_and_deterministic() {
    let dir = temp_dir("lna");
    copy_dir(&lna(), &dir, false);
    std::fs::write(
        dir.join("lna.pcb.toml"),
        "name = \"lna\"\nboard = \"lna\"\nschematic = \"lna\"\n",
    )
    .unwrap();
    let opts = PlaceOptions { seed: 7, ..Default::default() };
    let r = run_place(&dir, &opts);
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert_eq!(r.after.overlaps, 0);
    let again = run_place(&dir, &opts);
    let key = |r: &place::PlaceResult| format!("{:?}", r.placements);
    assert_eq!(key(&r), key(&again));
    assert_ne!(r.edges.get("J1"), None);
    for pm in &r.placements {
        for v in pm.at {
            assert!(((v / 0.05).round() * 0.05 - v).abs() < 1e-6 || pm.reference.starts_with('J'));
        }
    }
    write_placements(&dir, "", &r);
    let p = Project::load(&dir).unwrap();
    let bad: Vec<String> = p.layouts[0]
        .diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .filter(|d| {
            !matches!(d.rule.as_deref(), Some("unrouted") | Some("silk-text") | Some("watermark"))
        })
        .map(|d| d.to_string())
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
    assert!(rule(&p, "placement-connector-not-at-edge").is_empty());
    let holes: Vec<_> =
        p.layouts[0].item.parts.iter().filter(|q| q.reference.starts_with('H')).collect();
    for h in holes {
        let [x, y] = h.at.to_mm();
        assert!(x.min(36.0 - x) < 6.0 && y.min(24.0 - y) < 6.0, "{} at {x}, {y}", h.reference);
    }
}

#[test]
fn locked_parts_and_pinned_edges_are_kept() {
    let dir = temp_dir("lock");
    copy_dir(&lna(), &dir, false);
    std::fs::write(
        dir.join("lna.pcb.toml"),
        "name = \"lna\"\nboard = \"lna\"\nschematic = \"lna\"\n\n[place]\nedges = { J1 = \"left\", J2 = \"right\" }\n\n[[footprints]]\nref = \"U1\"\nat = [18.0, 12.0]\nrotation = 90\nlocked = true\n",
    )
    .unwrap();
    let opts = PlaceOptions { sides: Sides::Top, ..Default::default() };
    let r = run_place(&dir, &opts);
    assert!(r.kept.contains(&"U1".to_string()));
    assert!(r.placements.iter().all(|p| p.reference != "U1"));
    assert_eq!(r.edges.get("J1"), Some(&place::Edge::Left));
    assert_eq!(r.edges.get("J2"), Some(&place::Edge::Right));
    let j1 = r.placements.iter().find(|p| p.reference == "J1").unwrap();
    assert!(j1.at[0] < 5.0, "{:?}", j1.at);
}
