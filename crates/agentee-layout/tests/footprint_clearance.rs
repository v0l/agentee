use agentee_core::project::Project;

const SYMBOL: &str = r#"name = "S3"
reference = "U"
[[pins]]
number = "1"
type = "passive"
at = [0.0, 0.0]
side = "left"
length = 1.27
unit = 1
[[pins]]
number = "2"
type = "passive"
at = [0.0, 2.54]
side = "left"
length = 1.27
unit = 1
[[pins]]
number = "3"
type = "passive"
at = [0.0, 5.08]
side = "left"
length = 1.27
unit = 1
"#;

fn footprint(clearance: &str) -> String {
    let mut s = format!("name = \"TO\"\nmount = \"smd\"\n{clearance}");
    for (i, y) in [-2.54, 0.0, 2.54].iter().enumerate() {
        s += &format!(
            "\n[[pads]]\nnumber = \"{}\"\nkind = \"smd\"\nshape = \"rect\"\nat = [0.0, {y}]\nsize = [2.6, 1.5]\n",
            i + 1
        );
    }
    s
}

const BOARD: &str = r#"name = "t"
fab = "jlcpcb"
[outline]
size = [40, 24]
[stackup]
preset = "jlcpcb-2l-1.6mm"
[[vias]]
name = "std"
drill = "0.3mm"
diameter = "0.6mm"
[[netclasses]]
name = "Default"
track_width = "0.25mm"
voltage = "430VDC"
via = "std"
layers = ["F.Cu"]
[[netclasses]]
name = "Hv"
track_width = "0.5mm"
voltage = "430VDC"
via = "std"
layers = ["F.Cu"]
"#;

fn route(clearance: &str) -> (usize, usize) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-fpclear-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    std::fs::write(dir.join("symbols/S3.sym.toml"), SYMBOL).unwrap();
    std::fs::write(dir.join("footprints/TO.fp.toml"), footprint(clearance)).unwrap();
    std::fs::write(dir.join("t.board.toml"), BOARD).unwrap();
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    for (i, r) in ["Q1", "Q2"].iter().enumerate() {
        sch += &format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"S3\"\nvalue = \"x\"\nfootprint = \"TO\"\nat = [{}, 20.32]\n",
            10.16 * (i + 1) as f64
        );
    }
    sch += "\n[[nets]]\nname = \"SW\"\nclass = \"Hv\"\npins = [\"Q1.2\", \"Q2.2\"]\n";
    sch += "\n[[nets]]\nname = \"G\"\npins = [\"Q1.1\", \"Q2.1\"]\n";
    sch += "\n[[nets]]\nname = \"S\"\npins = [\"Q1.3\", \"Q2.3\"]\n";
    let pcb = "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n\n[[footprints]]\nref = \"Q1\"\nat = [8.0, 12.0]\n\n[[footprints]]\nref = \"Q2\"\nat = [32.0, 12.0]\n";
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(dir.join("t.pcb.toml"), pcb).unwrap();
    let p = Project::load(&dir).unwrap();
    let opts = agentee_core::route::RouteOptions {
        nets: vec!["SW".into()],
        grid: 0.05,
        via_cost: 3.0,
        bend_cost: 0.1,
        ..Default::default()
    };
    let r = agentee_layout::autoroute::route(&p.layouts[0].item, &p.boards[0].item, &opts).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    (r.routed, r.connections)
}

#[test]
fn a_footprint_clearance_lets_the_router_reach_a_middle_pin() {
    assert_eq!(route("clearance = \"1mm\"\n"), (1, 1));
}

#[test]
fn without_one_the_class_clearance_shuts_the_middle_pin_in() {
    assert_eq!(route(""), (0, 1), "the pads are 1 mm apart, under the 2.5 mm of the class");
}
