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

#[test]
fn a_bag_barcode_adds_to_the_shelf() {
    let lib = std::env::temp_dir().join(format!("agentee-intake-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&lib);
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    for (name, extra) in
        [("LM358", "mpn = \"LM358BIDDFR\"\n"), ("SW", "mpn = \"ST1102\"\nlcsc = \"C312270\"\n")]
    {
        let dir = lib.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(lna.join("symbols/R.sym.toml"), dir.join("R.sym.toml")).unwrap();
        std::fs::write(
            dir.join("part.toml"),
            format!("role = \"x\"\n{extra}symbol = \"R.sym.toml\"\n"),
        )
        .unwrap();
    }
    let digikey =
        "[)>\u{1e}06\u{1d}P296-LM358BIDDFRCT-ND\u{1d}1PLM358BIDDFR\u{1d}K\u{1d}Q10\u{1d}11ZPICK";
    let lcsc =
        "{pbn:PICK2009291337,on:SO2009291337,pc:C312270,pm:ST-1-102-A01-T000-RS,qty:2,mc:,cc:1}";
    let mouser =
        "[)>\u{1e}06\u{1d}KP0-1337\u{1d}14K011\u{1d}1PMC34063ADR\u{1d}Q3\u{1d}1VTI\u{1e}\u{4}";
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentee"))
        .args(["library", "intake", "--bin", "B2", "-p"])
        .arg(&lib)
        .env("AGENTEE_LIBRARY", &lib)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    let scans = format!("{digikey}\n{digikey}\n{lcsc}\n{mouser}\n");
    child.stdin.take().unwrap().write_all(scans.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "the unknown Mouser part fails the run: {text}");
    assert!(text.contains("+10 from DigiKey, LM358: 20 in your own stock in bin B2\n"), "{text}");
    assert!(text.contains("+2 from LCSC, SW: 2 in your own stock in bin B2\n"), "{text}");
    assert!(text.contains("MC34063ADR (Mouser 3): not in the library"), "{text}");
    let _ = std::fs::remove_dir_all(&lib);
}

#[test]
fn fill_pins_a_datasheet_and_stores_a_model_without_touching_the_shelf() {
    let root = std::env::temp_dir().join(format!("agentee-fill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (lib, cache) = (root.join("lib"), root.join("cache"));
    let dir = lib.join("C1");
    std::fs::create_dir_all(&dir).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    std::fs::copy(lna.join("symbols/C.sym.toml"), dir.join("C.sym.toml")).unwrap();
    let fp = std::fs::read_to_string(lna.join("footprints/C_0402_1005Metric.fp.toml")).unwrap();
    let fp: String = fp
        .lines()
        .filter(|l| !l.starts_with("model") && !l.starts_with("height"))
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(dir.join("C_0402_1005Metric.fp.toml"), fp).unwrap();
    std::fs::write(
        dir.join("part.toml"),
        "role = \"capacitor\"\nmpn = \"C1MPN\"\nsymbol = \"C.sym.toml\"\nfootprint = \"C_0402_1005Metric.fp.toml\"\n\n\
         [[stock]]\ndistributor = \"home\"\nsku = \"C1MPN\"\nstock = 0\nchecked = \"2026-01-01\"\nown = 12\nbin = \"B4\"\n",
    )
    .unwrap();
    let pdf = root.join("c1.pdf");
    std::fs::write(&pdf, b"%PDF-1.4\n% C1MPN capacitor\n%%EOF\n").unwrap();
    let step =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../agentee-3d/tests/data/filleted_box.step");
    let fill = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
            .args(["library", "fill", "C1", "--no-distributors", "-p"])
            .arg(&lib)
            .args(args)
            .env("AGENTEE_LIBRARY", &lib)
            .env("AGENTEE_DATASHEETS", &cache)
            .output()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
        (v, String::from_utf8_lossy(&out.stderr).into_owned(), out.status.success())
    };
    let (v, err, ok) = fill(&[]);
    assert!(!ok, "nothing to fill from offline: {v}{err}");
    assert_eq!(v["still_missing"][0]["missing"], serde_json::json!(["datasheet", "model"]), "{v}");
    let (v, err, ok) =
        fill(&["--datasheet", pdf.to_str().unwrap(), "--model", step.to_str().unwrap()]);
    assert!(ok, "{v}{err}");
    assert_eq!(v["filled"][0]["filled"], serde_json::json!(["datasheet", "model"]), "{v}");
    let part = std::fs::read_to_string(dir.join("part.toml")).unwrap();
    assert!(part.contains("[datasheet]") && part.contains("model = \"3dmodels/"), "{part}");
    assert!(part.contains("own = 12") && part.contains("bin = \"B4\""), "{part}");
    assert!(dir.join("3dmodels/filleted_box.step").exists());
    let fp = std::fs::read_to_string(dir.join("C_0402_1005Metric.fp.toml")).unwrap();
    assert!(fp.contains("model = \"3dmodels/filleted_box.step\""), "{fp}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn set_corrects_a_part_number_and_keeps_the_shelf() {
    let lib = std::env::temp_dir().join(format!("agentee-set-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&lib);
    let dir = lib.join("LED-Red");
    std::fs::create_dir_all(&dir).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    std::fs::copy(lna.join("symbols/R.sym.toml"), dir.join("R.sym.toml")).unwrap();
    std::fs::write(
        dir.join("part.toml"),
        "role = \"led\"\nmpn = \"73249841\"\nsymbol = \"R.sym.toml\"\n\n[datasheet]\nsha256 = \"abc\"\n\n\
         [[stock]]\ndistributor = \"DigiKey\"\nsku = \"X\"\nstock = 5\nchecked = \"2026-01-01\"\n\n\
         [[stock]]\ndistributor = \"home\"\nsku = \"73249841\"\nstock = 0\nchecked = \"2026-01-01\"\nown = 30\nbin = \"C2\"\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
        .args(["library", "set", "LED-Red", "--mpn", "150080RS75000"])
        .args(["--manufacturer", "Wurth Elektronik", "-p"])
        .arg(&lib)
        .env("AGENTEE_LIBRARY", &lib)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let part = std::fs::read_to_string(dir.join("part.toml")).unwrap();
    assert!(part.contains("mpn = \"150080RS75000\""), "{part}");
    assert!(part.contains("manufacturer = \"Wurth Elektronik\""), "{part}");
    assert!(part.contains("sku = \"150080RS75000\"") && part.contains("own = 30"), "{part}");
    assert!(part.contains("bin = \"C2\""), "{part}");
    assert!(!part.contains("DigiKey") && !part.contains("[datasheet]"), "{part}");
    let _ = std::fs::remove_dir_all(&lib);
}

#[test]
fn per_part_fill_options_need_a_part() {
    for flag in ["--like", "--model", "--datasheet"] {
        let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
            .args(["library", "fill", flag, "X", "--no-distributors"])
            .env("AGENTEE_LIBRARY", std::env::temp_dir().join("agentee-none"))
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success() && err.contains("name it"), "{flag}: {err}");
    }
}
