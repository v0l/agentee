use agentee_core::Project;
use std::path::{Path, PathBuf};

fn project(cutouts: &str) -> (Project, PathBuf) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-fab-cut-{}-{k}", std::process::id()));
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
            "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [40, 20]\n{cutouts}\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n[[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n"
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
        "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[footprints]]\nref = \"R1\"\nat = [3, 3]\n",
    )
    .unwrap();
    (Project::load(&dir).unwrap(), dir)
}

fn gerber_xy(p: [f64; 2]) -> String {
    format!("X{}Y{}", (p[0] * 1e6).round() as i64, (-p[1] * 1e6).round() as i64)
}

#[test]
fn board_cutouts_are_routed_along_the_edge_cuts_profile() {
    let (p, dir) =
        project("[[outline.cutouts]]\npoints = [[20, 8], [26, 8], [26, 12], [20, 12]]\n");
    let out = dir.join("fab");
    agentee_fab::package(&p.layouts[0].item, &p.boards[0].item, &p.schematics[0].item, &out)
        .unwrap();
    let edge = std::fs::read_to_string(out.join("Edge_Cuts.gbr")).unwrap();
    assert_eq!(edge.matches("D02*").count(), 2, "{edge}");
    for c in [[20.0, 8.0], [26.0, 8.0], [26.0, 12.0], [20.0, 12.0]] {
        assert!(edge.contains(&gerber_xy(c)), "missing cutout corner {c:?}");
    }
    assert!(edge.contains(&format!("{}D02*", gerber_xy([20.0, 8.0]))));
    assert!(!out.join("drill-NPTH.drl").exists());
    let notes = std::fs::read_to_string(out.join("fab-notes.txt")).unwrap();
    assert!(notes.contains("Internal cutouts 1, routed through the board along Edge_Cuts"));

    let (p, dir) = project("");
    let out = dir.join("fab");
    agentee_fab::package(&p.layouts[0].item, &p.boards[0].item, &p.schematics[0].item, &out)
        .unwrap();
    let edge = std::fs::read_to_string(out.join("Edge_Cuts.gbr")).unwrap();
    assert_eq!(edge.matches("D02*").count(), 1);
    let notes = std::fs::read_to_string(out.join("fab-notes.txt")).unwrap();
    assert!(!notes.contains("Internal cutouts"));
}
