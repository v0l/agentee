use std::path::{Path, PathBuf};
use std::process::Command;

fn copy_praline(tag: &str) -> PathBuf {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hackrf-pro");
    let dir = std::env::temp_dir().join(format!("agentee-cli-plan-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in ["", "symbols", "footprints"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
        for e in std::fs::read_dir(from.join(sub)).unwrap() {
            let path = e.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if name.ends_with(".toml") && !name.ends_with(".sim.toml") {
                std::fs::copy(&path, dir.join(sub).join(name)).unwrap();
            }
        }
    }
    dir
}

fn agentee(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_agentee")).args(args).current_dir(dir).output().unwrap()
}

#[test]
fn testplan_writes_the_file_and_prints_md_and_csv() {
    let dir = copy_praline("cli");
    let missing = agentee(&dir, &["check", "--info"]);
    assert!(String::from_utf8_lossy(&missing.stdout).contains("testplan-missing"));

    let out = agentee(&dir, &["testplan", "praline", "--md"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let md = String::from_utf8(out.stdout).unwrap();
    assert!(md.contains("# Test plan: praline") && md.contains("| [ ] |"), "{md}");
    assert!(dir.join("praline.testplan.toml").exists());

    let csv = agentee(&dir, &["testplan", "praline", "--csv"]);
    let csv = String::from_utf8(csv.stdout).unwrap();
    assert!(csv.starts_with("order,ref,net,"), "{csv}");
    assert!(csv.lines().count() > 1);

    let fresh = agentee(&dir, &["check", "--info"]);
    let text = String::from_utf8_lossy(&fresh.stdout);
    assert!(!text.contains("testplan-missing") && !text.contains("testplan-stale"), "{text}");

    let sch = dir.join("praline.sch.toml");
    let src = std::fs::read_to_string(&sch).unwrap();
    let at = src.find("value = \"").map(|i| i + 9).unwrap();
    let end = at + src[at..].find('"').unwrap();
    std::fs::write(&sch, format!("{}changed{}", &src[..at], &src[end..])).unwrap();
    let stale = agentee(&dir, &["check", "--info"]);
    assert!(String::from_utf8_lossy(&stale.stdout).contains("testplan-stale"));
}

#[test]
fn measured_compares_a_csv_with_the_plan_and_exits_nonzero_on_a_failure() {
    let dir = copy_praline("measured");
    agentee(&dir, &["testplan", "praline"]);
    let csv = String::from_utf8(agentee(&dir, &["testplan", "praline", "--csv"]).stdout).unwrap();
    let ok = dir.join("ok.csv");
    std::fs::write(&ok, "ref,measured\nTP1,\n").unwrap();
    let out = agentee(&dir, &["testplan", "praline", "--measured", "ok.csv"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(csv.lines().count() > 1);

    let plan = std::fs::read_to_string(dir.join("praline.testplan.toml")).unwrap();
    let plan = agentee_core::testplan::Plan::parse(&plan).unwrap();
    let row = plan
        .rows
        .iter()
        .find(|r| matches!(&r.expect, agentee_core::testplan::Expect::Reading(_)))
        .unwrap();
    let agentee_core::testplan::Expect::Reading(x) = &row.expect else { unreachable!() };
    let bad = dir.join("bad.csv");
    std::fs::write(&bad, format!("ref,measured\n{},{}\n", row.reference, x.max + 1.0)).unwrap();
    let out = agentee(&dir, &["testplan", "praline", "--measured", "bad.csv"]);
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains(&format!("FAIL {}", row.reference)) && text.contains("above"), "{text}");
}
