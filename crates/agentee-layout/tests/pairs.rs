use agentee_core::Severity;
use agentee_core::project::Project;
use agentee_core::route::RouteOptions;

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
size = [0.5, 0.5]
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
track_width = "0.2mm"
clearance = "0.15mm"
via = "std"
voltage = "3.3V"
[[netclasses]]
name = "DIFF"
track_width = "0.2mm"
clearance = "0.15mm"
diff_gap = "0.2mm"
via = "std"
voltage = "3.3V"
layers = ["F.Cu"]
"#;

fn routed(extra: &[(&str, [f64; 2])]) -> agentee_core::layout::Layout {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-pairs-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    std::fs::write(dir.join("symbols/S1.sym.toml"), SYMBOL).unwrap();
    std::fs::write(dir.join("footprints/ONE.fp.toml"), FOOTPRINT).unwrap();
    std::fs::write(dir.join("t.board.toml"), BOARD).unwrap();
    let mut parts =
        vec![("U1", [4.0, 9.4]), ("U2", [4.0, 10.6]), ("U3", [26.0, 5.4]), ("U4", [26.0, 6.6])];
    parts.extend_from_slice(extra);
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    let mut pcb = String::from("name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n");
    for (i, (r, at)) in parts.iter().enumerate() {
        sch += &format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"S1\"\nvalue = \"x\"\nfootprint = \"ONE\"\nat = [{}, 20.32]\n",
            10.16 * (i + 1) as f64
        );
        pcb += &format!("\n[[footprints]]\nref = \"{r}\"\nat = [{}, {}]\n", at[0], at[1]);
    }
    sch += "\n[[nets]]\nname = \"D_P\"\nclass = \"DIFF\"\npins = [\"U1.1\", \"U3.1\"]\n";
    sch += "\n[[nets]]\nname = \"D_N\"\nclass = \"DIFF\"\npins = [\"U2.1\", \"U4.1\"]\n";
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(dir.join("t.pcb.toml"), &pcb).unwrap();
    let p = Project::load(&dir).unwrap();
    let i = p.layouts.iter().position(|l| l.item.name == "t").unwrap();
    let inputs = p.layout_inputs(i).unwrap();
    let layout = &p.layouts[i].item;
    let opts = RouteOptions { nets: vec!["*".into()], ..Default::default() };
    let r = agentee_layout::autoroute::route(layout, &inputs.board, &opts).unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    let text = pcb + "\n" + &agentee_layout::routed_toml(&r);
    let e = inputs.resolve(&text).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let errors: Vec<String> =
        e.diags.iter().filter(|d| d.severity == Severity::Error).map(|d| d.to_string()).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    e.item
}

#[test]
fn a_pair_runs_coupled_at_its_gap() {
    let l = routed(&[]);
    let pair = &l.pairs[0];
    let longest = l.nets[pair.p].length_mm.max(l.nets[pair.n].length_mm);
    assert!(pair.coupled_mm >= 0.8 * longest, "{} of {longest} mm coupled", pair.coupled_mm);
    assert!(pair.skew_mm.abs() < 0.5, "skew {}", pair.skew_mm);
}

#[test]
fn a_pair_goes_around_an_obstacle_together() {
    let l = routed(&[("U5", [15.0, 7.4])]);
    let pair = &l.pairs[0];
    let longest = l.nets[pair.p].length_mm.max(l.nets[pair.n].length_mm);
    assert!(pair.coupled_mm >= 0.6 * longest, "{} of {longest} mm coupled", pair.coupled_mm);
}
