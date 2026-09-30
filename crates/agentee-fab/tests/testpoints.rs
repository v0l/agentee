use agentee_core::Project;
use agentee_core::testpoint;
use std::path::{Path, PathBuf};

fn project(pcb: &str) -> (Project, PathBuf) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-fab-tp-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    for f in ["symbols/R.sym.toml", "footprints/R_0402_1005Metric.fp.toml"] {
        std::fs::copy(lna.join(f), dir.join(f)).unwrap();
    }
    std::fs::write(dir.join("symbols/TestPoint.sym.toml"), testpoint::SYMBOL_TOML).unwrap();
    std::fs::write(
        dir.join("footprints/TestPoint_Pad_D1.0mm.fp.toml"),
        testpoint::PAD_FOOTPRINT_TOML,
    )
    .unwrap();
    std::fs::write(
        dir.join("t.board.toml"),
        "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [40, 30]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n[[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("t.sch.toml"),
        "name = \"t\"\nboard = \"t\"\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"1k\"\nat = [10.16, 20.32]\n[[parts]]\nref = \"TP1\"\nsymbol = \"TestPoint\"\nvalue = \"TP\"\nat = [20.32, 20.32]\n[[nets]]\nname = \"3V3\"\npins = [\"R1.1\", \"TP1.1\"]\n[[nets]]\nname = \"B\"\npins = [\"R1.2\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("t.pcb.toml"),
        format!(
            "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[footprints]]\nref = \"R1\"\nat = [20, 15]\n[[footprints]]\nref = \"TP1\"\nat = [12, 10]\nside = \"bottom\"\n[[tracks]]\nnet = \"3V3\"\nlayer = \"B.Cu\"\npoints = [[12, 10], [19.49, 10]]\n[[vias]]\nnet = \"3V3\"\nat = [19.49, 10]\n[[tracks]]\nnet = \"3V3\"\nlayer = \"F.Cu\"\npoints = [[19.49, 10], [19.49, 15]]\n{pcb}"
        ),
    )
    .unwrap();
    (Project::load(&dir).unwrap(), dir)
}

fn package(p: &Project, dir: &Path) {
    agentee_fab::package(&p.layouts[0].item, &p.boards[0].item, &p.schematics[0].item, dir)
        .unwrap();
}

#[test]
fn testpoints_csv_lists_every_test_point_for_the_fixture() {
    let (p, dir) = project("");
    let out = dir.join("fab");
    package(&p, &out);
    let csv = std::fs::read_to_string(out.join("testpoints.csv")).unwrap();
    assert_eq!(
        csv,
        "Ref,Pad,Net,X,Y,Side,Pad Diameter\nTP1,1,3V3,12.0000mm,-10.0000mm,Bottom,1.0000mm\n"
    );
    let notes = std::fs::read_to_string(out.join("fab-notes.txt")).unwrap();
    assert!(notes.contains("1 test points, probed from the bottom, listed in testpoints.csv."));
}

#[test]
fn the_netlist_marks_test_points_as_probe_access() {
    let (p, dir) = project("");
    let out = dir.join("fab");
    package(&p, &out);
    let d356 = std::fs::read_to_string(out.join("t.d356")).unwrap();
    let tp = d356.lines().find(|l| l.contains("TP1   -1")).unwrap();
    assert!(tp.starts_with("3273V3") && tp.contains("A02X") && tp.ends_with("S1"), "{tp}");
    let via = d356.lines().find(|l| l.contains("VIA")).unwrap();
    assert!(via.contains("MD") && via.ends_with("S3"), "{via}");
}

#[test]
fn untented_vias_count_as_access_and_open_the_probe_side_mask() {
    let (p, dir) = project("[test]\nvias = true\n");
    let out = dir.join("fab");
    package(&p, &out);
    let d356 = std::fs::read_to_string(out.join("t.d356")).unwrap();
    let via = d356.lines().find(|l| l.contains("VIA")).unwrap();
    assert!(via.contains(" D") && via.contains("PA02X") && via.ends_with("S1"), "{via}");
    let mask = std::fs::read_to_string(out.join("B_Mask.gbr")).unwrap();
    assert!(mask.contains("X19490000Y-10000000D03*"), "{mask}");
    let top = std::fs::read_to_string(out.join("F_Mask.gbr")).unwrap();
    assert!(!top.contains("D03*"));
}
