use agentee_core::Project;
use std::path::{Path, PathBuf};

const VIAS: &str = r#"
[[vias]]
name = "std"
drill = "0.3mm"
diameter = "0.6mm"
[[vias]]
name = "uv"
drill = "0.1mm"
diameter = "0.25mm"
type = "microvia"
from = "F.Cu"
to = "In1.Cu"
fill = "filled_capped"
[[vias]]
name = "bu"
drill = "0.2mm"
diameter = "0.45mm"
from = "In1.Cu"
to = "In4.Cu"
[[vias]]
name = "bd"
drill = "0.3mm"
diameter = "0.6mm"
backdrill = { from = "B.Cu", to = "In2.Cu", max_stub = "0.2mm" }
[[vias]]
name = "ub"
drill = "0.1mm"
diameter = "0.25mm"
type = "microvia"
from = "In4.Cu"
to = "B.Cu"
"#;

fn project(pcb_vias: &str) -> (Project, PathBuf) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-fab-vias-{}-{k}", std::process::id()));
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
            "name = \"t\"\nfab = \"hdi\"\n[outline]\nsize = [40, 20]\n[stackup]\npreset = \"hdi-6l-1n1\"\n{VIAS}\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n"
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
            "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[footprints]]\nref = \"R1\"\nat = [3, 3]\n{pcb_vias}"
        ),
    )
    .unwrap();
    (Project::load(&dir).unwrap(), dir)
}

fn via(kind: &str, at: [f64; 2]) -> String {
    format!("\n[[vias]]\nnet = \"A\"\nat = [{}, {}]\nvia = \"{kind}\"\n", at[0], at[1])
}

fn package(pcb: &str) -> PathBuf {
    let (p, dir) = project(pcb);
    let out = dir.join("fab");
    agentee_fab::package(&p.layouts[0].item, &p.boards[0].item, &p.schematics[0].item, &out)
        .unwrap();
    out
}

fn drill_files(out: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(out)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".drl"))
        .collect();
    v.sort();
    v
}

#[test]
fn only_through_vias_keep_the_one_plated_drill_file() {
    let out = package(&via("std", [10.0, 10.0]));
    assert_eq!(drill_files(&out), ["drill-PTH.drl"]);
    let notes = std::fs::read_to_string(out.join("fab-notes.txt")).unwrap();
    assert!(!notes.contains("Via types"), "{notes}");
    let pth = std::fs::read_to_string(out.join("drill-PTH.drl")).unwrap();
    assert!(pth.contains("TF.FileFunction,Plated,1,6,PTH") && pth.contains("T1C0.300"));
}

#[test]
fn each_span_gets_its_own_drill_file_and_fab_note() {
    let pcb = [
        via("std", [10.0, 10.0]),
        via("uv", [12.0, 10.0]),
        via("uv", [13.0, 10.0]),
        via("bu", [14.0, 10.0]),
        via("bd", [16.0, 10.0]),
    ]
    .concat();
    let out = package(&pcb);
    assert_eq!(
        drill_files(&out),
        [
            "drill-F.Cu-In1.Cu.drl",
            "drill-In1.Cu-In4.Cu.drl",
            "drill-PTH.drl",
            "drill-backdrill-B.Cu-In2.Cu.drl"
        ]
    );
    let uv = std::fs::read_to_string(out.join("drill-F.Cu-In1.Cu.drl")).unwrap();
    assert!(uv.contains("TF.FileFunction,Plated,1,2,Blind"), "{uv}");
    assert!(uv.contains("; span F.Cu to In1.Cu, microvia vias"), "{uv}");
    assert_eq!(uv.matches("\nX").count(), 2);
    let bu = std::fs::read_to_string(out.join("drill-In1.Cu-In4.Cu.drl")).unwrap();
    assert!(bu.contains("TF.FileFunction,Plated,2,5,Buried"), "{bu}");
    let pth = std::fs::read_to_string(out.join("drill-PTH.drl")).unwrap();
    assert_eq!(pth.matches("\nX").count(), 2, "{pth}");
    let bd = std::fs::read_to_string(out.join("drill-backdrill-B.Cu-In2.Cu.drl")).unwrap();
    assert!(bd.contains("TF.FileFunction,NonPlated,3,6,Blind") && bd.contains("T1C0.500"), "{bd}");
    let notes = std::fs::read_to_string(out.join("fab-notes.txt")).unwrap();
    assert!(
        notes.contains("uv: microvia F.Cu to In1.Cu, laser drilled, 0.100 mm drill, 0.250 mm pad, 2 holes, filled and capped (type VII), drill-F.Cu-In1.Cu.drl"),
        "{notes}"
    );
    assert!(notes.contains("bu: buried In1.Cu to In4.Cu, mechanically drilled"), "{notes}");
    assert!(notes.contains("backdrill from B.Cu with a 0.500 mm drill, keep In2.Cu connected, leave at most 0.200 mm of stub"), "{notes}");
    let d356 = std::fs::read_to_string(out.join("t.d356")).unwrap();
    let access: Vec<&str> = d356
        .lines()
        .filter(|l| l.contains("VIA"))
        .map(|l| &l[l.find("PA").unwrap() + 1..][..3])
        .collect();
    assert_eq!(access, ["A00", "A01", "A01", "A02", "A01"], "{d356}");
}

#[test]
fn netlist_access_codes_name_the_layer_count_for_the_bottom() {
    let pcb = ["side = \"bottom\"\n".to_string(), via("ub", [10.0, 10.0]), via("bu", [12.0, 10.0])]
        .concat();
    let out = package(&pcb);
    let d356 = std::fs::read_to_string(out.join("t.d356")).unwrap();
    let code = |l: &str| l[l.find("  A").unwrap() + 2..][..3].to_string();
    let pads: Vec<String> = d356.lines().filter(|l| l.contains("R1    -")).map(code).collect();
    assert_eq!(pads, ["A06", "A06"], "{d356}");
    let vias: Vec<String> = d356
        .lines()
        .filter(|l| l.contains("VIA"))
        .map(|l| l[l.find("PA").unwrap() + 1..][..3].to_string())
        .collect();
    assert_eq!(vias, ["A06", "A02"], "{d356}");
}
