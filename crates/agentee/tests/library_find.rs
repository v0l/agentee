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
