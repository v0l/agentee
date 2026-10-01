use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::EngineFile;
use agentee_core::route::{self, Corridor, Fence, RouteOptions, RouteResult};
use serde::Serialize;

pub struct Detail;

#[derive(Clone, Debug, Default, Serialize)]
pub struct DetailPlan {
    pub connections: usize,
    pub routed: usize,
    pub tracks: Vec<route::RoutedTrack>,
    pub vias: Vec<route::RoutedVia>,
    pub failed: Vec<route::Unrouted>,
}

impl Phase for Detail {
    fn name(&self) -> &'static str {
        "detail"
    }

    fn run(
        &self,
        model: &mut Model,
        cfg: &EngineFile,
        _field: &mut crate::field::CostField,
    ) -> PhaseReport {
        let mut report = PhaseReport { phase: "detail".into(), ..Default::default() };
        let corridors: Vec<Corridor> = model
            .global
            .as_ref()
            .map(|g| {
                g.corridors
                    .iter()
                    .map(|c| Corridor { net: c.net.clone(), cells: c.tiles.clone() })
                    .collect()
            })
            .unwrap_or_default();
        let dc = cfg.detail.clone().unwrap_or_default();
        let via_cost = dc
            .via_cost
            .or(cfg.global.as_ref().and_then(|g| g.via_cost))
            .map(|v| v.to_mm())
            .unwrap_or(1.0);
        let corridors = if dc.corridors.unwrap_or(true) { corridors } else { Vec::new() };
        let fences: Vec<Fence> = model
            .layout
            .parts
            .iter()
            .filter(|p| dc.fences.unwrap_or(true) && crate::escape::is_bga(p))
            .map(|p| {
                let mut b = agentee_core::graphic::Bounds::EMPTY;
                p.pads.iter().flat_map(|q| q.outlines.iter().flatten()).for_each(|q| b.add(*q));
                let nets: Vec<String> = p
                    .pads
                    .iter()
                    .filter_map(|q| q.net)
                    .map(|n| model.layout.nets[n].name.clone())
                    .collect();
                Fence {
                    nets,
                    outline: vec![b.min, [b.max[0], b.min[1]], b.max, [b.min[0], b.max[1]]],
                }
            })
            .collect();
        let tiers = if dc.tiers.unwrap_or(true) { tiers_of(model) } else { Vec::new() };
        let class_order =
            if dc.class_order.is_empty() { class_order_of(model) } else { dc.class_order.clone() };
        let opts = RouteOptions {
            class_order,
            tiers,
            fences,
            nets: vec!["*".into()],
            pairs: dc.pairs.unwrap_or(true),
            via_in_pad: cfg.escape.as_ref().and_then(|e| e.via_in_pad).unwrap_or(false),
            via_cost,
            corridors,
            rip_limit: dc.rip_limit.unwrap_or(8),
            ..Default::default()
        };
        let opts = RouteOptions {
            grid: dc.grid.map(|g| g.to_mm()).unwrap_or(opts.grid),
            bend_cost: dc.bend_cost.map(|b| b.to_mm()).unwrap_or(opts.bend_cost),
            ..opts
        };
        let result: RouteResult = match route::route(&model.layout, model.board, &opts) {
            Ok(r) => r,
            Err(e) => {
                report.failed.push(e);
                return report;
            }
        };
        report.notes.push(format!(
            "{} of {} connections, {} tracks, {} vias",
            result.routed,
            result.connections,
            result.tracks.len(),
            result.vias.len()
        ));
        report.failed = result.failed.iter().map(|f| format!("{}: {}", f.net, f.reason)).collect();
        report.changed = !result.tracks.is_empty();
        model.detail = Some(DetailPlan {
            connections: result.connections,
            routed: result.routed,
            tracks: result.tracks,
            vias: result.vias,
            failed: result.failed,
        });
        report
    }
}

fn tiers_of(model: &Model) -> Vec<Vec<String>> {
    let l = &model.layout;
    let b = model.board;
    let mut first = Vec::new();
    let mut second = Vec::new();
    let bundled: Vec<&str> = l
        .interfaces
        .iter()
        .flat_map(|i| i.lanes.iter().flat_map(|ln| ln.nets.iter().map(String::as_str)))
        .collect();
    for n in &l.nets {
        if agentee_core::place::is_power_net(b, &n.name, &n.class) {
            continue;
        }
        let class = b.netclasses.iter().find(|c| c.name == n.class);
        let one_layer = class.is_some_and(|c| c.layers.len() == 1);
        let controlled = class.is_some_and(|c| c.impedance.is_some());
        if one_layer || controlled {
            first.push(n.name.clone());
        } else if bundled.contains(&n.name.as_str()) {
            second.push(n.name.clone());
        }
    }
    vec![first, second]
}

fn class_order_of(model: &Model) -> Vec<String> {
    let b = model.board;
    let l = &model.layout;
    let mut classes: Vec<(u8, i64, usize, String)> = b
        .netclasses
        .iter()
        .map(|c| {
            let nets: Vec<&agentee_core::layout::LayoutNet> =
                l.nets.iter().filter(|n| n.class == c.name).collect();
            let supply = !nets.is_empty()
                && nets.iter().all(|n| agentee_core::place::is_power_net(b, &n.name, &n.class));
            let rank = if supply || c.current.is_some() {
                3
            } else if c.layers.len() == 1 || c.impedance.is_some() || c.diff_gap.is_some() {
                0
            } else {
                1
            };
            let clearance = (c.clearance.to_mm() * 1000.0) as i64;
            (rank, -clearance, nets.len(), c.name.clone())
        })
        .collect();
    classes.sort();
    classes.into_iter().map(|c| c.3).collect()
}
