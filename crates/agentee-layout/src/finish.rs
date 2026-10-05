use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::EngineFile;

pub struct Finish;

impl Phase for Finish {
    fn name(&self) -> &'static str {
        "finish"
    }

    fn run(&self, model: &mut Model, cfg: &EngineFile) -> PhaseReport {
        let mut report = PhaseReport { phase: "finish".into(), ..Default::default() };
        if model.detail.is_none() {
            report.notes.push("nothing routed to finish".into());
            return report;
        }
        if let Err(e) = model.ensure_base(&crate::detail::options(cfg)) {
            report.failed.push(e);
            return report;
        }
        let base = model.base.as_ref().expect("base is built");
        let plan = model.detail.as_mut().expect("detail plan");
        let undo = crate::negotiate::spread(&model.layout, base, &mut plan.tracks);
        report
            .notes
            .push(format!("spread: {} tracks moved to the middle of their free space", undo.len()));
        report.changed = !undo.is_empty();
        plan.undo = undo;
        report
    }
}

pub fn check_spread(model: &mut Model, cfg: &EngineFile, report: &mut PhaseReport) -> bool {
    if model.ensure_base(&crate::detail::options(cfg)).is_err() {
        return false;
    }
    let base = model.base.as_ref().expect("base is built");
    let Some(plan) = model.detail.as_mut() else { return false };
    let which: Vec<usize> = plan.undo.iter().map(|u| u.0).collect();
    let bad = crate::negotiate::illegal(&model.layout, base, &plan.tracks, &which);
    for (ti, old) in std::mem::take(&mut plan.undo) {
        if bad.contains(&ti) {
            plan.tracks[ti].points = old;
        }
    }
    if !bad.is_empty() {
        report
            .notes
            .push(format!("spread: {} moves put back, they crowded a neighbour", bad.len()));
    }
    !bad.is_empty()
}

pub fn tune(model: &mut Model, report: &mut PhaseReport) {
    let Some(plan) = model.detail.as_mut() else { return };
    let opts = agentee_core::tune::TuneOptions::default();
    let r = match agentee_core::tune::tune(&model.layout, model.board, &opts) {
        Ok(r) => r,
        Err(e) => {
            report.failed.push(format!("tune: {e}"));
            return;
        }
    };
    let first = model.file.tracks.len().saturating_sub(plan.tracks.len());
    let width_of = |t: &agentee_core::route::RoutedTrack| {
        t.width.unwrap_or_else(|| {
            model.layout.nets.iter().find(|n| n.name == t.net).map(|n| n.width).unwrap_or(0.1)
        })
    };
    let mut edited = 0;
    for e in &r.edits {
        let Some(k) = e.track.checked_sub(first).filter(|&k| k < plan.tracks.len()) else {
            continue;
        };
        let w = width_of(&plan.tracks[k]);
        let crowds = plan.tracks.iter().enumerate().any(|(j, o)| {
            j != k
                && o.net == plan.tracks[k].net
                && o.layer == plan.tracks[k].layer
                && e.points[1..e.points.len().saturating_sub(1)].iter().any(|p| {
                    o.points.windows(2).any(|s| {
                        agentee_core::geom::point_segment_distance(*p, s[0], s[1])
                            < (w + width_of(o)) / 2.0
                    })
                })
        });
        if !crowds && !folds(&e.points, w) {
            plan.tracks[k].points = e.points.clone();
            edited += 1;
        }
    }
    report.notes.push(format!(
        "tune: {} nets tuned, {} short of their length, {edited} tracks meandered",
        r.tuned.len(),
        r.failed.len()
    ));
    for f in &r.failed {
        report.failed.push(format!("{}: tune {}", f.net, f.why));
    }
    if edited > 0 {
        report.changed = true;
    }
}

pub fn neck(model: &mut Model, report: &mut PhaseReport) {
    let Some(plan) = model.detail.as_mut() else { return };
    let opts = agentee_core::neck::NeckOptions::default();
    let r = match agentee_core::neck::neck(&model.layout, model.board, &opts) {
        Ok(r) => r,
        Err(e) => {
            report.failed.push(format!("neck: {e}"));
            return;
        }
    };
    let first = model.file.tracks.len().saturating_sub(plan.tracks.len());
    let mut edited = 0;
    for e in &r.edits {
        let Some(k) = e.track.checked_sub(first).filter(|&k| k < plan.tracks.len()) else {
            continue;
        };
        plan.tracks[k].points = e.points.clone();
        let (net, layer) = (plan.tracks[k].net.clone(), plan.tracks[k].layer.clone());
        for n in &e.necks {
            plan.tracks.push(agentee_core::route::RoutedTrack {
                net: net.clone(),
                layer: layer.clone(),
                width: Some(n.width),
                points: n.points.clone(),
            });
        }
        edited += 1;
    }
    if edited > 0 {
        report.notes.push(format!("neck: {edited} track ends narrowed at their pads"));
        report.changed = true;
    }
}

fn folds(points: &[agentee_core::geom::P], width: f64) -> bool {
    let n = points.len();
    (0..n.saturating_sub(1)).any(|i| {
        (i + 2..n.saturating_sub(1)).any(|j| {
            agentee_core::geom::segment_segment_distance(
                points[i],
                points[i + 1],
                points[j],
                points[j + 1],
            ) < width
        })
    })
}
