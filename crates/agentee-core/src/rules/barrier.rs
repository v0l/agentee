use super::{Context, Rule, Violation};
use crate::board::Barrier;
use crate::drc::{CuShape, FillIndex, Owner};
use crate::geom::{self, P};
use crate::graphic::Bounds;
use crate::units::Length;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

fn planned<C: Context>(cx: &C, c: &Conductor) -> bool {
    match c.of {
        Of::Item(i) => cx.planned_item(i),
        Of::Zone(_) => false,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Of {
    Item(usize),
    Zone(usize),
}

struct Conductor {
    of: Of,
    net: usize,
    domain: usize,
    layer: String,
    bounds: Bounds,
    edges: Vec<(P, P)>,
}

pub fn ring_edges(ring: &[P], out: &mut Vec<(P, P)>) {
    for i in 0..ring.len() {
        out.push((ring[i], ring[(i + 1) % ring.len()]));
    }
}

fn shape_edges(s: &CuShape) -> Vec<(P, P)> {
    let mut out = Vec::new();
    match s {
        CuShape::Poly(rings) => rings.iter().for_each(|r| ring_edges(r, &mut out)),
        CuShape::Circle(c, r) => ring_edges(&geom::circle(*c, *r, 24), &mut out),
        CuShape::Seg(a, b, hw) => ring_edges(&capsule(*a, *b, *hw), &mut out),
    }
    out
}

fn capsule(a: P, b: P, hw: f64) -> Vec<P> {
    let l = geom::dist(a, b);
    let u = if l > 1e-9 { [(b[0] - a[0]) / l, (b[1] - a[1]) / l] } else { [1.0, 0.0] };
    let base = u[1].atan2(u[0]);
    let mut out = Vec::new();
    for (c, from) in
        [(b, base - std::f64::consts::FRAC_PI_2), (a, base + std::f64::consts::FRAC_PI_2)]
    {
        for k in 0..=8 {
            let t = from + std::f64::consts::PI * k as f64 / 8.0;
            out.push([c[0] + hw * t.cos(), c[1] + hw * t.sin()]);
        }
    }
    out
}

fn conductors<C: Context>(
    cx: &C,
    domain: &[Option<usize>],
    layers: &[String],
    reach: f64,
) -> Vec<Conductor> {
    let mut out = Vec::new();
    for i in cx.item_subjects(reach) {
        let c = cx.item(i);
        let Some(net) = c.net else { continue };
        let Some(d) = domain[net] else { continue };
        for l in c.layers.iter().filter(|l| layers.contains(l)) {
            out.push(Conductor {
                of: Of::Item(i),
                net,
                domain: d,
                layer: l.clone(),
                bounds: c.bounds,
                edges: shape_edges(&c.shape),
            });
        }
    }
    for (zi, z) in cx.zones().iter().enumerate() {
        let Some(d) = domain[z.net] else { continue };
        if !layers.contains(&z.layer) || z.rings.is_empty() {
            continue;
        }
        out.push(Conductor {
            of: Of::Zone(zi),
            net: z.net,
            domain: d,
            layer: z.layer.clone(),
            bounds: crate::drc::rings_bounds(&z.rings),
            edges: Vec::new(),
        });
    }
    out
}

fn grown(b: &Bounds, r: f64) -> Bounds {
    let mut g = *b;
    g.add([b.min[0] - r, b.min[1] - r]);
    g.add([b.max[0] + r, b.max[1] + r]);
    g
}

fn overlaps(a: &Bounds, b: &Bounds) -> bool {
    a.min[0] <= b.max[0] && b.min[0] <= a.max[0] && a.min[1] <= b.max[1] && b.min[1] <= a.max[1]
}

fn edges_in<'a, C: Context>(
    cx: &C,
    c: &'a Conductor,
    region: &Bounds,
) -> std::borrow::Cow<'a, [(P, P)]> {
    match c.of {
        Of::Item(_) => std::borrow::Cow::Borrowed(&c.edges),
        Of::Zone(zi) => {
            let f: &FillIndex = &cx.fills()[zi];
            let mut ks: Vec<usize> = Vec::new();
            for x in
                (region.min[0] / f.cell).floor() as i64..=(region.max[0] / f.cell).floor() as i64
            {
                for y in (region.min[1] / f.cell).floor() as i64
                    ..=(region.max[1] / f.cell).floor() as i64
                {
                    ks.extend(f.bins.get(&(x, y)).into_iter().flatten().copied());
                }
            }
            ks.sort_unstable();
            ks.dedup();
            std::borrow::Cow::Owned(ks.into_iter().map(|k| f.edges[k]).collect())
        }
    }
}

fn closest_on(p: P, a: P, b: P) -> P {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = d[0] * d[0] + d[1] * d[1];
    if l <= 1e-18 {
        return a;
    }
    let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l).clamp(0.0, 1.0);
    [a[0] + t * d[0], a[1] + t * d[1]]
}

pub fn closest(ea: &[(P, P)], eb: &[(P, P)]) -> Option<(f64, P, P)> {
    let mut best: Option<(f64, P, P)> = None;
    let mut take = |p: P, q: P| {
        let d = geom::dist(p, q);
        if best.is_none_or(|b| d < b.0) {
            best = Some((d, p, q));
        }
    };
    for &(a, b) in ea {
        for &(c, d) in eb {
            if geom::segments_intersect(a, b, c, d) {
                take(a, a);
                continue;
            }
            take(a, closest_on(a, c, d));
            take(b, closest_on(b, c, d));
            take(closest_on(c, a, b), c);
            take(closest_on(d, a, b), d);
        }
    }
    best
}

fn exempt<C: Context>(cx: &C, a: &Conductor, b: &Conductor) -> bool {
    let (Of::Item(i), Of::Item(j)) = (a.of, b.of) else { return false };
    let (Owner::Pad(p, k), Owner::Pad(q, m)) = (cx.item(i).owner, cx.item(j).owner) else {
        return false;
    };
    p == q && {
        let part = &cx.parts()[p];
        part.footprint.spark_gap(&part.pads[k].number, &part.pads[m].number).is_some()
    }
}

fn label<C: Context>(cx: &C, c: &Conductor) -> String {
    match c.of {
        Of::Item(i) => cx.describe(i),
        Of::Zone(_) => format!("the {} pour", cx.nets()[c.net].name),
    }
}

fn pairs<C: Context>(
    cx: &C,
    layers: &[String],
    across: bool,
    need: impl Fn(&Barrier) -> Option<f64>,
    mut each: impl FnMut(&Conductor, &Conductor, f64),
) {
    let iso = &cx.spacing().isolation;
    let reach: f64 = cx.board().barriers.iter().filter_map(&need).fold(0.0, f64::max);
    if reach <= 0.0 {
        return;
    }
    let all = conductors(cx, &iso.domain, layers, reach);
    let cell = reach.max(1.0);
    let mut grid: HashMap<(String, i64, i64), Vec<usize>> = HashMap::new();
    for (k, c) in all.iter().enumerate() {
        for x in (c.bounds.min[0] / cell).floor() as i64..=(c.bounds.max[0] / cell).floor() as i64 {
            for y in
                (c.bounds.min[1] / cell).floor() as i64..=(c.bounds.max[1] / cell).floor() as i64
            {
                grid.entry((c.layer.clone(), x, y)).or_default().push(k);
            }
        }
    }
    let (top, bottom) = (cx.copper().first(), cx.copper().last());
    for (i, a) in all.iter().enumerate() {
        let key = match (across, top, bottom) {
            (false, ..) => &a.layer,
            (true, Some(t), Some(b)) if &a.layer == t && t != b => b,
            _ => continue,
        };
        let g = grown(&a.bounds, reach);
        let mut near: Vec<usize> = Vec::new();
        for x in (g.min[0] / cell).floor() as i64..=(g.max[0] / cell).floor() as i64 {
            for y in (g.min[1] / cell).floor() as i64..=(g.max[1] / cell).floor() as i64 {
                near.extend(grid.get(&(key.clone(), x, y)).into_iter().flatten().copied());
            }
        }
        near.sort_unstable();
        near.dedup();
        for j in near.into_iter().filter(|&j| across || j > i) {
            let b = &all[j];
            if !cx.counts(planned(cx, a), planned(cx, b)) {
                continue;
            }
            let Some(c) = iso.barrier(cx.board(), a.net, b.net).and_then(&need) else {
                continue;
            };
            if !overlaps(&grown(&a.bounds, c), &b.bounds)
                || exempt(cx, a, b)
                || (across && (both_sides(cx, a) || both_sides(cx, b)))
            {
                continue;
            }
            each(a, b, c);
        }
    }
}

fn both_sides<C: Context>(cx: &C, c: &Conductor) -> bool {
    let Of::Item(i) = c.of else { return false };
    let layers = &cx.item(i).layers;
    [cx.copper().first(), cx.copper().last()].into_iter().flatten().all(|l| layers.contains(l))
}

fn region(a: &Conductor, b: &Conductor, c: f64) -> Bounds {
    let (ga, gb) = (grown(&a.bounds, c), grown(&b.bounds, c));
    Bounds {
        min: [ga.min[0].max(gb.min[0]), ga.min[1].max(gb.min[1])],
        max: [ga.max[0].min(gb.max[0]), ga.max[1].min(gb.max[1])],
    }
}

fn barrier_name<C: Context>(cx: &C, a: &Conductor, b: &Conductor) -> String {
    let between = cx
        .spacing()
        .isolation
        .barrier(cx.board(), a.net, b.net)
        .map(|x| x.between)
        .unwrap_or([a.domain, b.domain]);
    let domains = &cx.board().domains;
    format!("{}-{}", domains[between[0]].name, domains[between[1]].name)
}

#[derive(Clone, Copy, PartialEq)]
pub enum Rim {
    Edge,
    Cutout,
    Hole,
}

pub struct Surface {
    outline: Vec<P>,
    obstacles: Vec<Vec<P>>,
    rims: Vec<(Rim, Vec<P>)>,
}

const RIM_STEP: f64 = 0.1;

fn nearest(p: P, e: &[(P, P)]) -> P {
    e.iter()
        .map(|&(a, b)| closest_on(p, a, b))
        .min_by(|x, y| geom::dist(p, *x).total_cmp(&geom::dist(p, *y)))
        .unwrap_or(p)
}

fn gap(p: P, e: &[(P, P)]) -> f64 {
    geom::dist(p, nearest(p, e))
}

fn samples(e: &[(P, P)], keep: impl Fn(P) -> bool) -> Vec<P> {
    let mut out = Vec::new();
    for &(a, b) in e {
        let n = (geom::dist(a, b) / RIM_STEP).ceil().max(1.0) as usize;
        for k in 0..n {
            let t = k as f64 / n as f64;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if keep(p) {
                out.push(p);
            }
        }
    }
    out
}

fn ring_bounds(ring: &[P]) -> Bounds {
    let mut b = Bounds::EMPTY;
    ring.iter().for_each(|p| b.add(*p));
    b
}

impl Surface {
    pub fn new<C: Context>(cx: &C, groove: f64) -> Surface {
        let mut rims: Vec<(Rim, Vec<P>)> = Vec::new();
        let edge = cx.edge();
        if edge.outline.len() >= 3 {
            rims.push((Rim::Edge, edge.outline.to_vec()));
        }
        let mut obstacles = Vec::new();
        for c in edge.cutouts.iter().filter(|c| c.len() >= 3) {
            if geom::min_extent(c) + 1e-9 >= groove {
                obstacles.push(c.clone());
            }
            rims.push((Rim::Cutout, c.clone()));
        }
        for h in (0..cx.hole_count()).map(|i| cx.hole(i)).filter(|h| !h.plated) {
            let ring = capsule(h.a, h.b, h.r / (std::f64::consts::PI / 16.0).cos());
            if 2.0 * h.r + 1e-9 >= groove {
                obstacles.push(ring.clone());
            }
            rims.push((Rim::Hole, ring));
        }
        Surface { outline: edge.outline.to_vec(), obstacles, rims }
    }

    pub fn local(&self, reg: &Bounds) -> Surface {
        let keep = |r: &Vec<P>| overlaps(&ring_bounds(r), reg);
        Surface {
            outline: self.outline.clone(),
            obstacles: self.obstacles.iter().filter(|o| keep(o)).cloned().collect(),
            rims: self.rims.iter().filter(|(_, r)| keep(r)).cloned().collect(),
        }
    }

    pub fn across(
        &self,
        ea: &[(P, P)],
        eb: &[(P, P)],
        limit: f64,
        reg: &Bounds,
        thickness: f64,
    ) -> Option<(f64, Rim, P)> {
        let budget = limit - thickness;
        if budget <= 0.0 {
            return None;
        }
        let starts = samples(ea, |p| gap(p, eb) < budget);
        let ends = samples(eb, |p| gap(p, ea) < budget);
        let mut rim: Vec<(usize, P)> = Vec::new();
        for (ri, (_, ring)) in self.rims.iter().enumerate() {
            let mut e = Vec::new();
            ring_edges(ring, &mut e);
            let keep = |p: P| crate::drc::near(reg, p, 0.0) && gap(p, ea) + gap(p, eb) < budget;
            rim.extend(samples(&e, keep).into_iter().map(|p| (ri, p)));
            for &(a, b) in &e {
                for &s in starts.iter().chain(&ends) {
                    let foot = closest_on(s, a, b);
                    if keep(foot) {
                        rim.push((ri, foot));
                    }
                }
            }
        }
        if rim.is_empty() {
            return None;
        }
        let corners: Vec<P> =
            self.corners(*reg).into_iter().filter(|v| gap(*v, ea) + gap(*v, eb) < budget).collect();
        let (ns, nc, nr) = (starts.len(), corners.len(), rim.len());
        let low = ns + nc + nr;
        let at = |k: usize| -> P {
            match k {
                k if k < ns => starts[k],
                k if k < ns + nc => corners[k - ns],
                k if k < low => rim[k - ns - nc].1,
                k if k < low + nc => corners[k - low],
                k if k < low + nc + nr => rim[k - low - nc].1,
                k => ends[k - low - nc - nr],
            }
        };
        let total = low + nc + nr + ends.len();
        let mut best = vec![f64::MAX; total];
        let mut prev = vec![usize::MAX; total];
        let mut heap = BinaryHeap::new();
        for (k, b) in best.iter_mut().enumerate().take(ns) {
            *b = 0.0;
            heap.push(Node(0.0, k));
        }
        while let Some(Node(d, k)) = heap.pop() {
            if d > best[k] {
                continue;
            }
            if k >= low + nc + nr {
                let mut j = k;
                while prev[j] != usize::MAX && prev[j] >= low {
                    j = prev[j];
                }
                let (ri, q) = rim[j - low - nc];
                return Some((d, self.rims[ri].0, q));
            }
            let p = at(k);
            let mut steps: Vec<(usize, f64)> = Vec::new();
            let side = if k < low { ns..low } else { low..total };
            for m in side {
                let q = at(m);
                let step = geom::dist(p, q);
                if d + step < best[m].min(limit) && self.visible(p, q) {
                    steps.push((m, step));
                }
            }
            if (ns + nc..low).contains(&k) {
                let ri = rim[k - ns - nc].0;
                let ring = &self.rims[ri].1;
                for (j, &(rj, q)) in rim.iter().enumerate() {
                    let mid = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
                    if rj == ri && on_edge(mid, ring) {
                        steps.push((
                            low + nc + j,
                            (thickness.powi(2) + geom::dist(p, q).powi(2)).sqrt(),
                        ));
                    }
                }
            }
            for (m, step) in steps {
                let nd = d + step;
                if nd < best[m] && nd < limit {
                    best[m] = nd;
                    prev[m] = k;
                    heap.push(Node(nd, m));
                }
            }
        }
        None
    }

    fn crosses(p: P, q: P, ring: &[P]) -> bool {
        let n = ring.len();
        (0..n).any(|i| proper(p, q, ring[i], ring[(i + 1) % n]))
    }

    pub fn visible(&self, p: P, q: P) -> bool {
        if self.obstacles.iter().any(|o| Self::crosses(p, q, o)) {
            return false;
        }
        if self.outline.len() >= 3 && Self::crosses(p, q, &self.outline) {
            return false;
        }
        [0.25, 0.5, 0.75].iter().all(|t| {
            let m = [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
            !self.obstacles.iter().any(|o| strictly_inside(m, o))
                && (self.outline.len() < 3 || !strictly_outside(m, &self.outline))
        })
    }

    fn corners(&self, lo: Bounds) -> Vec<P> {
        self.obstacles
            .iter()
            .chain(std::iter::once(&self.outline))
            .flatten()
            .copied()
            .filter(|p| crate::drc::near(&lo, *p, 0.0))
            .collect()
    }

    pub fn path(&self, ea: &[(P, P)], eb: &[(P, P)], limit: f64, reg: &Bounds) -> f64 {
        let to = |p: P, e: &[(P, P)]| {
            e.iter()
                .map(|&(a, b)| closest_on(p, a, b))
                .min_by(|x, y| geom::dist(p, *x).total_cmp(&geom::dist(p, *y)))
                .unwrap_or(p)
        };
        let gap = |p: P, e: &[(P, P)]| geom::dist(p, to(p, e));
        let corners: Vec<P> =
            self.corners(*reg).into_iter().filter(|v| gap(*v, ea) + gap(*v, eb) < limit).collect();
        let sample = |e: &[(P, P)], other: &[(P, P)]| -> Vec<P> {
            let mut out = Vec::new();
            for &(a, b) in e {
                let n = (geom::dist(a, b) / 0.1).ceil().max(1.0) as usize;
                for k in 0..n {
                    let t = k as f64 / n as f64;
                    let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                    if gap(p, other) < limit {
                        out.push(p);
                    }
                }
            }
            out.extend(corners.iter().map(|v| to(*v, e)));
            out
        };
        let starts = sample(ea, eb);
        let ends = sample(eb, ea);
        let nodes: Vec<P> = starts.iter().chain(&corners).chain(&ends).copied().collect();
        let (ns, nc) = (starts.len(), corners.len());
        let is_end = |k: usize| k >= ns + nc;
        let mut best = vec![f64::MAX; nodes.len()];
        let mut heap = BinaryHeap::new();
        for (k, b) in best.iter_mut().enumerate().take(ns) {
            *b = 0.0;
            heap.push(Node(0.0, k));
        }
        while let Some(Node(d, k)) = heap.pop() {
            if d > best[k] {
                continue;
            }
            if is_end(k) {
                return d;
            }
            if d >= limit {
                break;
            }
            for m in ns..nodes.len() {
                let nd = d + geom::dist(nodes[k], nodes[m]);
                if nd < best[m] && nd < limit && self.visible(nodes[k], nodes[m]) {
                    best[m] = nd;
                    heap.push(Node(nd, m));
                }
            }
        }
        f64::MAX
    }
}

struct Node(f64, usize);

impl PartialEq for Node {
    fn eq(&self, o: &Self) -> bool {
        self.0 == o.0
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0)
    }
}

fn orient(a: P, b: P, c: P) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn proper(p: P, q: P, a: P, b: P) -> bool {
    let eps = 1e-9;
    let (d1, d2) = (orient(a, b, p), orient(a, b, q));
    let (d3, d4) = (orient(p, q, a), orient(p, q, b));
    ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps))
        && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
}

fn on_edge(p: P, ring: &[P]) -> bool {
    crate::drc::edge_distance(ring, p) < 1e-6
}

fn strictly_inside(p: P, ring: &[P]) -> bool {
    !on_edge(p, ring) && geom::point_in_polygon(p, ring)
}

fn strictly_outside(p: P, ring: &[P]) -> bool {
    !on_edge(p, ring) && !geom::point_in_polygon(p, ring)
}

pub struct IsolationClearance;

impl Rule for IsolationClearance {
    fn id(&self) -> &'static str {
        "isolation-clearance"
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        let copper = cx.copper().to_vec();
        pairs(
            cx,
            &copper,
            false,
            |b| b.clearance.map(Length::to_mm),
            |a, b, c| {
                let reg = region(a, b, c);
                let (ea, eb) = (edges_in(cx, a, &reg), edges_in(cx, b, &reg));
                let Some((d, p, q)) = closest(&ea, &eb) else { return };
                if d + crate::layout::DRC_EPSILON >= c || d <= 1e-6 {
                    return;
                }
                let detail = format!(
                    "{} is {} from {}, the {} barrier needs {}",
                    label(cx, a),
                    Length::mm(d),
                    label(cx, b),
                    barrier_name(cx, a, b),
                    Length::mm(c)
                );
                out.push(Violation {
                    rule: self.id(),
                    group: "isolation clearance".into(),
                    subject: label(cx, a),
                    other: label(cx, b),
                    gap: d,
                    need: c,
                    at: [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0],
                    detail,
                    layers: vec![a.layer.clone()],
                    nets: Some((a.net.min(b.net), a.net.max(b.net))),
                });
            },
        );
    }
}

pub struct Creepage;

impl Rule for Creepage {
    fn id(&self) -> &'static str {
        "creepage"
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        let outer: Vec<String> =
            [cx.copper().first(), cx.copper().last()].into_iter().flatten().cloned().collect();
        let surfaces: std::cell::RefCell<HashMap<u8, Surface>> = Default::default();
        let surface = |barrier: &Barrier, reg: &Bounds| {
            let groove = barrier.groove().to_mm();
            surfaces
                .borrow_mut()
                .entry(barrier.pollution_degree)
                .or_insert_with(|| Surface::new(cx, groove))
                .local(reg)
        };
        let iso = &cx.spacing().isolation;
        let mut found: Vec<(f64, f64, P, String, Vec<String>, usize, usize, String, String)> =
            Vec::new();
        pairs(
            cx,
            &outer,
            false,
            |b| b.creepage.map(Length::to_mm),
            |a, b, c| {
                let Some(barrier) = iso.barrier(cx.board(), a.net, b.net) else { return };
                let reg = region(a, b, c);
                let (ea, eb) = (edges_in(cx, a, &reg), edges_in(cx, b, &reg));
                let Some((d, p, q)) = closest(&ea, &eb) else { return };
                if d >= c || d <= 1e-6 {
                    return;
                }
                let s = surface(barrier, &reg);
                let length = if s.visible(p, q) { d } else { s.path(&ea, &eb, c, &reg) };
                if length + crate::layout::DRC_EPSILON >= c {
                    return;
                }
                let around = if length > d + 1e-3 {
                    format!(" around a slot ({} straight)", Length::mm(d))
                } else {
                    String::new()
                };
                let detail = format!(
                    "{} is {} from {} along the surface{around}, the {} barrier needs {} creepage",
                    label(cx, a),
                    Length::mm(length),
                    label(cx, b),
                    barrier_name(cx, a, b),
                    Length::mm(c)
                );
                found.push((
                    length,
                    c,
                    [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0],
                    detail,
                    vec![a.layer.clone()],
                    a.net,
                    b.net,
                    label(cx, a),
                    label(cx, b),
                ));
            },
        );
        let thickness = cx.board().stackup.thickness().to_mm();
        pairs(
            cx,
            &outer,
            true,
            |b| b.creepage.map(Length::to_mm),
            |a, b, c| {
                let Some(barrier) = iso.barrier(cx.board(), a.net, b.net) else { return };
                let reg = region(a, b, c);
                let (ea, eb) = (edges_in(cx, a, &reg), edges_in(cx, b, &reg));
                let Some((d, ..)) = closest(&ea, &eb) else { return };
                if d + thickness >= c {
                    return;
                }
                let s = surface(barrier, &reg);
                let Some((length, rim, wall)) = s.across(&ea, &eb, c, &reg, thickness) else {
                    return;
                };
                if length + crate::layout::DRC_EPSILON >= c {
                    return;
                }
                let way = match rim {
                    Rim::Edge => "round the board edge",
                    Rim::Cutout => "through a board cutout",
                    Rim::Hole => "through a non-plated hole",
                };
                let detail = format!(
                    "{} on {} is {} from {} on {} {way}, the {} barrier needs {} creepage",
                    label(cx, a),
                    a.layer,
                    Length::mm(length),
                    label(cx, b),
                    b.layer,
                    barrier_name(cx, a, b),
                    Length::mm(c)
                );
                found.push((
                    length,
                    c,
                    wall,
                    detail,
                    vec![a.layer.clone(), b.layer.clone()],
                    a.net,
                    b.net,
                    label(cx, a),
                    label(cx, b),
                ));
            },
        );
        for (length, c, at, detail, layers, na, nb, sa, sb) in found {
            out.push(Violation {
                rule: "creepage",
                group: "creepage".into(),
                subject: sa,
                other: sb,
                gap: length,
                need: c,
                at,
                detail,
                layers,
                nets: Some((na.min(nb), na.max(nb))),
            });
        }
    }
}
