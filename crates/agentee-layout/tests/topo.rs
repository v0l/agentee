use agentee_core::Severity;
use agentee_core::project::Project;

const SYMBOL: &str = r#"name = "S1"
reference = "U"
[[pins]]
number = "1"
type = "passive"
at = [0.0, 0.0]
side = "left"
length = 1.27
unit = 1
"#;

const FOOTPRINT: &str = r#"name = "ONE"
mount = "smd"
[[pads]]
number = "1"
kind = "smd"
shape = "rect"
at = [0.0, 0.0]
size = [1.0, 1.0]
"#;

const BOARD: &str = r#"name = "t"
fab = "jlcpcb"
[outline]
size = [30, 20]
[stackup]
preset = "jlcpcb-2l-1.6mm"
[[vias]]
name = "std"
drill = "0.3mm"
diameter = "0.6mm"
[[netclasses]]
name = "Default"
track_width = "0.15mm"
clearance = "0.15mm"
via = "std"
"#;

struct Routed {
    errors: Vec<String>,
    unrouted: usize,
    vias: usize,
    stages: Vec<String>,
    notes: Vec<String>,
    topo: Option<(usize, usize)>,
}

fn route(phases: &str) -> Routed {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-topo-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    std::fs::write(dir.join("symbols/S1.sym.toml"), SYMBOL).unwrap();
    std::fs::write(dir.join("footprints/ONE.fp.toml"), FOOTPRINT).unwrap();
    std::fs::write(dir.join("t.board.toml"), BOARD).unwrap();
    let parts = [
        ("U1", [5.0, 10.0]),
        ("U2", [25.0, 10.0]),
        ("U3", [15.0, 3.0]),
        ("U4", [15.0, 17.0]),
        ("U5", [5.0, 4.0]),
        ("U6", [25.0, 16.0]),
    ];
    let nets = [("A", ["U1.1", "U2.1"]), ("B", ["U3.1", "U4.1"]), ("C", ["U5.1", "U6.1"])];
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    let mut pcb = format!(
        "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n\n[engine]\nphases = [{phases}]\n"
    );
    for (i, (r, at)) in parts.iter().enumerate() {
        sch += &format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"S1\"\nvalue = \"x\"\nfootprint = \"ONE\"\nat = [{}, 20.32]\n",
            10.16 * (i + 1) as f64
        );
        pcb += &format!("\n[[footprints]]\nref = \"{r}\"\nat = [{}, {}]\n", at[0], at[1]);
    }
    for (name, [a, b]) in nets {
        sch += &format!("\n[[nets]]\nname = \"{name}\"\npins = [\"{a}\", \"{b}\"]\n");
    }
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(dir.join("t.pcb.toml"), &pcb).unwrap();
    let p = Project::load(&dir).unwrap();
    let inputs = p.layout_inputs(0).unwrap();
    let run = agentee_layout::Run { from: None, to: None, only: None, watch: None, stop: None };
    let r = agentee_layout::run_text(&inputs, &pcb, &run).unwrap();
    let e = inputs.resolve(&r.text).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    Routed {
        errors: e
            .diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| format!("{}: {}", d.at, d.message))
            .collect(),
        unrouted: e.item.nets.iter().map(|n| n.unrouted).sum(),
        vias: e.item.vias.len(),
        stages: r.phases.iter().map(|p| p.phase.clone()).collect(),
        notes: r
            .phases
            .iter()
            .flat_map(|p| p.notes.iter().chain(&p.failed).map(|n| format!("{}: {n}", p.phase)))
            .collect(),
        topo: r.topo.as_ref().map(|t| (t.routed, t.connections)),
    }
}

#[test]
fn topo_routes_crossing_nets_with_vias() {
    let r = route(r#""global", "topo""#);
    assert!(r.stages.iter().any(|s| s == "topo"), "{:?}", r.stages);
    assert_eq!(r.topo, Some((3, 3)), "{:#?}", r.notes);
    assert!(!r.stages.iter().any(|s| s == "detail"), "{:#?}", r.notes);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(r.unrouted, 0);
    assert!(r.vias >= 2, "three crossing nets on two layers took {} vias", r.vias);
}

#[test]
fn topo_guides_detail_when_both_run() {
    let r = route(r#""global", "topo", "detail""#);
    let topo = r.stages.iter().position(|s| s == "topo").expect("topo ran");
    let detail = r.stages.iter().position(|s| s == "detail").expect("detail ran");
    assert!(topo < detail, "{:?}", r.stages);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(r.unrouted, 0);
}

#[test]
fn topo_is_not_a_default_stage() {
    let phases = agentee_core::engine::EngineFile::default().phases();
    assert!(!phases.iter().any(|p| p == "topo"), "{phases:?}");
    let named = agentee_core::engine::EngineFile {
        phases: vec!["detail".into(), "topo".into()],
        ..Default::default()
    };
    assert_eq!(named.phases(), ["topo", "detail"]);
}
