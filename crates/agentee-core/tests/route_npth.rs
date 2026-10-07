use agentee_core::Project;
use agentee_core::route::{RouteOptions, route};
use std::path::{Path, PathBuf};

const HOLE: &str = r#"name = "HoleR"
mount = "smd"

[[pads]]
number = "1"
kind = "smd"
shape = "rect"
at = [0.0, -4.0]
size = [0.5, 0.5]
layers = ["F.Cu", "F.Paste", "F.Mask"]

[[pads]]
number = "2"
kind = "smd"
shape = "rect"
at = [0.0, 4.0]
size = [0.5, 0.5]
layers = ["F.Cu", "F.Paste", "F.Mask"]

[[pads]]
kind = "npth"
shape = "circle"
at = [0.0, 0.0]
size = [2.7, 2.7]
drill = 2.7
layers = ["*.Cu", "*.Mask"]
"#;

fn project(tracks: &str) -> (PathBuf, Project) {
    let dir = std::env::temp_dir().join(format!("agentee-route-npth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    for f in ["symbols/R.sym.toml", "footprints/R_0402_1005Metric.fp.toml"] {
        std::fs::copy(lna.join(f), dir.join(f)).unwrap();
    }
    std::fs::write(dir.join("footprints/HoleR.fp.toml"), HOLE).unwrap();
    std::fs::write(
        dir.join("t.board.toml"),
        "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [20, 14]\n[stackup]\n\
         preset = \"jlcpcb-2l-1.6mm\"\n[[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\n\
         diameter = \"0.6mm\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\n\
         clearance = \"0.1mm\"\nvia = \"std\"\n",
    )
    .unwrap();
    let parts = [
        ("R1", "R_0402_1005Metric", [3.0, 7.0]),
        ("R2", "R_0402_1005Metric", [17.0, 7.0]),
        ("R3", "HoleR", [10.0, 7.0]),
    ];
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    let mut pcb = String::from("name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n");
    for (i, (r, fp, at)) in parts.iter().enumerate() {
        sch += &format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"R\"\nvalue = \"x\"\nfootprint = \"{fp}\"\nat = [{}, 20.32]\n",
            10.16 * (i + 1) as f64
        );
        pcb += &format!(
            "\n[[footprints]]\nref = \"{r}\"\nat = [{}, {}]\nlabel = {{ hide = true }}\n",
            at[0], at[1]
        );
    }
    for (n, pins) in [
        ("A", "\"R1.2\", \"R2.1\""),
        ("B", "\"R1.1\""),
        ("C", "\"R2.2\""),
        ("X", "\"R3.1\""),
        ("Y", "\"R3.2\""),
    ] {
        sch += &format!("\n[[nets]]\nname = \"{n}\"\nclass = \"Default\"\npins = [{pins}]\n");
    }
    pcb += tracks;
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(dir.join("t.pcb.toml"), pcb).unwrap();
    let p = Project::load(&dir).unwrap();
    (dir, p)
}

#[test]
fn routes_keep_the_npth_clearance_not_the_net_clearance() {
    let (dir, p) = project("");
    let (layout, board) = (&p.layouts[0].item, &p.boards[0].item);
    let opts =
        RouteOptions { nets: vec!["A".into()], layers: vec!["F.Cu".into()], ..Default::default() };
    let r = route(layout, board, &opts).unwrap();
    assert_eq!(r.routed, r.connections, "{:?}", r.failed);
    let tracks: String = r
        .tracks
        .iter()
        .map(|t| {
            let pts: Vec<String> =
                t.points.iter().map(|q| format!("[{}, {}]", q[0], q[1])).collect();
            let width = t.width.map(|w| format!("width = {w}\n")).unwrap_or_default();
            format!(
                "\n[[tracks]]\nnet = \"{}\"\nlayer = \"{}\"\n{width}points = [{}]\n",
                t.net,
                t.layer,
                pts.join(", ")
            )
        })
        .collect();
    let (_, routed) = project(&tracks);
    let _ = std::fs::remove_dir_all(&dir);
    let npth: Vec<&str> = routed.layouts[0]
        .diags
        .iter()
        .filter(|d| d.rule.as_deref() == Some("npth-to-copper"))
        .map(|d| d.message.as_str())
        .collect();
    assert!(npth.is_empty(), "{npth:?}");
}
