use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::EngineFile;
use agentee_core::geom::{self, P};
use agentee_core::graphic::Bounds;
use agentee_core::place;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize)]
pub struct Move {
    pub reference: String,
    pub at: P,
    pub rotation: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PlacePlan {
    pub chains: Vec<Vec<String>>,
    pub moves: Vec<Move>,
}

#[derive(Clone, Debug)]
struct Cell {
    reference: String,
    fixed: bool,
    chained: bool,
    at: P,
    rotation: f64,
    box_off: P,
    half: [f64; 2],
    area: f64,
    pins: Vec<(usize, P)>,
    pin_count: usize,
}

struct Net {
    weight: f64,
    pins: Vec<(usize, P)>,
}

struct Board {
    cells: Vec<Cell>,
    nets: Vec<Net>,
    outline: Vec<P>,
    bounds: Bounds,
    keepouts: Vec<Vec<P>>,
}

fn rotate_cell(c: &mut Cell, delta: f64) {
    if delta.rem_euclid(360.0).abs() < 1e-9 {
        return;
    }
    c.rotation = (c.rotation + delta).rem_euclid(360.0);
    c.box_off = geom::rotate(c.box_off, delta);
    if (delta.rem_euclid(180.0) - 90.0).abs() < 1e-9 {
        c.half = [c.half[1], c.half[0]];
    }
    for p in c.pins.iter_mut() {
        p.1 = geom::rotate(p.1, delta);
    }
}

fn build(model: &Model) -> Board {
    let l = &model.layout;
    let b = model.board;
    let locked: Vec<&str> =
        model.file.footprints.iter().filter(|f| f.locked).map(|f| f.reference.as_str()).collect();
    let mut cells = Vec::new();
    for p in &l.parts {
        let at = p.at.to_mm();
        let layer = if p.bottom { "B.CrtYd" } else { "F.CrtYd" };
        let mut bb = Bounds::EMPTY;
        let t = p.transform();
        for lp in place::courtyard_loops(&p.footprint, "F.CrtYd") {
            for q in lp {
                bb.add(t.apply(q));
            }
        }
        let _ = layer;
        if bb.is_empty() {
            for pad in &p.pads {
                pad.outlines.iter().flatten().for_each(|q| bb.add(*q));
            }
            if !bb.is_empty() {
                bb.min = [bb.min[0] - 0.25, bb.min[1] - 0.25];
                bb.max = [bb.max[0] + 0.25, bb.max[1] + 0.25];
            }
        }
        if bb.is_empty() {
            bb.add([at[0] - 0.5, at[1] - 0.5]);
            bb.add([at[0] + 0.5, at[1] + 0.5]);
        }
        let half = [(bb.max[0] - bb.min[0]) / 2.0, (bb.max[1] - bb.min[1]) / 2.0];
        let c = bb.center();
        let mut pins = Vec::new();
        for pad in &p.pads {
            let Some(net) = pad.net else { continue };
            let mut pb = Bounds::EMPTY;
            pad.outlines.iter().flatten().for_each(|q| pb.add(*q));
            if pb.is_empty() {
                continue;
            }
            let q = pb.center();
            pins.push((net, [q[0] - at[0], q[1] - at[1]]));
        }
        let mounting = p.pads.iter().all(|q| q.net.is_none());
        cells.push(Cell {
            reference: p.reference.clone(),
            fixed: locked.contains(&p.reference.as_str()) || p.bottom || mounting,
            chained: false,
            at,
            rotation: p.rotation,
            box_off: [c[0] - at[0], c[1] - at[1]],
            half,
            area: 4.0 * half[0] * half[1],
            pin_count: p.pads.len(),
            pins,
        });
    }
    let mut by_net: HashMap<usize, Vec<(usize, P)>> = HashMap::new();
    for (ci, c) in cells.iter().enumerate() {
        for &(n, off) in &c.pins {
            by_net.entry(n).or_default().push((ci, off));
        }
    }
    let mut nets = Vec::new();
    for (n, pins) in by_net {
        let net = &l.nets[n];
        if pins.len() < 2 || place::is_power_net(b, &net.name, &net.class) {
            continue;
        }
        let weight = if place::is_fast_class(b, &net.class) || place::is_rf_class(b, &net.class) {
            3.0
        } else {
            1.0
        };
        nets.push(Net { weight, pins });
    }
    for (ci, c) in cells.iter().enumerate() {
        if c.fixed || !place::is_capacitor(&c.reference, "") || c.pins.len() != 2 {
            continue;
        }
        let Some(&(rail, off)) = c.pins.iter().find(|(n, _)| {
            let net = &l.nets[*n];
            !place::is_ground(&net.name) && place::is_power_net(b, &net.name, &net.class)
        }) else {
            continue;
        };
        let here = [c.at[0] + off[0], c.at[1] + off[1]];
        let target = cells
            .iter()
            .enumerate()
            .filter(|(cj, d)| *cj != ci && d.pin_count >= 8)
            .flat_map(|(cj, d)| {
                d.pins.iter().filter(|(n, _)| *n == rail).map(move |&(_, o)| (cj, o, d.at))
            })
            .min_by(|a, b| {
                let da = geom::dist(here, [a.2[0] + a.1[0], a.2[1] + a.1[1]]);
                let db = geom::dist(here, [b.2[0] + b.1[0], b.2[1] + b.1[1]]);
                da.total_cmp(&db)
            });
        if let Some((cj, o, _)) = target {
            nets.push(Net { weight: 4.0, pins: vec![(ci, off), (cj, o)] });
        }
    }
    let mut bounds = Bounds::EMPTY;
    l.outline.iter().for_each(|q| bounds.add(*q));
    Board { cells, nets, outline: l.outline.clone(), bounds, keepouts: model.keepouts.clone() }
}

fn moves_of(bd: &Board) -> Vec<Move> {
    bd.cells
        .iter()
        .filter(|c| !c.fixed)
        .map(|c| Move {
            reference: c.reference.clone(),
            at: [(c.at[0] * 1000.0).round() / 1000.0, (c.at[1] * 1000.0).round() / 1000.0],
            rotation: c.rotation,
        })
        .collect()
}

fn apply_plan(bd: &mut Board, plan: &PlacePlan) {
    for m in &plan.moves {
        if let Some(c) = bd.cells.iter_mut().find(|c| c.reference == m.reference) {
            c.at = m.at;
        }
    }
    for ch in &plan.chains {
        for r in ch {
            if let Some(c) = bd.cells.iter_mut().find(|c| c.reference == *r) {
                c.chained = true;
            }
        }
    }
}

fn hpwl(bd: &Board) -> f64 {
    bd.nets
        .iter()
        .map(|n| {
            let mut b = Bounds::EMPTY;
            for &(ci, o) in &n.pins {
                let c = &bd.cells[ci];
                b.add([c.at[0] + o[0], c.at[1] + o[1]]);
            }
            n.weight * ((b.max[0] - b.min[0]) + (b.max[1] - b.min[1]))
        })
        .sum()
}

pub struct Floorplan;

fn chains(bd: &Board, model: &Model) -> Vec<(usize, Vec<usize>)> {
    let l = &model.layout;
    let b = model.board;
    let mut on_net: HashMap<usize, Vec<usize>> = HashMap::new();
    for (ci, c) in bd.cells.iter().enumerate() {
        for &(n, _) in &c.pins {
            let v = on_net.entry(n).or_default();
            if !v.contains(&ci) {
                v.push(ci);
            }
        }
    }
    let signal = |n: usize| {
        let net = &l.nets[n];
        !place::is_power_net(b, &net.name, &net.class)
    };
    let mut out: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut used: Vec<usize> = Vec::new();
    for (ci, c) in bd.cells.iter().enumerate() {
        if !c.reference.starts_with('J') {
            continue;
        }
        for &(start, _) in &c.pins {
            if !signal(start) {
                continue;
            }
            let class = &l.nets[start].class;
            let rf = place::is_rf_class(b, class);
            let kin = |n: usize| {
                let c = &l.nets[n].class;
                signal(n) && (!rf || place::is_rf_class(b, c))
            };
            let shunt = |k: usize, net: usize| {
                bd.cells[k].pins.iter().all(|&(n, _)| n == net || !signal(n))
            };
            let mut chain = Vec::new();
            let mut net = start;
            for _ in 0..12 {
                let others: Vec<usize> = on_net[&net]
                    .iter()
                    .copied()
                    .filter(|&k| k != ci && !chain.contains(&k) && !shunt(k, net))
                    .collect();
                if others.len() != 1 {
                    break;
                }
                let next = others[0];
                let nc = &bd.cells[next];
                if nc.fixed || nc.pin_count > 16 || used.contains(&next) {
                    break;
                }
                chain.push(next);
                let onward: Vec<usize> = nc
                    .pins
                    .iter()
                    .map(|p| p.0)
                    .filter(|&n| n != net && kin(n))
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let Some(&n) = onward.first() else { break };
                net = n;
            }
            if chain.len() >= 2 {
                used.extend(&chain);
                out.push((ci, chain));
            }
        }
    }
    out
}

impl Phase for Floorplan {
    fn name(&self) -> &'static str {
        "floorplan"
    }

    fn run(
        &self,
        model: &mut Model,
        _cfg: &EngineFile,
        _field: &mut crate::field::CostField,
    ) -> PhaseReport {
        let mut report = PhaseReport { phase: "floorplan".into(), ..Default::default() };
        let mut bd = build(model);

        let found = chains(&bd, model);
        for (conn_i, ch) in &found {
            let conn = bd.cells[*conn_i].at;
            let edge_d = [
                (conn[0] - bd.bounds.min[0], [1.0, 0.0]),
                (bd.bounds.max[0] - conn[0], [-1.0, 0.0]),
                (conn[1] - bd.bounds.min[1], [0.0, 1.0]),
                (bd.bounds.max[1] - conn[1], [0.0, -1.0]),
            ];
            let dir: P = edge_d.iter().min_by(|a, b| a.0.total_cmp(&b.0)).map(|e| e.1).unwrap();
            let horizontal = dir[1] == 0.0;
            let cc = &bd.cells[*conn_i];
            let conn_cell = if horizontal {
                cc.half[0] + dir[0] * cc.box_off[0]
            } else {
                cc.half[1] + dir[1] * cc.box_off[1]
            };
            let mut s = conn_cell + 0.6;
            for &ci in ch.iter() {
                let c = &mut bd.cells[ci];
                if c.pins.len() == 2 {
                    let axis = [c.pins[1].1[0] - c.pins[0].1[0], c.pins[1].1[1] - c.pins[0].1[1]];
                    let along = if horizontal {
                        axis[0].abs() >= axis[1].abs()
                    } else {
                        axis[1].abs() >= axis[0].abs()
                    };
                    if !along {
                        rotate_cell(c, 90.0);
                    }
                }
                let len = if horizontal { c.half[0] } else { c.half[1] };
                s += len;
                c.at = [conn[0] + dir[0] * s - c.box_off[0], conn[1] + dir[1] * s - c.box_off[1]];
                if horizontal {
                    c.at[1] = conn[1] - c.box_off[1];
                } else {
                    c.at[0] = conn[0] - c.box_off[0];
                }
                s += len + 0.6;
                c.chained = true;
            }
        }

        for (_, ch) in &found {
            report.notes.push(format!(
                "chain: {}",
                ch.iter().map(|&i| bd.cells[i].reference.as_str()).collect::<Vec<_>>().join(" > ")
            ));
        }
        let chained: Vec<Vec<String>> = found
            .iter()
            .map(|(_, ch)| ch.iter().map(|&i| bd.cells[i].reference.clone()).collect())
            .collect();
        let moves: Vec<Move> = moves_of(&bd)
            .into_iter()
            .filter(|m| chained.iter().flatten().any(|r| *r == m.reference))
            .collect();
        report.changed = !moves.is_empty();
        model.placement = Some(PlacePlan { chains: chained, moves });
        report
    }
}

pub struct Place;

impl Phase for Place {
    fn name(&self) -> &'static str {
        "place"
    }

    fn run(
        &self,
        model: &mut Model,
        cfg: &EngineFile,
        _field: &mut crate::field::CostField,
    ) -> PhaseReport {
        let mut report = PhaseReport { phase: "place".into(), ..Default::default() };
        let mut bd = build(model);
        let prior = model.placement.clone().unwrap_or_default();
        apply_plan(&mut bd, &prior);
        let before = hpwl(&bd);
        let free: Vec<String> = bd
            .cells
            .iter()
            .filter(|c| !c.fixed && !c.chained)
            .map(|c| c.reference.clone())
            .collect();
        let l = &model.layout;
        let footprints: HashMap<&str, &agentee_core::footprint::Footprint> =
            l.parts.iter().map(|p| (p.footprint_name.as_str(), &p.footprint)).collect();
        let mut fast: Vec<String> =
            model.file.interfaces.iter().flat_map(|f| f.nets.clone()).collect();
        for pr in &model.file.pairs {
            fast.push(pr.p.clone());
            fast.push(pr.n.clone());
        }
        let mut placements = model.file.footprints.clone();
        for m in &prior.moves {
            if let Some(f) = placements.iter_mut().find(|f| f.reference == m.reference) {
                f.at = agentee_core::units::Point::mm(m.at[0], m.at[1]);
                f.rotation = Some(m.rotation);
            }
        }
        let spec = model.file.place.clone().unwrap_or_default();
        let cutouts = l.board_cutouts.clone();
        let input = place::PlaceInput {
            board: model.board,
            outline: &l.outline,
            cutouts: &cutouts,
            schematic: model.schematic,
            footprints: &footprints,
            placements: &placements,
            spec: &spec,
            fast_nets: fast,
            heat: model.heat.clone(),
            silk: Vec::new(),
            texts: Vec::new(),
        };
        let opts = place::PlaceOptions {
            parts: free.clone(),
            seed: cfg.place.as_ref().and_then(|p| p.seed).unwrap_or(1),
            ..Default::default()
        };
        let result = match place::place(&input, &opts) {
            Ok(r) => r,
            Err(e) => {
                report.failed.push(e);
                return report;
            }
        };
        for pm in &result.placements {
            if let Some(c) = bd.cells.iter_mut().find(|c| c.reference == pm.reference) {
                let delta = pm.rotation - c.rotation;
                rotate_cell(c, delta);
                c.at = pm.at;
            }
        }
        report.failed.extend(result.failed.iter().map(|f| format!("{f}: not placed")));
        report.notes.push(format!(
            "{} parts placed around {} chained, hpwl {before:.0} -> {:.0} mm, {} crossings",
            free.len(),
            bd.cells.iter().filter(|c| c.chained).count(),
            hpwl(&bd),
            result.after.crossings
        ));
        report.changed = true;
        let mut plan = prior;
        plan.moves = moves_of(&bd);
        model.placement = Some(plan);
        report
    }
}

pub struct Legalise;

fn rect_of(c: &Cell, at: P, gap: f64) -> [f64; 4] {
    let cx = at[0] + c.box_off[0];
    let cy = at[1] + c.box_off[1];
    [cx - c.half[0] - gap, cy - c.half[1] - gap, cx + c.half[0] + gap, cy + c.half[1] + gap]
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

impl Phase for Legalise {
    fn name(&self) -> &'static str {
        "legalise"
    }

    fn run(
        &self,
        model: &mut Model,
        _cfg: &EngineFile,
        _field: &mut crate::field::CostField,
    ) -> PhaseReport {
        let mut report = PhaseReport { phase: "legalise".into(), ..Default::default() };
        let mut bd = build(model);
        let prior = model.placement.clone().unwrap_or_default();
        apply_plan(&mut bd, &prior);
        let gap = 0.0;
        let edge = 0.3;
        let keep: Vec<Bounds> = bd
            .keepouts
            .iter()
            .map(|k| {
                let mut b = Bounds::EMPTY;
                k.iter().for_each(|q| b.add(*q));
                b
            })
            .collect();
        let mut placed: Vec<[f64; 4]> =
            bd.cells.iter().filter(|c| c.fixed).map(|c| rect_of(c, c.at, 0.0)).collect();
        let mut order: Vec<usize> = (0..bd.cells.len()).filter(|&i| !bd.cells[i].fixed).collect();
        order.sort_by(|&a, &b| {
            let (ca, cb) = (&bd.cells[a], &bd.cells[b]);
            cb.chained.cmp(&ca.chained).then(cb.area.total_cmp(&ca.area))
        });
        let inside = |r: [f64; 4]| {
            let corners = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
            corners.iter().all(|q| geom::point_in_polygon(*q, &bd.outline))
                && bd.outline.windows(2).all(|w| {
                    corners.iter().all(|q| geom::point_segment_distance(*q, w[0], w[1]) >= edge)
                })
        };
        let mut failed = Vec::new();
        let mut moved = 0.0;
        for &i in &order {
            let want = bd.cells[i].at;
            let mut found = None;
            'search: for ring in 0..400 {
                let r = ring as f64 * 0.1;
                let steps = if ring == 0 { 1 } else { (ring * 8).min(160) };
                for k in 0..steps {
                    let a = k as f64 / steps as f64 * std::f64::consts::TAU;
                    let at = [want[0] + r * a.cos(), want[1] + r * a.sin()];
                    for rot in [0.0, 90.0] {
                        let mut c = bd.cells[i].clone();
                        rotate_cell(&mut c, rot);
                        let rc = rect_of(&c, at, gap);
                        let bare = rect_of(&c, at, 0.0);
                        if !inside(bare)
                            || placed.iter().any(|p| overlaps(rc, *p))
                            || keep
                                .iter()
                                .any(|k| overlaps(bare, [k.min[0], k.min[1], k.max[0], k.max[1]]))
                        {
                            continue;
                        }
                        found = Some((at, rot, bare));
                        break 'search;
                    }
                }
            }
            match found {
                Some((at, rot, bare)) => {
                    moved += geom::dist(at, want);
                    rotate_cell(&mut bd.cells[i], rot);
                    bd.cells[i].at = at;
                    placed.push(bare);
                }
                None => failed.push(bd.cells[i].reference.clone()),
            }
        }
        report.notes.push(format!(
            "{} parts legal, {:.1} mm moved in total, hpwl {:.0} mm",
            order.len() - failed.len(),
            moved,
            hpwl(&bd)
        ));
        for f in failed {
            report.failed.push(format!("{f}: no free spot"));
        }
        report.changed = true;
        let mut plan = prior;
        plan.moves = moves_of(&bd);
        model.placement = Some(plan);
        report
    }
}
