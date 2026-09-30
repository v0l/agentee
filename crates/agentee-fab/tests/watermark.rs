use agentee_core::Project;
use agentee_core::font;
use std::path::{Path, PathBuf};

fn project(size: [f64; 2], pcb: &str) -> (Project, PathBuf) {
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
            "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [{}, {}]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n[[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n",
            size[0], size[1]
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("t.sch.toml"),
        "name = \"t\"\nboard = \"t\"\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"1k\"\nat = [10.16, 20.32]\n[[nets]]\nname = \"A\"\npins = [\"R1.1\"]\n[[nets]]\nname = \"B\"\npins = [\"R1.2\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("t.pcb.toml"),
        format!(
            "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[footprints]]\nref = \"R1\"\nat = [3, 3]\n{pcb}"
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
