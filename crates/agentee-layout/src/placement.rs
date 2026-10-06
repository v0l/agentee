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
    pub bottom: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hide_label: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct UnderVia {
    pub net: String,
    pub at: P,
    pub via: String,
    pub stub: Option<(String, P, f64)>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PlacePlan {
    pub chains: Vec<Vec<String>>,
    pub moves: Vec<Move>,
    pub under: Vec<UnderVia>,
    #[serde(skip)]
    pub texts: Vec<place::TextMove>,
}

#[derive(Clone, Debug)]
struct Cell {
    reference: String,
    fixed: bool,
    chained: bool,
    under: bool,
    at: P,
    rotation: f64,
    bottom: bool,
    through: bool,
    box_off: P,
    half: [f64; 2],
    area: f64,
    pins: Vec<(usize, P)>,
    numbers: Vec<String>,
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

fn flip_cell(c: &mut Cell, rotation: f64, bottom: bool) {
    let back = |w: P| {
        let l = geom::rotate(w, -c.rotation);
        if c.bottom { [-l[0], l[1]] } else { l }
    };
    let ahead = |l: P| geom::rotate(if bottom { [-l[0], l[1]] } else { l }, rotation);
    let turn = (rotation - c.rotation).rem_euclid(180.0);
    c.box_off = ahead(back(c.box_off));
    for p in c.pins.iter_mut() {
        p.1 = ahead(back(p.1));
    }
    if (turn - 90.0).abs() < 1e-9 {
        c.half = [c.half[1], c.half[0]];
    }
    c.rotation = rotation.rem_euclid(360.0);
    c.bottom = bottom;
}

fn decaps_below(cfg: &EngineFile) -> bool {
    cfg.place
        .as_ref()
        .and_then(|p| p.bga_decaps.as_deref())
        .is_none_or(|s| !s.eq_ignore_ascii_case("top"))
}

fn bga_decaps<'a>(model: &'a Model) -> Vec<&'a crate::constraints::Decap> {
    let Some(g) = model.constraints.as_ref() else { return Vec::new() };
    let l = &model.layout;
    g.decaps
        .iter()
        .filter(|d| {
            l.parts.iter().any(|p| {
                p.reference == d.ic
                    && !p.bottom
                    && crate::escape::is_bga(p)
                    && place::role_of(&p.reference, &p.footprint_name, &p.footprint)
                        == place::Role::Chip
            })
        })
        .collect()
}

fn build(model: &Model) -> Board {
    let movable_bottom: Vec<String> = bga_decaps(model).iter().map(|d| d.cap.clone()).collect();
    let l = &model.layout;
    let b = model.board;
    let locked: Vec<&str> =
        model.file.footprints.iter().filter(|f| f.locked).map(|f| f.reference.as_str()).collect();
    let mut cells = Vec::new();
    for p in &l.parts {
        let at = p.at.to_mm();
        let mut bb = Bounds::EMPTY;
        let t = p.transform();
        for lp in place::courtyard_loops(&p.footprint, "F.CrtYd") {
            for q in lp {
                bb.add(t.apply(q));
            }
        }
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
        let mut numbers = Vec::new();
        for pad in &p.pads {
            let Some(net) = pad.net else { continue };
            let mut pb = Bounds::EMPTY;
            pad.outlines.iter().flatten().for_each(|q| pb.add(*q));
            if pb.is_empty() {
                continue;
            }
            let q = pb.center();
            pins.push((net, [q[0] - at[0], q[1] - at[1]]));
            numbers.push(pad.number.clone());
        }
        let mounting = p.pads.iter().all(|q| q.net.is_none());
        let flips = p.bottom && movable_bottom.contains(&p.reference);
        let mut cell = Cell {
            reference: p.reference.clone(),
            fixed: locked.contains(&p.reference.as_str()) || mounting || (p.bottom && !flips),
            chained: false,
            under: false,
            at,
            rotation: p.rotation,
            bottom: p.bottom,
            through: p.pads.iter().any(|q| q.drill.is_some()),
            box_off: [c[0] - at[0], c[1] - at[1]],
            half,
            area: 4.0 * half[0] * half[1],
            pin_count: p.pads.len(),
            pins,
            numbers,
        };
        if flips {
            flip_cell(&mut cell, p.rotation, false);
        }
        cells.push(cell);
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
            bottom: c.bottom,
            hide_label: c.under,
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
    let free: Vec<String> = bd
        .cells
        .iter()
        .filter(|c| !c.fixed && !c.chained && !c.under)
        .map(|c| c.reference.clone())
        .collect();
    let l = &model.layout;
    let footprints: HashMap<&str, &agentee_core::footprint::Footprint> =
        l.parts.iter().map(|p| (p.footprint_name.as_str(), &p.footprint)).collect();
    let mut fast: Vec<String> = model.file.interfaces.iter().flat_map(|f| f.nets.clone()).collect();
    for pr in &model.file.pairs {
        fast.push(pr.p.clone());
        fast.push(pr.n.clone());
    }
    let mut placements = model.file.footprints.clone();
    for c in bd.cells.iter().filter(|c| !c.fixed) {
        if let Some(f) = placements.iter_mut().find(|f| f.reference == c.reference) {
            f.side = c.bottom.then_some(agentee_core::layout::BoardSide::Bottom);
            if c.chained || c.under {
                f.at = agentee_core::units::Point::mm(c.at[0], c.at[1]);
                f.rotation = Some(c.rotation);
                f.locked = true;
            }
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
            flip_cell(c, pm.rotation, pm.bottom);
            c.at = pm.at;
        }
    }
    let note = format!(
        "{} parts placed around {} chained and {} under BGAs, hpwl {before:.0} -> {:.0} mm, {} crossings",
        free.len(),
        bd.cells.iter().filter(|c| c.chained).count(),
        bd.cells.iter().filter(|c| c.under).count(),
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

fn rect_gap(q: P, c: P, h: [f64; 2]) -> f64 {
    let dx = ((q[0] - c[0]).abs() - h[0]).max(0.0);
    let dy = ((q[1] - c[1]).abs() - h[1]).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

struct BackPad {
    at: P,
    half: [f64; 2],
    net: usize,
}

fn under_bgas(
    model: &Model,
    cfg: &EngineFile,
    bd: &mut Board,
    out: &mut Vec<UnderVia>,
) -> (usize, usize) {
    out.clear();
    let l = &model.layout;
    let decaps = bga_decaps(model);
    let in_pad_cfg = cfg.access.as_ref().and_then(|a| a.via_in_pad).unwrap_or(false);
    let ground = |n: usize| place::is_ground(&l.nets[n].name);
    let clear = |a: usize, b: usize| l.nets[a].clearance.max(l.nets[b].clearance);
    let g = cfg
        .detail
        .as_ref()
        .and_then(|d| d.grid)
        .map(|g| g.to_mm())
        .unwrap_or(crate::negotiate::Options::default().grid);
    let slack = g * 0.1;
    let phase = crate::negotiate::ball_phase(l, g);
    let snaps = |s: P| -> Vec<P> {
        let t = [(s[0] - phase[0]) / g, (s[1] - phase[1]) / g];
        let near = |v: f64| {
            let mut k = vec![v.round(), (v - 0.5).round(), (v + 0.5).round()];
            k.sort_by(f64::total_cmp);
            k.dedup();
            k
        };
        let mut out: Vec<P> = near(t[0])
            .into_iter()
            .flat_map(|x| near(t[1]).into_iter().map(move |y| [x, y]))
            .map(|[x, y]| [phase[0] + x * g, phase[1] + y * g])
            .filter(|q| geom::dist(*q, s) <= 0.75 * g + 1e-9)
            .collect();
        out.sort_by(|a, b| geom::dist(*a, s).total_cmp(&geom::dist(*b, s)));
        out
    };
    let hole_smd = model.board.rules.min_hole_to_smd_pad.to_mm();
    let hole_to_hole = model.board.rules.min_hole_to_hole.to_mm();
    let mut taken: Vec<[f64; 4]> = bd
        .cells
        .iter()
        .filter(|c| (c.bottom || c.through) && !c.under)
        .map(|c| rect_of(c, c.at, 0.0))
        .collect();
    let mut ics: Vec<&str> = decaps.iter().map(|d| d.ic.as_str()).collect();
    ics.dedup();
    ics.sort();
    ics.dedup();
    let (mut placed, mut wanted) = (0, 0);
    for ic in ics {
        let Some(part) = l.parts.iter().find(|p| p.reference == ic) else { continue };
        let Some(ic_i) = bd.cells.iter().position(|c| c.reference == ic) else { continue };
        let pitch = crate::escape::pitch_of(part);
        let half = pitch / 2.0;
        let fit = crate::escape::via_fit(model, part, in_pad_cfg);
        let via_r = fit.diameter / 2.0;
        let drill_r = fit.drill / 2.0;
        let same_reach = via_r.max(drill_r + hole_smd) + 1e-3 + slack;
        let cell = bd.cells[ic_i].clone();
        let balls: Vec<(usize, P, String)> = cell
            .pins
            .iter()
            .zip(&cell.numbers)
            .map(|(&(n, o), num)| (n, [cell.at[0] + o[0], cell.at[1] + o[1]], num.clone()))
            .collect();
        if balls.is_empty() {
            continue;
        }
        let origin = balls[0].1;
        let key = |q: P| {
            (((q[0] - origin[0]) / half).round() as i64, ((q[1] - origin[1]) / half).round() as i64)
        };
        let corners = |q: P| {
            [[-half, -half], [half, -half], [-half, half], [half, half]]
                .map(|d| [q[0] + d[0], q[1] + d[1]])
        };
        let sites: Vec<(usize, P)> = if fit.in_pad {
            balls.iter().enumerate().map(|(i, b)| (i, b.1)).collect()
        } else {
            balls.iter().enumerate().flat_map(|(i, b)| corners(b.1).map(|c| (i, c))).collect()
        };
        let mut at_key: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (k, s) in sites.iter().enumerate() {
            at_key.entry(key(s.1)).or_default().push(k);
        }
        let mut used_balls: Vec<usize> = Vec::new();
        let mut sites_used: Vec<P> = Vec::new();
        let mut vias: Vec<(P, usize)> = Vec::new();
        let ball_shape: Vec<Vec<Vec<P>>> = part
            .pads
            .iter()
            .filter(|q| q.net.is_some())
            .filter_map(|q| {
                let mut b = Bounds::EMPTY;
                q.outlines.iter().flatten().for_each(|r| b.add(*r));
                let c = (!b.is_empty()).then(|| b.center())?;
                Some(
                    q.outlines
                        .iter()
                        .map(|o| o.iter().map(|r| [r[0] - c[0], r[1] - c[1]]).collect())
                        .collect(),
                )
            })
            .collect();
        let front_ok = |v: P, n: usize, own: usize| {
            balls.iter().zip(&ball_shape).enumerate().all(|(k, (b, shape))| {
                if geom::dist(v, b.1) > pitch * 2.0 {
                    return true;
                }
                let q = [v[0] - b.1[0], v[1] - b.1[1]];
                let gap = shape
                    .iter()
                    .map(|o| {
                        let edge = (0..o.len())
                            .map(|i| geom::point_segment_distance(q, o[i], o[(i + 1) % o.len()]))
                            .fold(f64::MAX, f64::min);
                        if geom::point_in_polygon(q, o) { -edge } else { edge }
                    })
                    .fold(f64::MAX, f64::min);
                if b.0 != n {
                    gap >= via_r + clear(b.0, n) + slack && gap >= same_reach
                } else if k == own && fit.in_pad {
                    gap <= -(via_r + 1e-3)
                } else {
                    gap >= same_reach
                }
            })
        };
        let holes_ok = |a: P, na: usize, b: P, nb: usize| {
            let d = geom::dist(a, b);
            if na == nb && d < 1e-6 {
                return true;
            }
            d >= fit.drill + hole_to_hole && (na == nb || d >= 2.0 * via_r + clear(na, nb))
        };
        let mut back: Vec<BackPad> = Vec::new();
        let mut caps: Vec<&&crate::constraints::Decap> =
            decaps.iter().filter(|d| d.ic == ic).collect();
        caps.sort_by(|a, b| a.farads.total_cmp(&b.farads));
        for d in caps {
            let Some(ci) = bd.cells.iter().position(|c| c.reference == d.cap) else { continue };
            let c = &bd.cells[ci];
            if c.fixed || c.chained || c.through || c.pins.len() != 2 {
                continue;
            }
            let Some(rail) = l.nets.iter().position(|n| n.name == d.net) else { continue };
            let Some(cp) = l.parts.iter().find(|p| p.reference == d.cap) else { continue };
            let turned = |r: f64| (r.rem_euclid(180.0) - 90.0).abs() < 1e-6;
            let local: Vec<(usize, [f64; 2])> = cp
                .pads
                .iter()
                .filter(|q| !q.copper.is_empty())
                .filter_map(|q| {
                    let mut b = Bounds::EMPTY;
                    q.outlines.iter().flatten().for_each(|r| b.add(*r));
                    let [w, h] = b.size();
                    let half =
                        if turned(cp.rotation) { [h / 2.0, w / 2.0] } else { [w / 2.0, h / 2.0] };
                    Some((q.net?, half))
                })
                .collect();
            let (Some(&(_, rail_half)), Some(&(gnd, gnd_half))) = (
                local.iter().find(|p| p.0 == rail),
                local.iter().find(|p| p.0 != rail && ground(p.0)),
            ) else {
                continue;
            };
            let span = geom::dist(c.pins[0].1, c.pins[1].1);
            if span > 1.5 * pitch {
                continue;
            }
            wanted += 1;
            let bound = balls.iter().find(|b| b.2 == d.pin).map(|b| b.1).unwrap_or(cell.at);
            let free_site = |s: P| !sites_used.iter().any(|v| geom::dist(*v, s) < 1e-3);
            let mut pairs: Vec<(f64, usize, usize, P, P)> = Vec::new();
            for &(b1, s1) in &sites {
                if balls[b1].0 != rail || used_balls.contains(&b1) || !free_site(s1) {
                    continue;
                }
                let k = key(s1);
                for (dx, dy) in [(2, 0), (-2, 0), (0, 2), (0, -2)] {
                    for &j in at_key.get(&(k.0 + dx, k.1 + dy)).into_iter().flatten() {
                        let (b2, s2) = sites[j];
                        if balls[b2].0 != gnd || used_balls.contains(&b2) || !free_site(s2) {
                            continue;
                        }
                        let cost = geom::dist(balls[b1].1, bound) + 0.1 * geom::dist(s2, bound);
                        pairs.push((cost, b1, b2, s1, s2));
                    }
                }
            }
            pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
            'pairs: for &(_, b1, b2, s1, s2) in &pairs {
                for v1 in snaps(s1) {
                    if !front_ok(v1, rail, b1) {
                        continue;
                    }
                    for v2 in snaps(s2) {
                        let run = [v2[0] - v1[0], v2[1] - v1[1]];
                        let want = [s2[0] - s1[0], s2[1] - s1[1]];
                        if (run[0] * want[1] - run[1] * want[0]).abs() > 1e-6
                            || run[0] * want[0] + run[1] * want[1] <= 0.0
                            || !front_ok(v2, gnd, b2)
                            || !holes_ok(v1, rail, v2, gnd)
                            || vias.iter().any(|&(q, n)| {
                                !holes_ok(q, n, v1, rail) || !holes_ok(q, n, v2, gnd)
                            })
                        {
                            continue;
                        }
                        for rot in [0.0, 90.0, 180.0, 270.0] {
                            let mut t = bd.cells[ci].clone();
                            flip_cell(&mut t, rot, true);
                            let (Some(o1), Some(o2)) = (
                                t.pins.iter().find(|p| p.0 == rail).map(|p| p.1),
                                t.pins.iter().find(|p| p.0 == gnd).map(|p| p.1),
                            ) else {
                                continue;
                            };
                            let along = [o2[0] - o1[0], o2[1] - o1[1]];
                            if (along[0] * run[1] - along[1] * run[0]).abs() > 1e-6
                                || along[0] * run[0] + along[1] * run[1] <= 0.0
                            {
                                continue;
                            }
                            let mid = [(v1[0] + v2[0]) / 2.0, (v1[1] + v2[1]) / 2.0];
                            t.at = [mid[0] - (o1[0] + o2[0]) / 2.0, mid[1] - (o1[1] + o2[1]) / 2.0];
                            let size = |h: [f64; 2]| if turned(rot) { [h[1], h[0]] } else { h };
                            let pads = [
                                BackPad {
                                    at: [t.at[0] + o1[0], t.at[1] + o1[1]],
                                    half: size(rail_half),
                                    net: rail,
                                },
                                BackPad {
                                    at: [t.at[0] + o2[0], t.at[1] + o2[1]],
                                    half: size(gnd_half),
                                    net: gnd,
                                },
                            ];
                            let holds = |p: &BackPad, v: P| {
                                (0..2).all(|k| (v[k] - p.at[k]).abs() + via_r + 1e-3 <= p.half[k])
                            };
                            if !holds(&pads[0], v1) || !holds(&pads[1], v2) {
                                continue;
                            }
                            let r = rect_of(&t, t.at, 0.0);
                            if taken.iter().any(|q| overlaps(r, *q)) {
                                continue;
                            }
                            let mut spots: Vec<(P, usize)> = vias.clone();
                            spots.push((v1, rail));
                            spots.push((v2, gnd));
                            if fit.in_pad {
                                spots.extend(
                                    balls
                                        .iter()
                                        .enumerate()
                                        .filter(|(i, _)| *i != b1 && *i != b2)
                                        .map(|(_, b)| (b.1, b.0)),
                                );
                            }
                            let pad_ok = |q: P, n: usize, p: &BackPad| {
                                if p.net == n {
                                    holds(p, q) || rect_gap(q, p.at, p.half) >= same_reach
                                } else {
                                    rect_gap(q, p.at, p.half) >= via_r + clear(p.net, n) + slack
                                }
                            };
                            if !spots.iter().all(|&(q, n)| pads.iter().all(|p| pad_ok(q, n, p))) {
                                continue;
                            }
                            if !back.iter().all(|p| pad_ok(v1, rail, p) && pad_ok(v2, gnd, p)) {
                                continue;
                            }
                            if !fit.in_pad {
                                let mut all: Vec<&BackPad> = back.iter().collect();
                                all.extend(pads.iter());
                                let reach = 2.0 * pitch;
                                let stranded = balls.iter().enumerate().any(|(i, b)| {
                                    i != b1
                                        && i != b2
                                        && !used_balls.contains(&i)
                                        && geom::dist(b.1, mid) < reach
                                        && corners(b.1).iter().all(|&q| {
                                            !free_site(q)
                                                || geom::dist(q, v1) < pitch / 2.0
                                                || geom::dist(q, v2) < pitch / 2.0
                                                || all.iter().any(|p| !pad_ok(q, b.0, p))
                                        })
                                });
                                if stranded {
                                    continue;
                                }
                            }
                            for (v, n, b) in [(v1, rail, b1), (v2, gnd, b2)] {
                                let ball = balls[b].1;
                                let off = geom::dist(v, ball) > 1e-6 && !fit.in_pad;
                                let width = l.nets[n].width.min(
                                    ball_shape[b]
                                        .iter()
                                        .flatten()
                                        .map(|q| geom::dist(*q, [0.0, 0.0]))
                                        .fold(f64::MAX, f64::min)
                                        * 2.0,
                                );
                                out.push(UnderVia {
                                    net: l.nets[n].name.clone(),
                                    at: v,
                                    via: fit.name.clone(),
                                    stub: off.then(|| (l.copper[0].clone(), ball, width)),
                                });
                            }
                            t.under = true;
                            bd.cells[ci] = t;
                            taken.push(r);
                            used_balls.extend([b1, b2]);
                            sites_used.extend([s1, s2]);
                            vias.extend([(v1, rail), (v2, gnd)]);
                            back.extend(pads);
                            placed += 1;
                            break 'pairs;
                        }
                    }
                }
            }
        }
    }
    (placed, wanted)
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
        if c.fixed || c.chained || c.under || c.pin_count > FACE_PINS || c.pins.len() < 2 {
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
    let held =
        |c: &Cell| c.fixed || c.chained || c.under || anchored.contains(&c.reference.as_str());
    let gap_of = |i: usize| grow.get(&i).copied().unwrap_or(0.0);
    let side = |c: &Cell| {
        if c.through {
            3u8
        } else if c.bottom {
            2
        } else {
            1
        }
    };
    let mut placed: Vec<([f64; 4], u8)> = (0..bd.cells.len())
        .filter(|&i| held(&bd.cells[i]))
        .map(|i| (rect_of(&bd.cells[i], bd.cells[i].at, gap_of(i)), side(&bd.cells[i])))
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
                        || placed.iter().any(|p| p.1 & side(&c) != 0 && overlaps(rc, p.0))
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
                placed.push((rc, side(&bd.cells[i])));
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
        let below = decaps_below(cfg);
        let bgas_held =
            bd.cells.iter().all(|c| {
                c.fixed
                    || c.chained
                    || !model.layout.parts.iter().any(|p| {
                        p.reference == c.reference && crate::escape::is_bga(p) && !p.bottom
                    })
            });
        let mut under = (0, 0);
        if below && bgas_held {
            under = under_bgas(model, cfg, &mut bd, &mut plan.under);
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
                if over > 0.0 && !c.fixed && !c.chained && !c.under {
                    grow.insert(i, (over * HOT_GROW).min(MAX_GROW));
                }
            }
            report.notes.push(format!(
                "{} hot tiles, {} parts inflated by their overflow",
                model.hot.len(),
                grow.len()
            ));
        }
        if below && !bgas_held {
            under = under_bgas(model, cfg, &mut bd, &mut plan.under);
        }
        if below {
            report.notes.push(format!(
                "{} of {} BGA decaps on the back, each pad on a ball's via",
                under.0, under.1
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
