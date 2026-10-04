use crate::negotiate::{Base, Need, um};
use crate::placement::Hot;
use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::{Direction, EngineFile};
use agentee_core::geom::P;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

pub struct Global;

#[derive(Clone, Debug, Default, Serialize)]
pub struct GlobalPlan {
    pub tile: f64,
    pub origin: P,
    pub nets: usize,
    pub connections: usize,
    pub overflow: f64,
    pub over_edges: usize,
    pub rounds: usize,
    pub corridors: Vec<Corridor>,
    pub hot: Vec<Hot>,
    #[serde(skip)]
    pub hist: Vec<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Corridor {
    pub net: String,
    pub tiles: Vec<(usize, i64, i64)>,
}

struct Graph {
    nx: usize,
    ny: usize,
    nl: usize,
    tile: f64,
    origin: P,
    cap: Vec<f32>,
    dem: Vec<f32>,
    pin: Vec<f32>,
    hist: Vec<f32>,
    vcap: Vec<f32>,
    vdem: Vec<f32>,
    vhist: Vec<f32>,
    hpen: Vec<f32>,
    vpen: Vec<f32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Edge {
    Planar(usize),
    Via(usize),
}

impl Graph {
    fn plane(&self) -> usize {
        self.nx * self.ny
    }

    fn node(&self, l: usize, x: usize, y: usize) -> usize {
        l * self.plane() + y * self.nx + x
    }

    fn split(&self, n: usize) -> (usize, usize, usize) {
        let c = n % self.plane();
        (n / self.plane(), c % self.nx, c / self.nx)
    }

    fn h_edge(&self, l: usize, x: usize, y: usize) -> usize {
        2 * (l * self.plane() + y * self.nx + x)
    }

    fn v_edge(&self, l: usize, x: usize, y: usize) -> usize {
        2 * (l * self.plane() + y * self.nx + x) + 1
    }

    fn tile_of(&self, p: P) -> (usize, usize) {
        let x = ((p[0] - self.origin[0]) / self.tile).floor().max(0.0) as usize;
        let y = ((p[1] - self.origin[1]) / self.tile).floor().max(0.0) as usize;
        (x.min(self.nx - 1), y.min(self.ny - 1))
    }

    fn ends(&self, e: usize) -> [(usize, usize); 2] {
        let c = (e / 2) % self.plane();
        let (x, y) = (c % self.nx, c / self.nx);
        if e.is_multiple_of(2) { [(x, y), (x + 1, y)] } else { [(x, y), (x, y + 1)] }
    }

    fn centre(&self, x: usize, y: usize) -> P {
        [
            self.origin[0] + (x as f64 + 0.5) * self.tile,
            self.origin[1] + (y as f64 + 0.5) * self.tile,
        ]
    }
}

fn build(base: &Base, tile: f64, model: &Model, cfg: &EngineFile) -> Graph {
    let grid = &base.grid;
    let rules = &base.rules;
    let k = (tile / grid.g).round().max(2.0) as usize;
    let tile = k as f64 * grid.g;
    let nx = grid.w.div_ceil(k);
    let ny = grid.h.div_ceil(k);
    let nl = grid.nl;
    let mut g = Graph {
        nx,
        ny,
        nl,
        tile,
        origin: [grid.x0, grid.y0],
        cap: vec![0.0; 2 * nl * nx * ny],
        dem: vec![0.0; 2 * nl * nx * ny],
        pin: vec![0.0; 2 * nl * nx * ny],
        hist: vec![0.0; 2 * nl * nx * ny],
        vcap: vec![0.0; nx * ny],
        vdem: vec![0.0; nx * ny],
        vhist: vec![0.0; nx * ny],
        hpen: vec![1.0; nl],
        vpen: vec![1.0; nl],
    };
    let direction = cfg.global.as_ref().map(|g| g.direction.clone()).unwrap_or_default();
    for (l, name) in model.layout.copper.iter().enumerate() {
        match direction.get(name) {
            Some(Direction::H) => g.vpen[l] = 2.0,
            Some(Direction::V) => g.hpen[l] = 2.0,
            None => {}
        }
    }
    let mut hmin = vec![f64::MAX; nl];
    let mut cmin = f64::MAX;
    for r in rules.nets.iter().flatten() {
        cmin = cmin.min(r.clearance);
        for (h, (&on, &w)) in hmin.iter_mut().zip(r.track.iter().zip(&r.width)) {
            if on {
                *h = h.min(w / 2.0);
            }
        }
    }
    if cmin == f64::MAX {
        cmin = 0.1;
    }
    let plane = grid.plane();
    for (l, &h) in hmin.iter().enumerate() {
        if h == f64::MAX {
            continue;
        }
        let need = Need { d: um(h + cmin + rules.slack), q: um(h + rules.slack), hole: 0 };
        let pitch = (2.0 * h + cmin) as f32;
        let free = |fx: usize, fy: usize| {
            fx < grid.w && fy < grid.h && grid.open(l * plane + fy * grid.w + fx, need)
        };
        for y in 0..ny {
            for x in 0..nx {
                if x + 1 < nx {
                    let fx = (x + 1) * k;
                    let n = (y * k..y * k + k).filter(|&fy| free(fx, fy)).count();
                    if n > 0 {
                        let e = g.h_edge(l, x, y);
                        g.cap[e] = n as f32 * grid.g as f32 + pitch;
                    }
                }
                if y + 1 < ny {
                    let fy = (y + 1) * k;
                    let n = (x * k..x * k + k).filter(|&fx| free(fx, fy)).count();
                    if n > 0 {
                        let e = g.v_edge(l, x, y);
                        g.cap[e] = n as f32 * grid.g as f32 + pitch;
                    }
                }
            }
        }
    }
    if let Some((vk, v)) = rules.vias.iter().enumerate().min_by(|a, b| a.1.r.total_cmp(&b.1.r)) {
        let need = Need {
            d: um(v.dr + rules.hole_cu + rules.slack),
            q: um(v.r + cmin + rules.slack),
            hole: um(v.dr + rules.hole_gap + rules.slack),
        };
        let pitch = 2.0 * v.r + cmin;
        let per = (grid.g * grid.g / (pitch * pitch)) as f32;
        for y in 0..ny {
            for x in 0..nx {
                let mut n = 0;
                for fy in y * k..(y * k + k).min(grid.h) {
                    for fx in x * k..(x * k + k).min(grid.w) {
                        if grid.via_open(fy * grid.w + fx, &v.layers, vk, need) {
                            n += 1;
                        }
                    }
                }
                g.vcap[y * nx + x] = n as f32 * per;
            }
        }
    }
    g
}

#[derive(Clone, Copy, PartialEq)]
struct Item {
    f: f32,
    n: u32,
}

impl Eq for Item {}

impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.total_cmp(&self.f).then(o.n.cmp(&self.n))
    }
}

impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

struct NetJob {
    net: usize,
    pitch: Vec<f32>,
    track: Vec<bool>,
    via: bool,
    groups: Vec<Vec<usize>>,
    crit: f64,
    span: f64,
    used: Vec<Edge>,
    nodes: HashSet<usize>,
    pins: HashSet<(usize, usize)>,
}

fn over(dem: f32, cap: f32, p: f32) -> f32 {
    ((dem + p - cap).max(0.0) / p.max(1e-3)).min(50.0)
}

fn search(
    g: &Graph,
    job: &NetJob,
    from: &[usize],
    to: &HashSet<usize>,
    pres: f32,
    via_cost: f32,
) -> Option<Vec<(usize, Option<Edge>)>> {
    if to.is_empty() || from.is_empty() {
        return None;
    }
    let (mut lo, mut hi) = ((usize::MAX, usize::MAX), (0, 0));
    for &n in to {
        let (_, x, y) = g.split(n);
        lo = (lo.0.min(x), lo.1.min(y));
        hi = (hi.0.max(x), hi.1.max(y));
    }
    let t = g.tile as f32;
    let heur = |n: usize| {
        let (_, x, y) = g.split(n);
        let dx = if x < lo.0 { lo.0 - x } else { x.saturating_sub(hi.0) };
        let dy = if y < lo.1 { lo.1 - y } else { y.saturating_sub(hi.1) };
        (dx + dy) as f32 * t
    };
    let total = g.plane() * g.nl;
    let mut cost = vec![f32::INFINITY; total];
    let mut prev: Vec<(u32, Option<Edge>)> = vec![(u32::MAX, None); total];
    let mut heap = BinaryHeap::new();
    for &s in from {
        cost[s] = 0.0;
        heap.push(Item { f: heur(s), n: s as u32 });
    }
    let mut hit = None;
    while let Some(Item { f, n }) = heap.pop() {
        let n = n as usize;
        if f > cost[n] + heur(n) + 1e-3 {
            continue;
        }
        if to.contains(&n) {
            hit = Some(n);
            break;
        }
        let (l, x, y) = g.split(n);
        let here = cost[n];
        let mut relax = |m: usize, step: f32, e: Edge, heap: &mut BinaryHeap<Item>| {
            let c = here + step;
            if c < cost[m] {
                cost[m] = c;
                prev[m] = (n as u32, Some(e));
                heap.push(Item { f: c + heur(m), n: m as u32 });
            }
        };
        if job.track[l] {
            let p = job.pitch[l];
            let mut planar = |m: usize, e: usize, pen: f32, heap: &mut BinaryHeap<Item>| {
                let pinned = g.ends(e).iter().any(|t| job.pins.contains(t));
                let cap = if pinned { g.cap[e].max(p) } else { g.cap[e] };
                let step = t * pen * (1.0 + g.hist[e]) * (1.0 + pres * over(g.dem[e], cap, p));
                let step = if cap <= 0.0 { step + 20.0 * t } else { step };
                relax(m, step, Edge::Planar(e), heap);
            };
            if x + 1 < g.nx {
                planar(g.node(l, x + 1, y), g.h_edge(l, x, y), g.hpen[l], &mut heap);
            }
            if x > 0 {
                planar(g.node(l, x - 1, y), g.h_edge(l, x - 1, y), g.hpen[l], &mut heap);
            }
            if y + 1 < g.ny {
                planar(g.node(l, x, y + 1), g.v_edge(l, x, y), g.vpen[l], &mut heap);
            }
            if y > 0 {
                planar(g.node(l, x, y - 1), g.v_edge(l, x, y - 1), g.vpen[l], &mut heap);
            }
        }
        if job.via {
            let v = y * g.nx + x;
            let step =
                via_cost * (1.0 + g.vhist[v]) * (1.0 + pres * over(g.vdem[v], g.vcap[v], 1.0));
            for l2 in 0..g.nl {
                if l2 != l {
                    relax(g.node(l2, x, y), step, Edge::Via(v), &mut heap);
                }
            }
        }
    }
    let end = hit?;
    let mut path = Vec::new();
    let mut c = end;
    loop {
        let (p, e) = prev[c];
        path.push((c, e));
        if p == u32::MAX {
            break;
        }
        c = p as usize;
    }
    Some(path)
}

fn apply(g: &mut Graph, job: &NetJob, add: bool) {
    let s = if add { 1.0 } else { -1.0 };
    for e in &job.used {
        match *e {
            Edge::Planar(i) => {
                let l = i / 2 / g.plane();
                g.dem[i] += s * job.pitch[l];
                if g.ends(i).iter().any(|t| job.pins.contains(t)) {
                    g.pin[i] += s * job.pitch[l];
                }
            }
            Edge::Via(v) => g.vdem[v] += s,
        }
    }
}

fn route_net(g: &Graph, job: &mut NetJob, pres: f32, via_cost: f32) -> usize {
    job.used.clear();
    job.nodes.clear();
    let mut failed = 0;
    let mut rest: Vec<usize> = (1..job.groups.len()).collect();
    job.nodes.extend(job.groups[0].iter().copied());
    while !rest.is_empty() {
        let tile = |n: usize| {
            let (_, x, y) = g.split(n);
            (x as i64, y as i64)
        };
        let dist = |gi: usize| {
            job.groups[gi]
                .iter()
                .flat_map(|&a| {
                    job.nodes.iter().map(move |&b| {
                        let (p, q) = (tile(a), tile(b));
                        (p.0 - q.0).abs() + (p.1 - q.1).abs()
                    })
                })
                .min()
                .unwrap_or(i64::MAX)
        };
        let k = (0..rest.len()).min_by_key(|&i| dist(rest[i])).unwrap();
        let gi = rest.remove(k);
        let from = job.groups[gi].clone();
        match search(g, job, &from, &job.nodes.clone(), pres, via_cost) {
            Some(path) => {
                for (n, e) in path {
                    job.nodes.insert(n);
                    if let Some(e) = e {
                        job.used.push(e);
                    }
                }
            }
            None => {
                failed += 1;
                job.nodes.extend(from);
            }
        }
    }
    failed
}

fn overflowed(g: &Graph) -> (f64, Vec<usize>, Vec<usize>) {
    let mut total = 0.0;
    let mut edges = Vec::new();
    let mut vias = Vec::new();
    for (i, (&d, &c)) in g.dem.iter().zip(&g.cap).enumerate() {
        let d = d - g.pin[i];
        if d > c + 1e-3 {
            total += (d - c) as f64;
            edges.push(i);
        }
    }
    for (v, (&d, &c)) in g.vdem.iter().zip(&g.vcap).enumerate() {
        if d > c.max(1.0) + 1e-3 {
            total += (d - c.max(1.0)) as f64;
            vias.push(v);
        }
    }
    (total, edges, vias)
}

impl Phase for Global {
    fn name(&self) -> &'static str {
        "global"
    }

    fn run(&self, model: &mut Model, cfg: &EngineFile) -> PhaseReport {
        let mut report = PhaseReport { phase: "global".into(), ..Default::default() };
        let Some(base) = model.base.as_ref() else {
            report.failed.push("no board model to route on".into());
            return report;
        };
        let gc = cfg.global.clone().unwrap_or_default();
        let tile = gc.tile.map(|t| t.to_mm()).unwrap_or(1.0);
        let rounds = gc.rounds.unwrap_or(30) as usize;
        let via_cost = gc.via_cost.map(|v| v.to_mm()).unwrap_or(1.0) as f32;
        let mut g = build(base, tile, model, cfg);
        if let Some(prev) =
            model.global.as_ref().filter(|p| p.hist.len() == g.hist.len() + g.vhist.len())
        {
            let n = g.hist.len();
            g.hist.copy_from_slice(&prev.hist[..n]);
            g.vhist.copy_from_slice(&prev.hist[n..]);
        }
        if let Some(d) = model.detail.as_ref() {
            let copper = &model.layout.copper;
            for (_, layer, at) in &d.overlap {
                let Some(l) = copper.iter().position(|c| c == layer) else { continue };
                let (x, y) = g.tile_of(*at);
                for e in [
                    (x + 1 < g.nx).then(|| g.h_edge(l, x, y)),
                    (x > 0).then(|| g.h_edge(l, x - 1, y)),
                    (y + 1 < g.ny).then(|| g.v_edge(l, x, y)),
                    (y > 0).then(|| g.v_edge(l, x, y - 1)),
                ]
                .into_iter()
                .flatten()
                {
                    g.hist[e] += 0.5;
                }
            }
        }
        let l = &model.layout;
        let rules = &base.rules;
        let mut jobs: Vec<NetJob> = Vec::new();
        let mut planes = 0;
        for &n in &base.nets {
            let (terms, island) = base.terminals(l, n);
            if island {
                planes += 1;
                continue;
            }
            if terms.len() < 2 {
                continue;
            }
            let rule = rules.rule(n);
            let groups: Vec<Vec<usize>> = terms
                .iter()
                .map(|grp| {
                    let mut v: Vec<usize> = grp
                        .iter()
                        .flat_map(|(at, layers)| {
                            let (x, y) = g.tile_of(*at);
                            layers.iter().map(move |&ly| (ly, x, y)).collect::<Vec<_>>()
                        })
                        .map(|(ly, x, y)| g.node(ly, x, y))
                        .collect();
                    v.sort_unstable();
                    v.dedup();
                    v
                })
                .collect();
            let pts: Vec<P> = terms.iter().flatten().map(|t| t.0).collect();
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for q in &pts {
                lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                hi = [hi[0].max(q[0]), hi[1].max(q[1])];
            }
            jobs.push(NetJob {
                net: n,
                pitch: (0..g.nl).map(|ly| (rule.width[ly] + rule.clearance) as f32).collect(),
                track: rule.track.clone(),
                via: !rule.vias.is_empty(),
                groups,
                crit: rule.crit,
                span: (hi[0] - lo[0]) + (hi[1] - lo[1]),
                used: Vec::new(),
                nodes: HashSet::new(),
                pins: terms.iter().flatten().map(|t| g.tile_of(t.0)).collect(),
            });
        }
        jobs.sort_by(|a, b| b.crit.total_cmp(&a.crit).then(a.span.total_cmp(&b.span)));
        let connections: usize = jobs.iter().map(|j| j.groups.len() - 1).sum();
        let mut pres = 0.5f32;
        let mut todo: Vec<usize> = (0..jobs.len()).collect();
        let mut best = f64::MAX;
        let mut since = 0;
        let mut done = 0;
        let mut failed = 0;
        let (mut total, mut over_edges, mut over_vias) = (0.0, Vec::new(), Vec::new());
        for round in 0..rounds.max(1) {
            done = round + 1;
            failed = 0;
            for &j in &todo {
                apply(&mut g, &jobs[j], false);
                failed += route_net(&g, &mut jobs[j], pres, via_cost);
                apply(&mut g, &jobs[j], true);
            }
            (total, over_edges, over_vias) = overflowed(&g);
            report.notes.push(format!(
                "round {round}: {} nets rerouted, overflow {total:.1} mm on {} edges and {} via tiles",
                todo.len(),
                over_edges.len(),
                over_vias.len()
            ));
            if over_edges.is_empty() && over_vias.is_empty() {
                break;
            }
            for &e in &over_edges {
                g.hist[e] += 0.3;
            }
            for &v in &over_vias {
                g.vhist[v] += 0.3;
            }
            if total < best - 1e-6 {
                best = total;
                since = 0;
            } else {
                since += 1;
                if since >= 5 {
                    break;
                }
            }
            let hot_e: HashSet<Edge> = over_edges
                .iter()
                .map(|&e| Edge::Planar(e))
                .chain(over_vias.iter().map(|&v| Edge::Via(v)))
                .collect();
            todo = (0..jobs.len())
                .filter(|&j| jobs[j].used.iter().any(|e| hot_e.contains(e)))
                .collect();
            pres *= 1.5;
        }
        if report.notes.len() > 6 {
            let last = report.notes.split_off(report.notes.len() - 3);
            report.notes.truncate(2);
            report.notes.push("...".into());
            report.notes.extend(last);
        }
        let mut heat: HashMap<(usize, usize), f64> = HashMap::new();
        for &e in &over_edges {
            let c = (e / 2) % g.plane();
            let (x, y) = (c % g.nx, c / g.nx);
            let v = (g.dem[e] - g.pin[e] - g.cap[e]) as f64;
            *heat.entry((x, y)).or_default() += v / 2.0;
            let other = if e.is_multiple_of(2) { (x + 1, y) } else { (x, y + 1) };
            *heat.entry(other).or_default() += v / 2.0;
        }
        for &v in &over_vias {
            *heat.entry((v % g.nx, v / g.nx)).or_default() +=
                (g.vdem[v] - g.vcap[v].max(1.0)) as f64 * 0.3;
        }
        let hot: Vec<Hot> = heat
            .into_iter()
            .map(|((x, y), overflow)| Hot { at: g.centre(x, y), size: g.tile, overflow })
            .collect();
        let corridors: Vec<Corridor> = jobs
            .iter()
            .map(|j| Corridor {
                net: l.nets[j.net].name.clone(),
                tiles: j
                    .nodes
                    .iter()
                    .map(|&n| {
                        let (ly, x, y) = g.split(n);
                        (ly, x as i64, y as i64)
                    })
                    .collect(),
            })
            .collect();
        report.notes.push(format!(
            "{} nets, {connections} connections on {:.2} mm tiles, {planes} plane nets left to detail, overflow {total:.1} mm after {done} rounds",
            jobs.len(),
            g.tile
        ));
        if failed > 0 {
            report.failed.push(format!("{failed} connections found no tile path"));
        }
        let mut hist = g.hist.clone();
        hist.extend_from_slice(&g.vhist);
        report.changed = true;
        model.global = Some(GlobalPlan {
            tile: g.tile,
            origin: g.origin,
            nets: jobs.len(),
            connections,
            overflow: total,
            over_edges: over_edges.len() + over_vias.len(),
            rounds: done,
            corridors,
            hot,
            hist,
        });
        report
    }
}

pub fn guide(model: &Model) -> crate::negotiate::Guide {
    let mut out = crate::negotiate::Guide::default();
    let l = &model.layout;
    if let Some(g) = &model.global {
        out.tile = g.tile;
        out.origin = g.origin;
        for c in &g.corridors {
            if let Some(n) = l.nets.iter().position(|x| x.name == c.net) {
                out.corridors.insert(n, c.tiles.iter().copied().collect());
            }
        }
    }
    if let Some(a) = &model.access {
        for &(n, ly, p, q) in &a.prefer {
            out.prefer.entry(n).or_default().push((ly, p, q));
        }
        for &(n, at) in &a.prefer_vias {
            out.prefer_vias.entry(n).or_default().push(at);
        }
    }
    out
}
