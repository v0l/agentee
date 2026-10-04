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
        let moved = crate::negotiate::spread(&model.layout, base, &mut plan.tracks);
        report
            .notes
            .push(format!("spread: {moved} segments moved to the middle of their free space"));
        report.changed = moved > 0;
        report
    }
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
    let mut edited = 0;
    for e in &r.edits {
        if e.track >= first
            && let Some(t) = plan.tracks.get_mut(e.track - first)
        {
            t.points = e.points.clone();
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
