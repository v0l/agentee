use crate::negotiate::{self, Options};
use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::EngineFile;
use agentee_core::geom::P;
use agentee_core::route;
use serde::Serialize;

pub struct Detail;

#[derive(Clone, Debug, Default, Serialize)]
pub struct DetailPlan {
    pub connections: usize,
    pub routed: usize,
    pub rounds: usize,
    pub overlap_left: usize,
    pub tracks: Vec<route::RoutedTrack>,
    pub vias: Vec<route::RoutedVia>,
    pub failed: Vec<route::Unrouted>,
    #[serde(skip)]
    pub overlap: Vec<(String, String, P)>,
    #[serde(skip)]
    pub undo: Vec<(usize, Vec<P>)>,
}

pub fn options(cfg: &EngineFile) -> Options {
    let dc = cfg.detail.clone().unwrap_or_default();
    let d = Options::default();
    Options {
        grid: dc.grid.map(|g| g.to_mm()).unwrap_or(d.grid),
        via_cost: dc.via_cost.map(|v| v.to_mm()).unwrap_or(d.via_cost),
        bend_cost: dc.bend_cost.map(|b| b.to_mm()).unwrap_or(d.bend_cost),
        rounds: dc.rounds.map(|r| r as usize).unwrap_or(d.rounds),
        via_in_pad: cfg.access.as_ref().and_then(|a| a.via_in_pad).unwrap_or(false),
        fences: dc.fences.unwrap_or(true),
        criticality: dc.criticality.clone(),
        ..d
    }
}

impl Phase for Detail {
    fn name(&self) -> &'static str {
        "detail"
    }

    fn run(&self, model: &mut Model, cfg: &EngineFile) -> PhaseReport {
        let mut report = PhaseReport { phase: "detail".into(), ..Default::default() };
        let opts = options(cfg);
        if let Err(e) = model.ensure_base(&opts) {
            report.failed.push(e);
            return report;
        }
        let mut guide = crate::global::guide(model);
        guide.hist = model.hist.take();
        let base = model.base.as_ref().expect("base is built");
        let out = negotiate::route_on(&model.layout, base, &opts, &guide);
        model.hist = Some(out.hist);
        let l = &model.layout;
        let r = out.result;
        report.notes.push(format!(
            "{} of {} connections, {} tracks, {} vias, {} rounds, {} nets still overlapping before the hard pass",
            r.routed,
            r.connections,
            r.tracks.len(),
            r.vias.len(),
            out.rounds,
            out.overlap_left
        ));
        report.failed = r.failed.iter().map(|f| format!("{}: {}", f.net, f.reason)).collect();
        report.changed = !r.tracks.is_empty();
        model.detail = Some(DetailPlan {
            connections: r.connections,
            routed: r.routed,
            rounds: out.rounds,
            overlap_left: out.overlap_left,
            tracks: r.tracks,
            vias: r.vias,
            failed: r.failed,
            overlap: out
                .overlap
                .iter()
                .map(|&(n, ly, at)| (l.nets[n].name.clone(), l.copper[ly].clone(), at))
                .collect(),
            undo: Vec::new(),
        });
        report
    }
}
