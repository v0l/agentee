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
    #[serde(skip)]
    pub texts: Vec<place::TextMove>,
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
    weight: HashMap<usize, f64>,
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
    let mut weights = HashMap::new();
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
        weights.insert(n, weight);
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
    Board {
        cells,
        nets,
        weight: weights,
        outline: l.outline.clone(),
        bounds,
        keepouts: model.keepouts.clone(),
    }
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

const SPACING: f64 = 0.2;
const FACING: f64 = 0.05;
const FACING_WEIGHT: f64 = 100.0;
use crate::score::FACE_PINS;
const STANDOFF: f64 = 0.6;
const SPREAD: f64 = 1.5;
const HOT_GROW: f64 = 0.25;
const MAX_GROW: f64 = 0.5;

pub struct Spread {
    pub spacing: f64,
    pub standoff: f64,
}

pub fn spread_of(cfg: &EngineFile, pass: usize) -> Spread {
    let p = cfg.place.clone().unwrap_or_default();
    let k = p.spread.unwrap_or(SPREAD).max(1.0).powi(pass as i32);
    Spread {
        spacing: p.spacing.map(|l| l.to_mm()).unwrap_or(SPACING) * k,
        standoff: p.standoff.map(|l| l.to_mm()).unwrap_or(STANDOFF) * k,
    }
}

pub struct Place;

#[derive(Clone, Debug, Serialize)]
pub struct Hot {
    pub at: P,
    pub size: f64,
    pub overflow: f64,
}

fn chain_pins(c: &Cell, nets: &[usize]) -> Option<P> {
    let hits: Vec<P> = c.pins.iter().filter(|p| nets.contains(&p.0)).map(|p| p.1).collect();
    (!hits.is_empty()).then(|| {
        let n = hits.len() as f64;
        [hits.iter().map(|q| q[0]).sum::<f64>() / n, hits.iter().map(|q| q[1]).sum::<f64>() / n]
    })
}

fn template_chains(
    bd: &mut Board,
    chains: &[crate::constraints::Chain],
    signal: &[bool],
) -> Vec<Vec<String>> {
    let index = |bd: &Board, r: &str| bd.cells.iter().position(|c| c.reference == r);
    let nets_of = |c: &Cell| -> Vec<usize> {
        c.pins.iter().map(|p| p.0).filter(|&n| signal.get(n).copied().unwrap_or(false)).collect()
    };
    let mut placed = Vec::new();
    for ch in chains {
        let Some(conn_i) = index(bd, &ch.from) else { continue };
        let members: Vec<usize> = ch
            .parts
            .iter()
            .filter_map(|r| index(bd, r))
            .filter(|&i| !bd.cells[i].fixed && !bd.cells[i].chained)
            .collect();
        if members.is_empty() {
            continue;
        }
        let conn = bd.cells[conn_i].at;
        let edge_d = [
            (conn[0] - bd.bounds.min[0], [1.0, 0.0]),
            (bd.bounds.max[0] - conn[0], [-1.0, 0.0]),
            (conn[1] - bd.bounds.min[1], [0.0, 1.0]),
            (bd.bounds.max[1] - conn[1], [0.0, -1.0]),
        ];
        let dir: P = edge_d.iter().min_by(|a, b| a.0.total_cmp(&b.0)).map(|e| e.1).unwrap();
        let horizontal = dir[1] == 0.0;
        let along = |q: P| q[0] * dir[0] + q[1] * dir[1];
        let first_nets = nets_of(&bd.cells[members[0]]);
        let cc = &bd.cells[conn_i];
        let line =
            chain_pins(cc, &first_nets).map(|o| [cc.at[0] + o[0], cc.at[1] + o[1]]).unwrap_or(conn);
        let conn_edge = along([cc.at[0] + cc.box_off[0], cc.at[1] + cc.box_off[1]])
            + if horizontal { cc.half[0] } else { cc.half[1] };
        let mut s = along(line).max(conn_edge) + 0.6;
        let mut prev_nets = nets_of(&bd.cells[conn_i]);
        for (k, &ci) in members.iter().enumerate() {
            let next_nets: Vec<usize> =
                members.get(k + 1).map(|&n| nets_of(&bd.cells[n])).unwrap_or_default();
            let mut best = 0.0;
            let mut best_rot = 0.0;
            for rot in [0.0, 90.0, 180.0, 270.0] {
                let mut c = bd.cells[ci].clone();
                rotate_cell(&mut c, rot);
                let Some(a) = chain_pins(&c, &prev_nets) else { continue };
                let ends = c
                    .pins
                    .iter()
                    .map(|p| along(p.1))
                    .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
                let b = chain_pins(&c, &next_nets);
                let facing = [Some(along(a) - ends.0), b.map(|b| ends.1 - along(b))]
                    .into_iter()
                    .flatten()
                    .filter(|&d| d < FACING)
                    .count();
                let v = facing as f64 * FACING_WEIGHT + b.map_or(ends.1, along) - along(a);
                if v > best + 1e-6 {
                    best = v;
                    best_rot = rot;
                }
            }
            let c = &mut bd.cells[ci];
            rotate_cell(c, best_rot);
            let entry = chain_pins(c, &prev_nets).unwrap_or(c.box_off);
            let len = if horizontal { c.half[0] } else { c.half[1] };
            let centre = s + len;
            c.at = if horizontal {
                [centre * dir[0] - c.box_off[0], line[1] - entry[1]]
            } else {
                [line[0] - entry[0], centre * dir[1] - c.box_off[1]]
            };
            s = centre + len + 0.6;
            c.chained = true;
            prev_nets = nets_of(c).into_iter().filter(|n| !prev_nets.contains(n)).collect();
            if prev_nets.is_empty() {
                prev_nets = nets_of(c);
            }
        }
        placed.push(members.iter().map(|&i| bd.cells[i].reference.clone()).collect());
    }
    placed
}

fn global_place(
    model: &Model,
    cfg: &EngineFile,
    bd: &mut Board,
    texts: &mut Vec<place::TextMove>,
    spread: &Spread,
) -> Result<(String, Vec<String>), String> {
    let free: Vec<String> =
        bd.cells.iter().filter(|c| !c.fixed && !c.chained).map(|c| c.reference.clone()).collect();
    let l = &model.layout;
    let footprints: HashMap<&str, &agentee_core::footprint::Footprint> =
        l.parts.iter().map(|p| (p.footprint_name.as_str(), &p.footprint)).collect();
    let mut fast: Vec<String> = model.file.interfaces.iter().flat_map(|f| f.nets.clone()).collect();
    for pr in &model.file.pairs {
        fast.push(pr.p.clone());
        fast.push(pr.n.clone());
    }
    let mut placements = model.file.footprints.clone();
    for c in bd.cells.iter().filter(|c| c.chained) {
        if let Some(f) = placements.iter_mut().find(|f| f.reference == c.reference) {
            f.at = agentee_core::units::Point::mm(c.at[0], c.at[1]);
            f.rotation = Some(c.rotation);
            f.locked = true;
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
        silk: place::board_silk(&l.graphics, &l.artwork),
        texts: place::movable_texts(&l.graphics),
    };
    let opts = place::PlaceOptions {
        parts: free.clone(),
        seed: cfg.place.as_ref().and_then(|p| p.seed).unwrap_or(1),
        spacing: spread.spacing,
        standoff: spread.standoff,
        ..Default::default()
    };
    let before = hpwl(bd);
    let result = place::place(&input, &opts)?;
    for pm in &result.placements {
        if let Some(c) = bd.cells.iter_mut().find(|c| c.reference == pm.reference) {
            let delta = pm.rotation - c.rotation;
            rotate_cell(c, delta);
            c.at = pm.at;
        }
    }
    let note = format!(
        "{} parts placed around {} chained, hpwl {before:.0} -> {:.0} mm, {} crossings",
        free.len(),
        bd.cells.iter().filter(|c| c.chained).count(),
        hpwl(bd),
        result.after.crossings
    );
    *texts = result.texts_moved.clone();
    Ok((note, result.failed.iter().map(|f| format!("{f}: not placed")).collect()))
}

fn rect_of(c: &Cell, at: P, gap: f64) -> [f64; 4] {
    let cx = at[0] + c.box_off[0];
    let cy = at[1] + c.box_off[1];
    [cx - c.half[0] - gap, cy - c.half[1] - gap, cx + c.half[0] + gap, cy + c.half[1] + gap]
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn facing_cost(bd: &Board, ci: usize, c: &Cell, on_net: &HashMap<usize, Vec<usize>>) -> f64 {
    let centre = [c.at[0] + c.box_off[0], c.at[1] + c.box_off[1]];
    let mut cost = 0.0;
    for &(n, off) in &c.pins {
        let Some(&w) = bd.weight.get(&n) else { continue };
        let pin = [c.at[0] + off[0], c.at[1] + off[1]];
        let partner = on_net[&n]
            .iter()
            .filter(|&&cj| cj != ci)
            .flat_map(|&cj| {
                let d = &bd.cells[cj];
                d.pins
                    .iter()
                    .filter(move |p| p.0 == n)
                    .map(move |p| [d.at[0] + p.1[0], d.at[1] + p.1[1]])
            })
            .min_by(|a, b| geom::dist(*a, pin).total_cmp(&geom::dist(*b, pin)));
        if let Some(t) = partner {
            cost += w
                * (crate::score::behind(centre, c.half, pin, t) * FACING_WEIGHT
                    + geom::dist(pin, t));
        }
    }
    cost
}

fn face(bd: &mut Board) -> usize {
    let mut on_net: HashMap<usize, Vec<usize>> = HashMap::new();
    for (ci, c) in bd.cells.iter().enumerate() {
        for &(n, _) in &c.pins {
            let v = on_net.entry(n).or_default();
            if v.last() != Some(&ci) {
                v.push(ci);
            }
        }
    }
    let mut turned = 0;
    for ci in 0..bd.cells.len() {
        let c = &bd.cells[ci];
        if c.fixed || c.chained || c.pin_count > FACE_PINS || c.pins.len() < 2 {
            continue;
        }
        let here = facing_cost(bd, ci, c, &on_net);
        let mut best = (here, 0.0);
        for rot in [90.0, 180.0, 270.0] {
            let mut t = bd.cells[ci].clone();
            rotate_cell(&mut t, rot);
            let cost = facing_cost(bd, ci, &t, &on_net);
            if cost < best.0 - 1e-6 {
                best = (cost, rot);
            }
        }
        if best.1 != 0.0 {
            rotate_cell(&mut bd.cells[ci], best.1);
            turned += 1;
        }
    }
    turned
}

fn legalise(
    model: &Model,
    bd: &mut Board,
    grow: &HashMap<usize, f64>,
    spacing: f64,
) -> (String, Vec<String>) {
    let edge = model.board.rules.min_copper_to_edge.to_mm().max(0.3);
    let keep: Vec<Bounds> = bd
        .keepouts
        .iter()
        .map(|k| {
            let mut b = Bounds::EMPTY;
            k.iter().for_each(|q| b.add(*q));
            b
        })
        .collect();
    let anchored: Vec<&str> = model
        .layout
        .parts
        .iter()
        .filter(|p| {
            place::edge_mount(&p.footprint)
                || place::role_of(&p.reference, &p.footprint_name, &p.footprint)
                    == place::Role::Hole
        })
        .map(|p| p.reference.as_str())
        .collect();
    let held = |c: &Cell| c.fixed || anchored.contains(&c.reference.as_str());
    let gap_of = |i: usize| grow.get(&i).copied().unwrap_or(0.0);
    let mut placed: Vec<[f64; 4]> = (0..bd.cells.len())
        .filter(|&i| held(&bd.cells[i]))
        .map(|i| rect_of(&bd.cells[i], bd.cells[i].at, gap_of(i)))
        .collect();
    let mut order: Vec<usize> = (0..bd.cells.len()).filter(|&i| !held(&bd.cells[i])).collect();
    order.sort_by(|&a, &b| {
        let (ca, cb) = (&bd.cells[a], &bd.cells[b]);
        cb.chained.cmp(&ca.chained).then(cb.area.total_cmp(&ca.area))
    });
    let inside = |r: [f64; 4]| {
        let corners = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
        let o = &bd.outline;
        corners.iter().all(|q| geom::point_in_polygon(*q, o))
            && (0..o.len()).all(|e| {
                let (a, b) = (o[e], o[(e + 1) % o.len()]);
                (0..4).all(|k| {
                    geom::segment_segment_distance(corners[k], corners[(k + 1) % 4], a, b) >= edge
                })
            })
    };
    let mut failed = Vec::new();
    let mut moved = 0.0;
    for &i in &order {
        let want = bd.cells[i].at;
        let mut found = None;
        'search: for (ring, gap) in (0..400)
            .map(|r| (r, gap_of(i) + spacing))
            .chain((0..400).map(|r| (r, spacing.min(SPACING))))
        {
            let r = ring as f64 * 0.1;
            let steps = if ring == 0 { 1 } else { (ring * 8).min(160) };
            for k in 0..steps {
                let a = k as f64 / steps as f64 * std::f64::consts::TAU;
                let at = [want[0] + r * a.cos(), want[1] + r * a.sin()];
                let rots: &[f64] = if bd.cells[i].chained { &[0.0] } else { &[0.0, 90.0] };
                for &rot in rots {
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
                    found = Some((at, rot, rc));
                    break 'search;
                }
            }
        }
        match found {
            Some((at, rot, rc)) => {
                moved += geom::dist(at, want);
                rotate_cell(&mut bd.cells[i], rot);
                bd.cells[i].at = at;
                placed.push(rc);
            }
            None => failed.push(format!("{}: no free spot", bd.cells[i].reference)),
        }
    }
    (
        format!(
            "{} parts legal, {:.1} mm moved in total, hpwl {:.0} mm",
            order.len() - failed.len(),
            moved,
            hpwl(bd)
        ),
        failed,
    )
}

impl Phase for Place {
    fn name(&self) -> &'static str {
        "place"
    }

    fn run(&self, model: &mut Model, cfg: &EngineFile) -> PhaseReport {
        let mut report = PhaseReport { phase: "place".into(), ..Default::default() };
        let mut bd = build(model);
        let mut plan = model.placement.clone().unwrap_or_default();
        apply_plan(&mut bd, &plan);
        let mut grow: HashMap<usize, f64> = HashMap::new();
        let spread = spread_of(cfg, model.pass);
        report.notes.push(format!(
            "spread pass {}: {:.2} mm between parts, decaps pulled no closer than {:.2} mm",
            model.pass + 1,
            spread.spacing,
            spread.standoff
        ));
        let chains = model.constraints.as_ref().map(|g| g.chains.clone()).unwrap_or_default();
        let signal: Vec<bool> = model
            .layout
            .nets
            .iter()
            .map(|n| !place::is_power_net(model.board, &n.name, &n.class))
            .collect();
        let laid = template_chains(&mut bd, &chains, &signal);
        if !laid.is_empty() {
            plan.chains = laid;
        }
        for ch in &plan.chains {
            report.notes.push(format!("chain in a line: {}", ch.join(" > ")));
        }
        match global_place(model, cfg, &mut bd, &mut plan.texts, &spread) {
            Ok((note, failed)) => {
                report.notes.push(note);
                report.failed.extend(failed);
            }
            Err(e) => report.failed.push(e),
        }
        if !model.hot.is_empty() {
            for (i, c) in bd.cells.iter().enumerate() {
                let r = rect_of(c, c.at, 0.0);
                let over: f64 = model
                    .hot
                    .iter()
                    .filter(|h| {
                        let s = h.size / 2.0;
                        overlaps(r, [h.at[0] - s, h.at[1] - s, h.at[0] + s, h.at[1] + s])
                    })
                    .map(|h| h.overflow)
                    .sum();
                if over > 0.0 && !c.fixed {
                    grow.insert(i, (over * HOT_GROW).min(MAX_GROW));
                }
            }
            report.notes.push(format!(
                "{} hot tiles, {} parts inflated by their overflow",
                model.hot.len(),
                grow.len()
            ));
        }
        let turned = face(&mut bd);
        if turned > 0 {
            report.notes.push(format!("{turned} parts turned to face what they connect to"));
        }
        let (note, failed) = legalise(model, &mut bd, &grow, spread.spacing);
        report.notes.push(note);
        report.failed.extend(failed);
        report.changed = true;
        plan.moves = moves_of(&bd);
        model.placement = Some(plan);
        report
    }
}
