use crate::field::CostField;
use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::{Direction, EngineFile};
use agentee_core::geom::{self, P};
use agentee_core::graphic::Bounds;
use agentee_core::place;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

pub struct Global;

#[derive(Clone, Debug, Default, Serialize)]
pub struct GlobalPlan {
    pub corridors: Vec<Corridor>,
    pub rounds: u32,
    pub overflow: f64,
    pub unrouted: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Corridor {
    pub net: String,
    pub tiles: Vec<(String, [f64; 2], [f64; 2])>,
    pub vias: usize,
    pub length_mm: f64,
}

#[derive(Clone, Copy, PartialEq)]
struct Node {
    f: f64,
    i: usize,
}
impl Eq for Node {}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.partial_cmp(&self.f).unwrap_or(Ordering::Equal).then(o.i.cmp(&self.i))
    }
}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

struct NetTask {
    net: usize,
    prefer: Option<usize>,
    load: f32,
    layers: Vec<usize>,
    conns: Vec<(usize, usize)>,
    terminals: Vec<Term>,
}

#[derive(Clone)]
struct Term {
    tile: (usize, usize),
    layers: Vec<usize>,
}

impl Phase for Global {
    fn name(&self) -> &'static str {
        "global"
    }

    fn run(&self, model: &mut Model, cfg: &EngineFile, field: &mut CostField) -> PhaseReport {
        let mut report = PhaseReport { phase: "global".into(), ..Default::default() };
        let l = &model.layout;
        let copper = &l.copper;
        let plane_layers = plane_layers(model);
        let direction: HashMap<usize, Direction> = cfg
            .global
            .as_ref()
            .map(|g| {
                g.direction
                    .iter()
                    .filter_map(|(k, v)| copper.iter().position(|c| c == k).map(|i| (i, *v)))
                    .collect()
            })
            .unwrap_or_default();
        let via_cost =
            cfg.global.as_ref().and_then(|g| g.via_cost).map(|v| v.to_mm()).unwrap_or(1.0);
        let rounds = cfg.rounds.unwrap_or(crate::DEFAULT_ROUNDS).max(1) * 8;

        capacity(model, field, &plane_layers);
        let tasks = tasks(model, field, &plane_layers);
        let nx = field.nx;
        let ny = field.ny;
        let nl = field.layers.len();
        let idx = |l: usize, x: usize, y: usize| (l * ny + y) * nx + x;
        let mut demand_of: Vec<Vec<usize>> = vec![Vec::new(); tasks.len()];
        let mut paths: Vec<Vec<Vec<usize>>> = vec![Vec::new(); tasks.len()];
        let mut pres = 1.0f32;
        let mut done_rounds = 0;
        let mut best_over = f64::MAX;
        let mut stale = 0;
        for round in 0..rounds {
            done_rounds = round + 1;
            for (ti, t) in tasks.iter().enumerate() {
                let dirty = round == 0
                    || paths[ti].len() < t.conns.len()
                    || demand_of[ti].iter().any(|&c| field.demand[c] > field.capacity[c].max(1.0));
                if !dirty {
                    continue;
                }
                for &c in &demand_of[ti] {
                    field.demand[c] -= t.load;
                }
                demand_of[ti].clear();
                paths[ti].clear();
                let mut new_cells = Vec::new();
                for &(a, b) in &t.conns {
                    let path = dijkstra(
                        field,
                        &t.terminals[a],
                        &t.terminals[b],
                        &t.layers,
                        &direction,
                        via_cost,
                        pres,
                        idx,
                        nl,
                        t.load,
                        t.prefer,
                    );
                    if let Some(p) = path {
                        for &c in &p {
                            field.demand[c] += t.load;
                            new_cells.push(c);
                        }
                        paths[ti].push(p);
                    }
                }
                demand_of[ti] = new_cells;
            }
            let over = field.overflow();
            if over <= 0.0 {
                break;
            }
            if over < best_over * 0.97 {
                best_over = over;
                stale = 0;
            } else {
                stale += 1;
                if stale >= 4 {
                    break;
                }
            }
            field.add_history(1.0);
            pres *= 2.0;
        }
        let mut plan =
            GlobalPlan { rounds: done_rounds, overflow: field.overflow(), ..Default::default() };
        let mut per_layer = Vec::new();
        for layer in 0..nl {
            let mut over = 0.0;
            for y in 0..ny {
                for x in 0..nx {
                    let i = idx(layer, x, y);
                    over += (field.demand[i] - field.capacity[i].max(1.0)).max(0.0) as f64;
                }
            }
            if over > 0.0 {
                per_layer.push(format!("{} {over:.0}", copper[layer]));
            }
        }
        if !per_layer.is_empty() {
            report.notes.push(format!("overflow by layer: {}", per_layer.join(", ")));
        }
        for (ti, t) in tasks.iter().enumerate() {
            let name = l.nets[t.net].name.clone();
            if paths[ti].len() < t.conns.len() {
                plan.unrouted.push(name.clone());
            }
            let mut tiles = Vec::new();
            let mut vias = 0;
            let mut length = 0.0;
            for p in &paths[ti] {
                for w in p.windows(2) {
                    let (la, lb) = (w[0] / (nx * ny), w[1] / (nx * ny));
                    if la != lb {
                        vias += 1;
                    } else {
                        length += field.tile;
                    }
                }
                for &c in p {
                    let layer = c / (nx * ny);
                    let x = (c % (nx * ny)) % nx;
                    let y = (c % (nx * ny)) / nx;
                    let lo = [
                        field.origin[0] + x as f64 * field.tile,
                        field.origin[1] + y as f64 * field.tile,
                    ];
                    let hi = [lo[0] + field.tile, lo[1] + field.tile];
                    tiles.push((copper[layer].clone(), lo, hi));
                }
            }
            if !tiles.is_empty() {
                plan.corridors.push(Corridor { net: name, tiles, vias, length_mm: length });
            }
        }
        report.notes.push(format!(
            "{} nets, {} rounds, overflow {:.1}, {} without a corridor",
            tasks.len(),
            plan.rounds,
            plan.overflow,
            plan.unrouted.len()
        ));
        report.failed = plan.unrouted.iter().map(|n| format!("{n}: no corridor")).collect();
        report.changed = !plan.corridors.is_empty();
        model.global = Some(plan);
        report
    }
}

fn plane_layers(model: &Model) -> Vec<usize> {
    let copper = &model.layout.copper;
    model
        .file
        .zones
        .iter()
        .filter(|z| z.outline.as_ref().is_none_or(|o| o.is_empty()))
        .filter(|z| {
            model
                .layout
                .nets
                .iter()
                .find(|n| n.name == z.net)
                .is_some_and(|n| place::is_power_net(model.board, &n.name, &n.class))
        })
        .flat_map(|z| z.layers.iter().filter_map(|l| copper.iter().position(|c| c == l)))
        .filter(|&i| i != 0 && i + 1 != copper.len())
        .collect()
}

fn capacity(model: &Model, field: &mut CostField, plane_layers: &[usize]) {
    let l = &model.layout;
    let full = (field.tile / 0.2).max(1.0) as f32;
    for layer in 0..field.layers.len() {
        let cap = if plane_layers.contains(&layer) { 0.0 } else { full };
        field.fill_capacity(layer, cap);
    }
    let edge = l.edge();
    for y in 0..field.ny {
        for x in 0..field.nx {
            if !edge.contains(field.centre(x, y)) {
                for layer in 0..field.layers.len() {
                    let i = field.idx(layer, x, y);
                    field.capacity[i] = 0.0;
                }
            }
        }
    }
    let area = field.tile * field.tile;
    let taken = |at: P, size: [f64; 2], layers: &[usize], field: &mut CostField| {
        let lo = [at[0] - size[0] / 2.0, at[1] - size[1] / 2.0];
        let hi = [at[0] + size[0] / 2.0, at[1] + size[1] / 2.0];
        let (Some(a), Some(z)) = (field.tile_of(lo), field.tile_of(hi)) else { return };
        for y in a.1..=z.1 {
            for x in a.0..=z.0 {
                let tx0 = field.origin[0] + x as f64 * field.tile;
                let ty0 = field.origin[1] + y as f64 * field.tile;
                let w = (hi[0].min(tx0 + field.tile) - lo[0].max(tx0)).max(0.0);
                let h = (hi[1].min(ty0 + field.tile) - lo[1].max(ty0)).max(0.0);
                let frac = (w * h / area) as f32;
                for &layer in layers {
                    let i = field.idx(layer, x, y);
                    field.capacity[i] = (field.capacity[i] - full * frac).max(0.0);
                }
            }
        }
    };
    let clear = model.board.rules.min_clearance.to_mm();
    for p in &l.parts {
        for pad in &p.pads {
            let mut b = Bounds::EMPTY;
            pad.outlines.iter().flatten().for_each(|q| b.add(*q));
            if b.is_empty() {
                continue;
            }
            let layers: Vec<usize> =
                pad.copper.iter().filter_map(|c| l.copper.iter().position(|k| k == c)).collect();
            let [w, h] = b.size();
            taken(b.center(), [w + 2.0 * clear, h + 2.0 * clear], &layers, field);
        }
    }
    for v in &l.vias {
        let layers: Vec<usize> =
            v.layers.iter().filter_map(|c| l.copper.iter().position(|k| k == c)).collect();
        let d = v.diameter + 2.0 * clear;
        taken(v.at, [d, d], &layers, field);
    }
    for t in &l.tracks {
        let Some(layer) = l.copper.iter().position(|k| *k == t.layer) else { continue };
        let mut seen: Vec<usize> = Vec::new();
        for w in t.points.windows(2) {
            let n = (geom::dist(w[0], w[1]) / field.tile).ceil().max(1.0) as usize;
            for k in 0..=n {
                let s = k as f64 / n as f64;
                let q = [w[0][0] + (w[1][0] - w[0][0]) * s, w[0][1] + (w[1][1] - w[0][1]) * s];
                if let Some((x, y)) = field.tile_of(q) {
                    let i = field.idx(layer, x, y);
                    if !seen.contains(&i) {
                        seen.push(i);
                        field.capacity[i] =
                            (field.capacity[i] - ((t.width + clear) / 0.2) as f32).max(0.0);
                    }
                }
            }
        }
    }
}

fn tasks(model: &Model, field: &CostField, plane_layers: &[usize]) -> Vec<NetTask> {
    let l = &model.layout;
    let b = model.board;
    let mut out = Vec::new();
    for (ni, net) in l.nets.iter().enumerate() {
        let power = place::is_power_net(b, &net.name, &net.class);
        if power && place::is_ground(&net.name) {
            continue;
        }
        let class = b.netclasses.iter().find(|c| c.name == net.class);
        let layers: Vec<usize> = match class.filter(|c| !c.layers.is_empty()) {
            Some(c) => {
                c.layers.iter().filter_map(|n| l.copper.iter().position(|k| k == n)).collect()
            }
            None => (0..l.copper.len()).filter(|i| !plane_layers.contains(i)).collect(),
        };
        let escaped: Vec<(usize, usize)> = l
            .tracks
            .iter()
            .filter(|t| t.net == ni)
            .filter_map(|t| field.tile_of(*t.points.first()?))
            .collect();
        let mut terms: Vec<Term> = Vec::new();
        let mut push = |at: P, lay: Vec<usize>| {
            let Some(t) = field.tile_of(at) else { return };
            if let Some(e) = terms.iter_mut().find(|e| e.tile == t) {
                for x in lay {
                    if !e.layers.contains(&x) {
                        e.layers.push(x);
                    }
                }
            } else {
                terms.push(Term { tile: t, layers: lay });
            }
        };
        if power {
            for &(a, z, n) in &l.ratsnest {
                if n == ni {
                    push(a, layers.clone());
                    push(z, layers.clone());
                }
            }
        }
        for p in l.parts.iter().filter(|_| !power) {
            for pad in p.pads.iter().filter(|q| q.net == Some(ni)) {
                let mut bb = Bounds::EMPTY;
                pad.outlines.iter().flatten().for_each(|q| bb.add(*q));
                let lay: Vec<usize> = pad
                    .copper
                    .iter()
                    .filter_map(|c| l.copper.iter().position(|k| k == c))
                    .collect();
                if !bb.is_empty()
                    && field.tile_of(bb.center()).is_none_or(|t| !escaped.contains(&t))
                {
                    push(bb.center(), lay);
                }
            }
        }
        for t in l.tracks.iter().filter(|t| t.net == ni && !power) {
            let Some(layer) = l.copper.iter().position(|k| *k == t.layer) else { continue };
            if let Some(last) = t.points.last() {
                push(*last, vec![layer]);
            }
        }
        if terms.len() < 2 {
            continue;
        }
        let mut conns = Vec::new();
        let mut in_tree = vec![false; terms.len()];
        in_tree[0] = true;
        for _ in 1..terms.len() {
            let mut best: Option<(f64, usize, usize)> = None;
            for i in (0..terms.len()).filter(|&i| in_tree[i]) {
                for j in (0..terms.len()).filter(|&j| !in_tree[j]) {
                    let d = (terms[i].tile.0 as f64 - terms[j].tile.0 as f64).abs()
                        + (terms[i].tile.1 as f64 - terms[j].tile.1 as f64).abs();
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, i, j));
                    }
                }
            }
            let (_, i, j) = best.unwrap();
            in_tree[j] = true;
            conns.push((i, j));
        }
        let load = ((net.width + net.clearance) / 0.2).min(field.tile / 0.2) as f32;
        let prefer = model
            .layers
            .as_ref()
            .and_then(|p| p.layer_of(&net.name))
            .and_then(|ly| l.copper.iter().position(|c| c == ly));
        out.push(NetTask { net: ni, prefer, load, layers, conns, terminals: terms });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn dijkstra(
    field: &CostField,
    a: &Term,
    b: &Term,
    layers: &[usize],
    direction: &HashMap<usize, Direction>,
    via_cost: f64,
    pres: f32,
    idx: impl Fn(usize, usize, usize) -> usize,
    nl: usize,
    width_tiles: f32,
    prefer: Option<usize>,
) -> Option<Vec<usize>> {
    let (nx, ny) = (field.nx, field.ny);
    let n = nx * ny * nl;
    let mut dist = vec![f64::INFINITY; n];
    let mut prev = vec![usize::MAX; n];
    let mut heap = BinaryHeap::new();
    let start_layers: Vec<usize> =
        if a.layers.is_empty() { layers.to_vec() } else { a.layers.clone() };
    for &l in &start_layers {
        let i = idx(l, a.tile.0, a.tile.1);
        dist[i] = 0.0;
        heap.push(Node { f: 0.0, i });
    }
    let goal_layers: Vec<usize> =
        if b.layers.is_empty() { layers.to_vec() } else { b.layers.clone() };
    let goals: Vec<usize> = goal_layers.iter().map(|&l| idx(l, b.tile.0, b.tile.1)).collect();
    let cell_cost = |i: usize| -> f64 {
        let cap = field.capacity[i].max(1.0);
        let d = field.demand[i];
        let blocked = if field.capacity[i] <= 0.0 { 2.0 } else { 0.0 };
        let over = blocked + ((d + width_tiles - cap).max(0.0) / cap) as f64;
        let off = if prefer.is_some_and(|p| p != i / (nx * ny)) { 2.0 } else { 0.0 };
        field.tile * (1.0 + off + pres as f64 * over) + field.history[i] as f64
    };
    let h = |i: usize| -> f64 {
        let x = (i % (nx * ny)) % nx;
        let y = (i % (nx * ny)) / nx;
        ((x as f64 - b.tile.0 as f64).abs() + (y as f64 - b.tile.1 as f64).abs()) * field.tile
    };
    while let Some(Node { f, i }) = heap.pop() {
        if f > dist[i] + h(i) + 1e-9 {
            continue;
        }
        if goals.contains(&i) {
            let mut path = vec![i];
            let mut c = i;
            while prev[c] != usize::MAX {
                c = prev[c];
                path.push(c);
            }
            path.reverse();
            return Some(path);
        }
        let l = i / (nx * ny);
        let x = (i % (nx * ny)) % nx;
        let y = (i % (nx * ny)) / nx;
        let mut step = |j: usize, cost: f64| {
            let nd = dist[i] + cost;
            if nd < dist[j] {
                dist[j] = nd;
                prev[j] = i;
                heap.push(Node { f: nd + h(j), i: j });
            }
        };
        for (dx, dy) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
            let (x2, y2) = (x as i64 + dx, y as i64 + dy);
            if x2 < 0 || y2 < 0 || x2 >= nx as i64 || y2 >= ny as i64 {
                continue;
            }
            let j = idx(l, x2 as usize, y2 as usize);
            let against = match direction.get(&l) {
                Some(Direction::H) => dy != 0,
                Some(Direction::V) => dx != 0,
                None => false,
            };
            let mut c = cell_cost(j);
            if against {
                c *= 2.0;
            }
            step(j, c);
        }
        for &l2 in layers {
            if l2 == l {
                continue;
            }
            let j = idx(l2, x, y);
            step(j, via_cost + cell_cost(j));
        }
    }
    None
}
