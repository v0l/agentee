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
track_width = "0.25mm"
clearance = "0.2mm"
via = "std"
layers = ["F.Cu"]

"#;

fn project(
    extra: &str,
    parts: &[(&str, [f64; 2])],
    nets: &[(&str, [&str; 2])],
) -> (Project, std::path::PathBuf, String) {
    let board = BOARD;
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-keepout-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    std::fs::write(dir.join("symbols/S1.sym.toml"), SYMBOL).unwrap();
    std::fs::write(dir.join("footprints/ONE.fp.toml"), FOOTPRINT).unwrap();
    std::fs::write(dir.join("t.board.toml"), board).unwrap();
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    let mut pcb = String::from("name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n");
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
    pcb += extra;
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(dir.join("t.pcb.toml"), &pcb).unwrap();
    (Project::load(&dir).unwrap(), dir, pcb)
}

fn route(extra: &str) -> (Vec<String>, f64, Vec<Vec<[f64; 2]>>) {
    let (p, dir, pcb) = project(extra, &PARTS, &NETS);
    let inputs = p.layout_inputs(0).unwrap();
    let run = agentee_layout::Run {
        from: Some("global".into()),
        to: None,
        only: None,
        watch: None,
        stop: None,
    };
    let r = agentee_layout::run_text(&inputs, &pcb, &run).unwrap();
    let e = inputs.resolve(&r.text).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let errors = e
        .diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{}: {}", d.at, d.message))
        .collect();
    let length = e
        .item
        .tracks
        .iter()
        .flat_map(|t| t.points.windows(2).map(|w| agentee_core::geom::dist(w[0], w[1])))
        .sum();
    let tracks = e.item.tracks.iter().map(|t| t.points.clone()).collect();
    (errors, length, tracks)
}

const PARTS: [(&str, [f64; 2]); 2] = [("U1", [8.0, 10.0]), ("U2", [22.0, 10.0])];
const NETS: [(&str, [&str; 2]); 1] = [("S", ["U1.1", "U2.1"])];
const WALL: [[f64; 2]; 4] = [[14.0, 5.0], [16.0, 5.0], [16.0, 15.0], [14.0, 15.0]];

fn keepout(more: &str) -> String {
    format!(
        "\n[[keepouts]]\npoints = [[14, 5], [16, 5], [16, 15], [14, 15]]\nlayers = [\"F.Cu\"]\n{more}"
    )
}

fn clear_of_wall(tracks: &[Vec<[f64; 2]>]) -> f64 {
    tracks
        .iter()
        .map(|t| agentee_core::geom::polyline_polygon_distance(t, &WALL))
        .fold(f64::MAX, f64::min)
}

#[test]
fn the_layout_engine_routes_around_a_keepout() {
    let (errors, length, tracks) = route(&keepout(""));
    assert!(errors.is_empty(), "{errors:?}");
    assert!(clear_of_wall(&tracks) >= 0.125 - 1e-6, "{tracks:?}");
    let (_, straight, _) = route("");
    assert!(length > straight + 1.0, "{length} against {straight} with no keepout");
    let (errors, allowed, _) = route(&keepout("nets = [\"S\"]\n"));
    assert!(errors.is_empty(), "{errors:?}");
    assert!(allowed < straight + 0.5, "{allowed} against {straight} for a listed net");
}

#[test]
fn agentee_route_keeps_out_of_a_keepout() {
    let (p, dir, _) = project(&keepout(""), &PARTS, &NETS);
    let opts = agentee_core::route::RouteOptions {
        nets: vec!["S".into()],
        grid: 0.1,
        ..Default::default()
    };
    let r = agentee_core::route::route(&p.layouts[0].item, &p.boards[0].item, &opts).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(r.routed, 1, "{:?}", r.failed);
    let tracks: Vec<Vec<[f64; 2]>> = r.tracks.iter().map(|t| t.points.clone()).collect();
    assert!(clear_of_wall(&tracks) >= 0.125 - 1e-6, "{tracks:?}");
}

#[test]
fn check_flags_copper_inside_a_keepout() {
    let track = "\n[[tracks]]\nnet = \"S\"\nlayer = \"F.Cu\"\npoints = [[8, 10], [22, 10]]\n";
    let errors = |extra: &str| {
        let (p, dir, _) = project(extra, &PARTS, &NETS);
        let _ = std::fs::remove_dir_all(&dir);
        p.layouts[0]
            .diags
            .iter()
            .filter(|d| d.rule.as_deref() == Some("keepout"))
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };
    let hit = errors(&format!("{track}{}", keepout("")));
    assert!(hit.len() == 1 && hit[0].contains("enters keepouts[0] on F.Cu"), "{hit:?}");
    assert!(errors(&format!("{track}{}", keepout("nets = [\"S\"]\n"))).is_empty());
    let bottom = keepout("").replace("F.Cu", "B.Cu");
    assert!(errors(&format!("{track}{bottom}")).is_empty());
    let bad = errors(&keepout("nets = [\"NOPE\"]\n"));
    assert!(bad.is_empty());
    let (p, dir, _) = project(&keepout("nets = [\"NOPE\"]\n"), &PARTS, &NETS);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        p.layouts[0].diags.iter().any(|d| d.message.contains("net `NOPE` is not in the schematic"))
    );
}
