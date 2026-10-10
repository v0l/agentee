use std::path::Path;
use std::process::Command;

#[test]
fn show_carries_the_readings_of_a_map_sim() {
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
        .args(["show", "sim:lna-thermal"])
        .current_dir(&lna)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let result = &v["result"];
    assert_eq!(result["kind"], "thermal", "{result}");
    let readings = result["readings"].as_array().unwrap();
    assert!(readings.iter().any(|r| r["label"] == "board peak"), "{result}");
    let maps = result["maps"].as_array().unwrap();
    assert!(!maps.is_empty() && maps.iter().all(|m| m.get("data").is_none()), "{result}");
}

#[test]
fn show_names_every_net_of_a_layout() {
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let out = Command::new(env!("CARGO_BIN_EXE_agentee"))
        .args(["show", "pcb:lna"])
        .current_dir(&lna)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let named = |key: &str| {
        let items = v[key].as_array().unwrap();
        assert!(!items.is_empty(), "no {key}");
        for item in items {
            assert!(item["net"].is_string(), "{key}: {item}");
        }
    };
    named("tracks");
    named("vias");
    named("zones");
    for part in v["parts"].as_array().unwrap() {
        for pad in part["pads"].as_array().unwrap() {
            assert!(pad["net"].is_string() || pad["net"].is_null(), "{pad}");
        }
    }
    for pair in v["pairs"].as_array().unwrap() {
        assert!(pair["p"].is_string() && pair["n"].is_string(), "{pair}");
    }
}
