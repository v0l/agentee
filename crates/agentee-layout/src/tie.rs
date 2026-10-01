use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::EngineFile;

pub struct Tie;

impl Phase for Tie {
    fn name(&self) -> &'static str {
        "tie"
    }

    fn run(
        &self,
        model: &mut Model,
        _cfg: &EngineFile,
        _field: &mut crate::field::CostField,
    ) -> PhaseReport {
        let mut report = PhaseReport { phase: "tie".into(), ..Default::default() };
        if agentee_core::tie::plane_nets(&model.layout).is_empty() {
            report.notes.push("no net has a zone, nothing to tie".into());
            return report;
        }
        match agentee_core::tie::tie(&model.layout, model.board, &[]) {
            Ok(r) => {
                report.notes.push(format!(
                    "{} plane pads tied with a via, {} had one already",
                    r.tied, r.already
                ));
                report.failed.extend(r.failed.iter().cloned());
                report.changed = r.tied > 0;
                model.tie = Some(r);
            }
            Err(e) => report.failed.push(e),
        }
        report
    }
}
