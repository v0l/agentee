use agentee_core::Project;
use agentee_core::font;
use std::path::{Path, PathBuf};

const PRESET: &str = "preset = \"jlcpcb-2l-1.6mm\"\n";

fn project(size: [f64; 2], pcb: &str) -> (Project, PathBuf) {
    project_with(size, PRESET, pcb)
}

fn project_with(size: [f64; 2], stackup: &str, pcb: &str) -> (Project, PathBuf) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-fab-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    for f in ["symbols/R.sym.toml", "footprints/R_0402_1005Metric.fp.toml"] {
        std::fs::copy(lna.join(f), dir.join(f)).unwrap();
    }
    std::fs::write(
        dir.join("t.board.toml"),
        format!(
            "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [{}, {}]\n[stackup]\n{stackup}[[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n",
            size[0], size[1]
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("t.sch.toml"),
        "name = \"t\"\nboard = \"t\"\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"1k\"\nat = [10.16, 20.32]\n[[nets]]\nname = \"A\"\npins = [\"R1.1\"]\n[[nets]]\nname = \"B\"\npins = [\"R1.2\"]\n",
    )
    .unwrap();
    let (top, tables) = if pcb.starts_with('[') { ("", pcb) } else { (pcb, "") };
    std::fs::write(
        dir.join("t.pcb.toml"),
        format!(
            "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n{top}[[footprints]]\nref = \"R1\"\nat = [3, 3]\n{tables}"
        ),
    )
    .unwrap();
    (Project::load(&dir).unwrap(), dir)
}

fn package(p: &Project, dir: &Path) -> Result<agentee_fab::Report, String> {
    agentee_fab::package(&p.layouts[0].item, &p.boards[0].item, &p.schematics[0].item, dir)
}

fn gerber_xy(p: [f64; 2]) -> String {
    format!("X{}Y{}", (p[0] * 1e6).round() as i64, (-p[1] * 1e6).round() as i64)
}

#[test]
fn the_watermark_is_stroked_into_the_bottom_silk_and_named_in_the_notes() {
    let (p, dir) = project([40.0, 20.0], "");
    let w = p.layouts[0].item.watermark.clone().expect("a clear spot");
    assert_eq!(w.layer, "B.SilkS");
    let drawn = p.layouts[0].item.board_texts();
    assert!(drawn.iter().any(|t| t.owner == "watermark" && t.text == w.text), "not drawn");
    assert_eq!(w.text, agentee_core::version::watermark());
    assert!(w.text.starts_with(&format!("agentee v{}-", env!("CARGO_PKG_VERSION"))));
    let out = dir.join("fab");
    package(&p, &out).unwrap();
    let silk = std::fs::read_to_string(out.join("B_SilkS.gbr")).unwrap();
    let strokes = font::strokes(&w.text, w.at, w.size, w.rotation, w.anchor, true);
    assert!(strokes.len() > 20);
    for st in &strokes {
        assert!(silk.contains(&format!("{}D02*", gerber_xy(st[0]))), "missing stroke {st:?}");
    }
    let notes = std::fs::read_to_string(out.join("fab-notes.txt")).unwrap();
    assert!(notes.contains(&w.text));
    assert!(notes.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn routing_does_not_steer_around_the_watermark() {
    let (p, _) = project([40.0, 20.0], "");
    let l = &p.layouts[0].item;
    let w = l.watermark.clone().expect("a clear spot");
    assert!(!l.silk.iter().any(|b| b.text == w.text), "the build id would move routes");
}

#[test]
fn the_watermark_follows_the_layout_field() {
    let (p, dir) =
        project([40.0, 20.0], "[watermark]\nat = [20, 15]\nlayer = \"F.SilkS\"\nrotation = 0\n");
    let w = p.layouts[0].item.watermark.clone().unwrap();
    assert_eq!((w.layer.as_str(), w.at), ("F.SilkS", [20.0, 15.0]));
    let out = dir.join("fab");
    package(&p, &out).unwrap();
    let silk = std::fs::read_to_string(out.join("F_SilkS.gbr")).unwrap();
    let first = &font::strokes(&w.text, w.at, w.size, 0.0, w.anchor, false)[0];
    assert!(silk.contains(&format!("{}D02*", gerber_xy(first[0]))));
}

#[test]
fn a_board_too_small_for_the_watermark_fails_fab_with_where_to_put_it() {
    let (p, dir) = project([8.0, 6.0], "");
    assert!(p.layouts[0].item.watermark.is_none());
    let e = package(&p, &dir.join("fab")).err().unwrap();
    assert!(e.contains("no clear spot") && e.contains("[watermark] at = [x, y]"), "{e}");
    assert!(p.layouts[0].diags.iter().any(|d| d.rule.as_deref() == Some("watermark")));
}

#[test]
fn a_watermark_spot_on_a_pad_is_flagged() {
    let (p, _) = project([40.0, 20.0], "[watermark]\nat = [3, 3]\nlayer = \"F.SilkS\"\n");
    let d: Vec<_> =
        p.layouts[0].diags.iter().filter(|d| d.rule.as_deref() == Some("watermark")).collect();
    assert!(d.iter().any(|d| d.message.contains("sits on pads R1.")), "{d:?}");
}

#[test]
fn a_stackup_with_no_silk_fails_fab_naming_the_layer_to_add() {
    let layer = |kind: &str, t: &str| format!("[[stackup.layers]]\nkind = \"{kind}\"\n{t}");
    let stackup = [
        layer("mask", "thickness = \"15um\"\n"),
        layer("copper", "thickness = \"1oz\"\n"),
        layer("core", "thickness = \"1.5mm\"\ner = 4.5\n"),
        layer("copper", "thickness = \"1oz\"\n"),
        layer("mask", "thickness = \"15um\"\n"),
    ]
    .concat();
    let (p, dir) = project_with([40.0, 20.0], &stackup, "");
    assert!(p.layouts[0].item.watermark.is_none());
    let e = package(&p, &dir.join("fab")).err().unwrap();
    assert!(e.contains("no silk layer") && e.contains("kind = \"silk\""), "{e}");
}

fn overlap(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    agentee_core::geom::polygon_distance(a, b) <= 0.0
}

#[test]
fn a_one_line_title_lands_on_the_front_silk_beside_the_watermark() {
    let (p, dir) = project([40.0, 20.0], "title = \"Buggy Guard v1.0\"\n");
    let l = &p.layouts[0].item;
    let t = l.title.clone().expect("a clear spot for the title");
    let w = l.watermark.clone().expect("a clear spot for the watermark");
    assert_eq!((t.text.as_str(), t.layer.as_str(), t.size), ("Buggy Guard v1.0", "F.SilkS", 1.5));
    assert!(t.layer != w.layer || !overlap(&t.outline(), &w.outline()), "title on the watermark");
    assert!(l.board_texts().iter().any(|x| x.owner == "title" && x.text == t.text));
    assert!(!p.layouts[0].diags.iter().any(|d| d.rule.as_deref() == Some("board-title")));
    let out = dir.join("fab");
    package(&p, &out).unwrap();
    let silk = std::fs::read_to_string(out.join("F_SilkS.gbr")).unwrap();
    let first = &font::strokes(&t.text, t.at, t.size, t.rotation, t.anchor, false)[0];
    assert!(silk.contains(&format!("{}D02*", gerber_xy(first[0]))), "title not plotted");
}

#[test]
fn a_title_table_places_it_where_it_says() {
    let (p, _) = project(
        [40.0, 20.0],
        "title = { text = \"T rev B\", at = [25, 4], layer = \"B.SilkS\", size = \"2mm\" }\n",
    );
    let t = p.layouts[0].item.title.clone().unwrap();
    assert_eq!((t.at, t.layer.as_str(), t.size), ([25.0, 4.0], "B.SilkS", 2.0));
}

#[test]
fn a_title_with_no_room_is_a_board_title_error_with_a_spot_to_paste() {
    let (p, _) = project([8.0, 6.0], "title = \"A very long board name v1.0\"\n");
    assert!(p.layouts[0].item.title.is_none());
    let d: Vec<_> =
        p.layouts[0].diags.iter().filter(|d| d.rule.as_deref() == Some("board-title")).collect();
    assert!(
        d.iter()
            .any(|d| d.message.contains("no clear spot") && d.message.contains("title = { text =")),
        "{d:?}"
    );
}

#[test]
fn a_title_table_with_a_typo_does_not_load() {
    let (p, _) = project([40.0, 20.0], "title = { text = \"T\", sise = 2 }\n");
    assert!(p.layouts.is_empty());
    assert!(p.failures.iter().any(|f| f.message.contains("sise")), "{:?}", p.failures);
}

#[test]
fn the_title_rule_only_applies_to_a_layout_with_a_title() {
    let applies = |pcb: &str| {
        let (p, _) = project([40.0, 20.0], pcb);
        let l = &p.layouts[0].item;
        let b = &p.boards[0].item;
        let setup = agentee_core::drc::Setup::of(&agentee_core::drc::Ctx::of_layout(b, l));
        agentee_core::drc::status(b, &setup)
            .into_iter()
            .find(|r| r.id == "board-title")
            .unwrap()
            .applies
    };
    assert!(!applies(""));
    assert!(applies("title = \"T\"\n"));
}
