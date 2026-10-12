use agentee_core::Project;
use std::path::{Path, PathBuf};

const FOOTPRINT: &str = "name = \"THT_R\"\nmount = \"tht\"\n\
[[pads]]\nnumber = \"1\"\nkind = \"tht\"\nshape = \"circle\"\nat = [0.0, 0.0]\nsize = [0.6, 0.6]\ndrill = 0.3\nlayers = [\"*.Cu\", \"*.Mask\"]\n\
[[pads]]\nnumber = \"2\"\nkind = \"tht\"\nshape = \"circle\"\nat = [2.54, 0.0]\nsize = [0.6, 0.6]\ndrill = 0.3\nlayers = [\"*.Cu\", \"*.Mask\"]\n";

fn board(name: &str, preset: &str) -> String {
    format!(
        "name = \"{name}\"\nfab = \"jlcpcb\"\n[outline]\nsize = [30, 20]\n[stackup]\n\
         preset = \"{preset}\"\n[[netclasses]]\nname = \"Default\"\n\
         track_width = \"0.2mm\"\nclearance = \"0.15mm\"\n"
    )
}

fn design(dir: &Path, name: &str) {
    std::fs::write(
        dir.join(format!("{name}.sch.toml")),
        format!(
            "name = \"{name}\"\nboard = \"{name}\"\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\n\
             value = \"1k\"\nfootprint = \"THT_R\"\nat = [10.16, 20.32]\n[[nets]]\nname = \"A\"\n\
             pins = [\"R1.1\", \"R1.2\"]\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("{name}.pcb.toml")),
        format!(
            "name = \"{name}\"\nboard = \"{name}\"\nschematic = \"{name}\"\n\
             [[footprints]]\nref = \"R1\"\nat = [10, 10]\n"
        ),
    )
    .unwrap();
}

fn project(name: &str, two_layer_uses_it: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agentee-fprules-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    std::fs::copy(lna.join("symbols/R.sym.toml"), dir.join("symbols/R.sym.toml")).unwrap();
    std::fs::write(dir.join("footprints/THT_R.fp.toml"), FOOTPRINT).unwrap();
    std::fs::write(dir.join("four.board.toml"), board("four", "JLC04161H-7628")).unwrap();
    std::fs::write(dir.join("two.board.toml"), board("two", "jlcpcb-2l-1.6mm")).unwrap();
    design(&dir, "four");
    if two_layer_uses_it {
        design(&dir, "two");
    }
    dir
}

fn ring_errors(name: &str, two_layer_uses_it: bool) -> usize {
    let dir = project(name, two_layer_uses_it);
    let p = Project::load(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    p.footprints
        .iter()
        .filter(|f| f.name == "THT_R")
        .flat_map(|f| &f.diags)
        .filter(|d| d.message.contains("annular ring"))
        .count()
}

#[test]
fn footprint_is_held_to_the_boards_that_place_it() {
    assert_eq!(ring_errors("four-only", false), 0);
    assert_eq!(ring_errors("both", true), 2);
}

#[test]
fn a_kicad_mounting_pad_needs_no_pin() {
    let dir = std::env::temp_dir().join(format!("agentee-fprules-{}-mp", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let sym = std::fs::read_to_string(lna.join("symbols/R.sym.toml")).unwrap();
    std::fs::write(dir.join("symbols/R.sym.toml"), sym.replace("R_0402_1005Metric", "THT_R"))
        .unwrap();
    let pad = |n: &str, x: f64| {
        format!(
            "[[pads]]\nnumber = \"{n}\"\nkind = \"tht\"\nshape = \"circle\"\nat = [{x}, 0.0]\n\
             size = [1.6, 1.6]\ndrill = 0.8\nlayers = [\"*.Cu\", \"*.Mask\"]\n"
        )
    };
    let fp = |extra: &str| {
        format!(
            "name = \"THT_R\"\nmount = \"tht\"\n{}{}{}",
            pad("1", 0.0),
            pad("2", 2.54),
            pad(extra, 6.0)
        )
    };
    let unused = |extra: &str| {
        std::fs::write(dir.join("footprints/THT_R.fp.toml"), fp(extra)).unwrap();
        std::fs::write(dir.join("t.board.toml"), board("t", "jlcpcb-2l-1.6mm")).unwrap();
        design(&dir, "t");
        let p = Project::load(&dir).unwrap();
        p.symbols
            .iter()
            .flat_map(|s| &s.diags)
            .filter(|d| d.message.contains("have no pin"))
            .count()
    };
    assert_eq!(unused("MP"), 0);
    assert_eq!(unused("3"), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
