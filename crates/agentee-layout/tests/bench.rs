use agentee_core::Severity;
use agentee_core::project::Project;
use agentee_layout::start::Reset;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let path = e.unwrap().path();
        let name = path.file_name().unwrap();
        if path.is_dir() {
            if name != "3dmodels" {
                copy(&path, &to.join(name));
            }
        } else if path.extension().is_some_and(|x| x == "toml") {
            std::fs::copy(&path, to.join(name)).unwrap();
        }
    }
}

fn fresh(example: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let k = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("agentee-bench-{}-{example}-{k}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(example), &dir);
    dir
}

pub struct Outcome {
    pub layout: agentee_core::layout::Layout,
    pub hand: agentee_core::layout::Layout,
    pub routed: usize,
    pub connections: usize,
    pub vias: usize,
    pub unrouted: usize,
    pub drc: Vec<String>,
    pub score: f64,
    pub seconds: f64,
    pub text: String,
}

fn bench(example: &str, layout: &str, from: Option<&str>) -> Outcome {
    bench_with(example, layout, from, &[])
}

fn bench_with(example: &str, layout: &str, from: Option<&str>, rules: &[(&str, f64)]) -> Outcome {
    let dir = fresh(example);
    for e in std::fs::read_dir(&dir).unwrap() {
        let path = e.unwrap().path();
        if rules.is_empty() || !path.to_string_lossy().ends_with(".board.toml") {
            continue;
        }
        let set: String = rules.iter().map(|(k, v)| format!("{k} = {v}\n")).collect();
        let text = std::fs::read_to_string(&path).unwrap().replacen(
            "[rules]\n",
            &format!("[rules]\n{set}"),
            1,
        );
        std::fs::write(&path, text).unwrap();
    }
    let p = Project::load(&dir).unwrap();
    let i = p.layouts.iter().position(|l| l.item.name == layout).unwrap();
    let inputs = p.layout_inputs(i).unwrap();
    let text = std::fs::read_to_string(&p.layouts[i].path).unwrap();
    let hand_entry = inputs.resolve(&text).unwrap();
    let errors = |diags: &[agentee_core::Diagnostic]| -> std::collections::BTreeMap<String, usize> {
        let mut m = std::collections::BTreeMap::new();
        for d in diags.iter().filter(|d| d.severity == Severity::Error) {
            *m.entry(d.rule.clone().unwrap_or_default()).or_insert(0) += 1;
        }
        m
    };
    let hand_errors = errors(&hand_entry.diags);
    let hand = hand_entry.item;
    let text = agentee_layout::start::reset(&text, &Reset { routing: true, ..Default::default() })
        .unwrap();
    let run = agentee_layout::Run {
        from: from.map(str::to_string),
        to: None,
        only: None,
        watch: None,
        stop: None,
    };
    let t0 = Instant::now();
    let r = agentee_layout::run_text(&inputs, &text, &run).unwrap();
    let seconds = t0.elapsed().as_secs_f64();
    let e = inputs.resolve(&r.text).unwrap();
    let drc: Vec<String> = e
        .diags
        .iter()
        .filter(|d| d.severity == Severity::Error && d.rule.as_deref() != Some("unrouted"))
        .map(|d| format!("{}: {}", d.at, d.message))
        .collect();
    let unrouted: usize = e.item.nets.iter().map(|n| n.unrouted).sum();
    let detail = r.detail.clone().unwrap_or_default();
    eprintln!("\n== {example} {} ==", if from.is_some() { "route only" } else { "full flow" });
    for ph in &r.phases {
        eprintln!(
            "{:<12} {:>7} ms  score {:>9.1}  {} failed",
            ph.phase,
            ph.ms,
            ph.score.as_ref().map(|s| s.total).unwrap_or(0.0),
            ph.failed.len()
        );
        for n in &ph.notes {
            eprintln!("    {n}");
        }
        for f in ph.failed.iter().take(8) {
            eprintln!("    failed: {f}");
        }
    }
    eprintln!(
        "{} of {} connections, {} vias, {unrouted} unrouted, {} DRC errors, score {:.1}, {seconds:.1} s",
        detail.routed,
        detail.connections,
        detail.vias.len(),
        drc.len(),
        r.score.total
    );
    eprint!("{}", r.time.table());
    eprintln!("errors by rule {:?}", errors(&e.diags));
    eprintln!("hand layout     {hand_errors:?}");
    for d in drc.iter().filter(|d| !d.starts_with("silk")).take(10) {
        eprintln!("    {d}");
    }
    if let Ok(out) = std::env::var("AGENTEE_BENCH_OUT") {
        let tag = if from.is_some() { "route" } else { "full" };
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(Path::new(&out).join(format!("{example}-{tag}.pcb.toml")), &r.text).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
    Outcome {
        layout: e.item,
        hand,
        routed: detail.routed,
        connections: detail.connections,
        vias: detail.vias.len(),
        unrouted,
        drc,
        score: r.score.total,
        seconds,
        text: r.text,
    }
}

fn pad(l: &agentee_core::layout::Layout, r: &str, n: &str) -> [f64; 2] {
    let p = l.parts.iter().find(|p| p.reference == r).unwrap();
    let q = p.pads.iter().find(|q| q.number == n).unwrap();
    let mut b = agentee_core::graphic::Bounds::EMPTY;
    q.outlines.iter().flatten().for_each(|x| b.add(*x));
    b.center()
}

fn length(l: &agentee_core::layout::Layout, net: &str) -> f64 {
    l.nets.iter().find(|n| n.name == net).map(|n| n.length_mm).unwrap_or(0.0)
}

#[test]
fn lna_route_only() {
    let o = bench("lna", "lna", Some("access"));
    assert_eq!(o.routed, o.connections);
    assert_eq!(o.unrouted, 0);
    assert!(o.drc.is_empty(), "{:?}", o.drc);
    for net in ["RF_IN", "RF_AMP_IN", "RF_AMP_OUT", "RF_OUT"] {
        let (got, hand) = (length(&o.layout, net), length(&o.hand, net));
        eprintln!("{net}: {got:.2} mm, hand {hand:.2} mm");
        assert!(got <= hand * 1.1 + 0.05, "{net} is {got:.2} mm, the hand layout {hand:.2} mm");
    }
}

#[test]
fn lna_full_flow() {
    let o = bench("lna", "lna", None);
    assert_eq!(o.routed, o.connections);
    assert_eq!(o.unrouted, 0);
    assert!(o.drc.is_empty(), "{:?}", o.drc);
    let l = &o.layout;
    let line = [
        pad(l, "J1", "1"),
        pad(l, "C1", "1"),
        pad(l, "C1", "2"),
        pad(l, "U1", "1"),
        pad(l, "U1", "3"),
        pad(l, "C2", "1"),
        pad(l, "C2", "2"),
    ];
    eprintln!("RF line {line:?}, J2.1 {:?}", pad(l, "J2", "1"));
    for q in &line {
        assert!((q[1] - line[0][1]).abs() < 0.01, "the RF chain is off the line: {line:?}");
    }
}

#[test]
#[ignore = "takes minutes, run with --include-ignored"]
fn sdr_route_only() {
    let o = bench("sdr", "sdr", Some("access"));
    assert!(o.connections > 0);
}

#[test]
#[ignore = "takes minutes, run with --include-ignored"]
fn sdr_full_flow() {
    let o = bench("sdr", "sdr", None);
    assert!(o.connections > 0);
}

#[test]
#[ignore = "takes minutes, run with --include-ignored"]
fn praline_route_only() {
    let o =
        bench_with("hackrf-pro", "praline", Some("access"), &[("min_via_hole_to_copper", 0.18)]);
    assert!(o.connections > 0);
}
