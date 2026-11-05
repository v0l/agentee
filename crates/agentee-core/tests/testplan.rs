use agentee_core::Project;
use agentee_core::testplan::{self, Expect, Plan};
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
}

#[test]
fn the_sdr_example_has_a_plan_with_a_summary() {
    let plan = example("sdr", "sdr");
    assert!(plan.summary.contains("test points"));
}
