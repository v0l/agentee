use agentee_core::Project;
use agentee_core::testplan::{self, Expect, FIRMWARE, Models, Plan};
use std::path::Path;

const FIXED: &str = "name = \"LDO33\"\nreference = \"U\"\ndatasheet = \"ldo33.pdf\"\n\
[regulator]\noutput = \"OUT\"\ninput = \"IN\"\nenable = \"EN\"\nvout = \"3.3V\"\naccuracy = \"2%\"\n\
[[pins]]\nnumber = \"1\"\nname = \"IN\"\ntype = \"power_in\"\nat = [-7.62, 0.0]\nside = \"left\"\n\
[[pins]]\nnumber = \"2\"\nname = \"EN\"\ntype = \"input\"\nat = [-7.62, 2.54]\nside = \"left\"\n\
[[pins]]\nnumber = \"3\"\nname = \"OUT\"\ntype = \"power_out\"\nat = [7.62, 0.0]\nside = \"right\"\n";

const ADJ: &str = "name = \"BUCK\"\nreference = \"U\"\n\
[regulator]\noutput = \"OUT\"\ninput = \"IN\"\nfeedback = \"FB\"\nvref = \"0.8V\"\naccuracy = \"1%\"\n\
[[pins]]\nnumber = \"1\"\nname = \"IN\"\ntype = \"power_in\"\nat = [-7.62, 0.0]\nside = \"left\"\n\
[[pins]]\nnumber = \"2\"\nname = \"FB\"\ntype = \"input\"\nat = [-7.62, 2.54]\nside = \"left\"\n\
[[pins]]\nnumber = \"3\"\nname = \"OUT\"\ntype = \"power_out\"\nat = [7.62, 0.0]\nside = \"right\"\n";

const TP: &str = "name = \"TestPoint\"\nreference = \"TP\"\n\
[[pins]]\nnumber = \"1\"\nat = [0.0, 0.0]\nside = \"bottom\"\ntype = \"passive\"\n";

struct Case {
    dir: std::path::PathBuf,
}

fn part(r: &str, symbol: &str, value: &str, fields: &str, x: f64) -> String {
    let f = if fields.is_empty() { String::new() } else { format!("fields = {{ {fields} }}\n") };
    format!(
        "[[parts]]\nref = \"{r}\"\nsymbol = \"{symbol}\"\nvalue = \"{value}\"\nat = [{x}, 25.4]\n{f}"
    )
}

fn net(name: &str, class: &str, pins: &[&str]) -> String {
    let pins: Vec<String> = pins.iter().map(|p| format!("\"{p}\"")).collect();
    format!("[[nets]]\nname = \"{name}\"\nclass = \"{class}\"\npins = [{}]\n", pins.join(", "))
}

fn design(tag: &str, rtop: &str, extra_parts: &str, extra_nets: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("agentee-plan-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("symbols")).unwrap();
    let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    std::fs::copy(lna.join("symbols/R.sym.toml"), dir.join("symbols/R.sym.toml")).unwrap();
    std::fs::write(dir.join("symbols/LDO33.sym.toml"), FIXED).unwrap();
    std::fs::write(dir.join("symbols/BUCK.sym.toml"), ADJ).unwrap();
    std::fs::write(dir.join("symbols/TestPoint.sym.toml"), TP).unwrap();
    let mut s = String::from("name = \"t\"\nno_connect = [\"U1.2\"]\n");
    s += &part("U1", "LDO33", "LDO33", "mpn = \"X LDO33\"", 25.4);
    s += &part("U2", "BUCK", "BUCK", "mpn = \"X BUCK\"", 50.8);
    s += &part("R1", "R", rtop, "tolerance = \"1%\"", 76.2);
    s += &part("R2", "R", "8k", "tolerance = \"1%\"", 101.6);
    s += &part("TP1", "TestPoint", "TP", "", 127.0);
    s += &part("TP2", "TestPoint", "TP", "", 139.7);
    s += &part("TP3", "TestPoint", "TP", "", 152.4);
    s += &part("TP4", "TestPoint", "TP", "", 165.1);
    s += extra_parts;
    s += &net("VIN", "Default", &["U1.1"]);
    s += &net("3V3", "Default", &["U1.3", "U2.1", "TP1.1"]);
    s += &net("1V8", "Default", &["U2.3", "R1.1", "TP2.1"]);
    s += &net("FB", "Default", &["U2.2", "R1.2", "R2.1"]);
    s += &net("GND", "Default", &["R2.2", "TP3.1"]);
    s += &net("MYSTERY", "Default", &["TP4.1"]);
    s += extra_nets;
    std::fs::write(dir.join("t.sch.toml"), s).unwrap();
    Case { dir }
}

fn plan(c: &Case) -> Plan {
    let p = Project::load(&c.dir).unwrap();
    let sch = &p.schematics[0].item;
    testplan::build(sch, None, None)
}

fn row<'a>(p: &'a Plan, r: &str) -> &'a testplan::Row {
    p.rows.iter().find(|x| x.reference == r).unwrap()
}

fn reading(p: &Plan, r: &str) -> testplan::Reading {
    match &row(p, r).expect {
        Expect::Reading(x) => x.clone(),
        other => panic!("{r}: {other:?}"),
    }
}

#[test]
fn a_fixed_rail_takes_its_tolerance_from_the_part() {
    let p = plan(&design("fixed", "10k", "", ""));
    let r = reading(&p, "TP1");
    assert_eq!(r.nominal, Some(3.3));
    assert!((r.min - 3.234).abs() < 1e-9 && (r.max - 3.366).abs() < 1e-9, "{r:?}");
    assert!(row(&p, "TP1").conditions.iter().any(|c| c.contains("VIN")), "{:?}", row(&p, "TP1"));
}

#[test]
fn a_divider_set_rail_matches_the_worked_corners() {
    let p = plan(&design("divider", "10k", "", ""));
    let r = reading(&p, "TP2");
    let hi = 0.808 * (1.0 + 10.1 / 7.92);
    let lo = 0.792 * (1.0 + 9.9 / 8.08);
    assert!((r.nominal.unwrap() - 1.8).abs() < 1e-6, "{r:?}");
    assert!((r.max - hi).abs() < 1e-5 && (r.min - lo).abs() < 1e-5, "{r:?} {lo} {hi}");
    let order = |n| row(&p, n).order;
    assert!(order("TP1") < order("TP2"), "3V3 is checked before the rail it feeds");
    assert!(row(&p, "TP2").source.iter().any(|s| s.kind == "datasheet"));
}

#[test]
fn a_resistor_without_a_tolerance_field_is_flagged_assumed() {
    let c = design("assumed", "10k", "", "");
    let sch = std::fs::read_to_string(c.dir.join("t.sch.toml")).unwrap().replacen(
        "fields = { tolerance = \"1%\" }",
        "",
        1,
    );
    std::fs::write(c.dir.join("t.sch.toml"), sch).unwrap();
    let p = plan(&c);
    assert!(row(&p, "TP2").source.iter().any(|s| s.kind == "assumed"));
    assert_eq!(p.assumed(), 2, "{}", p.summary);
}

#[test]
fn ground_reads_zero() {
    let p = plan(&design("ground", "10k", "", ""));
    let r = reading(&p, "TP3");
    assert_eq!(r.nominal, Some(0.0));
    assert_eq!(row(&p, "TP3").role, "ground");
    assert_eq!(row(&p, "TP3").order, 0);
}

#[test]
fn a_net_with_no_model_is_unknown_with_a_reason() {
    let p = plan(&design("unknown", "10k", "", ""));
    let r = row(&p, "TP4");
    assert_eq!(r.expect, Expect::Unknown("unknown".into()));
    assert!(r.reason.as_deref().is_some_and(|x| !x.is_empty()));
    assert_eq!(p.unknown(), 1);
    assert!(p.summary.contains("1 unknown"), "{}", p.summary);
}

#[test]
fn every_test_point_appears_once_and_the_plan_round_trips() {
    let p = plan(&design("once", "10k", "", ""));
    let mut refs: Vec<&str> = p.rows.iter().map(|r| r.reference.as_str()).collect();
    refs.sort();
    assert_eq!(refs, ["TP1", "TP2", "TP3", "TP4"]);
    let text = p.to_toml().unwrap();
    let back = Plan::parse(&text).unwrap();
    assert_eq!(back.rows.len(), 4);
    assert_eq!(back.hash, p.hash);
}

#[test]
fn changing_a_resistor_makes_the_plan_stale() {
    let a = design("stale-a", "10k", "", "");
    let plan_a = plan(&a);
    let b = design("stale-b", "12k", "", "");
    let pb = Project::load(&b.dir).unwrap();
    assert!(plan_a.is_stale(&pb.schematics[0].item));
    let pa = Project::load(&a.dir).unwrap();
    assert!(!plan_a.is_stale(&pa.schematics[0].item));
}

#[test]
fn a_sense_resistor_gives_a_current_range() {
    let c = design(
        "sense",
        "10k",
        &(part("R3", "R", "0.1", "", 177.8) + &part("TP5", "TestPoint", "TP", "", 190.5)),
        &net("LOAD", "Power", &["R3.2", "TP5.1"]),
    );
    let s = std::fs::read_to_string(c.dir.join("t.sch.toml")).unwrap().replace(
        "pins = [\"U1.3\", \"U2.1\", \"TP1.1\"]",
        "pins = [\"U1.3\", \"U2.1\", \"TP1.1\", \"R3.1\"]",
    );
    std::fs::write(c.dir.join("t.sch.toml"), s).unwrap();
    std::fs::write(
        c.dir.join("t.board.toml"),
        "name = \"t\"\n[outline]\nsize = [40, 30]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n\
         [[vias]]\nname = \"std\"\ndrill = \"0.3mm\"\ndiameter = \"0.6mm\"\n\
         [[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\nvia = \"std\"\n\
         [[netclasses]]\nname = \"Power\"\ntrack_width = \"0.5mm\"\nclearance = \"0.2mm\"\nvia = \"std\"\ncurrent = \"2A\"\n",
    )
    .unwrap();
    let p = Project::load(&c.dir).unwrap();
    let plan = testplan::build(&p.schematics[0].item, None, p.boards.first().map(|b| &b.item));
    let r = reading(&plan, "TP5");
    assert_eq!(row(&plan, "TP5").role, "current sense");
    assert_eq!((r.unit.as_str(), r.min, r.max), ("A", 0.0, 2.0), "{r:?}");
    assert!(r.basis.contains("0.1"), "{}", r.basis);
    assert!(row(&plan, "TP5").measure.contains("shunt"));
}

const XTAL: &str = "name = \"XTAL\"\nreference = \"X\"\ndescription = \"Crystal\"\n\
[[pins]]\nnumber = \"1\"\nname = \"XIN\"\ntype = \"passive\"\nat = [-5.08, 0.0]\nside = \"left\"\n\
[[pins]]\nnumber = \"2\"\nname = \"XOUT\"\ntype = \"passive\"\nat = [5.08, 0.0]\nside = \"right\"\n";

const MCU: &str = "name = \"MCU\"\nreference = \"U\"\n\
[[pins]]\nnumber = \"1\"\nname = \"PA0\"\ntype = \"bidirectional\"\nat = [-7.62, 0.0]\nside = \"left\"\n\
[[pins]]\nnumber = \"2\"\nname = \"PA1\"\ntype = \"input\"\nat = [-7.62, 2.54]\nside = \"left\"\n\
[[pins]]\nnumber = \"3\"\nname = \"SDA\"\ntype = \"bidirectional\"\nat = [7.62, 0.0]\nside = \"right\"\n";

const DIODE: &str = "name = \"ESD\"\nreference = \"D\"\n\
[[pins]]\nnumber = \"1\"\nname = \"K\"\ntype = \"passive\"\nat = [-5.08, 0.0]\nside = \"left\"\n\
[[pins]]\nnumber = \"2\"\nname = \"A\"\ntype = \"passive\"\nat = [5.08, 0.0]\nside = \"right\"\n";

fn rich(tag: &str, xfields: &str) -> Case {
    let parts = part("X1", "XTAL", "40MHz", xfields, 190.5)
        + &part("U3", "MCU", "MCU", "", 203.2)
        + &part("R4", "R", "4.7k", "tolerance = \"1%\"", 215.9)
        + &part("D1", "ESD", "ESD", "", 228.6)
        + &part("TP5", "TestPoint", "TP", "", 241.3)
        + &part("TP6", "TestPoint", "TP", "", 254.0)
        + &part("TP7", "TestPoint", "TP", "", 266.7)
        + &part("TP8", "TestPoint", "TP", "", 279.4)
        + &part("TP9", "TestPoint", "TP", "", 292.1)
        + &part("TP10", "TestPoint", "TP", "", 304.8)
        + &part("TP11", "TestPoint", "TP", "", 317.5)
        + &part("TP12", "TestPoint", "TP", "", 330.2);
    let nets = net("XTAL_OUT", "Default", &["X1.1", "TP5.1"])
        + &net("I2C_SDA", "Default", &["U3.3", "R4.2", "TP6.1"])
        + &net("GPIO1", "Default", &["U3.1", "TP7.1"])
        + &net("ESDLINE", "Default", &["D1.2", "TP8.1"])
        + &net("RF_IN", "Default", &["TP9.1"])
        + &net("SPARE", "Default", &["TP10.1", "TP11.1"])
        + &net("PA1_IN", "Default", &["U3.2", "TP12.1"]);
    let c = design(tag, "10k", &parts, &nets);
    std::fs::write(c.dir.join("symbols/XTAL.sym.toml"), XTAL).unwrap();
    std::fs::write(c.dir.join("symbols/MCU.sym.toml"), MCU).unwrap();
    std::fs::write(c.dir.join("symbols/ESD.sym.toml"), DIODE).unwrap();
    let mut s = std::fs::read_to_string(c.dir.join("t.sch.toml")).unwrap();
    s = s.replace("pins = [\"R2.2\", \"TP3.1\"]", "pins = [\"R2.2\", \"TP3.1\", \"X1.2\", \"D1.1\"]");
    s = s.replace("\"U1.3\", \"U2.1\", \"TP1.1\"", "\"U1.3\", \"U2.1\", \"TP1.1\", \"R4.1\"");
    s = s.replace("no_connect = [\"U1.2\"]", "no_connect = [\"U1.2\"]");
    std::fs::write(c.dir.join("t.sch.toml"), s).unwrap();
    c
}

#[test]
fn a_crystal_gives_frequency_and_ppm_range() {
    let p = plan(&rich("xtal", "ppm = \"20ppm\""));
    let r = reading(&p, "TP5");
    assert_eq!(row(&p, "TP5").role, "clock");
    assert_eq!((r.unit.as_str(), r.nominal), ("Hz", Some(40e6)), "{r:?}");
    assert!((r.min - 39_999_200.0).abs() < 1e-3 && (r.max - 40_000_800.0).abs() < 1e-3, "{r:?}");
    assert!(!row(&p, "TP5").source.iter().any(|s| s.kind == "assumed"));
    assert!(row(&p, "TP5").conditions.iter().any(|c| c.contains("probe")));
}

#[test]
fn a_crystal_without_ppm_is_flagged_assumed() {
    let p = plan(&rich("xtal-assumed", ""));
    assert!(row(&p, "TP5").source.iter().any(|s| s.kind == "assumed" && s.detail.contains("ppm")));
    assert!(row(&p, "TP5").is_assumed());
}

#[test]
fn an_idle_bus_reads_its_pull_up_level() {
    let p = plan(&rich("pullup", ""));
    let Expect::Logic(l) = &row(&p, "TP6").expect else { panic!("{:?}", row(&p, "TP6")) };
    assert_eq!(l.idle.as_deref(), Some("1"));
    let [lo, hi] = l.idle_volts.unwrap();
    assert!((lo - 3.234).abs() < 1e-9 && (hi - 3.366).abs() < 1e-9, "{l:?}");
    assert!(row(&p, "TP6").source.iter().any(|s| s.detail.contains("R4")));
}

#[test]
fn a_firmware_net_names_its_pins() {
    let p = plan(&rich("firmware", ""));
    let r = row(&p, "TP7");
    assert_eq!(r.expect, Expect::Unknown(FIRMWARE.into()));
    assert!(r.reason.as_deref().unwrap().contains("U3.1 PA0"), "{r:?}");
    assert!(p.summary.contains("firmware"), "{}", p.summary);
    assert_eq!(p.firmware(), 2, "{}", p.summary);
}

#[test]
fn a_logic_sim_expect_gives_the_simulated_level() {
    let c = rich("logicsim", "");
    std::fs::write(
        c.dir.join("t.sim.toml"),
        "name = \"tl\"\nkind = \"logic\"\nschematic = \"t\"\nduration = \"1us\"\n\
         [[stimulus]]\nnet = \"GPIO1\"\nconstant = 1\n\
         [[expect]]\nnet = \"GPIO1\"\nat = \"500ns\"\nvalue = 1\n",
    )
    .unwrap();
    let project = Project::load(&c.dir).unwrap();
    let sch = &project.schematics[0].item;
    let models = Models::from_project(&project, sch);
    assert_eq!(models.logic.len(), 1);
    let p = testplan::build_with(sch, None, None, &models);
    let Expect::Logic(l) = &row(&p, "TP7").expect else { panic!("{:?}", row(&p, "TP7")) };
    assert_eq!(l.states.len(), 1);
    assert_eq!(l.states[0].level, "1");
    assert!(l.states[0].when.contains("tl"), "{l:?}");
    assert!(row(&p, "TP7").source.iter().any(|s| s.kind == "simulation"));
}

#[test]
fn receiver_thresholds_come_from_the_pin_levels() {
    let c = rich("thresholds", "");
    let mcu = MCU.replace(
        "name = \"PA1\"\ntype = \"input\"",
        "name = \"PA1\"\ntype = \"input\"\nlevels = { vih = \"2.0V\", vil = \"0.8V\" }",
    );
    std::fs::write(c.dir.join("symbols/MCU.sym.toml"), mcu).unwrap();
    let s = std::fs::read_to_string(c.dir.join("t.sch.toml")).unwrap().replace(
        "pins = [\"U3.2\", \"TP12.1\"]",
        "pins = [\"U3.2\", \"TP12.1\", \"R4.2\"]",
    );
    std::fs::write(c.dir.join("t.sch.toml"), s).unwrap();
    let p = plan(&c);
    let Expect::Logic(l) = &row(&p, "TP12").expect else { panic!("{:?}", row(&p, "TP12")) };
    assert_eq!((l.vih, l.vil), (Some(2.0), Some(0.8)), "{l:?}");
}

#[test]
fn an_rf_net_takes_its_level_from_the_rf_result_with_a_probe_caveat() {
    let c = rich("rf", "");
    let project = Project::load(&c.dir).unwrap();
    let models = Models {
        logic: Vec::new(),
        rf: vec![testplan::RfModel {
            sim: "rf".into(),
            verified: true,
            links: vec![testplan::RfLink {
                from_port: "IN".into(),
                to_port: "OUT".into(),
                from_net: "VIN".into(),
                to_net: "RF_IN".into(),
                db_min: -3.5,
                db_max: -0.5,
                f_min: 70e6,
                f_max: 6e9,
            }],
        }],
    };
    let p = testplan::build_with(&project.schematics[0].item, None, None, &models);
    let r = reading(&p, "TP9");
    assert_eq!((r.unit.as_str(), r.min, r.max), ("dB", -3.5, -0.5), "{r:?}");
    assert_eq!(row(&p, "TP9").role, "RF");
    assert!(row(&p, "TP9").conditions.iter().any(|c| c.contains("probe")));
    assert!(row(&p, "TP9").source.iter().any(|s| s.kind == "simulation"));
}

#[test]
fn an_rf_net_without_a_result_is_unknown_and_says_not_to_probe() {
    let p = plan(&rich("rf-none", ""));
    let r = row(&p, "TP9");
    assert_eq!(r.expect, Expect::Unknown("unknown".into()));
    assert!(r.reason.as_deref().unwrap().contains("probe"), "{r:?}");
}

#[test]
fn the_unpowered_list_has_rail_resistance_continuity_and_diode_drop() {
    let p = plan(&rich("unpowered", ""));
    let kinds = |k: &str| p.checks.iter().filter(|c| c.kind == k).collect::<Vec<_>>();
    let rail = kinds("resistance to ground");
    assert!(rail.iter().any(|c| c.a == "TP1" && c.b == "TP3" && c.min > 0.0 && c.max.is_none()));
    assert!(rail.iter().any(|c| c.a == "TP2"), "{rail:?}");
    let tie = kinds("continuity");
    assert_eq!(tie.len(), 1, "{tie:?}");
    assert_eq!((tie[0].a.as_str(), tie[0].b.as_str()), ("TP10", "TP11"));
    assert_eq!(tie[0].max, Some(testplan::ASSUMED_CONTINUITY_OHMS));
    let diode = kinds("diode drop");
    assert_eq!(diode.len(), 1, "{diode:?}");
    assert_eq!((diode[0].a.as_str(), diode[0].b.as_str()), ("TP8", "TP3"));
    assert!(diode[0].is_assumed());
    assert!(p.summary.contains("unpowered checks"), "{}", p.summary);
    let back = Plan::parse(&p.to_toml().unwrap()).unwrap();
    assert_eq!(back.checks, p.checks);
}

#[test]
fn a_declared_forward_voltage_sets_the_diode_range() {
    let c = rich("vf", "");
    let s = std::fs::read_to_string(c.dir.join("t.sch.toml"))
        .unwrap()
        .replace("symbol = \"ESD\"\nvalue = \"ESD\"", "symbol = \"ESD\"\nvalue = \"ESD\"\nfields = { vf = \"0.5V\" }")
        .replace("at = [228.6, 25.4]\nfields = { vf = \"0.5V\" }\n", "at = [228.6, 25.4]\n");
    std::fs::write(c.dir.join("t.sch.toml"), s).unwrap();
    let p = plan(&c);
    let d = p.checks.iter().find(|c| c.kind == "diode drop").unwrap();
    assert!(!d.is_assumed(), "{d:?}");
    assert!((d.min - 0.4).abs() < 1e-9 && (d.max.unwrap() - 0.6).abs() < 1e-9, "{d:?}");
}

fn example(name: &str, file: &str) -> Plan {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
    let p = Project::load(&dir).unwrap();
    let sch = p.schematics.iter().find(|s| s.name == file).unwrap();
    let plan = testplan::build(&sch.item, None, None);
    eprintln!("{name}: {}", plan.summary);
    plan
}

#[test]
fn the_hackrf_pro_example_lists_every_test_point_once() {
    let plan = example("hackrf-pro", "praline");
    let mut refs: Vec<&str> = plan.rows.iter().map(|r| r.reference.as_str()).collect();
    let total = refs.len();
    refs.sort();
    refs.dedup();
    assert_eq!(refs.len(), total);
    assert_eq!(total, 66);
    assert!(plan.summary.contains("66 test points"), "{}", plan.summary);
    assert!(plan.assumed() >= 10, "{}", plan.summary);
    for r in plan.rows.iter().filter(|r| r.role == "power rail") {
        if let Expect::Reading(x) = &r.expect {
            assert!(x.max > x.min, "{} {x:?}", r.reference);
        }
    }
}

#[test]
fn the_sdr_example_has_no_test_points_and_an_empty_plan() {
    let plan = example("sdr", "sdr");
    assert_eq!(plan.summary, "0 test points, 0 unknown, 0 assumed");
}

fn with_rail(tag: &str, rails: &str) -> Plan {
    let c = design(
        tag,
        "10k",
        &(part("R3", "R", "1k", "", 177.8) + &part("TP5", "TestPoint", "TP", "", 190.5)),
        &net("5V0", "Default", &["R3.1", "TP5.1"]),
    );
    let s = std::fs::read_to_string(c.dir.join("t.sch.toml")).unwrap();
    let s = s.replacen("\n", &format!("\n{rails}\n"), 1);
    std::fs::write(c.dir.join("t.sch.toml"), s).unwrap();
    plan(&c)
}

#[test]
fn a_zero_width_rail_gets_the_assumed_band() {
    let p = with_rail("zero", "rails = { \"5V0\" = 5.0 }");
    let r = reading(&p, "TP5");
    assert_eq!(r.nominal, Some(5.0));
    assert!((r.min - 4.75).abs() < 1e-9 && (r.max - 5.25).abs() < 1e-9, "{r:?}");
    assert!(r.basis.contains("assumed 5%"), "{}", r.basis);
    let s = &row(&p, "TP5").source;
    assert!(s.iter().all(|x| x.confidence == "assumed"), "{s:?}");
}

#[test]
fn a_declared_rail_range_keeps_its_band() {
    let p = with_rail("declared", "rails = { \"5V0\" = [4.75, 5.25] }");
    let r = reading(&p, "TP5");
    assert!((r.min - 4.75).abs() < 1e-9 && (r.max - 5.25).abs() < 1e-9, "{r:?}");
    let s = &row(&p, "TP5").source;
    assert!(s.iter().all(|x| x.confidence == "declared"), "{s:?}");
}

#[test]
fn a_regulator_rail_keeps_its_accuracy_over_the_rails_table() {
    let c = design("regkeep", "10k", "", "");
    let s = std::fs::read_to_string(c.dir.join("t.sch.toml")).unwrap();
    let s = s.replacen("\n", "\nrails = { \"3V3\" = 3.3 }\n", 1);
    std::fs::write(c.dir.join("t.sch.toml"), s).unwrap();
    let r = reading(&plan(&c), "TP1");
    assert!((r.min - 3.234).abs() < 1e-9 && (r.max - 3.366).abs() < 1e-9, "{r:?}");
}

#[test]
fn the_bench_example_covers_regulator_divider_shunt_and_unknown() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/testplan");
    let p = Project::load(&dir).unwrap();
    let plan = testplan::build(&p.schematics[0].item, None, p.boards.first().map(|b| &b.item));
    assert_eq!(plan.rows.len(), 5, "{}", plan.summary);
    assert_eq!(plan.unknown(), 1);
    assert_eq!(row(&plan, "TP5").role, "current sense");
    assert_eq!(reading(&plan, "TP1").nominal, Some(3.3));
}

fn rules(c: &Case) -> Vec<String> {
    let p = Project::load(&c.dir).unwrap();
    p.diagnostics().iter().filter_map(|d| d.rule.clone()).collect()
}

#[test]
fn check_reports_a_missing_then_a_stale_plan() {
    let c = design("rules", "10k", "", "");
    let missing: Vec<String> =
        rules(&c).into_iter().filter(|r| r.starts_with("testplan")).collect();
    assert_eq!(missing, ["testplan-missing"]);
    let p = plan(&c);
    std::fs::write(c.dir.join("t.testplan.toml"), p.to_toml().unwrap()).unwrap();
    assert!(!rules(&c).iter().any(|r| r.starts_with("testplan")));
    let sch = std::fs::read_to_string(c.dir.join("t.sch.toml")).unwrap();
    std::fs::write(c.dir.join("t.sch.toml"), sch.replacen("\"10k\"", "\"12k\"", 1)).unwrap();
    let stale: Vec<String> = rules(&c).into_iter().filter(|r| r.starts_with("testplan")).collect();
    assert_eq!(stale, ["testplan-stale"]);
}

fn bench_plan() -> Plan {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/testplan");
    let p = Project::load(&dir).unwrap();
    testplan::build(&p.schematics[0].item, None, p.boards.first().map(|b| &b.item))
}

fn measured(rows: &[(&str, f64)]) -> Vec<(String, Option<f64>)> {
    rows.iter().map(|(r, v)| (r.to_string(), Some(*v))).collect()
}

#[test]
fn all_readings_in_range_pass() {
    let plan = bench_plan();
    let c = plan.compare(&measured(&[("TP1", 3.3), ("TP3", 0.0)]));
    assert_eq!(c.failed(), 0, "{}", c.to_text());
    assert!(c.ok());
    assert_eq!(c.passed(), 2);
}

#[test]
fn one_reading_out_of_range_reports_that_pad_and_the_difference() {
    let plan = bench_plan();
    let c = plan.compare(&measured(&[("TP1", 3.5), ("TP3", 0.0)]));
    assert_eq!(c.failed(), 1);
    assert!(!c.ok());
    let bad = c.rows.iter().find(|r| r.reference == "TP1").unwrap();
    assert_eq!(bad.verdict, testplan::Verdict::Fail);
    assert!((bad.off_by - 0.134).abs() < 1e-9, "{}", bad.off_by);
    assert!((bad.from_nominal.unwrap() - 0.2).abs() < 1e-9);
    let text = c.to_text();
    assert!(text.contains("FAIL TP1") && text.contains("0.134 V above"), "{text}");
    assert!(!text.contains("FAIL TP3"), "{text}");
}

#[test]
fn a_reading_below_the_range_is_negative_and_unknown_pads_are_skipped() {
    let plan = bench_plan();
    let c = plan.compare(&measured(&[("TP1", 3.0), ("TP9", 1.0)]));
    let bad = c.rows.iter().find(|r| r.reference == "TP1").unwrap();
    assert!(bad.off_by < 0.0);
    assert_eq!(c.unmatched, ["TP9"]);
    assert!(c.to_text().contains("below"));
}

#[test]
fn the_measured_csv_reads_the_ref_and_measured_columns() {
    let rows = testplan::parse_measured("order,ref,measured\n1,TP1,3.31\n2,\"TP2\",\n").unwrap();
    assert_eq!(rows, [("TP1".to_string(), Some(3.31)), ("TP2".to_string(), None)]);
    assert!(testplan::parse_measured("ref,measured\nTP1,abc\n").is_err());
    assert!(testplan::parse_measured("ref,volts\n").is_err());
}
