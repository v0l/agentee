use super::{Category, Ctx, CuShape, FillIndex, Owner, Report, Rule, Setup};
use crate::board::Barrier;
use crate::diag::Severity;
use crate::geom::{self, P};
use crate::graphic::{Bounds, Fill};
use crate::units::Length;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, HashMap};

pub static RULES: &[Rule] = &[
    Rule {
        id: "isolation-domain",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "a net whose class or name puts it in two isolation domains",
        when: "boards with [[domains]]",
        applies: has_domains,
        check: domain_overlap,
    },
    Rule {
        id: "isolation-unassigned",
        category: Category::Copper,
        severity: Severity::Warning,
        summary: "nets in no isolation domain, which no barrier covers",
        when: "boards with [[domains]]",
        applies: has_domains,
        check: unassigned,
    },
    Rule {
        id: "isolation-clearance",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "copper of two domains on one layer closer than the clearance of the barrier between them, pads of one footprint and pours included",
        when: "boards with [[barriers]] that set `clearance`, or netclass voltages that need one",
        applies: has_barriers,
        check: barrier_clearance,
    },
    Rule {
        id: "creepage",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "copper of two domains closer along an outer surface than the barrier creepage; board cutouts and non-plated holes at least the pollution degree's groove width lengthen the path, narrower ones are bridged",
        when: "boards with [[barriers]] that set `creepage`, or netclass voltages that need one",
        applies: has_creepage,
        check: creepage,
    },
    Rule {
        id: "spark-gap",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "a footprint spark gap whose electrodes are not the declared gap apart, sit under the fab clearance minimum, or have solder mask across the gap",
        when: "footprints with [[spark_gaps]]",
        applies: has_spark_gaps,
        check: spark_gaps,
    },
];

fn has_domains(s: &Setup) -> bool {
    s.domains
}

fn has_barriers(s: &Setup) -> bool {
    s.barrier_clearance
}

fn has_creepage(s: &Setup) -> bool {
    s.creepage
}

fn has_spark_gaps(s: &Setup) -> bool {
    s.spark_gaps
}

fn domains_of(cx: &Ctx) -> Vec<Vec<usize>> {
    cx.nets.iter().map(|n| cx.board.domain_of(&n.name, &n.class)).collect()
}

fn domain_overlap(cx: &Ctx, r: &mut Report) {
    for (n, ds) in domains_of(cx).iter().enumerate() {
        if ds.len() > 1 {
            let names: Vec<&str> = ds.iter().map(|&d| cx.board.domains[d].name.as_str()).collect();
            r.emit(
                format!("net {}", cx.nets[n].name),
                format!(
                    "{} (class {}) is in domains {}: a net sits in one domain, narrow the `classes` or `nets` of the others",
                    cx.nets[n].name,
                    cx.nets[n].class,
                    names.join(" and ")
                ),
            );
        }
    }
}

fn unassigned(cx: &Ctx, r: &mut Report) {
    let free: Vec<String> = domains_of(cx)
        .iter()
        .enumerate()
        .filter(|(_, ds)| ds.is_empty())
        .map(|(n, _)| cx.nets[n].name.clone())
        .collect();
    if !free.is_empty() {
        r.emit(
            "domains",
            format!(
                "{} nets are in no isolation domain, so no barrier is checked for them: {}",
                free.len(),
                super::list(&free)
            ),
        );
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

fn ring_edges(ring: &[P], out: &mut Vec<(P, P)>) {
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

fn conductors(cx: &Ctx, domain: &[Option<usize>], layers: &[String]) -> Vec<Conductor> {
    let mut out = Vec::new();
    for (i, c) in cx.copper_items().iter().enumerate() {
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
    for (zi, z) in cx.zones.iter().enumerate() {
        let Some(d) = domain[z.net] else { continue };
        if !layers.contains(&z.layer) || z.rings.is_empty() {
            continue;
        }
        out.push(Conductor {
            of: Of::Zone(zi),
            net: z.net,
            domain: d,
            layer: z.layer.clone(),
            bounds: super::rings_bounds(&z.rings),
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

fn edges_in<'a>(cx: &Ctx, c: &'a Conductor, region: &Bounds) -> std::borrow::Cow<'a, [(P, P)]> {
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

fn closest(ea: &[(P, P)], eb: &[(P, P)]) -> Option<(f64, P, P)> {
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

fn exempt(cx: &Ctx, a: &Conductor, b: &Conductor) -> bool {
    let items = cx.copper_items();
    let (Of::Item(i), Of::Item(j)) = (a.of, b.of) else { return false };
    let (Owner::Pad(p, k), Owner::Pad(q, m)) = (items[i].owner, items[j].owner) else {
        return false;
    };
    p == q && {
        let part = &cx.parts[p];
        part.footprint.spark_gap(&part.pads[k].number, &part.pads[m].number).is_some()
    }
}

fn label(cx: &Ctx, c: &Conductor) -> String {
    match c.of {
        Of::Item(i) => cx.describe(&cx.copper_items()[i]),
        Of::Zone(_) => format!("the {} pour", cx.nets[c.net].name),
    }
}

struct Hit {
    length: f64,
    at: P,
    what: String,
    layers: Vec<String>,
    count: usize,
}

fn note(found: &mut BTreeMap<(usize, usize), Hit>, key: (usize, usize), hit: Hit) {
    match found.get_mut(&key) {
        Some(h) => {
            h.count += 1;
            for l in &hit.layers {
                if !h.layers.contains(l) {
                    h.layers.push(l.clone());
                }
            }
            if hit.length < h.length - 1e-9 {
                let layers = std::mem::take(&mut h.layers);
                *h = Hit { count: h.count, layers, ..hit };
            }
        }
        None => {
            found.insert(key, hit);
        }
    }
}

fn pairs(
    cx: &Ctx,
    layers: &[String],
    across: bool,
    need: impl Fn(&Barrier) -> Option<f64>,
    mut each: impl FnMut(&Conductor, &Conductor, f64),
) {
    let iso = &cx.spacing().isolation;
    let all = conductors(cx, &iso.domain, layers);
    let reach: f64 = cx.board.barriers.iter().filter_map(&need).fold(0.0, f64::max);
    if reach <= 0.0 {
        return;
    }
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
    let (top, bottom) = (cx.copper.first(), cx.copper.last());
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
            let Some(c) = iso.barrier(cx.board, a.net, b.net).and_then(&need) else {
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

fn both_sides(cx: &Ctx, c: &Conductor) -> bool {
    let Of::Item(i) = c.of else { return false };
    let layers = &cx.copper_items()[i].layers;
    [cx.copper.first(), cx.copper.last()].into_iter().flatten().all(|l| layers.contains(l))
}

fn region(a: &Conductor, b: &Conductor, c: f64) -> Bounds {
    let (ga, gb) = (grown(&a.bounds, c), grown(&b.bounds, c));
    Bounds {
        min: [ga.min[0].max(gb.min[0]), ga.min[1].max(gb.min[1])],
        max: [ga.max[0].min(gb.max[0]), ga.max[1].min(gb.max[1])],
    }
}

fn barrier_name(cx: &Ctx, a: &Conductor, b: &Conductor) -> String {
    let between = cx
        .spacing()
        .isolation
        .barrier(cx.board, a.net, b.net)
        .map(|x| x.between)
        .unwrap_or([a.domain, b.domain]);
    format!("{}-{}", cx.board.domains[between[0]].name, cx.board.domains[between[1]].name)
}

fn barrier_clearance(cx: &Ctx, r: &mut Report) {
    let mut found = BTreeMap::new();
    pairs(
        cx,
        cx.copper,
        false,
        |b| b.clearance.map(Length::to_mm),
        |a, b, c| {
            let reg = region(a, b, c);
            let (ea, eb) = (edges_in(cx, a, &reg), edges_in(cx, b, &reg));
            let Some((d, p, q)) = closest(&ea, &eb) else { return };
            if d + crate::layout::DRC_EPSILON >= c || d <= 1e-6 {
                return;
            }
            let what = format!(
                "{} is {} from {}, the {} barrier needs {}",
                label(cx, a),
                Length::mm(d),
                label(cx, b),
                barrier_name(cx, a, b),
                Length::mm(c)
            );
            let at = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
            note(
                &mut found,
                (a.net.min(b.net), a.net.max(b.net)),
                Hit { length: d, at, what, layers: vec![a.layer.clone()], count: 1 },
            );
        },
    );
    emit(cx, r, found, "isolation clearance");
}

fn emit(cx: &Ctx, r: &mut Report, found: BTreeMap<(usize, usize), Hit>, what: &str) {
    for ((a, b), h) in found {
        let more = if h.count > 1 { format!(", {} places in all", h.count) } else { String::new() };
        r.emit(
            format!("{what} {} {}", cx.nets[a].name, cx.nets[b].name),
            format!(
                "{} on {} at [{:.3}, {:.3}]{more}",
                h.what,
                h.layers.join(", "),
                h.at[0],
                h.at[1]
            ),
        );
    }
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
    pub fn new(cx: &Ctx, groove: f64) -> Surface {
        let mut rims: Vec<(Rim, Vec<P>)> = Vec::new();
        if cx.outline.len() >= 3 {
            rims.push((Rim::Edge, cx.outline.to_vec()));
        }
        let mut obstacles = Vec::new();
        for c in cx.board_cutouts.iter().filter(|c| c.len() >= 3) {
            if geom::min_extent(c) + 1e-9 >= groove {
                obstacles.push(c.clone());
            }
            rims.push((Rim::Cutout, c.clone()));
        }
        for h in cx.holes().iter().filter(|h| !h.plated) {
            let ring = capsule(h.a, h.b, h.r / (std::f64::consts::PI / 16.0).cos());
            if 2.0 * h.r + 1e-9 >= groove {
                obstacles.push(ring.clone());
            }
            rims.push((Rim::Hole, ring));
        }
        Surface { outline: cx.outline.to_vec(), obstacles, rims }
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
            let keep = |p: P| super::near(reg, p, 0.0) && gap(p, ea) + gap(p, eb) < budget;
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
            .filter(|p| super::near(&lo, *p, 0.0))
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
    super::edge_distance(ring, p) < 1e-6
}

fn strictly_inside(p: P, ring: &[P]) -> bool {
    !on_edge(p, ring) && geom::point_in_polygon(p, ring)
}

fn strictly_outside(p: P, ring: &[P]) -> bool {
    !on_edge(p, ring) && !geom::point_in_polygon(p, ring)
}

fn creepage(cx: &Ctx, r: &mut Report) {
    let outer: Vec<String> =
        [cx.copper.first(), cx.copper.last()].into_iter().flatten().cloned().collect();
    let mut surfaces: HashMap<u8, Surface> = HashMap::new();
    let mut found = BTreeMap::new();
    pairs(
        cx,
        &outer,
        false,
        |b| b.creepage.map(Length::to_mm),
        |a, b, c| {
            let Some(barrier) = cx.spacing().isolation.barrier(cx.board, a.net, b.net) else {
                return;
            };
            let reg = region(a, b, c);
            let (ea, eb) = (edges_in(cx, a, &reg), edges_in(cx, b, &reg));
            let Some((d, p, q)) = closest(&ea, &eb) else { return };
            if d >= c || d <= 1e-6 {
                return;
            }
            let groove = barrier.groove().to_mm();
            let surface = surfaces
                .entry(barrier.pollution_degree)
                .or_insert_with(|| Surface::new(cx, groove))
                .local(&reg);
            let length = if surface.visible(p, q) { d } else { surface.path(&ea, &eb, c, &reg) };
            if length + crate::layout::DRC_EPSILON >= c {
                return;
            }
            let around = if length > d + 1e-3 {
                format!(" around a slot ({} straight)", Length::mm(d))
            } else {
                String::new()
            };
            let what = format!(
                "{} is {} from {} along the surface{around}, the {} barrier needs {} creepage",
                label(cx, a),
                Length::mm(length),
                label(cx, b),
                barrier_name(cx, a, b),
                Length::mm(c)
            );
            let at = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
            note(
                &mut found,
                (a.net.min(b.net), a.net.max(b.net)),
                Hit { length, at, what, layers: vec![a.layer.clone()], count: 1 },
            );
        },
    );
    let thickness = cx.board.stackup.thickness().to_mm();
    pairs(
        cx,
        &outer,
        true,
        |b| b.creepage.map(Length::to_mm),
        |a, b, c| {
            let Some(barrier) = cx.spacing().isolation.barrier(cx.board, a.net, b.net) else {
                return;
            };
            let reg = region(a, b, c);
            let (ea, eb) = (edges_in(cx, a, &reg), edges_in(cx, b, &reg));
            let Some((d, ..)) = closest(&ea, &eb) else { return };
            if d + thickness >= c {
                return;
            }
            let groove = barrier.groove().to_mm();
            let surface = surfaces
                .entry(barrier.pollution_degree)
                .or_insert_with(|| Surface::new(cx, groove))
                .local(&reg);
            let Some((length, rim, wall)) = surface.across(&ea, &eb, c, &reg, thickness) else {
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
            let what = format!(
                "{} on {} is {} from {} on {} {way}, the {} barrier needs {} creepage",
                label(cx, a),
                a.layer,
                Length::mm(length),
                label(cx, b),
                b.layer,
                barrier_name(cx, a, b),
                Length::mm(c)
            );
            note(
                &mut found,
                (a.net.min(b.net), a.net.max(b.net)),
                Hit {
                    length,
                    at: wall,
                    what,
                    layers: vec![a.layer.clone(), b.layer.clone()],
                    count: 1,
                },
            );
        },
    );
    emit(cx, r, found, "creepage");
}

fn spark_gaps(cx: &Ctx, r: &mut Report) {
    let min = cx.board.rules.min_clearance.to_mm();
    for (pi, part) in cx.parts.iter().enumerate() {
        for gap in &part.footprint.spark_gaps {
            let pads = |n: &str| -> Vec<usize> {
                (0..part.pads.len()).filter(|&k| part.pads[k].number == n).collect()
            };
            let (left, right) = (pads(&gap.pads[0]), pads(&gap.pads[1]));
            let at = format!("{} spark gap {}-{}", part.reference, gap.pads[0], gap.pads[1]);
            let mut measured: Option<(f64, P, P, String)> = None;
            for &k in &left {
                for &m in &right {
                    let (a, b) = (&part.pads[k], &part.pads[m]);
                    for layer in a.copper.iter().filter(|l| b.copper.contains(l)) {
                        let mut ea = Vec::new();
                        let mut eb = Vec::new();
                        a.outlines.iter().for_each(|o| ring_edges(o, &mut ea));
                        b.outlines.iter().for_each(|o| ring_edges(o, &mut eb));
                        if let Some((d, p, q)) = closest(&ea, &eb)
                            && measured.as_ref().is_none_or(|x| d < x.0)
                        {
                            measured = Some((d, p, q, layer.clone()));
                        }
                    }
                }
            }
            let Some((d, p, q, layer)) = measured else {
                r.emit(
                    &at,
                    "the two electrodes share no copper layer, so there is no gap to fire across",
                );
                continue;
            };
            if left.iter().chain(&right).any(|&k| part.pads[k].net.is_none())
                || left.iter().any(|&k| right.iter().any(|&m| part.pads[k].net == part.pads[m].net))
            {
                r.emit(&at, "both electrodes need a net, and different nets, for the gap to protect anything");
            }
            if d + 1e-6 < min {
                r.emit(
                    &at,
                    format!(
                        "the electrodes are {} apart on {layer}, under the {} fab clearance minimum",
                        Length::mm(d),
                        Length::mm(min)
                    ),
                );
            }
            if (d - gap.gap).abs() > 0.01 {
                r.emit(
                    &at,
                    format!(
                        "the electrodes are {} apart on {layer}, the footprint declares {}",
                        Length::mm(d),
                        Length::mm(gap.gap)
                    ),
                );
            }
            let mask = if layer == cx.copper.first().map(String::as_str).unwrap_or("F.Cu") {
                "F.Mask"
            } else if Some(&layer) == cx.copper.last() {
                "B.Mask"
            } else {
                continue;
            };
            let open = mask_openings(cx, pi, mask);
            let covered = (1..10).any(|k| {
                let t = k as f64 / 10.0;
                let m = [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
                !open.iter().any(|o| geom::point_in_polygon(m, o))
            });
            if covered {
                r.emit(
                    &at,
                    format!(
                        "solder mask covers the gap on {mask} at [{:.3}, {:.3}]; draw a filled {mask} shape over it in the footprint",
                        (p[0] + q[0]) / 2.0,
                        (p[1] + q[1]) / 2.0
                    ),
                );
            }
        }
    }
}

fn mask_openings(cx: &Ctx, pi: usize, mask: &str) -> Vec<Vec<P>> {
    let part = &cx.parts[pi];
    let mut out: Vec<Vec<P>> = part
        .pads
        .iter()
        .filter(|q| q.mask.iter().any(|m| m == mask))
        .flat_map(|q| q.outlines.iter().cloned())
        .collect();
    let tf = part.transform();
    for g in &part.footprint.graphics {
        if part.flip_layer(&g.layer) != mask || g.fill != Fill::Solid {
            continue;
        }
        let path: Vec<P> =
            crate::footprint::graphic_path(g).into_iter().map(|p| tf.apply(p)).collect();
        if path.len() >= 3 {
            out.push(path);
        }
    }
    out
}
