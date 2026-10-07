use agentee_core::{Project, Severity};
use std::path::Path;

const MCU: &str = "name = \"MCU\"\nreference = \"U\"\n\
[[pins]]\nnumber = \"1\"\nname = \"VDD\"\ntype = \"power_in\"\nat = [-7.62, 0.0]\nside = \"left\"\n\
[[pins]]\nnumber = \"2\"\nname = \"GND\"\ntype = \"power_in\"\nat = [-7.62, 2.54]\nside = \"left\"\n\
[[pins]]\nnumber = \"3\"\nname = \"IO\"\ntype = \"bidirectional\"\nat = [7.62, 0.0]\nside = \"right\"\n\
[[pins]]\nnumber = \"4\"\nname = \"IN\"\ntype = \"input\"\nat = [7.62, 2.54]\nside = \"right\"\n\
[levels]\nsupply = \"VDD\"\nvih = \"75%\"\nvil = \"25%\"\nmin = \"-0.3V\"\nmax = \"100%+0.3V\"\nleakage = \"50nA\"\n";

const SW: &str = "name = \"SW_Push\"\nreference = \"SW\"\n\
[[pins]]\nnumber = \"1\"\nat = [-5.08, 0.0]\nside = \"left\"\n\
[[pins]]\nnumber = \"2\"\nat = [5.08, 0.0]\nside = \"right\"\n";

fn part(r: &str, symbol: &str, value: &str, x: f64) -> String {
    format!(
        "[[parts]]\nref = \"{r}\"\nsymbol = \"{symbol}\"\nvalue = \"{value}\"\nat = [{x}, 25.4]\n"
    )
}

fn net(name: &str, pins: &[&str]) -> String {
    let pins: Vec<String> = pins.iter().map(|p| format!("\"{p}\"")).collect();
    format!("[[nets]]\nname = \"{name}\"\npins = [{}]\n", pins.join(", "))
}

fn level_findings(
    name: &str,
    top: &str,
    body: &str,
    on_3v3: &[&str],
    on_gnd: &[&str],
) -> Vec<(Severity, String)> {
    let dir = std::env::temp_dir().join(format!("agentee-levels-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    std::fs::copy(lna.join("symbols/R.sym.toml"), dir.join("symbols/R.sym.toml")).unwrap();
    std::fs::write(dir.join("symbols/MCU.sym.toml"), MCU).unwrap();
    std::fs::write(dir.join("symbols/SW_Push.sym.toml"), SW).unwrap();
    let mut supply = vec!["U1.1"];
    supply.extend(on_3v3);
    let mut ground = vec!["U1.2"];
    ground.extend(on_gnd);
    let sch = format!(
        "name = \"t\"\nno_connect = [\"U1.3\", \"U1.4\"]\n{top}\n{}{body}{}{}",
        part("U1", "MCU", "MCU", 25.4),
        net("3V3", &supply),
        net("GND", &ground),
    );
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    let p = Project::load(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    p.schematics
        .iter()
        .flat_map(|s| &s.diags)
        .filter(|d| d.at.starts_with("net ") && d.severity != Severity::Info)
        .filter(|d| !d.message.contains("only one pin") && !d.message.contains("no_connect"))
        .map(|d| (d.severity, d.message.clone()))
        .collect()
}

const RAILS: &str = "rails = { GND = 0, \"3V3\" = [3.25, 3.35], VBUS = [4.75, 5.25] }";

fn divider(top: &str, upper: &str, lower: &str) -> Vec<(Severity, String)> {
    let body = format!(
        "{}{}{}{}",
        part("R1", "R", upper, 50.8),
        part("R2", "R", lower, 76.2),
        net("VBUS", &["R1.1"]),
        net("SENSE", &["R1.2", "R2.1", "U1.3"]),
    );
    level_findings(&format!("div-{upper}-{lower}-{}", top.len()), top, &body, &[], &["R2.2"])
}

#[test]
fn a_half_divider_from_usb_lands_between_vil_and_vih() {
    let f = divider(RAILS, "100k", "100k");
    assert!(
        f.iter().any(|(s, m)| *s == Severity::Error && m.contains("not a valid level")),
        "{f:?}"
    );
}

#[test]
fn a_divider_that_clears_vih_passes() {
    let f = divider(RAILS, "330k", "470k");
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn a_divider_just_over_vih_is_a_warning() {
    let f = divider(RAILS, "100k", "120k");
    assert!(f.iter().any(|(s, m)| *s == Severity::Warning && m.contains("above VIH")), "{f:?}");
}

#[test]
fn nothing_is_checked_without_rails() {
    assert!(divider("", "100k", "100k").is_empty());
}

#[test]
fn a_divider_above_the_supply_is_overdriven() {
    let f = divider(RAILS, "1k", "100k");
    assert!(f.iter().any(|(_, m)| m.contains("overdriven")), "{f:?}");
}

#[test]
fn an_analog_pin_skips_the_threshold_band() {
    let top = format!("analog = [\"U1.3\"]\n{RAILS}");
    assert!(divider(&top, "100k", "100k").is_empty());
}

fn button(pull: Option<&str>) -> Vec<(Severity, String)> {
    let mut body = part("SW1", "SW_Push", "SW", 50.8);
    let mut pins = vec!["SW1.2", "U1.4"];
    let mut up = vec![];
    if let Some(r) = pull {
        body += &part("R1", "R", r, 76.2);
        pins.push("R1.2");
        up.push("R1.1");
    }
    body += &net("BTN", &pins);
    level_findings(&format!("btn-{pull:?}"), RAILS, &body, &up, &["SW1.1"])
}

#[test]
fn a_button_without_a_pull_floats_when_open() {
    let f = button(None);
    assert!(f.iter().any(|(_, m)| m.contains("floats with SW1 open")), "{f:?}");
}

#[test]
fn input_leakage_through_a_huge_pull_is_too_weak() {
    let f = button(Some("100M"));
    assert!(f.iter().any(|(_, m)| m.contains("pull is too weak with SW1 open")), "{f:?}");
}

#[test]
fn a_button_with_a_10k_pull_up_is_clean() {
    assert!(button(Some("10k")).is_empty(), "{:?}", button(Some("10k")));
}

#[test]
fn a_bidirectional_pin_drives_the_input_beside_it() {
    let body = net("LINE", &["U1.3", "U1.4"]);
    let f = level_findings("bidir", RAILS, &body, &[], &[]);
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn a_gpio_feeding_only_a_filter_is_an_output() {
    let body = format!(
        "{}{}{}",
        part("R1", "R", "10k", 50.8),
        net("PWM", &["U1.3", "R1.1"]),
        net("FILT", &["R1.2"]),
    );
    let f = level_findings("filter", RAILS, &body, &[], &[]);
    assert!(f.is_empty(), "{f:?}");
}
