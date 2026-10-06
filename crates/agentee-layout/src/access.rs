use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::EngineFile;
use agentee_core::geom::P;
use serde::Serialize;

pub struct Access;

#[derive(Clone, Debug, Default, Serialize)]
pub struct AccessPlan {
    pub pads: usize,
    pub stuck: Vec<crate::negotiate::PadAccess>,
    #[serde(skip)]
    pub prefer: Vec<(usize, usize, P, P)>,
    #[serde(skip)]
    pub prefer_vias: Vec<(usize, P)>,
    pub escaped_balls: usize,
}

impl Phase for Access {
    fn name(&self) -> &'static str {
        "access"
    }

    fn run(&self, model: &mut Model, cfg: &EngineFile) -> PhaseReport {
        let mut report = PhaseReport { phase: "access".into(), ..Default::default() };
        let pc = cfg.planes.clone().unwrap_or_default();
        let (mut planes, notes, failed) = crate::planes::plan(model, &pc);
        report.notes.extend(notes);
        let pins = crate::planes::pin_pours(model);
        if !pins.is_empty() {
            report.notes.push(format!(
                "{} runs of same-net pins narrower than their track joined by a pour",
                pins.len()
            ));
        }
        planes.zones.extend(pins);
        report.failed.extend(failed);
        model.planes = Some(planes);
        let ac = cfg.access.clone().unwrap_or_default();
        let (esc, failed) =
            crate::escape::plan(model, &ac.escape_layers, ac.via_in_pad.unwrap_or(false));
        report.failed.extend(failed);
        let copper = &model.layout.copper;
        let mut plan = AccessPlan::default();
        for t in &esc.tracks {
            let Some(l) = copper.iter().position(|c| *c == t.layer) else { continue };
            for w in t.points.windows(2) {
                plan.prefer.push((t.net, l, w[0], w[1]));
            }
        }
        plan.prefer_vias = esc.vias.iter().map(|v| (v.net, v.at)).collect();
        plan.escaped_balls = esc.per_part.iter().map(|p| p.balls).sum();

        for pe in &esc.per_part {
            report.notes.push(format!(
                "{}: pitch {:.2}, {} balls, escape plan kept as a preference for detail",
                pe.reference, pe.pitch, pe.balls
            ));
        }
        report.changed = true;
        model.access = Some(plan);
        report
    }
}

pub fn pin_access(model: &mut Model, report: &mut PhaseReport, opts: &crate::negotiate::Options) {
    let Some(base) = model.base.as_ref() else { return };
    let pads = crate::negotiate::access_report(&model.layout, base, opts);
    let total = pads.len();
    let stuck: Vec<crate::negotiate::PadAccess> =
        pads.into_iter().filter(|p| p.exits == 0 && p.vias == 0).collect();
    report.notes.push(format!("{total} pads, {} with no legal exit", stuck.len()));
    for s in &stuck {
        report
            .failed
            .push(format!("{}: pad at [{:.2}, {:.2}] has no legal exit", s.net, s.at[0], s.at[1]));
    }
    let plan = model.access.get_or_insert_with(AccessPlan::default);
    plan.pads = total;
    plan.stuck = stuck;
}
