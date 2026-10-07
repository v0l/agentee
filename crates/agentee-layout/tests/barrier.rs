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

[[domains]]
name = "primary"
nets = ["P*"]
[[domains]]
name = "secondary"
nets = ["S*"]
[[barriers]]
between = ["primary", "secondary"]
clearance = "3mm"
creepage = "4mm"
"#;

fn project(
    board: &str,
    parts: &[(&str, [f64; 2])],
    nets: &[(&str, [&str; 2])],
) -> (Project, std::path::PathBuf, String) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-barrier-{}-{k}", std::process::id()));
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
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(dir.join("t.pcb.toml"), &pcb).unwrap();
    (Project::load(&dir).unwrap(), dir, pcb)
}

fn route(
    board: &str,
    parts: &[(&str, [f64; 2])],
    nets: &[(&str, [&str; 2])],
) -> (Vec<String>, f64) {
    let (p, dir, pcb) = project(board, parts, nets);
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
    (errors, length)
}

#[test]
fn a_track_detours_around_a_pad_across_the_barrier() {
    let parts =
        [("U1", [8.0, 8.0]), ("U2", [22.0, 8.0]), ("U3", [15.0, 9.5]), ("U4", [15.0, 18.0])];
    let nets = [("S_LINK", ["U1.1", "U2.1"]), ("P_HOT", ["U3.1", "U4.1"])];
    let (errors, length) = route(BOARD, &parts, &nets);
    assert!(errors.is_empty(), "{errors:?}");
    let open = BOARD.split("\n[[domains]]").next().unwrap();
    let (_, straight) = route(open, &parts, &nets);
    assert!(length > straight + 1.0, "{length} against {straight} with no barrier");
}

#[test]
fn two_routed_nets_keep_the_barrier_between_them() {
    let parts =
        [("U1", [5.0, 10.0]), ("U2", [25.0, 10.0]), ("U3", [12.0, 12.5]), ("U4", [18.0, 12.5])];
    let (errors, _) =
        route(BOARD, &parts, &[("P_LINE", ["U1.1", "U2.1"]), ("S_LINK", ["U3.1", "U4.1"])]);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn agentee_route_keeps_the_barrier_from_a_pad_in_another_domain() {
    let parts =
        [("U1", [8.0, 8.0]), ("U2", [22.0, 8.0]), ("U3", [15.0, 9.5]), ("U4", [15.0, 18.0])];
    let nets = [("S_LINK", ["U1.1", "U2.1"]), ("P_HOT", ["U3.1", "U4.1"])];
    let (p, dir, _) = project(BOARD, &parts, &nets);
    let opts = agentee_core::route::RouteOptions {
        nets: vec!["S_LINK".into()],
        grid: 0.1,
        ..Default::default()
    };
    let r = agentee_core::route::route(&p.layouts[0].item, &p.boards[0].item, &opts).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(r.routed, 1, "{:?}", r.failed);
    let pad = [[14.5, 9.0], [15.5, 9.0], [15.5, 10.0], [14.5, 10.0]];
    let closest = r
        .tracks
        .iter()
        .map(|t| {
            agentee_core::geom::polyline_polygon_distance(&t.points, &pad)
                - t.width.unwrap_or(0.25) / 2.0
        })
        .fold(f64::MAX, f64::min);
    assert!(closest >= 4.0 - 0.05, "the track runs {closest:.2} mm from the other domain's pad");
}

fn placed_gap(board: &str) -> f64 {
    let parts =
        [("U1", [15.0, 10.0]), ("U2", [15.0, 10.0]), ("U3", [15.0, 10.0]), ("U4", [15.0, 10.0])];
    let nets = [("P_A", ["U1.1", "U2.1"]), ("S_B", ["U3.1", "U4.1"])];
    let (p, dir, _) = project(board, &parts, &nets);
    let layout = &p.layouts[0].item;
    let file: agentee_core::layout::LayoutFile =
        agentee_core::project::parse(&std::fs::read_to_string(&p.layouts[0].path).unwrap())
            .unwrap();
    let footprints: std::collections::HashMap<&str, &agentee_core::footprint::Footprint> =
        p.footprints.iter().map(|e| (e.name.as_str(), &e.item)).collect();
    let spec = agentee_core::place::PlaceFile::default();
    let input = agentee_core::place::PlaceInput {
        board: &p.boards[0].item,
        outline: &layout.outline,
        cutouts: &layout.board_cutouts,
        schematic: &p.schematics[0].item,
        footprints: &footprints,
        placements: &file.footprints,
        spec: &spec,
        fast_nets: Vec::new(),
        heat: Vec::new(),
        silk: Vec::new(),
        texts: Vec::new(),
        tracks: &[],
        vias: &[],
    };
    let r = agentee_core::place::place(&input, &Default::default()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    let at = |name: &str| r.placements.iter().find(|q| q.reference == name).unwrap().at;
    let mut gap = f64::MAX;
    for a in ["U1", "U2"] {
        for b in ["U3", "U4"] {
            let (pa, pb) = (at(a), at(b));
            let d = ((pa[0] - pb[0]).abs() - 1.0)
                .max(0.0)
                .hypot(((pa[1] - pb[1]).abs() - 1.0).max(0.0));
            gap = gap.min(d);
        }
    }
    gap
}

#[test]
fn the_placer_keeps_parts_in_other_domains_apart() {
    let open = BOARD.split("\n[[domains]]").next().unwrap();
    let close = placed_gap(open);
    assert!(close < 4.0, "the parts land {close:.2} mm apart with no barrier");
    let gap = placed_gap(BOARD);
    assert!(gap >= 4.0 - 1e-6, "the parts land {gap:.2} mm apart across the barrier");
}
