use std::path::{Path, PathBuf};
use std::process::Command;

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if e.file_type().unwrap().is_dir() {
            if name == "symbols" || name == "footprints" {
                copy(&e.path(), &to.join(&*name));
            }
        } else if name.ends_with(".toml") {
            std::fs::copy(e.path(), to.join(&*name)).unwrap();
        }
    }
}

fn setup(tag: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("agentee-assemble-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let project = root.join("lna");
    copy(&lna, &project);
    let part = root.join("lib/GRM155");
    std::fs::create_dir_all(&part).unwrap();
    std::fs::copy(lna.join("symbols/C.sym.toml"), part.join("C.sym.toml")).unwrap();
    std::fs::write(
        part.join("part.toml"),
        "role = \"capacitor\"\nmpn = \"GRM155R71C104KA88D\"\nsymbol = \"C.sym.toml\"\n\n\
         [[stock]]\ndistributor = \"home\"\nsku = \"GRM155R71C104KA88D\"\nstock = 0\nchecked = \"2026-01-01\"\nown = 10\nbin = \"A3\"\n",
    )
    .unwrap();
    (root.join("lib"), project)
}

fn assemble(lib: &Path, project: &Path, args: &[&str]) -> (String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
        .args(["assemble", "lna", "-p"])
        .arg(project)
        .args(args)
        .env("AGENTEE_LIBRARY", lib)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (text, out.status.success())
}

#[test]
fn a_sheet_groups_parts_by_side_with_their_bin() {
    let (lib, project) = setup("sheet");
    let (text, ok) = assemble(&lib, &project, &[]);
    assert!(ok, "{text}");
    assert!(text.starts_with("top side\n"), "{text}");
    assert!(text.contains(". 100n 0402, bin A3, C1 C2 C3 C5, 4 pcs\n"), "{text}");
    let first = text.lines().nth(1).unwrap();
    assert!(first.contains("SOT-89-3, U1"), "the hot-air part goes first: {text}");
    let last = text.lines().filter(|l| l.contains(" pcs")).last().unwrap();
    assert!(last.contains("PinHeader"), "through-hole goes last: {text}");
    let _ = std::fs::remove_dir_all(lib.parent().unwrap());
}

#[test]
fn built_boards_come_off_the_shelf() {
    let (lib, project) = setup("built");
    let (text, ok) = assemble(&lib, &project, &["--built", "2"]);
    assert!(ok, "{text}");
    assert!(text.contains("GRM155: took 8, 2 left\n"), "{text}");
    assert!(text.contains("not in the library, so not taken off stock: "), "{text}");
    let part = std::fs::read_to_string(lib.join("GRM155/part.toml")).unwrap();
    assert!(part.contains("own = 2") && part.contains("bin = \"A3\""), "{part}");
    let (text, ok) = assemble(&lib, &project, &["--built", "1"]);
    assert!(!ok, "a short shelf fails: {text}");
    assert!(text.contains("GRM155: took 2, 0 left, 2 short\n"), "{text}");
    let part = std::fs::read_to_string(lib.join("GRM155/part.toml")).unwrap();
    assert!(!part.contains("own ="), "{part}");
    let _ = std::fs::remove_dir_all(lib.parent().unwrap());
}
