use agentee_core::Project;
use agentee_core::diag::Severity;

fn project(sim: &str) -> Project {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("agentee-volt-{}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    std::fs::create_dir_all(dir.join("footprints")).unwrap();
    let pin = |n: usize, y: f64| {
        format!(
            "\n[[pins]]\nnumber = \"{n}\"\nat = [0.0, {y}]\nside = \"left\"\ntype = \"passive\"\n"
        )
    };
    std::fs::write(
        dir.join("symbols/P.sym.toml"),
        format!(
            "name = \"P\"\nreference = \"J\"\nfootprint = \"TWO\"{}{}",
            pin(1, 0.0),
            pin(2, 2.54)
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("symbols/C.sym.toml"),
        format!(
            "name = \"C\"\nreference = \"C\"\nfootprint = \"TWO\"{}{}",
            pin(1, 0.0),
            pin(2, 2.54)
        ),
    )
    .unwrap();
    let pad = |n: usize, x: f64| {
        format!(
            "\n[[pads]]\nnumber = \"{n}\"\nkind = \"smd\"\nshape = \"rect\"\nat = [{x}, 0.0]\nsize = [1.0, 1.0]\n"
        )
    };
    std::fs::write(
        dir.join("footprints/TWO.fp.toml"),
        format!("name = \"TWO\"\nmount = \"smd\"{}{}", pad(1, -1.5), pad(2, 1.5)),
    )
    .unwrap();
    std::fs::write(
        dir.join("t.board.toml"),
        r#"name = "t"
fab = "jlcpcb"
[outline]
size = [60, 30]
[stackup]
preset = "jlcpcb-2l-1.6mm"
[[vias]]
name = "std"
drill = "0.3mm"
diameter = "0.6mm"
[[netclasses]]
name = "Default"
track_width = "0.2mm"
via = "std"
[[netclasses]]
name = "Ground"
track_width = "0.3mm"
via = "std"
voltage = "0V"
[[netclasses]]
name = "Rail"
track_width = "0.3mm"
via = "std"
voltage = "4.8VDC"
[[netclasses]]
name = "Hv"
track_width = "0.3mm"
via = "std"
voltage = "400VDC"
[[netclasses]]
name = "Mains"
track_width = "0.5mm"
via = "std"
voltage = "230VAC"
"#,
    )
    .unwrap();
    let part = |r: &str, sym: &str, value: &str, x: f64| {
        format!(
            "\n[[parts]]\nref = \"{r}\"\nsymbol = \"{sym}\"\nvalue = \"{value}\"\nat = [{x}, 20.32]\n"
        )
    };
    let net = |name: &str, class: &str, pins: &str| {
        format!("\n[[nets]]\nname = \"{name}\"\nclass = \"{class}\"\npins = [{pins}]\n")
    };
    let sch = [
        "name = \"t\"\nboard = \"t\"\n".to_string(),
        part("J1", "P", "IN", 10.16),
        part("J2", "P", "AC", 20.32),
        part("C1", "C", "100n/6.3V", 30.48),
        part("C2", "C", "10n/250V", 40.64),
        part("C3", "C", "10n/450V", 50.8),
        net("V5", "Rail", "\"J1.1\", \"C1.1\""),
        net("GND", "Ground", "\"J1.2\", \"C1.2\", \"C2.2\", \"C3.2\""),
        net("HV", "Hv", "\"C2.1\", \"C3.1\""),
        net("L", "Mains", "\"J2.1\""),
        net("N", "Mains", "\"J2.2\""),
    ]
    .concat();
    std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
    let at = |r: &str, x: f64| format!("\n[[footprints]]\nref = \"{r}\"\nat = [{x}, 15]\n");
    let pcb = [
        "name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n".to_string(),
        at("J1", 5.0),
        at("J2", 50.0),
        at("C1", 15.0),
        at("C2", 25.0),
        at("C3", 35.0),
    ]
    .concat();
    std::fs::write(dir.join("t.pcb.toml"), pcb).unwrap();
    std::fs::write(dir.join("t.sim.toml"), sim).unwrap();
    Project::load(&dir).unwrap()
}

const DC: &str = "name = \"dc\"\nkind = \"dc\"\nlayout = \"t\"\n[[supplies]]\npad = \"J1.1\"\n[[supplies]]\npad = \"J1.2\"\n[[loads]]\npad = \"C1.1\"\ncurrent = \"10mA\"\nreturn = \"C1.2\"\n";

fn messages(p: &Project, severity: Severity) -> Vec<String> {
    p.schematics[0]
        .diags
        .iter()
        .filter(|d| d.severity == severity)
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn a_dc_supply_with_no_voltage_takes_its_class_voltage() {
    let p = project(DC);
    assert!(p.failures.is_empty(), "{:?}", p.failures);
    let sim = &p.sims[0];
    let errors: Vec<_> = sim.diags.iter().filter(|d| d.severity == Severity::Error).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let volts: Vec<f64> = sim.item.supplies.iter().map(|s| s.volts).collect();
    assert_eq!(volts, vec![4.8, 0.0]);
}

#[test]
fn an_ac_supply_pad_needs_a_dc_voltage_written() {
    let p = project(&DC.replace("J1.1", "J2.1"));
    let e: Vec<_> = p
        .sims
        .iter()
        .flat_map(|s| &s.diags)
        .chain(&p.failures)
        .map(|d| d.message.clone())
        .collect();
    assert!(e.iter().any(|m| m.contains("J2.1 is on L of netclass Mains at 230VAC")), "{e:?}");
}

#[test]
fn dc_classes_become_rails_for_the_level_check() {
    let p = project(DC);
    let rails = &p.schematics[0].item.rails;
    assert_eq!(rails.get("V5"), Some(&[4.8, 4.8]));
    assert_eq!(rails.get("GND"), Some(&[0.0, 0.0]));
    assert!(rails.get("L").is_none(), "an AC net is no logic rail");
}

#[test]
fn a_capacitor_under_the_voltage_across_it_is_an_error_and_thin_headroom_a_warning() {
    let p = project(DC);
    let e = messages(&p, Severity::Error);
    assert!(e.iter().any(|m| m.contains("C2 is rated 250V but sees 400V peak")), "{e:?}");
    let w = messages(&p, Severity::Warning);
    assert!(w.iter().any(|m| m.contains("C3 is rated 450V and sees 400V peak")), "{w:?}");
    assert!(!e.iter().chain(&w).any(|m| m.contains("C1 ")), "C1 has 31% headroom");
}
