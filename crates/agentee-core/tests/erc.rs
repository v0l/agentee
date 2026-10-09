use agentee_core::{Project, Severity};
use std::path::Path;

fn pin(number: &str, name: &str, kind: &str, y: f64) -> String {
    format!(
        "[[pins]]\nnumber = \"{number}\"\nname = \"{name}\"\ntype = \"{kind}\"\nat = [-7.62, {y}]\nside = \"left\"\n"
    )
}

fn chip(name: &str, pins: &[(&str, &str, &str)]) -> String {
    let mut s = format!("name = \"{name}\"\nreference = \"U\"\n");
    for (i, (n, label, kind)) in pins.iter().enumerate() {
        s += &pin(n, label, kind, i as f64 * 2.54);
    }
    s
}

fn part(r: &str, symbol: &str, x: f64) -> String {
    format!(
        "[[parts]]\nref = \"{r}\"\nsymbol = \"{symbol}\"\nvalue = \"{symbol}\"\nat = [{x}, 25.4]\n"
    )
}

fn net(name: &str, pins: &[&str]) -> String {
    let pins: Vec<String> = pins.iter().map(|p| format!("\"{p}\"")).collect();
    format!("[[nets]]\nname = \"{name}\"\npins = [{}]\n", pins.join(", "))
}

fn erc(name: &str, symbols: &[(&str, String)], body: &str) -> Vec<(Severity, String, String)> {
    let dir = std::env::temp_dir().join(format!("agentee-erc-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    for s in ["R", "C"] {
        std::fs::copy(
            lna.join(format!("symbols/{s}.sym.toml")),
            dir.join(format!("symbols/{s}.sym.toml")),
        )
        .unwrap();
    }
    for (n, text) in symbols {
        std::fs::write(dir.join(format!("symbols/{n}.sym.toml")), text).unwrap();
    }
    std::fs::write(dir.join("t.sch.toml"), format!("name = \"t\"\n{body}")).unwrap();
    let p = Project::load(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    p.schematics
        .iter()
        .flat_map(|s| &s.diags)
        .filter_map(|d| {
            let r = d.rule.as_deref()?;
            r.starts_with("erc-").then(|| (d.severity, r.to_string(), d.message.clone()))
        })
        .collect()
}

fn has(f: &[(Severity, String, String)], sev: Severity, rule: &str) -> bool {
    f.iter().any(|(s, r, _)| *s == sev && r == rule)
}

fn mcu() -> (&'static str, String) {
    ("MCU", chip("MCU", &[("1", "VDD", "power_in"), ("2", "IN", "input"), ("3", "OUT", "output")]))
}

fn ldo() -> (&'static str, String) {
    ("LDO", chip("LDO", &[("1", "VIN", "power_in"), ("2", "VOUT", "power_out")]))
}

#[test]
fn a_rail_of_power_inputs_and_caps_is_unsupplied() {
    let body = part("U1", "MCU", 25.4)
        + &part("U2", "MCU", 50.8)
        + &part("C1", "C", 76.2)
        + &net("3V3", &["U1.1", "U2.1", "C1.1"])
        + &net("A", &["U1.3", "U2.2"]);
    let f = erc("unsupplied", &[mcu()], &body);
    assert!(has(&f, Severity::Error, "erc-unsupplied"), "{f:?}");
}

#[test]
fn a_regulator_output_supplies_its_rail() {
    let body = part("U1", "MCU", 25.4)
        + &part("U2", "LDO", 50.8)
        + &part("R1", "R", 76.2)
        + &net("3V3", &["U1.1", "U2.2"])
        + &net("VIN", &["U2.1", "R1.1"]);
    let f = erc("supplied", &[mcu(), ldo()], &body);
    assert!(!has(&f, Severity::Error, "erc-unsupplied"), "{f:?}");
}

#[test]
fn one_power_pin_with_only_a_cap_is_a_warning() {
    let body = part("U1", "MCU", 25.4) + &part("C1", "C", 50.8) + &net("VCAP", &["U1.1", "C1.1"]);
    let f = erc("vcap", &[mcu()], &body);
    assert!(has(&f, Severity::Warning, "erc-unsupplied"), "{f:?}");
    assert!(!has(&f, Severity::Error, "erc-unsupplied"), "{f:?}");
}

#[test]
fn inputs_with_nothing_else_are_undriven() {
    let body = part("U1", "MCU", 25.4) + &part("U2", "MCU", 50.8) + &net("EN", &["U1.2", "U2.2"]);
    let f = erc("undriven", &[mcu()], &body);
    assert!(has(&f, Severity::Error, "erc-undriven"), "{f:?}");
}

#[test]
fn an_input_behind_a_coupling_cap_is_not_undriven() {
    let body = part("U1", "MCU", 25.4) + &part("C1", "C", 50.8) + &net("AC", &["U1.2", "C1.1"]);
    let f = erc("coupled", &[mcu()], &body);
    assert!(!has(&f, Severity::Error, "erc-undriven"), "{f:?}");
}

#[test]
fn two_outputs_on_one_net_conflict() {
    let body = part("U1", "MCU", 25.4) + &part("U2", "MCU", 50.8) + &net("X", &["U1.3", "U2.3"]);
    let f = erc("conflict", &[mcu()], &body);
    assert!(has(&f, Severity::Error, "erc-drive-conflict"), "{f:?}");
}

#[test]
fn two_power_pins_of_one_regulator_do_not_conflict() {
    let reg = ("REG", chip("REG", &[("1", "VOUT", "power_out"), ("2", "VOUT", "power_out")]));
    let body =
        part("U1", "REG", 25.4) + &part("R1", "R", 50.8) + &net("V", &["U1.1", "U1.2", "R1.1"]);
    let f = erc("same-part", &[reg], &body);
    assert!(!has(&f, Severity::Error, "erc-drive-conflict"), "{f:?}");
}

#[test]
fn an_output_fights_an_open_collector() {
    let oc = ("OC", chip("OC", &[("1", "INT", "open_collector")]));
    let body = part("U1", "MCU", 25.4)
        + &part("U2", "OC", 50.8)
        + &part("R1", "R", 76.2)
        + &net("INT", &["U1.3", "U2.1", "R1.1"]);
    let f = erc("oc-fight", &[mcu(), oc], &body);
    assert!(has(&f, Severity::Error, "erc-drive-conflict"), "{f:?}");
}

#[test]
fn an_open_collector_needs_a_pull_up() {
    let oc = ("OC", chip("OC", &[("1", "INT", "open_collector")]));
    let bare = part("U1", "MCU", 25.4) + &part("U2", "OC", 50.8) + &net("INT", &["U1.2", "U2.1"]);
    let f = erc("oc-bare", &[mcu(), oc.clone()], &bare);
    assert!(has(&f, Severity::Warning, "erc-no-pull"), "{f:?}");
    let pulled = bare.replace("\"U2.1\"]", "\"U2.1\", \"R1.1\"]") + &part("R1", "R", 76.2);
    let f = erc("oc-pulled", &[mcu(), oc], &pulled);
    assert!(!has(&f, Severity::Warning, "erc-no-pull"), "{f:?}");
}

#[test]
fn an_output_beside_a_tri_state_pin_contends() {
    let tri = ("TRI", chip("TRI", &[("1", "D", "tri_state")]));
    let body = part("U1", "MCU", 25.4) + &part("U2", "TRI", 50.8) + &net("D", &["U1.3", "U2.1"]);
    let f = erc("tri", &[mcu(), tri], &body);
    assert!(has(&f, Severity::Warning, "erc-tristate-contention"), "{f:?}");
}

#[test]
fn a_no_connect_pin_in_a_net_is_flagged() {
    let nc = ("NC", chip("NC", &[("1", "NC", "no_connect")]));
    let body = part("U1", "NC", 25.4) + &part("R1", "R", 50.8) + &net("GND", &["U1.1", "R1.1"]);
    let f = erc("nc", &[nc], &body);
    assert!(has(&f, Severity::Warning, "erc-nc-pin"), "{f:?}");
}

#[test]
fn a_dnp_regulator_supplies_nothing() {
    let body = part("U1", "MCU", 25.4)
        + &part("U2", "MCU", 50.8)
        + &part("U3", "LDO", 76.2)
            .replace("at = [76.2, 25.4]\n", "at = [76.2, 25.4]\ndnp = true\n")
        + &net("3V3", &["U1.1", "U2.1", "U3.2"]);
    let f = erc("dnp", &[mcu(), ldo()], &body);
    assert!(has(&f, Severity::Error, "erc-unsupplied"), "{f:?}");
}
