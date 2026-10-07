use std::path::Path;
use std::process::Command;

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let name = e.file_name();
        if name.to_string_lossy().ends_with(".result.json") {
            continue;
        }
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &to.join(&name));
        } else {
            std::fs::copy(e.path(), to.join(&name)).unwrap();
        }
    }
}

#[test]
fn search_tries_seeds_and_writes_the_best() {
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let dir = std::env::temp_dir().join(format!("agentee-search-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy(&lna, &dir);
    let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
        .args(["layout", "lna", "--search", "4", "--keep", "2", "--json"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let tried = v["search"]["tried"].as_array().unwrap();
    assert_eq!(tried.len(), 4, "{tried:?}");
    let full: Vec<&serde_json::Value> = tried.iter().filter(|t| t.get("total").is_some()).collect();
    assert!((2..=3).contains(&full.len()), "{tried:?}");
    assert!(tried[0].get("total").is_some(), "the current settings must get a full run");
    let best = &tried[v["search"]["best"].as_u64().unwrap() as usize];
    let seed = best["knobs"]["place.seed"].as_str().unwrap().to_string();
    let pcb = std::fs::read_to_string(dir.join("lna.pcb.toml")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(pcb.contains(&format!("seed = {seed}")), "the best seed {seed} is not pinned");
}

#[test]
fn a_knob_the_engine_lacks_is_refused() {
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let dir = std::env::temp_dir().join(format!("agentee-search-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy(&lna, &dir);
    let pcb = dir.join("lna.pcb.toml");
    let text = std::fs::read_to_string(&pcb).unwrap();
    std::fs::write(
        &pcb,
        format!("{text}\n[engine.search]\nknobs = {{ \"place.sed\" = [1, 2] }}\n"),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
        .args(["layout", "lna", "--search", "--dry-run"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown field `sed`"));
}
