use agentee_core::Project;
use std::path::{Path, PathBuf};

fn default_class_warnings(p: &Project) -> usize {
    p.schematics
        .iter()
        .flat_map(|s| &s.diags)
        .filter(|d| d.message.contains("Default netclass"))
        .count()
}

fn temp_project(name: &str, with_layout: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agentee-default-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    std::fs::copy(lna.join("symbols/R.sym.toml"), dir.join("symbols/R.sym.toml")).unwrap();
    let fp = "footprints/R_0402_1005Metric.fp.toml";
    std::fs::copy(lna.join(fp), dir.join(fp)).unwrap();
    std::fs::write(
        dir.join("t.board.toml"),
        "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [30, 20]\n[stackup]\n\
         preset = \"jlcpcb-2l-1.6mm\"\n[[netclasses]]\nname = \"Default\"\n\
         track_width = \"0.2mm\"\nclearance = \"0.15mm\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("t.sch.toml"),
        "name = \"t\"\nboard = \"t\"\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nvalue = \"1k\"\n\
         at = [10.16, 20.32]\n[[nets]]\nname = \"A\"\npins = [\"R1.1\", \"R1.2\"]\n",
    )
    .unwrap();
    if with_layout {
        std::fs::write(
            dir.join("t.pcb.toml"),
            "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n[[footprints]]\nref = \"R1\"\n\
             at = [10, 10]\n",
        )
        .unwrap();
    }
    dir
}

fn load_once(name: &str, with_layout: bool) -> Project {
    let dir = temp_project(name, with_layout);
    let p = Project::load(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    p
}

#[test]
fn default_class_warning_needs_a_layout_of_the_schematic() {
    assert_eq!(default_class_warnings(&load_once("laid", true)), 1);
    assert_eq!(default_class_warnings(&load_once("bare", false)), 0);
}

#[test]
fn logic_example_without_a_board_has_no_default_class_warnings() {
    let logic = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logic");
    let p = Project::load(&logic).unwrap();
    assert!(!p.schematics.is_empty());
    assert_eq!(default_class_warnings(&p), 0);
}
