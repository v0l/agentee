use super::defaults::{JSTD020_MAX_TIME_25_TO_PEAK_S, JSTD020_TP_WINDOW_C};
use super::inputs::PartInput;
use super::limits::{self, Resolved};
use super::oven::{self, OvenPlan};
use super::passes::{self, PassPlan};
use super::paste::{self, Paste};
use super::thermal::{self, Board, Geometry};
use super::{AMBIENT_C, Capable, Finding, OvenFile, OvenKind, ReflowFile, Termination};
use crate::diag::Severity;
use serde::Serialize;

pub const DT_S: f64 = 0.5;
pub const PEAK_STEP_C: f64 = 1.0;
pub const DWELL_STEP_S: f64 = 4.0;
pub const SOAK_END_COOL_C: f64 = 60.0;
pub const SAC_BALL_SOLIDUS_C: f64 = 217.0;

pub const MODEL_BASIS: &[&str] = &[
    "Each part is a first-order lag behind the board, the board a first-order lag behind the oven air (two-node model).",
    "Lag time constants come from heat capacity over convective conductance, using package volume, FR-4 and copper heat capacity and an effective heat transfer coefficient per oven kind (thermal.rs constants, override with [oven] h_eff).",
    "Pass and floor-life rules follow J-STD-020 and J-STD-033; the pad-area hold rule is the 30 g per square inch of pad rule of thumb for parts left hanging on the second pass.",
    "Constants are estimates for ranking and tolerance: confirm with a thermocouple run on the first board.",
];

#[derive(Clone, Debug, Serialize)]
pub struct Board3 {
    pub thickness_mm: f64,
    pub copper_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Stage {
    pub name: String,
    pub start_s: f64,
    pub end_s: f64,
    pub start_c: f64,
    pub end_c: f64,
    pub rate_c_s: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PartResult {
    pub reference: String,
    pub tau_s: f64,
    pub peak_c: f64,
    pub time_above_liquidus_s: f64,
    pub time_at_peak_s: f64,
    pub max_ramp_up_c_s: f64,
    pub max_ramp_down_c_s: f64,
    pub limit_peak_c: f64,
    pub hot_margin_c: f64,
    pub cold_margin_c: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Series {
    pub time_s: Vec<f64>,
    pub oven_c: Vec<f64>,
    pub board_c: Vec<f64>,
    pub coldest_c: Vec<f64>,
    pub hottest_c: Vec<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Profile {
    pub oven_peak_c: f64,
    pub board_peak_c: f64,
    pub dwell_s: f64,
    pub ramp_up_c_s: f64,
    pub ramp_down_c_s: f64,
    pub preheat_c: [f64; 2],
    pub soak_s: f64,
    pub peak_window_c: [f64; 2],
    pub liquidus_c: f64,
    pub required_joint_peak_c: f64,
    pub stages: Vec<Stage>,
    pub parts: Vec<PartResult>,
    pub limiter: String,
    pub limiter_limit_c: f64,
    pub limiter_margin_c: f64,
    pub coldest_joint: String,
    pub coldest_joint_peak_c: f64,
    pub coldest_margin_c: f64,
    pub hot_cold_spread_c: f64,
    pub tolerance_c: f64,
    pub time_to_peak_s: f64,
    pub feasible: bool,
    #[serde(skip)]
    pub series: Series,
}

#[derive(Clone, Debug, Serialize)]
pub struct Option_ {
    pub action: String,
    pub detail: String,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Conflict {
    pub message: String,
    pub parts: Vec<String>,
    pub options: Vec<Option_>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Thermocouple {
    pub reference: String,
    pub why: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub reflow: String,
    pub layout: String,
    pub paste: Paste,
    pub parts: Vec<Resolved>,
    pub passes: PassPlan,
    pub profile: Option<Profile>,
    pub conflict: Option<Conflict>,
    pub oven: Option<OvenPlan>,
    pub thermocouples: Vec<Thermocouple>,
    pub model: Vec<String>,
    pub findings: Vec<Finding>,
    pub ok: bool,
}

#[derive(Clone)]
pub struct Node {
    pub reference: String,
    pub limits: limits::Limits,
    pub geometry: Geometry,
}

struct Shape {
    ramp_up: f64,
    ramp_down: f64,
    preheat: [f64; 2],
    soak_s: f64,
}

fn drive(shape: &Shape, peak: f64, dwell: f64) -> (Vec<f64>, Vec<Stage>) {
    let mut stages: Vec<Stage> = Vec::new();
    let mut t = 0.0;
    let mut at = AMBIENT_C;
    let mut push = |name: &str, to: f64, dur: f64, t: &mut f64, at: &mut f64| {
        stages.push(Stage {
            name: name.into(),
            start_s: *t,
            end_s: *t + dur,
            start_c: *at,
            end_c: to,
            rate_c_s: if dur > 0.0 { (to - *at) / dur } else { 0.0 },
        });
        *t += dur;
        *at = to;
    };
    push(
        "preheat ramp",
        shape.preheat[0],
        (shape.preheat[0] - at) / shape.ramp_up,
        &mut t,
        &mut at,
    );
    push("soak", shape.preheat[1], shape.soak_s, &mut t, &mut at);
    push("ramp to peak", peak, (peak - at) / shape.ramp_up, &mut t, &mut at);
    push("peak dwell", peak, dwell, &mut t, &mut at);
    push("cooling", SOAK_END_COOL_C, (peak - SOAK_END_COOL_C) / shape.ramp_down, &mut t, &mut at);
    let end = t;
    let n = (end / DT_S).ceil() as usize + 1;
    let series = (0..n)
        .map(|i| {
            let x = i as f64 * DT_S;
            stages
                .iter()
                .find(|s| x <= s.end_s + 1e-9)
                .map(|s| {
                    if s.end_s > s.start_s {
                        s.start_c
                            + (s.end_c - s.start_c)
                                * ((x - s.start_s) / (s.end_s - s.start_s)).clamp(0.0, 1.0)
                    } else {
                        s.end_c
                    }
                })
                .unwrap_or(SOAK_END_COOL_C)
        })
        .collect();
    (series, stages)
}

struct Measure {
    peak: f64,
    tal: f64,
    tp: f64,
    up: f64,
    down: f64,
}

fn measure(series: &[f64], liquidus: f64) -> Measure {
    let peak = series.iter().cloned().fold(f64::MIN, f64::max);
    let mut tal = 0.0;
    let mut tp = 0.0;
    let mut up: f64 = 0.0;
    let mut down: f64 = 0.0;
    for w in series.windows(2) {
        if w[0] >= liquidus && w[1] >= liquidus {
            tal += DT_S;
        } else if w[0] >= liquidus || w[1] >= liquidus {
            let (lo, hi) = (w[0].min(w[1]), w[0].max(w[1]));
            tal += DT_S * (hi - liquidus) / (hi - lo);
        }
        if w[0] >= peak - JSTD020_TP_WINDOW_C && w[1] >= peak - JSTD020_TP_WINDOW_C {
            tp += DT_S;
        }
        let slope = (w[1] - w[0]) / DT_S;
        up = up.max(slope);
        down = down.max(-slope);
    }
    Measure { peak, tal, tp, up, down }
}

struct Candidate {
    peak: f64,
    dwell: f64,
    soak: f64,
    cold: f64,
    hot: f64,
    time_slack: f64,
    violations: u32,
    spread: f64,
}

impl Candidate {
    fn score(&self) -> (i64, i64) {
        let m = self.cold.min(self.hot);
        ((m * 10.0).round() as i64, (self.time_slack * 10.0).round() as i64)
    }

    fn feasible(&self) -> bool {
        self.violations == 0
    }
}

struct Spec<'a> {
    paste: &'a Paste,
    nodes: Vec<&'a Node>,
    board: Board,
    required_peak: f64,
    shape_base: Shape,
    soak_options: Vec<f64>,
}

fn evaluate(spec: &Spec, peak: f64, dwell: f64, soak: f64) -> Candidate {
    let shape = Shape {
        soak_s: soak,
        ..Shape {
            ramp_up: spec.shape_base.ramp_up,
            ramp_down: spec.shape_base.ramp_down,
            preheat: spec.shape_base.preheat,
            soak_s: soak,
        }
    };
    let (board, _) = drive(&shape, peak, dwell);
    let liquidus = spec.paste.liquidus;
    let mut cold = f64::MAX;
    let mut hot = f64::MAX;
    let mut slack = f64::MAX;
    let mut violations = 0;
    let (mut hi, mut lo) = (f64::MIN, f64::MAX);
    let mut cache: Vec<(i64, Measure)> = Vec::new();
    for n in &spec.nodes {
        let tau = thermal::own_tau(&spec.board, &n.geometry);
        let key = (tau * 20.0).round() as i64;
        let idx = match cache.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                let s = thermal::follow(&board, tau, DT_S, AMBIENT_C);
                cache.push((key, measure(&s, liquidus)));
                cache.len() - 1
            }
        };
        let m = &cache[idx].1;
        hi = hi.max(m.peak);
        lo = lo.min(m.peak);
        cold = cold.min(m.peak - spec.required_peak);
        hot = hot.min(n.limits.max_peak - m.peak).min(spec.paste.peak[1] - m.peak);
        let tal_min = spec.paste.time_above_liquidus[0];
        let tal_max = spec.paste.time_above_liquidus[1].min(n.limits.max_time_above_liquidus);
        slack =
            slack.min(m.tal - tal_min).min(tal_max - m.tal).min(n.limits.max_time_at_peak - m.tp);
        if m.peak - spec.required_peak < 0.0 {
            violations += 1;
        }
        if n.limits.max_peak - m.peak < 0.0 || spec.paste.peak[1] - m.peak < 0.0 {
            violations += 1;
        }
        if m.tal < tal_min || m.tal > tal_max || m.tp > n.limits.max_time_at_peak {
            violations += 1;
        }
        if m.up > n.limits.max_ramp_up + 1e-6 || m.down > n.limits.max_ramp_down + 1e-6 {
            violations += 1;
        }
    }
    Candidate { peak, dwell, soak, cold, hot, time_slack: slack, violations, spread: hi - lo }
}

fn search(spec: &Spec) -> Option<Candidate> {
    let lo = spec.required_peak.floor();
    let hi = (spec.paste.peak[1] + 60.0).ceil();
    let tal_max = spec.paste.time_above_liquidus[1];
    let mut best: Option<Candidate> = None;
    let mut best_any: Option<Candidate> = None;
    let mut peak = lo;
    while peak <= hi {
        let mut dwell = 0.0;
        while dwell <= tal_max {
            for &soak in &spec.soak_options {
                let c = evaluate(spec, peak, dwell, soak);
                let better = |b: &Option<Candidate>, c: &Candidate| {
                    b.as_ref().is_none_or(|b| c.score() > b.score())
                };
                if c.feasible() && better(&best, &c) {
                    best = Some(Candidate { ..c });
                } else if !c.feasible() {
                    let worse_any = best_any.as_ref().is_none_or(|b| {
                        (c.violations, -c.score().0) < (b.violations, -b.score().0)
                    });
                    if worse_any {
                        best_any = Some(c);
                    }
                }
            }
            dwell += DWELL_STEP_S;
        }
        peak += PEAK_STEP_C;
    }
    best.or(best_any)
}

fn nodes_of<'a>(nodes: &'a [Node]) -> Vec<&'a Node> {
    nodes.iter().collect()
}

pub struct Solved {
    pub profile: Profile,
}

fn shape_for(
    paste: &Paste,
    nodes: &[&Node],
    oven: Option<&OvenFile>,
    out: &mut Vec<Finding>,
) -> Option<Shape> {
    let part_up = nodes.iter().map(|n| n.limits.max_ramp_up).fold(f64::MAX, f64::min);
    let part_down = nodes.iter().map(|n| n.limits.max_ramp_down).fold(f64::MAX, f64::min);
    let oven_up = oven.and_then(|o| o.max_ramp_up).unwrap_or(f64::MAX);
    let ramp_up = paste.max_ramp_up.min(part_up).min(oven_up);
    let want_down = paste.cooling[1].min(part_down);
    if want_down < paste.cooling[0] {
        out.push(Finding::new(
            Severity::Warning,
            "reflow-cooling",
            "profile",
            format!("the parts allow only {want_down:.1} C/s of cooling, under the paste's {:.1} C/s minimum", paste.cooling[0]),
        ));
    }
    let oven_down = oven.and_then(|o| o.max_ramp_down);
    let ramp_down = match oven_down {
        Some(o) if o < want_down => {
            if o < paste.cooling[0] {
                out.push(Finding::new(
                    Severity::Warning,
                    "reflow-oven-cooling",
                    "oven",
                    format!("the oven cools at {o:.1} C/s, under the paste's {:.1} C/s minimum: open the door or fan assist", paste.cooling[0]),
                ));
            }
            o
        }
        _ => want_down,
    };
    if ramp_up <= 0.0 || ramp_down <= 0.0 {
        out.push(Finding::new(Severity::Error, "reflow-ramp", "profile", "a ramp limit is zero"));
        return None;
    }
    let preheat = [paste.preheat[0], paste.preheat[1].min(paste.liquidus - 10.0)];
    Some(Shape { ramp_up, ramp_down, preheat, soak_s: paste.soak_time[0] })
}

fn solve_nodes(
    paste: &Paste,
    nodes: &[Node],
    board: &Board,
    oven: Option<&OvenFile>,
    out: &mut Vec<Finding>,
) -> Option<(Profile, Candidate)> {
    let refs = nodes_of(nodes);
    if refs.is_empty() {
        return None;
    }
    let shape = shape_for(paste, &refs, oven, out)?;
    let required = paste.peak[0].max(paste.liquidus);
    let mut soaks = vec![paste.soak_time[0]];
    let mid = (paste.soak_time[0] + paste.soak_time[1]) / 2.0;
    if mid > soaks[0] {
        soaks.push(mid);
    }
    if paste.soak_time[1] > mid {
        soaks.push(paste.soak_time[1]);
    }
    let spec = Spec {
        paste,
        nodes: refs,
        board: board.clone(),
        required_peak: required,
        shape_base: shape,
        soak_options: soaks,
    };
    let best = search(&spec)?;
    let profile = build(&spec, &best);
    Some((profile, best))
}

fn build(spec: &Spec, c: &Candidate) -> Profile {
    let shape = Shape {
        ramp_up: spec.shape_base.ramp_up,
        ramp_down: spec.shape_base.ramp_down,
        preheat: spec.shape_base.preheat,
        soak_s: c.soak,
    };
    let (board, stages) = drive(&shape, c.peak, c.dwell);
    let d = thermal::oven_for_board(&board, spec.board.tau(), DT_S, AMBIENT_C);
    let liquidus = spec.paste.liquidus;
    let mut parts = Vec::new();
    let mut hottest: Option<(f64, Vec<f64>)> = None;
    let mut coldest: Option<(f64, Vec<f64>)> = None;
    for n in &spec.nodes {
        let tau = thermal::own_tau(&spec.board, &n.geometry);
        let s = thermal::follow(&board, tau, DT_S, AMBIENT_C);
        let m = measure(&s, liquidus);
        parts.push(PartResult {
            reference: n.reference.clone(),
            tau_s: tau,
            peak_c: m.peak,
            time_above_liquidus_s: m.tal,
            time_at_peak_s: m.tp,
            max_ramp_up_c_s: m.up,
            max_ramp_down_c_s: m.down,
            limit_peak_c: n.limits.max_peak,
            hot_margin_c: n.limits.max_peak.min(spec.paste.peak[1]) - m.peak,
            cold_margin_c: m.peak - spec.required_peak,
        });
        if hottest.as_ref().is_none_or(|(p, _)| m.peak > *p) {
            hottest = Some((m.peak, s.clone()));
        }
        if coldest.as_ref().is_none_or(|(p, _)| m.peak < *p) {
            coldest = Some((m.peak, s));
        }
    }
    let by_hot =
        parts.iter().min_by(|a, b| a.hot_margin_c.total_cmp(&b.hot_margin_c)).expect("parts");
    let by_cold =
        parts.iter().min_by(|a, b| a.cold_margin_c.total_cmp(&b.cold_margin_c)).expect("parts");
    let limiter_is_paste = spec.paste.peak[1] < by_hot.limit_peak_c;
    let times: Vec<f64> = (0..d.len()).map(|i| i as f64 * DT_S).collect();
    let time_to_peak = stages.iter().take(3).map(|s| s.end_s - s.start_s).sum::<f64>();
    if time_to_peak > JSTD020_MAX_TIME_25_TO_PEAK_S {
        // reported as a finding by the caller
    }
    Profile {
        oven_peak_c: d.iter().cloned().fold(f64::MIN, f64::max),
        board_peak_c: c.peak,
        dwell_s: c.dwell,
        ramp_up_c_s: shape.ramp_up,
        ramp_down_c_s: shape.ramp_down,
        preheat_c: shape.preheat,
        soak_s: c.soak,
        peak_window_c: spec.paste.peak,
        liquidus_c: liquidus,
        required_joint_peak_c: spec.required_peak,
        stages,
        limiter: if limiter_is_paste { "paste".into() } else { by_hot.reference.clone() },
        limiter_limit_c: if limiter_is_paste { spec.paste.peak[1] } else { by_hot.limit_peak_c },
        limiter_margin_c: by_hot.hot_margin_c,
        coldest_joint: by_cold.reference.clone(),
        coldest_joint_peak_c: by_cold.peak_c,
        coldest_margin_c: by_cold.cold_margin_c,
        hot_cold_spread_c: c.spread,
        tolerance_c: c.cold.min(c.hot).max(0.0),
        time_to_peak_s: time_to_peak,
        feasible: c.feasible(),
        parts,
        series: Series {
            time_s: times,
            oven_c: d,
            board_c: board,
            coldest_c: coldest.map(|x| x.1).unwrap_or_default(),
            hottest_c: hottest.map(|x| x.1).unwrap_or_default(),
        },
    }
}

pub struct Setup {
    pub board: Board,
}

pub fn board_for(info: &Board3, file: &ReflowFile) -> Board {
    let fill =
        file.assembly.as_ref().and_then(|a| a.copper_fill).unwrap_or(thermal::BOARD_COPPER_FILL);
    let kind = file.oven.as_ref().map(|o| o.kind).unwrap_or_default();
    let h = file.oven.as_ref().and_then(|o| o.h_eff).unwrap_or(match kind {
        OvenKind::Toaster => thermal::TOASTER_OVEN_H_W_M2K,
        OvenKind::Hotplate => thermal::HOTPLATE_H_W_M2K,
        OvenKind::Convection | OvenKind::Zones => thermal::REFLOW_OVEN_H_W_M2K,
    });
    Board {
        thickness_mm: info.thickness_mm,
        copper_mm: info.copper_mm,
        copper_fill: fill,
        h_eff: h,
    }
}

fn lead_checks(paste: &Paste, resolved: &[Resolved], out: &mut Vec<Finding>) {
    for r in resolved {
        if r.limits.capable != Capable::Reflow {
            continue;
        }
        let Some(t) = r.limits.termination else { continue };
        let is_bga = r.family == super::defaults::Family::Bga;
        match (paste.lead_free, t) {
            (false, Termination::LeadFree) => out.push(Finding::new(
                if is_bga { Severity::Error } else { Severity::Warning },
                "reflow-alloy-mix",
                &r.reference,
                format!("{} has lead-free terminations{} but the paste {} is leaded: a SnPb peak does not collapse SAC balls, and mixed joints are weak", r.reference, if is_bga { " (BGA balls)" } else { "" }, paste.name),
            )),
            (true, Termination::TinLead) => out.push(Finding::new(
                if is_bga { Severity::Error } else { Severity::Warning },
                "reflow-alloy-mix",
                &r.reference,
                format!("{} has SnPb terminations but the paste {} is lead-free: the Pb-free peak overheats the part and the joint alloy mixes", r.reference, paste.name),
            )),
            _ => {}
        }
        if is_bga && t == Termination::LeadFree && paste.liquidus < SAC_BALL_SOLIDUS_C - 1.0 {
            out.push(Finding::new(
                Severity::Error,
                "reflow-ball-melt",
                &r.reference,
                format!("{} has SAC balls (solidus {} C) that do not melt at the {} C liquidus of {}: the balls will not collapse", r.reference, SAC_BALL_SOLIDUS_C, paste.liquidus, paste.name),
            ));
        }
    }
}

fn node_for(p: &PartInput, r: &Resolved) -> Node {
    Node {
        reference: r.reference.clone(),
        limits: r.limits.clone(),
        geometry: Geometry {
            family: r.family,
            body: p.body,
            footprint: p.footprint_mm,
            pad_area_mm2: p.pad_area_mm2,
            mass_g: r.mass_g,
            local_copper_fill: r.local_copper_fill,
        },
    }
}

fn explain(
    paste: &Paste,
    nodes: &[Node],
    resolved: &[Resolved],
    board: &Board,
    oven: Option<&OvenFile>,
    best: &Profile,
) -> Conflict {
    let mut parts: Vec<String> = Vec::new();
    let cold = &best.coldest_joint;
    let hot = &best.limiter;
    let mut why = Vec::new();
    if best.coldest_margin_c < 0.0 {
        why.push(format!(
            "the coldest joint {cold} peaks at {:.0} C and needs {:.0} C",
            best.coldest_joint_peak_c, best.required_joint_peak_c
        ));
        parts.push(cold.clone());
    }
    if best.limiter_margin_c < 0.0 {
        why.push(format!(
            "{hot} (limit {:.0} C) reaches {:.0} C at the same time",
            best.limiter_limit_c,
            best.limiter_limit_c - best.limiter_margin_c
        ));
        if hot != "paste" {
            parts.push(hot.clone());
        }
    }
    for p in &best.parts {
        let Some(n) = nodes.iter().find(|n| n.reference == p.reference) else { continue };
        let tal_max = paste.time_above_liquidus[1].min(n.limits.max_time_above_liquidus);
        if p.time_above_liquidus_s > tal_max
            || p.time_above_liquidus_s < paste.time_above_liquidus[0]
        {
            why.push(format!(
                "{} spends {:.0} s above liquidus, outside {:.0}-{:.0} s",
                p.reference, p.time_above_liquidus_s, paste.time_above_liquidus[0], tal_max
            ));
            if !parts.contains(&p.reference) {
                parts.push(p.reference.clone());
            }
        }
        if p.max_ramp_up_c_s > n.limits.max_ramp_up + 1e-6
            || p.max_ramp_down_c_s > n.limits.max_ramp_down + 1e-6
        {
            why.push(format!("{} exceeds its ramp limit", p.reference));
            if !parts.contains(&p.reference) {
                parts.push(p.reference.clone());
            }
        }
        if p.time_at_peak_s > n.limits.max_time_at_peak {
            why.push(format!(
                "{} stays within 5 C of its peak for {:.0} s, over its {:.0} s",
                p.reference, p.time_at_peak_s, n.limits.max_time_at_peak
            ));
            if !parts.contains(&p.reference) {
                parts.push(p.reference.clone());
            }
        }
    }
    let message = format!(
        "no profile meets every limit with {} (liquidus {:.0} C, joint peak at least {:.0} C): {}",
        paste.name,
        paste.liquidus,
        best.required_joint_peak_c,
        why.join("; ")
    );
    let mut options = Vec::new();
    let mut low = paste::low_temperature();
    low.name = "low temperature SnBiAg".into();
    let lead_free_ok = nodes.iter().all(|n| n.limits.termination != Some(Termination::TinLead));
    if paste.liquidus > low.liquidus + 20.0 && lead_free_ok {
        let mut sink = Vec::new();
        let ok = solve_nodes(&low, nodes, board, oven, &mut sink).is_some_and(|(p, _)| p.feasible);
        options.push(Option_ {
            action: "low temperature paste".into(),
            detail: format!(
                "Sn42Bi57.6Ag0.4 melts at {:.0}-{:.0} C: the same parts fit with a {:.0}-{:.0} C joint peak{}. Bismuth joints are brittle under shock and mixing with SAC balls is not allowed.",
                low.solidus, low.liquidus, low.peak[0], low.peak[1],
                if ok { " (checked by this solver)" } else { " (still conflicts)" }
            ),
            verified: ok,
        });
    }
    for r in parts.iter().filter(|r| !r.eq(&"paste")) {
        let Some(res) = resolved.iter().find(|x| &x.reference == r) else { continue };
        let rest: Vec<Node> = nodes.iter().filter(|n| &n.reference != r).cloned().collect();
        if rest.is_empty() {
            continue;
        }
        let mut sink = Vec::new();
        let ok = solve_nodes(paste, &rest, board, oven, &mut sink).is_some_and(|(p, _)| p.feasible);
        let selective = res.family != super::defaults::Family::Bga;
        if selective {
            options.push(Option_ {
                action: format!("hand or selective solder {r}"),
                detail: format!(
                    "leave {r} off the oven pass and solder it after reflow with a {:.0} C iron or selective fountain: the rest of the board {} with {}",
                    res.limits.max_peak.min(350.0),
                    if ok { "then solves" } else { "still conflicts" },
                    paste.name
                ),
                verified: ok,
            });
        }
        options.push(Option_ {
            action: format!("swap {r}"),
            detail: format!(
                "choose a {r} rated for at least {:.0} C peak, or a package with less thermal mass; the part now limits at {:.0} C for {:.0} s",
                best.required_joint_peak_c.max(paste.peak[0]) + 5.0,
                res.limits.max_peak,
                res.limits.max_time_at_peak
            ),
            verified: false,
        });
    }
    options.push(Option_ {
        action: "second pass step".into(),
        detail: "place the limited part on its own side of a double sided board and reflow that side with a cooler low temperature profile, or add it in a separate low temperature pass after the main reflow".into(),
        verified: false,
    });
    options.push(Option_ {
        action: "double sided order".into(),
        detail: "put the limited part on the side that sees one pass, and the parts that need the full profile on the side reflowed first only if they hold by surface tension".into(),
        verified: false,
    });
    Conflict { message, parts, options }
}

pub fn run(
    file: &ReflowFile,
    parts: &[PartInput],
    info: &Board3,
    mut findings: Vec<Finding>,
) -> Report {
    let paste = match paste::resolve(&file.paste) {
        Ok(p) => p,
        Err(e) => {
            findings.push(Finding::new(Severity::Error, "reflow-paste", "paste", e));
            let blank =
                paste::resolve(&super::PasteFile { alloy: "SAC305".into(), ..Default::default() })
                    .expect("builtin");
            return Report {
                reflow: file.name.clone(),
                layout: file.layout.clone(),
                paste: blank,
                parts: Vec::new(),
                passes: passes::plan(&[], &[], None, &mut Vec::new()),
                profile: None,
                conflict: None,
                oven: None,
                thermocouples: Vec::new(),
                model: MODEL_BASIS.iter().map(|s| s.to_string()).collect(),
                findings,
                ok: false,
            };
        }
    };
    if paste.confidence != super::Confidence::Datasheet {
        findings.push(Finding::new(
            Severity::Warning,
            "reflow-paste-assumed",
            "paste",
            format!("paste `{}` profile windows are not all from a paste datasheet ({}): assumed, give `source` and the datasheet's values", paste.name, if paste.assumed.is_empty() { "no source".to_string() } else { paste.assumed.join(", ") }),
        ));
    }
    limits::check_design(file, parts, &mut findings);
    let resolved: Vec<Resolved> =
        parts.iter().map(|p| limits::resolve_part(file, p, &paste, &mut findings)).collect();
    lead_checks(&paste, &resolved, &mut findings);
    let plan = passes::plan(parts, &resolved, file.assembly.as_ref(), &mut findings);
    let board = board_for(info, file);
    let nodes: Vec<Node> = parts
        .iter()
        .zip(&resolved)
        .filter(|(_, r)| r.limits.capable == Capable::Reflow)
        .map(|(p, r)| node_for(p, r))
        .collect();
    for r in &resolved {
        if r.limits.capable != Capable::Reflow {
            findings.push(Finding::new(
                Severity::Info,
                "reflow-excluded",
                &r.reference,
                format!(
                    "{} is {} only: left out of the oven profile",
                    r.reference,
                    if r.limits.capable == Capable::Selective {
                        "selective solder"
                    } else {
                        "hand solder"
                    }
                ),
            ));
        }
    }
    let oven_file = file.oven.as_ref();
    let mut profile = None;
    let mut conflict = None;
    if !nodes.is_empty() {
        match solve_nodes(&paste, &nodes, &board, oven_file, &mut findings) {
            Some((p, _)) => {
                if !p.feasible {
                    conflict = Some(explain(&paste, &nodes, &resolved, &board, oven_file, &p));
                    findings.push(Finding::new(
                        Severity::Error,
                        "reflow-conflict",
                        "profile",
                        conflict.as_ref().map(|c| c.message.clone()).unwrap_or_default(),
                    ));
                } else if p.time_to_peak_s > JSTD020_MAX_TIME_25_TO_PEAK_S {
                    findings.push(Finding::new(
                        Severity::Warning,
                        "reflow-slow",
                        "profile",
                        format!(
                            "{:.0} s from ambient to peak is over the J-STD-020 limit of {:.0} s",
                            p.time_to_peak_s, JSTD020_MAX_TIME_25_TO_PEAK_S
                        ),
                    ));
                }
                profile = Some(p);
            }
            None => findings.push(Finding::new(
                Severity::Error,
                "reflow-conflict",
                "profile",
                "no profile could be built",
            )),
        }
    } else {
        findings.push(Finding::new(
            Severity::Warning,
            "reflow-empty",
            "profile",
            "no part is reflow capable: nothing to profile",
        ));
    }
    let (oven_plan, tcs) = match &profile {
        Some(p) => {
            let plan = oven::plan(p, file.oven.as_ref(), &mut findings);
            let tcs = thermocouples(p, &nodes);
            (Some(plan), tcs)
        }
        None => (None, Vec::new()),
    };
    let ok = !findings.iter().any(|f| f.severity == Severity::Error);
    Report {
        reflow: file.name.clone(),
        layout: file.layout.clone(),
        paste,
        parts: resolved,
        passes: plan,
        profile,
        conflict,
        oven: oven_plan,
        thermocouples: tcs,
        model: MODEL_BASIS.iter().map(|s| s.to_string()).collect(),
        findings,
        ok,
    }
}

fn thermocouples(p: &Profile, nodes: &[Node]) -> Vec<Thermocouple> {
    let mut out = Vec::new();
    let heaviest = p.parts.iter().max_by(|a, b| a.tau_s.total_cmp(&b.tau_s));
    if let Some(h) = heaviest {
        out.push(Thermocouple {
            reference: h.reference.clone(),
            why: format!("largest thermal mass ({:.0} s lag, predicted peak {:.0} C): the coldest joint, so it proves the liquidus window is met", h.tau_s, h.peak_c),
        });
    }
    out.push(Thermocouple {
        reference: p.limiter.clone(),
        why: format!(
            "limiting part (limit {:.0} C, margin {:.1} C): the hottest reading must stay under it",
            p.limiter_limit_c, p.limiter_margin_c
        ),
    });
    if out.len() == 2 && out[0].reference == out[1].reference {
        out.pop();
    }
    let _ = nodes;
    out.push(Thermocouple {
        reference: "board edge".into(),
        why: "bare copper near the board edge, to compare the oven's own ramp against the target curve".into(),
    });
    out
}
