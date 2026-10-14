use agentee_core::logic::{Cell, MarkKind, Trace, Wave};
use agentee_sim::logic::run;
use sims_logic::Level::{H, L};

type VcdVar = (usize, String, Vec<(u64, String)>);

fn read_vcd(text: &str) -> Vec<VcdVar> {
    let mut vars: Vec<(String, VcdVar)> = Vec::new();
    let mut time = 0;
    let mut words = text.split_whitespace().peekable();
    while let Some(w) = words.next() {
        if w == "$var" {
            let (_, width, id) = (words.next(), words.next().unwrap(), words.next().unwrap());
            let mut reference = Vec::new();
            for r in words.by_ref() {
                if r == "$end" {
                    break;
                }
                reference.push(r);
            }
            let var = (width.parse().unwrap(), reference.join(" "), Vec::new());
            vars.push((id.to_string(), var));
        } else if let Some(t) = w.strip_prefix('#') {
            time = t.parse().unwrap();
        } else if let Some(bits) = w.strip_prefix('b') {
            let id = words.next().unwrap();
            let v = vars.iter_mut().find(|v| v.0 == id).unwrap();
            v.1.2.push((time, bits.to_string()));
        } else if !w.starts_with('$') && w.len() > 1 {
            let (level, id) = w.split_at(1);
            if let Some(v) = vars.iter_mut().find(|v| v.0 == id) {
                v.1.2.push((time, level.to_string()));
            }
        }
    }
    vars.into_iter().map(|v| v.1).collect()
}

fn assert_vectors_match_bits(vars: &[VcdVar], traces: &[Trace], names: &[(&str, &[&str])]) {
    for (reference, bits) in names {
        let (width, _, changes) = vars.iter().find(|v| v.1 == *reference).unwrap();
        assert_eq!(*width, bits.len(), "{reference}");
        let bits: Vec<&Trace> =
            bits.iter().map(|b| traces.iter().find(|t| t.name == *b).unwrap()).collect();
        let mut times: Vec<u64> = bits.iter().flat_map(|t| t.times.clone()).collect();
        times.push(0);
        times.sort_unstable();
        times.dedup();
        assert_eq!(changes.len(), times.len(), "{reference}");
        for (time, value) in changes {
            let want: String = bits.iter().map(|t| level_at(t, *time)).collect();
            assert_eq!(*value, want, "{reference} at {time}");
        }
    }
}

fn level_at(t: &Trace, time: u64) -> char {
    match t.times.partition_point(|x| *x <= time) {
        0 => 'z',
        n => t.values.chars().nth(n - 1).unwrap_or('x'),
    }
}

#[test]
fn i2c_example_pulls_up_acks_and_catches_a_late_latch() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logic");
    let p = agentee_core::Project::load(&root).unwrap();
    let s = p.sims.iter().find(|s| s.name == "i2c").unwrap();
    let spec = s.item.logic.as_ref().unwrap();
    let drivers: Vec<&Cell> = spec
        .circuit
        .cells
        .iter()
        .filter(|c| ["U3", "U4", "U5"].contains(&c.part.as_str()))
        .collect();
    assert_eq!(drivers.len(), 3);
    assert!(drivers.iter().all(|c| c.open_drain));
    let (res, text) = run(spec, "i2c", 0, "i2c.vcd");
    let rx = ["RX7", "RX6", "RX5", "RX4", "RX3", "RX2", "RX1", "RX0"];
    assert_vectors_match_bits(&read_vcd(&text), &res.traces, &[("RX [7:0]", &rx)]);
    assert_eq!(res.failures, Vec::<String>::new());
    assert_eq!(res.passed, spec.expects.len());
    assert!(res.marks.is_empty());
    assert_eq!(res.buses[0].name, "RX");
    let mut floating = spec.clone();
    floating.circuit.cells.retain(|c| c.part != "R3");
    let (res, _) = run(&floating, "i2c", 0, "i2c.vcd");
    assert!(
        res.failures.iter().any(|f| f.contains("expected 1 at 2.4us, got z")),
        "{:?}",
        res.failures
    );
    let mut late = spec.clone();
    let rck = late.stimuli.iter_mut().find(|x| x.name == "RCK").unwrap();
    rck.wave = Wave::Steps(vec![(0, L), (8_505_000, H), (8_700_000, L)]);
    let (res, _) = run(&late, "i2c", 0, "i2c.vcd");
    assert!(
        res.failures[0].starts_with("setup: U6 CLK edge 1ns before the LATCH edge at 8.505us"),
        "{:?}",
        res.failures
    );
    let timing = res.marks.iter().find(|m| m.kind == MarkKind::Timing).unwrap();
    assert_eq!(timing.nets, vec!["SCL".to_string(), "RCK".into()]);
    let failed = res.marks.iter().find(|m| m.kind == MarkKind::Assertion).unwrap();
    assert_eq!(failed.time, 9_000_000);
    assert!(failed.text.contains("got xxxxxxxx"), "{}", failed.text);
}

#[test]
fn logic_example_counts_and_decodes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logic");
    let p = agentee_core::Project::load(&root).unwrap();
    let s = p.sims.iter().find(|s| s.name == "counter").unwrap();
    let spec = s.item.logic.as_ref().unwrap();
    let (res, text) = run(spec, "counter", 0, "counter.vcd");
    assert_eq!(res.failures, Vec::<String>::new());
    assert_eq!(res.passed, spec.expects.len());
    let ys = ["Y7_N", "Y6_N", "Y5_N", "Y4_N", "Y3_N", "Y2_N", "Y1_N", "Y0_N"];
    assert_vectors_match_bits(
        &read_vcd(&text),
        &res.traces,
        &[("Q [3:0]", &["Q3", "Q2", "Q1", "Q0"]), ("Y_N [7:0]", &ys)],
    );
    let mut broken = spec.clone();
    broken.circuit.cells.retain(|c| c.part != "R1");
    let (res, _) = run(&broken, "counter", 0, "counter.vcd");
    assert!(res.passed < spec.expects.len());
}
