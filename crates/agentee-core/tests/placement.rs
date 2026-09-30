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
    Project::load(&small_dir(drc, pcb)).unwrap()
}

fn small_dir(drc: &str, pcb: &str) -> PathBuf {
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
    dir
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
        cutouts: &layout.board_cutouts,
        schematic: sch,
        footprints: &footprints,
        placements: &file.footprints,
        spec: &spec,
        fast_nets: Vec::new(),
        heat: vec![("U1".into(), 0.45)],
        silk: place::board_silk(&layout.graphics, &layout.artwork),
        texts: place::movable_texts(&layout.graphics),
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
        if let Some((at, rotation)) = pm.label {
            text += &format!("label = {{ at = [{}, {}], rotation = {rotation} }}\n", at[0], at[1]);
        }
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
    assert!(rule(&p, "mlcc-flex-zone").is_empty());
    assert!(rule(&p, "mlcc-flex-zone-case").is_empty());
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

fn courtyards(p: &agentee_core::layout::Placed) -> Vec<Vec<[f64; 2]>> {
    let t = p.transform();
    ["F.CrtYd", "B.CrtYd"]
        .iter()
        .flat_map(|l| place::courtyard_loops(&p.footprint, l))
        .map(|l| l.into_iter().map(|q| t.apply(q)).collect())
        .collect()
}

#[test]
fn placement_keeps_parts_out_of_board_cutouts() {
    let dir = temp_dir("cutout");
    copy_dir(&lna(), &dir, false);
    let board = std::fs::read_to_string(dir.join("lna.board.toml")).unwrap();
    std::fs::write(
        dir.join("lna.board.toml"),
        format!("{board}\n[[outline.cutouts]]\npoints = [[14, 7], [22, 7], [22, 15], [14, 15]]\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join("lna.pcb.toml"),
        "name = \"lna\"\nboard = \"lna\"\nschematic = \"lna\"\n",
    )
    .unwrap();
    let r = run_place(&dir, &PlaceOptions::default());
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert_eq!(r.after.overlaps, 0);
    write_placements(&dir, "", &r);
    let p = Project::load(&dir).unwrap();
    let layout = &p.layouts[0].item;
    assert_eq!(layout.board_cutouts.len(), 1);
    let cutout = &layout.board_cutouts[0];
    let body = p.boards[0].item.rules.min_body_to_edge.to_mm();
    for part in &layout.parts {
        for c in courtyards(part) {
            let gap = agentee_core::geom::polygon_distance(&c, cutout);
            assert!(gap >= body - 1e-6, "{} is {gap:.2} mm from the cutout", part.reference);
        }
    }
}

#[test]
fn fiducials_go_to_corners_clear_of_the_edge_and_parts() {
    let dir = temp_dir("fid");
    copy_dir(&lna(), &dir, false);
    let hackrf = lna().join("../hackrf-pro");
    std::fs::copy(
        hackrf.join("footprints/Fiducial_1mm_Mask2mm.fp.toml"),
        dir.join("footprints/Fiducial_1mm_Mask2mm.fp.toml"),
    )
    .unwrap();
    std::fs::copy(
        hackrf.join("symbols/KiCad_Fiducial_1mm_Mask2mm.sym.toml"),
        dir.join("symbols/KiCad_Fiducial_1mm_Mask2mm.sym.toml"),
    )
    .unwrap();
    let sch = std::fs::read_to_string(dir.join("lna.sch.toml")).unwrap();
    let (head, tail) = sch.split_at(sch.find("\n[[nets]]").unwrap_or(sch.len()));
    let mut fids = String::new();
    for k in 1..=3 {
        fids += &format!(
            "\n[[parts]]\nref = \"FID{k}\"\nsymbol = \"KiCad_Fiducial_1mm_Mask2mm\"\nvalue = \"Fiducial\"\nfootprint = \"Fiducial_1mm_Mask2mm\"\nat = [{}, 80]\n",
            20 * k
        );
    }
    std::fs::write(dir.join("lna.sch.toml"), format!("{head}{fids}{tail}")).unwrap();
    std::fs::write(
        dir.join("lna.pcb.toml"),
        "name = \"lna\"\nboard = \"lna\"\nschematic = \"lna\"\n",
    )
    .unwrap();
    let r = run_place(&dir, &PlaceOptions::default());
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert_eq!(r.after.overlaps, 0);
    write_placements(&dir, "", &r);
    let p = Project::load(&dir).unwrap();
    let layout = &p.layouts[0].item;
    let edge = layout.edge();
    let fids: Vec<_> = layout.parts.iter().filter(|q| q.reference.starts_with("FID")).collect();
    assert_eq!(fids.len(), 3);
    let mut corners = Vec::new();
    for f in &fids {
        let [x, y] = f.at.to_mm();
        for pad in f.pads.iter().flat_map(|q| q.outlines.iter().flatten()) {
            assert!(
                edge.distance(*pad) >= place::FIDUCIAL_TO_EDGE - 1e-6,
                "{} at {x}, {y}",
                f.reference
            );
        }
        assert!(x.min(36.0 - x) < 9.0 && y.min(24.0 - y) < 9.0, "{} at {x}, {y}", f.reference);
        corners.push((x < 18.0, y < 12.0));
        let mine = courtyards(f);
        for other in layout.parts.iter().filter(|q| q.reference != f.reference) {
            for c in courtyards(other) {
                for m in &mine {
                    let gap = agentee_core::geom::polygon_distance(m, &c);
                    assert!(gap > 0.0, "{} touches {}", f.reference, other.reference);
                }
            }
        }
    }
    corners.sort();
    corners.dedup();
    assert_eq!(corners.len(), 3, "{corners:?}");
}

#[test]
fn settled_labels_clear_the_silk_errors_of_a_fresh_placement() {
    let dir = temp_dir("labels");
    copy_dir(&lna(), &dir, false);
    std::fs::write(
        dir.join("lna.pcb.toml"),
        "name = \"lna\"\nboard = \"lna\"\nschematic = \"lna\"\n",
    )
    .unwrap();
    let r = run_place(&dir, &PlaceOptions::default());
    write_placements(&dir, "", &r);
    let p = Project::load(&dir).unwrap();
    let before = rule(&p, "silk-text").len();
    let (moved, failing) = p.layouts[0].item.settle_labels(&p.boards[0].item);
    assert!(before > 0 && !moved.is_empty());
    assert!(failing.is_empty(), "{failing:?}");
    let mut text = std::fs::read_to_string(dir.join("lna.pcb.toml")).unwrap();
    for f in &moved {
        let at = f.at.unwrap();
        let from = format!("ref = \"{}\"\n", f.reference);
        let to =
            format!("{from}label = {{ at = [{}, {}], rotation = {} }}\n", at[0], at[1], f.rotation);
        text = text.replacen(&from, &to, 1);
    }
    std::fs::write(dir.join("lna.pcb.toml"), text).unwrap();
    let p = Project::load(&dir).unwrap();
    let after = rule(&p, "silk-text");
    assert!(after.len() < before, "{after:#?}");
    let labels: Vec<_> = after
        .iter()
        .filter(|(_, m)| r.placements.iter().any(|q| m.starts_with(&format!("`{}`", q.reference))))
        .collect();
    assert!(labels.is_empty(), "{labels:#?}");
}

#[test]
fn hot_parts_come_from_the_thermal_sims_of_the_layout() {
    let pcb = placed([10.0, 10.0], [13.0, 10.0], [0.9, 20.0], 180.0);
    let dir = small_dir("", &pcb);
    let p = Project::load(&dir).unwrap();
    assert!(rule(&p, "placement-hot-parts-close").is_empty());
    std::fs::write(
        dir.join("t-thermal.sim.toml"),
        "name = \"t-thermal\"\nkind = \"thermal\"\nlayout = \"t\"\nambient = 25.0\n\n[[sources]]\nref = \"U1\"\npower = \"0.5W\"\n\n[[sources]]\nref = \"C1\"\npower = \"300mW\"\n",
    )
    .unwrap();
    let p = Project::load(&dir).unwrap();
    let hot = rule(&p, "placement-hot-parts-close");
    assert_eq!(hot.len(), 1, "{hot:?}");
    assert!(hot[0].1.contains("U1") && hot[0].1.contains("C1"), "{hot:?}");
    std::fs::write(
        dir.join("t-thermal.sim.toml"),
        "name = \"t-thermal\"\nkind = \"thermal\"\nlayout = \"other\"\nambient = 25.0\n\n[[sources]]\nref = \"U1\"\npower = \"0.5W\"\n\n[[sources]]\nref = \"C1\"\npower = \"300mW\"\n",
    )
    .unwrap();
    let p = Project::load(&dir).unwrap();
    assert!(rule(&p, "placement-hot-parts-close").is_empty());
}

#[test]
fn chip_length_reads_metric_and_imperial_cases_then_pads() {
    let p = small_project("", &placed([20.0, 15.0], [24.0, 15.0], [3.0, 15.0], 180.0));
    let fp = &p.footprints.iter().find(|f| f.name == "C_0402_1005Metric").unwrap().item;
    let sot = &p.footprints.iter().find(|f| f.name == "SOT-89-3").unwrap().item;
    assert_eq!(place::chip_length("C_0402_1005Metric", fp), Some(1.0));
    assert_eq!(place::chip_length("C_0805", fp), Some(2.0));
    assert_eq!(place::chip_length("R_0402", fp), Some(1.0));
    assert_eq!(place::chip_length("0603", fp), Some(1.6));
    assert_eq!(place::chip_length("C_1206_HandSolder", fp), Some(3.2));
    let pitch = place::chip_length("C_custom", fp).unwrap();
    assert!((pitch - 0.96).abs() < 0.05, "{pitch}");
    assert_eq!(place::chip_length("SOT-89-3", sot), None);
}

fn board_art(text: &str) -> String {
    let mut out = String::new();
    let mut keep = false;
    for line in text.lines() {
        if line.starts_with('[') {
            keep = line == "[[graphics]]" || line == "[[artwork]]";
        }
        if keep {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn parts_stay_off_board_silk_text() {
    let dir = temp_dir("boardsilk");
    copy_dir(&lna(), &dir, false);
    let art = board_art(&std::fs::read_to_string(lna().join("lna.pcb.toml")).unwrap())
        .replace("kind = \"text\"", "kind = \"text\"\nlocked = true");
    assert!(art.contains("RF OUT") && art.contains("locked = true"));
    std::fs::write(
        dir.join("lna.pcb.toml"),
        format!("name = \"lna\"\nboard = \"lna\"\nschematic = \"lna\"\n{art}"),
    )
    .unwrap();
    let r = run_place(&dir, &PlaceOptions::default());
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    write_placements(&dir, &art, &r);
    let p = Project::load(&dir).unwrap();
    let on_text: Vec<String> = p.layouts[0]
        .diags
        .iter()
        .filter(|d| d.rule.as_deref() == Some("silk-text") && d.at.contains("board text"))
        .filter(|d| {
            ["sits on pads", "silk outline", "hides under"].iter().any(|w| d.message.contains(w))
        })
        .map(|d| d.to_string())
        .collect();
    assert!(on_text.is_empty(), "{on_text:#?}");
}

#[test]
fn place_moves_unlocked_board_text_off_parts() {
    let dir = temp_dir("movetext");
    copy_dir(&lna(), &dir, false);
    let text = "[[graphics]]\nkind = \"text\"\nlayer = \"F.SilkS\"\nat = [18, 12]\ntext = \"MIDDLE\"\nsize = 1\n";
    std::fs::write(
        dir.join("lna.pcb.toml"),
        format!("name = \"lna\"\nboard = \"lna\"\nschematic = \"lna\"\n{text}"),
    )
    .unwrap();
    let r = run_place(&dir, &PlaceOptions::default());
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    let [m] = r.texts_moved.as_slice() else { panic!("{:?} {:?}", r.texts_moved, r.texts_stuck) };
    assert_eq!(m.text, "MIDDLE");
    let moved = text.replace("at = [18, 12]", &format!("at = [{}, {}]", m.to[0], m.to[1]));
    write_placements(&dir, &moved, &r);
    let p = Project::load(&dir).unwrap();
    let on_parts: Vec<String> = p.layouts[0]
        .diags
        .iter()
        .filter(|d| d.rule.as_deref() == Some("silk-text") && d.at.contains("MIDDLE"))
        .map(|d| d.to_string())
        .collect();
    assert!(on_parts.is_empty(), "{on_parts:#?}");

    let locked = text.replace("kind = \"text\"", "kind = \"text\"\nlocked = true");
    std::fs::write(
        dir.join("lna.pcb.toml"),
        format!("name = \"lna\"\nboard = \"lna\"\nschematic = \"lna\"\n{locked}"),
    )
    .unwrap();
    let r = run_place(&dir, &PlaceOptions::default());
    assert!(r.texts_moved.is_empty() && r.texts_stuck.is_empty(), "{:?}", r.texts_moved);
}

#[test]
fn roomy_boards_reserve_clear_reference_labels() {
    let dir = temp_dir("labelroom");
    copy_dir(&lna(), &dir, false);
    std::fs::write(
        dir.join("lna.pcb.toml"),
        "name = \"lna\"\nboard = \"lna\"\nschematic = \"lna\"\n",
    )
    .unwrap();
    let r = run_place(&dir, &PlaceOptions { sides: Sides::Both, ..Default::default() });
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert!(r.labels.reserved, "{:?}", r.labels);
    let labelled: Vec<&str> =
        r.placements.iter().filter(|q| q.label.is_some()).map(|q| q.reference.as_str()).collect();
    assert!(labelled.len() >= 10, "{labelled:?}");
    write_placements(&dir, "", &r);
    let p = Project::load(&dir).unwrap();
    let crowded: Vec<String> = rule(&p, "silk-text")
        .into_iter()
        .map(|(_, m)| m)
        .filter(|m| labelled.iter().any(|r| m.starts_with(&format!("`{r}`"))))
        .filter(|m| {
            ["sits on pads", "silk outline", "runs off", "prints over"]
                .iter()
                .any(|w| m.contains(w))
                || labelled.iter().any(|r| m.contains(&format!("crowds `{r}`")))
        })
        .collect();
    assert!(crowded.is_empty(), "{crowded:#?}");
}
