use agentee_core::Project;
use agentee_core::diag::Severity;
use agentee_core::geom::{self, P};
use agentee_core::neck::{NeckOptions, neck};
use agentee_core::route::{RouteOptions, RoutedTrack, route};
use std::path::{Path, PathBuf};

const PARTS: [(&str, [f64; 2]); 4] =
    [("R1", [5.0, 5.0]), ("R2", [12.0, 5.0]), ("R3", [6.62, 9.0]), ("R4", [8.2, 10.0])];

fn dir() -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("agentee-neck-{}-{k}", std::process::id()))
}

fn load(dir: &Path, pcb: &str) -> Project {
    load_on(dir, pcb, "")
}

fn load_on(dir: &Path, pcb: &str, board: &str) -> Project {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    for f in ["symbols/R.sym.toml", "footprints/R_0402_1005Metric.fp.toml"] {
        std::fs::copy(lna.join(f), dir.join(f)).unwrap();
    }
    std::fs::write(
        dir.join("t.board.toml"),
        r#"name = "t"
fab = "jlcpcb"
[outline]
size = [20, 14]
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
[[netclasses]]
name = "Power"
track_width = "0.8mm"
clearance = "0.15mm"
via = "std"
[[netclasses]]
name = "Mid"
track_width = "0.4mm"
clearance = "0.15mm"
neckdown = "2mm"
via = "std"
"#
        .to_string()
            + board,
    )
    .unwrap();
    let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
    let mut layout = String::from("name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n");
    for (i, (r, at)) in PARTS.iter().enumerate() {
        sch += &format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"R\"\nvalue = \"x\"\nfootprint = \"R_0402_1005Metric\"\nat = [{}, 20.32]\n",
            10.16 * (i + 1) as f64
        );
        layout += &format!(
            "\n[[footprints]]\nref = \"{r}\"\nat = [{}, {}]\nlabel = {{ hide = true }}\n",
            at[0], at[1]
        );
    }
    for (n, class, pins) in [
        ("P", "Power", "\"R1.2\", \"R2.1\""),
        ("A", "Default", "\"R1.1\""),
        ("B", "Default", "\"R2.2\""),
        ("M", "Mid", "\"R3.2\""),
        ("C", "Default", "\"R3.1\""),
        ("D", "Default", "\"R4.1\""),
        ("E", "Default", "\"R4.2\""),
    ] {
        sch += &format!("\n[[nets]]\nname = \"{n}\"\nclass = \"{class}\"\npins = [{pins}]\n");
    }
    layout += pcb;
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    std::fs::write(dir.join("t.pcb.toml"), layout).unwrap();
    Project::load(dir).unwrap()
}

fn errors(p: &Project) -> Vec<String> {
    p.layouts[0]
        .diags
        .iter()
        .filter(|d| d.severity == Severity::Error && d.rule.as_deref() != Some("unrouted"))
        .map(|d| d.message.clone())
        .collect()
}

fn rule(p: &Project, id: &str) -> usize {
    p.layouts[0].diags.iter().filter(|d| d.rule.as_deref() == Some(id)).count()
}

fn track(net: &str, layer: &str, width: Option<f64>, points: &[P]) -> String {
    let pts: Vec<String> = points.iter().map(|q| format!("[{}, {}]", q[0], q[1])).collect();
    let width = width.map(|w| format!("width = {w}\n")).unwrap_or_default();
    format!(
        "\n[[tracks]]\nnet = \"{net}\"\nlayer = \"{layer}\"\n{width}points = [{}]\n",
        pts.join(", ")
    )
}

fn written(tracks: &[RoutedTrack]) -> String {
    tracks.iter().map(|t| track(&t.net, &t.layer, t.width, &t.points)).collect()
}

fn length(points: &[P]) -> f64 {
    points.windows(2).map(|w| geom::dist(w[0], w[1])).sum()
}

#[test]
fn the_router_necks_a_wide_class_into_small_pads() {
    let d = dir();
    let p = load(&d, "");
    let (layout, board) = (&p.layouts[0].item, &p.boards[0].item);
    let opts =
        RouteOptions { nets: vec!["P".into()], layers: vec!["F.Cu".into()], ..Default::default() };
    let r = route(layout, board, &opts).unwrap();
    assert_eq!(r.routed, r.connections, "{:?}", r.failed);
    let necks: Vec<&RoutedTrack> = r.tracks.iter().filter(|t| t.width.is_some()).collect();
    assert_eq!(necks.len(), 2, "{:?}", r.tracks);
    for n in &necks {
        let w = n.width.unwrap();
        assert!((0.15..=0.54 + 1e-9).contains(&w), "neck width {w}");
        assert!((w * 100.0 - (w * 100.0).round()).abs() < 1e-6, "neck width {w} is not on 0.01 mm");
        assert!(length(&n.points) <= 0.5 + 1e-9, "neck {:?}", n.points);
    }
    let p = load(&d, &written(&r.tracks));
    assert!(errors(&p).is_empty(), "{:?}", errors(&p));
    assert_eq!(rule(&p, "neckdown"), 2);
    assert_eq!(rule(&p, "class-width"), 0);
}

#[test]
fn neck_rewrites_an_end_that_breaks_clearance_near_its_pad() {
    let d = dir();
    let wide = track("M", "F.Cu", None, &[[7.13, 9.0], [7.13, 11.5]]);
    let p = load(&d, &wide);
    assert!(rule(&p, "clearance") > 0, "{:?}", errors(&p));
    let (layout, board) = (&p.layouts[0].item, &p.boards[0].item);
    let r = neck(layout, board, &NeckOptions::default()).unwrap();
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert_eq!(r.necked.len(), 1, "{:?}", r.necked);
    assert_eq!(r.necked[0].why, "clearance near the pad");
    let e = &r.edits[0];
    assert_eq!(e.necks.len(), 1);
    let n = &e.necks[0];
    assert!(n.width < 0.4 && n.width >= 0.15, "neck width {}", n.width);
    assert!(length(&n.points) <= 2.0 + 1e-9);
    assert_eq!(n.points[0], [7.13, 9.0]);
    assert_eq!(n.points[n.points.len() - 1], e.points[0]);
    let mut pcb = track("M", "F.Cu", None, &e.points);
    pcb += &track("M", "F.Cu", Some(n.width), &n.points);
    let p = load(&d, &pcb);
    assert!(errors(&p).is_empty(), "{:?}", errors(&p));
    assert_eq!(rule(&p, "neckdown"), 1);
    let (layout, board) = (&p.layouts[0].item, &p.boards[0].item);
    let again = neck(layout, board, &NeckOptions::default()).unwrap();
    assert!(again.edits.is_empty() && again.failed.is_empty(), "{:?}", again.necked);
}

#[test]
fn neck_steps_a_taper_down_to_a_narrow_pad() {
    let d = dir();
    let wide = track("P", "F.Cu", None, &[[5.51, 5.0], [5.51, 7.5]]);
    let p = load(&d, &wide);
    let (layout, board) = (&p.layouts[0].item, &p.boards[0].item);
    let plain = neck(layout, board, &NeckOptions::default()).unwrap();
    assert_eq!(plain.necked.len(), 1);
    assert_eq!(plain.necked[0].why, "wider than the pad");
    assert_eq!(plain.edits[0].necks.len(), 1);
    let n = &plain.edits[0].necks[0];
    assert!(n.width <= 0.54 + 1e-9, "neck width {}", n.width);
    assert!(n.points[n.points.len() - 1][1] >= 5.32 - 1e-9, "{:?}", plain.edits);
    let opts = NeckOptions { taper: true, ..Default::default() };
    let tapered = neck(layout, board, &opts).unwrap();
    let e = &tapered.edits[0];
    assert!(e.necks.len() > 1, "{:?}", e.necks);
    assert!(e.necks.windows(2).all(|w| w[0].width < w[1].width && w[1].width < 0.8));
    assert!(e.necks.windows(2).all(|w| w[0].points[w[0].points.len() - 1] == w[1].points[0]));
    let total: f64 = e.necks.iter().map(|n| length(&n.points)).sum();
    assert!(total <= 0.5 + 1e-9, "taper {total} mm");
    let mut pcb = track("P", "F.Cu", None, &e.points);
    for n in &e.necks {
        pcb += &track("P", "F.Cu", Some(n.width), &n.points);
    }
    let p = load(&d, &pcb);
    assert!(errors(&p).is_empty(), "{:?}", errors(&p));
    assert_eq!(rule(&p, "class-width"), 0);
    assert_eq!(rule(&p, "neckdown"), e.necks.len());
}

#[test]
fn neck_leaves_a_track_at_min_width_and_reports_it() {
    let d = dir();
    let thin = track("C", "F.Cu", Some(0.1), &[[6.11, 9.0], [6.11, 11.0]]);
    let beside = track("E", "F.Cu", Some(0.1), &[[6.26, 9.5], [6.26, 11.0]]);
    let p = load(&d, &format!("{thin}{beside}"));
    let (layout, board) = (&p.layouts[0].item, &p.boards[0].item);
    let r = neck(layout, board, &NeckOptions { nets: vec!["C".into()], taper: false }).unwrap();
    assert!(r.edits.is_empty());
    assert_eq!(r.failed.len(), 1, "{:?}", r.failed);
    assert!(r.failed[0].why.contains("min_track_width"), "{}", r.failed[0].why);
}

#[test]
fn neck_keeps_a_track_off_another_nets_via_hole() {
    let d = dir();
    let board = r#"
[[vias]]
name = "thin"
drill = "0.3mm"
diameter = "0.5mm"
[rules]
min_via_hole_to_copper = "0.5mm"
"#;
    let mut pcb = track("M", "F.Cu", None, &[[7.13, 9.0], [7.13, 6.5]]);
    pcb += "\n[[vias]]\nnet = \"C\"\nvia = \"thin\"\nat = [7.88, 8.0]\n";
    let p = load_on(&d, &pcb, board);
    assert_eq!(rule(&p, "clearance"), 0, "{:?}", errors(&p));
    assert!(rule(&p, "hole-to-copper") > 0, "{:?}", errors(&p));
    let (layout, board_item) = (&p.layouts[0].item, &p.boards[0].item);
    let r = neck(layout, board_item, &NeckOptions::default()).unwrap();
    assert_eq!(r.necked.len(), 1, "{:?} {:?}", r.necked, r.failed);
    let e = &r.edits[0];
    let n = &e.necks[0];
    let mut fixed = track("M", "F.Cu", None, &e.points);
    fixed += &track("M", "F.Cu", Some(n.width), &n.points);
    fixed += "\n[[vias]]\nnet = \"C\"\nvia = \"thin\"\nat = [7.88, 8.0]\n";
    let p = load_on(&d, &fixed, board);
    assert_eq!(rule(&p, "hole-to-copper"), 0, "{:?}", errors(&p));
}
