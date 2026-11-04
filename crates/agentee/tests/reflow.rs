use std::path::Path;
use std::process::Command;

fn lna() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna")
}

#[test]
fn reflow_on_the_lna_prints_a_profile_names_the_limiter_and_writes_the_exports() {
    let dir = std::env::temp_dir().join(format!("agentee-reflow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("lna.csv");
    let svg = dir.join("lna.svg");
    let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
        .args(["reflow", "lna", "--csv"])
        .arg(&csv)
        .arg("--plot")
        .arg(&svg)
        .current_dir(lna())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}{}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("profile (feasible)") && text.contains("limiter D4"), "{text}");
    assert!(text.contains("thermocouples:") && text.contains("oven (Convection) setpoints"));
    assert!(text.contains("SPF5189Z") && text.contains("Assumed"));
    let csv = std::fs::read_to_string(csv).unwrap();
    assert!(csv.contains("limiter D4"));
    let svg = std::fs::read_to_string(svg).unwrap();
    assert!(svg.contains("limiter D4"));
}

#[test]
fn reflow_json_reports_sources_and_margins() {
    let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
        .args(["reflow", "lna", "--json"])
        .current_dir(lna())
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["profile"]["limiter"], "D4");
    assert!(v["profile"]["limiter_margin_c"].as_f64().unwrap() >= 0.0);
    let parts = v["parts"].as_array().unwrap();
    let c1 = parts.iter().find(|p| p["reference"] == "C1").unwrap();
    assert!(c1["source"].as_str().unwrap().contains("murata"));
    assert_eq!(c1["page"], "16");
}

#[test]
fn reflow_exits_nonzero_on_a_missing_file() {
    let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
        .args(["reflow", "nothing"])
        .current_dir(lna())
        .output()
        .unwrap();
    assert!(!out.status.success());
}
