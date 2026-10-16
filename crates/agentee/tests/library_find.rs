use std::path::Path;
use std::process::Command;

fn find(lib: &Path, words: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
        .args(["library", "find", "--json", "-p"])
        .arg(lib)
        .args(words)
        .env("AGENTEE_LIBRARY", lib)
        .env("AGENTEE_DISTRIBUTORS", lib.join("none.toml"))
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn find_offers_the_library_part_before_searching() {
    let lib = std::env::temp_dir().join(format!("agentee-find-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&lib);
    let dir = lib.join("AP63203");
    std::fs::create_dir_all(&dir).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna/symbols");
    let sym = std::fs::read_dir(&lna).unwrap().next().unwrap().unwrap().path();
    std::fs::copy(&sym, dir.join("U.sym.toml")).unwrap();
    std::fs::write(
        dir.join("part.toml"),
        "role = \"regulator\"\nmpn = \"AP63203WU-7\"\nnotes = \"3.3 V buck\"\nsymbol = \"U.sym.toml\"\n",
    )
    .unwrap();
    let v = find(&lib, &["buck"]);
    assert_eq!(v["library"][0]["name"], "AP63203", "{v}");
    assert!(v["next"].as_str().unwrap().contains("library use"), "{v}");
    assert!(v["errors"].as_array().unwrap().is_empty(), "the library match needs no search: {v}");
    let v = find(&lib, &["ldo"]);
    assert!(v["library"].as_array().unwrap().is_empty(), "{v}");
    assert!(v["next"].as_str().unwrap().contains("library add"), "{v}");
    assert!(!v["errors"].as_array().unwrap().is_empty(), "no library match searches: {v}");
    let _ = std::fs::remove_dir_all(&lib);
}

fn use_part(lib: &Path, project: &Path) -> (serde_json::Value, String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
        .args(["library", "use", "R0603", "-p"])
        .arg(project)
        .env("AGENTEE_LIBRARY", lib)
        .env("AGENTEE_DISTRIBUTORS", lib.join("none.toml"))
        .output()
        .unwrap();
    (
        serde_json::from_slice(&out.stdout).unwrap_or_default(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn use_writes_nothing_when_a_file_conflicts() {
    let root = std::env::temp_dir().join(format!("agentee-use-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (lib, project) = (root.join("lib"), root.join("project"));
    let dir = lib.join("R0603");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(project.join("footprints")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    std::fs::copy(lna.join("symbols/R.sym.toml"), dir.join("R.sym.toml")).unwrap();
    let fp = std::fs::read_to_string(lna.join("footprints/R_0402_1005Metric.fp.toml")).unwrap();
    let fp: String = fp
        .lines()
        .filter(|l| !l.starts_with("model") && !l.starts_with("height"))
        .map(|l| format!("{l}\n"))
        .collect();
    let (head, tail) = fp.split_once('\n').unwrap();
    let lib_fp = format!("{head}\nmodel = \"3dmodels/R.step\"\nheight = \"0.35mm\"\n{tail}");
    std::fs::write(dir.join("R_0402_1005Metric.fp.toml"), &lib_fp).unwrap();
    std::fs::create_dir_all(dir.join("3dmodels")).unwrap();
    std::fs::write(dir.join("3dmodels/R.step"), "step").unwrap();
    std::fs::write(
        dir.join("part.toml"),
        "role = \"resistor\"\nsymbol = \"R.sym.toml\"\nfootprint = \"R_0402_1005Metric.fp.toml\"\nmodel = \"3dmodels/R.step\"\n",
    )
    .unwrap();
    let proj_fp = project.join("footprints/R_0402_1005Metric.fp.toml");
    let kicad = format!("{head}\nmodel = \"${{KICAD9_3DMODEL_DIR}}/R.step\"\n{tail}");
    std::fs::write(&proj_fp, &kicad).unwrap();
    let (v, err, ok) = use_part(&lib, &project);
    assert!(ok, "{err}{v}");
    assert!(v["files"].to_string().contains("kept"), "{v}");
    assert_eq!(std::fs::read_to_string(&proj_fp).unwrap(), kicad);
    assert!(project.join("symbols/R.sym.toml").exists());
    assert!(!project.join("3dmodels/R.step").exists());

    std::fs::remove_file(project.join("symbols/R.sym.toml")).unwrap();
    std::fs::write(&proj_fp, kicad.replace("mount = \"smd\"", "mount = \"tht\"")).unwrap();
    let (_, err, ok) = use_part(&lib, &project);
    assert!(!ok && err.contains("nothing was written"), "{err}");
    assert!(!project.join("symbols/R.sym.toml").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn stock_keeps_its_bin() {
    let lib = std::env::temp_dir().join(format!("agentee-bin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&lib);
    let dir = lib.join("R1K");
    std::fs::create_dir_all(&dir).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    std::fs::copy(lna.join("symbols/R.sym.toml"), dir.join("R.sym.toml")).unwrap();
    std::fs::write(dir.join("part.toml"), "role = \"resistor\"\nsymbol = \"R.sym.toml\"\n")
        .unwrap();
    let stock = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
            .args(["library", "stock", "R1K"])
            .args(args)
            .args(["-p"])
            .arg(&lib)
            .env("AGENTEE_LIBRARY", &lib)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert_eq!(stock(&["40", "--bin", "A3"]).trim(), "R1K: 40 in your own stock in bin A3");
    assert_eq!(stock(&["25"]).trim(), "R1K: 25 in your own stock in bin A3");
    let part = std::fs::read_to_string(dir.join("part.toml")).unwrap();
    assert!(part.contains("bin = \"A3\""), "{part}");
    let _ = std::fs::remove_dir_all(&lib);
}
