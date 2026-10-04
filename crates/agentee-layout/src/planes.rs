use crate::Model;
use agentee_core::engine::PlanesFile;
use agentee_core::footprint::PadKind;
use agentee_core::geom::{self, P};
use agentee_core::graphic::Bounds;
use agentee_core::place;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

#[derive(Clone, Debug, Default, Serialize)]
pub struct PlaneZone {
    pub net: String,
    pub layer: String,
    pub priority: i32,
    pub outline: Vec<P>,
    pub area: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PlanesPlan {
    pub zones: Vec<PlaneZone>,
}

const FREE: u16 = u16::MAX;

struct Raster {
    origin: P,
    step: f64,
    w: usize,
    h: usize,
    inside: Vec<bool>,
}

impl Raster {
    fn cell(&self, q: P) -> Option<usize> {
        let x = ((q[0] - self.origin[0]) / self.step).floor();
        let y = ((q[1] - self.origin[1]) / self.step).floor();
        (x >= 0.0 && y >= 0.0 && (x as usize) < self.w && (y as usize) < self.h)
            .then(|| y as usize * self.w + x as usize)
    }

    fn center(&self, i: usize) -> P {
        [
            self.origin[0] + ((i % self.w) as f64 + 0.5) * self.step,
            self.origin[1] + ((i / self.w) as f64 + 0.5) * self.step,
        ]
    }

    fn neighbours(&self, i: usize) -> impl Iterator<Item = (usize, f32)> + '_ {
        let (x, y) = ((i % self.w) as i64, (i / self.w) as i64);
        [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)]
            .into_iter()
            .filter_map(move |(dx, dy): (i64, i64)| {
                let (nx, ny) = (x + dx, y + dy);
                (nx >= 0 && ny >= 0 && (nx as usize) < self.w && (ny as usize) < self.h).then(
                    || {
                        let cost = if dx != 0 && dy != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
                        (ny as usize * self.w + nx as usize, cost)
                    },
                )
            })
            .filter(|&(j, _)| self.inside[j])
    }

    fn side_neighbours(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let (x, y) = ((i % self.w) as i64, (i / self.w) as i64);
        [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .filter_map(move |(dx, dy): (i64, i64)| {
                let (nx, ny) = (x + dx, y + dy);
                (nx >= 0 && ny >= 0 && (nx as usize) < self.w && (ny as usize) < self.h)
                    .then(|| ny as usize * self.w + nx as usize)
            })
            .filter(|&j| self.inside[j])
    }

    fn disc(&self, i: usize, r: f64) -> Vec<usize> {
        let k = (r / self.step).ceil() as i64;
        let (x, y) = ((i % self.w) as i64, (i / self.w) as i64);
        let mut out = Vec::new();
        for dy in -k..=k {
            for dx in -k..=k {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx as usize >= self.w || ny as usize >= self.h {
                    continue;
                }
                if ((dx * dx + dy * dy) as f64).sqrt() * self.step > r + 1e-9 {
                    continue;
                }
                let j = ny as usize * self.w + nx as usize;
                if self.inside[j] {
                    out.push(j);
                }
            }
        }
        out
    }
}

#[derive(PartialEq)]
struct Node(f32, usize);
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}

fn components(r: &Raster, owner: &[u16], rail: u16) -> Vec<Vec<usize>> {
    let mut seen = vec![false; owner.len()];
    let mut out = Vec::new();
    for s in 0..owner.len() {
        if seen[s] || owner[s] != rail || !r.inside[s] {
            continue;
        }
        let mut comp = vec![s];
        seen[s] = true;
        let mut k = 0;
        while k < comp.len() {
            let i = comp[k];
            k += 1;
            for j in r.side_neighbours(i) {
                if !seen[j] && owner[j] == rail {
                    seen[j] = true;
                    comp.push(j);
                }
            }
        }
        out.push(comp);
    }
    out
}

fn path_between(
    r: &Raster,
    from: &[usize],
    to: &HashSet<usize>,
    limit: f32,
    cost: impl Fn(usize) -> Option<f32>,
) -> Option<Vec<usize>> {
    let mut dist = vec![f32::MAX; r.inside.len()];
    let mut prev = vec![usize::MAX; r.inside.len()];
    let mut heap = BinaryHeap::new();
    for &s in from {
        dist[s] = 0.0;
        heap.push(Node(0.0, s));
    }
    while let Some(Node(d, i)) = heap.pop() {
        if d > dist[i] {
            continue;
        }
        if d > limit {
            return None;
        }
        if to.contains(&i) {
            let mut path = vec![i];
            let mut c = i;
            while prev[c] != usize::MAX {
                c = prev[c];
                path.push(c);
            }
            return Some(path);
        }
        for j in r.side_neighbours(i) {
            let Some(c) = cost(j) else { continue };
            let nd = d + c;
            if nd < dist[j] {
                dist[j] = nd;
                prev[j] = i;
                heap.push(Node(nd, j));
            }
        }
    }
    None
}

fn outer_loop(r: &Raster, cells: &[usize]) -> Vec<P> {
    let set: HashSet<usize> = cells.iter().copied().collect();
    let has = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && (x as usize) < r.w
            && (y as usize) < r.h
            && set.contains(&(y as usize * r.w + x as usize))
    };
    let mut next: HashMap<(i64, i64), Vec<(i64, i64)>> = HashMap::new();
    for &i in cells {
        let (x, y) = ((i % r.w) as i64, (i / r.w) as i64);
        if !has(x, y - 1) {
            next.entry((x, y)).or_default().push((x + 1, y));
        }
        if !has(x + 1, y) {
            next.entry((x + 1, y)).or_default().push((x + 1, y + 1));
        }
        if !has(x, y + 1) {
            next.entry((x + 1, y + 1)).or_default().push((x, y + 1));
        }
        if !has(x - 1, y) {
            next.entry((x, y + 1)).or_default().push((x, y));
        }
    }
    let mut loops: Vec<Vec<(i64, i64)>> = Vec::new();
    while let Some(&start) = next.keys().next() {
        let mut lp = vec![start];
        let mut cur = start;
        let mut dir = (0i64, 0i64);
        loop {
            let outs = next.get_mut(&cur).unwrap();
            let pick = if outs.len() > 1 {
                let left = (dir.1, -dir.0);
                outs.iter().position(|o| (o.0 - cur.0, o.1 - cur.1) == left).unwrap_or(0)
            } else {
                0
            };
            let to = outs.swap_remove(pick);
            if outs.is_empty() {
                next.remove(&cur);
            }
            dir = (to.0 - cur.0, to.1 - cur.1);
            cur = to;
            if cur == start {
                break;
            }
            lp.push(cur);
        }
        loops.push(lp);
    }
    let area = |lp: &[(i64, i64)]| {
        let mut a = 0i64;
        for k in 0..lp.len() {
            let (p, q) = (lp[k], lp[(k + 1) % lp.len()]);
            a += p.0 * q.1 - q.0 * p.1;
        }
        a.abs()
    };
    let Some(best) = loops.into_iter().max_by_key(|l| area(l)) else { return Vec::new() };
    let n = best.len();
    best.iter()
        .enumerate()
        .filter(|&(k, p)| {
            let (a, b) = (best[(k + n - 1) % n], best[(k + 1) % n]);
            (p.0 - a.0) * (b.1 - p.1) != (p.1 - a.1) * (b.0 - p.0)
        })
        .map(|(_, p)| [r.origin[0] + p.0 as f64 * r.step, r.origin[1] + p.1 as f64 * r.step])
        .collect()
}

pub fn plan(model: &Model, pc: &PlanesFile) -> (PlanesPlan, Vec<String>, Vec<String>) {
    let mut notes: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let l = &model.layout;
    let step = pc.step.map(|s| s.to_mm()).unwrap_or(0.1);
    let reach = pc.reach.map(|s| s.to_mm()).unwrap_or(1.5);
    let neck = pc.neck.map(|s| s.to_mm()).unwrap_or(0.8);
    let keep = pc.keep.map(|s| s.to_mm()).unwrap_or(0.35);
    let span = pc.span.map(|s| s.to_mm()).unwrap_or(6.0);
    let net_id = |name: &str| l.nets.iter().position(|n| n.name == name);
    let full_zone =
        |z: &agentee_core::layout::ZoneFile| z.outline.as_ref().is_none_or(|o| o.is_empty());

    let mut layers: Vec<(String, Vec<String>)> = pc.layers.clone().into_iter().collect();
    if layers.is_empty() {
        for z in model.file.zones.iter().filter(|z| full_zone(z)) {
            if geom_is_ground(&z.net) {
                continue;
            }
            for layer in &z.layers {
                if !layers.iter().any(|(ly, _)| ly == layer) {
                    layers.push((layer.clone(), vec![z.net.clone()]));
                }
            }
        }
        let full: HashSet<&str> =
            model.file.zones.iter().filter(|z| full_zone(z)).map(|z| z.net.as_str()).collect();
        for (_, rails) in layers.iter_mut() {
            for n in &l.nets {
                if full.contains(n.name.as_str())
                    || geom_is_ground(&n.name)
                    || !place::is_power_net(model.board, &n.name, &n.class)
                {
                    continue;
                }
                let carries =
                    model.board.netclasses.iter().any(|c| c.name == n.class && c.current.is_some());
                let ni = net_id(&n.name).unwrap();
                let reach = l
                    .parts
                    .iter()
                    .flat_map(|p| p.pads.iter())
                    .filter(|q| q.net == Some(ni))
                    .count();
                if carries && reach >= 2 {
                    rails.push(n.name.clone());
                }
            }
        }
    }

    let mut b = Bounds::EMPTY;
    l.outline.iter().for_each(|q| b.add(*q));
    let (w, h) = (
        ((b.max[0] - b.min[0]) / step).ceil() as usize,
        ((b.max[1] - b.min[1]) / step).ceil() as usize,
    );
    let mut raster = Raster { origin: b.min, step, w, h, inside: vec![false; w * h] };
    for i in 0..w * h {
        let c = raster.center(i);
        raster.inside[i] = geom::point_in_polygon(c, &l.outline)
            && !l.board_cutouts.iter().any(|k| geom::point_in_polygon(c, k));
    }

    let mut plan = PlanesPlan::default();
    for (layer, rails) in &layers {
        if rails.len() < 2 {
            continue;
        }
        let rail_of: HashMap<usize, u16> = rails
            .iter()
            .enumerate()
            .filter_map(|(k, n)| net_id(n).map(|ni| (ni, k as u16)))
            .collect();
        let mut terminals: Vec<(usize, u16)> = Vec::new();
        let mut foreign: HashSet<usize> = HashSet::new();
        for v in l.vias.iter().filter(|v| v.layers.contains(layer)) {
            let Some(c) = raster.cell(v.at) else { continue };
            match rail_of.get(&v.net) {
                Some(&k) => terminals.push((c, k)),
                None => foreign.extend(raster.disc(c, v.diameter / 2.0 + keep)),
            }
        }
        for p in &l.parts {
            for pad in &p.pads {
                if pad.drill.is_none() {
                    if let (Some(&k), Some(c)) =
                        (pad.net.and_then(|ni| rail_of.get(&ni)), raster.cell(pad_centre(pad)))
                    {
                        terminals.push((c, k));
                    }
                    continue;
                }
                if !pad.copper.contains(layer) {
                    continue;
                }
                let Some(ni) = pad.net else { continue };
                let Some(at) = pad.drill.map(|d| d.0) else { continue };
                let Some(c) = raster.cell(at) else { continue };
                match rail_of.get(&ni) {
                    Some(&k) => terminals.push((c, k)),
                    None if !matches!(pad.kind, PadKind::Npth) => {
                        foreign.extend(raster.disc(c, 0.5 + keep))
                    }
                    None => {}
                }
            }
        }

        let n = w * h;
        let mut owner = vec![0u16; n];
        let mut dist = vec![f32::MAX; n];
        let mut heap = BinaryHeap::new();
        for &(c, k) in &terminals {
            if k == 0 {
                continue;
            }
            owner[c] = k;
            dist[c] = 0.0;
            heap.push(Node(0.0, c));
        }
        let limit = (reach / step) as f32;
        while let Some(Node(d, i)) = heap.pop() {
            if d > dist[i] {
                continue;
            }
            for (j, s) in raster.neighbours(i) {
                let nd = d + s;
                if nd <= limit && nd < dist[j] && !foreign.contains(&j) {
                    dist[j] = nd;
                    owner[j] = owner[i];
                    heap.push(Node(nd, j));
                }
            }
        }
        let base_terms: HashSet<usize> =
            terminals.iter().filter(|t| t.1 == 0).map(|t| t.0).collect();
        let mut protected: HashSet<usize> = HashSet::new();
        for &c in &base_terms {
            for j in raster.disc(c, 0.3 + keep) {
                owner[j] = 0;
                protected.insert(j);
            }
        }
        for i in 0..n {
            if !raster.inside[i] {
                owner[i] = FREE;
            }
        }

        let term_of =
            |k: u16| -> Vec<usize> { terminals.iter().filter(|t| t.1 == k).map(|t| t.0).collect() };
        let limit = (span / step) as f32 + 60.0;
        let join_rail = |owner: &mut Vec<u16>, protected: &HashSet<usize>, k: u16| -> usize {
            let terms: HashSet<usize> = term_of(k).into_iter().collect();
            loop {
                let mut comps: Vec<Vec<usize>> = components(&raster, owner, k)
                    .into_iter()
                    .filter(|c| c.iter().any(|i| terms.contains(i)))
                    .collect();
                if comps.len() < 2 {
                    return 0;
                }
                comps.sort_by_key(|c| c.len());
                let mut joined = false;
                for ci in 0..comps.len() {
                    let target: HashSet<usize> = comps
                        .iter()
                        .enumerate()
                        .filter(|&(cj, _)| cj != ci)
                        .flat_map(|(_, c)| c.iter().copied())
                        .collect();
                    let path =
                        path_between(&raster, &comps[ci], &target, limit, |j| match owner[j] {
                            o if o == k => Some(0.05),
                            0 if !protected.contains(&j) && !foreign.contains(&j) => Some(1.0),
                            _ => None,
                        });
                    let Some(path) = path else { continue };
                    if path.iter().filter(|&&j| owner[j] == 0).count() as f64 * step > span {
                        continue;
                    }
                    for c in path {
                        for j in raster.disc(c, neck / 2.0) {
                            if owner[j] == 0 && !protected.contains(&j) && !foreign.contains(&j) {
                                owner[j] = k;
                            }
                        }
                    }
                    joined = true;
                    break;
                }
                if !joined {
                    return comps.len() - 1;
                }
            }
        };
        for k in 1..rails.len() as u16 {
            let left = join_rail(&mut owner, &protected, k);
            if left > 0 {
                failed.push(format!(
                    "{} on {layer}: {left} pieces could not be joined, left to routing",
                    rails[k as usize]
                ));
            }
        }
        let rail_terms: HashSet<usize> =
            terminals.iter().filter(|t| t.1 != 0).map(|t| t.0).collect();
        let rail_rings: HashSet<usize> =
            rail_terms.iter().flat_map(|&c| raster.disc(c, 0.3 + keep)).collect();
        let mut touched: HashSet<u16> = HashSet::new();
        for _ in 0..64 {
            let comps: Vec<Vec<usize>> = components(&raster, &owner, 0)
                .into_iter()
                .filter(|c| c.iter().any(|i| base_terms.contains(i)))
                .collect();
            if comps.len() < 2 {
                break;
            }
            let main = comps.iter().max_by_key(|c| c.len()).unwrap();
            let Some(lost) =
                comps.iter().filter(|c| !std::ptr::eq(*c, main)).min_by_key(|c| c.len())
            else {
                break;
            };
            let target: HashSet<usize> = main.iter().copied().collect();
            let path = path_between(&raster, lost, &target, limit, |j| match owner[j] {
                0 => Some(0.05),
                FREE => None,
                _ if rail_rings.contains(&j) || foreign.contains(&j) => None,
                _ => Some(1.0),
            });
            let Some(path) = path else { break };
            for c in path {
                for j in raster.disc(c, neck / 2.0) {
                    if owner[j] != FREE && owner[j] != 0 && !rail_rings.contains(&j) {
                        touched.insert(owner[j]);
                        owner[j] = 0;
                    }
                    if owner[j] == 0 {
                        protected.insert(j);
                    }
                }
            }
        }
        for k in touched {
            let left = join_rail(&mut owner, &protected, k);
            if left > 0 {
                failed.push(format!(
                    "{} on {layer}: {left} pieces left after opening the base, left to routing",
                    rails[k as usize]
                ));
            }
        }
        let base_comps: Vec<Vec<usize>> = components(&raster, &owner, 0)
            .into_iter()
            .filter(|c| c.iter().any(|i| base_terms.contains(i)))
            .collect();
        if base_comps.len() > 1 {
            let stranded: usize = base_comps
                .iter()
                .map(|c| c.iter().filter(|i| base_terms.contains(i)).count())
                .sum::<usize>()
                - base_comps
                    .iter()
                    .map(|c| c.iter().filter(|i| base_terms.contains(i)).count())
                    .max()
                    .unwrap_or(0);
            failed.push(format!(
                "{} on {layer}: {stranded} vias cut off from the plane by other rails",
                rails[0]
            ));
        }

        let mut zones: Vec<PlaneZone> = Vec::new();
        for k in 1..rails.len() as u16 {
            let terms: HashSet<usize> = term_of(k).into_iter().collect();
            for comp in components(&raster, &owner, k) {
                if !comp.iter().any(|i| terms.contains(i)) {
                    continue;
                }
                let outline = outer_loop(&raster, &comp);
                if outline.len() < 3 {
                    continue;
                }
                zones.push(PlaneZone {
                    net: rails[k as usize].clone(),
                    layer: layer.clone(),
                    priority: 0,
                    area: geom::signed_area(&outline).abs(),
                    outline,
                });
            }
        }
        zones.sort_by(|a, b| a.area.total_cmp(&b.area));
        let top = zones.len() as i32;
        for (i, z) in zones.iter_mut().enumerate() {
            z.priority = 10 + top - i as i32;
        }
        notes.push(format!(
            "{layer}: base {}, {} rail regions for {}",
            rails[0],
            zones.len(),
            rails[1..].join(", ")
        ));
        plan.zones.extend(zones);
    }
    (plan, notes, failed)
}

fn geom_is_ground(name: &str) -> bool {
    place::is_ground(name)
}

fn pad_centre(pad: &agentee_core::layout::PlacedPad) -> P {
    let mut b = Bounds::EMPTY;
    pad.outlines.iter().flatten().for_each(|q| b.add(*q));
    b.center()
}
