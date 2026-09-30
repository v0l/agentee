use crate::board::Board;
use crate::footprint::PadKind;
use crate::geom::{self, P};
use crate::layout::{Layout, glob};
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub struct RouteOptions {
    pub nets: Vec<String>,
    pub layers: Vec<String>,
    pub grid: f64,
    pub via: Option<String>,
    pub via_cost: f64,
    pub margin: f64,
    pub pairs: bool,
}

impl Default for RouteOptions {
    fn default() -> Self {
        RouteOptions {
            nets: Vec::new(),
            layers: Vec::new(),
            grid: 0.05,
            via: None,
            via_cost: 1.0,
            margin: 5.0,
            pairs: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RoutedTrack {
    pub net: String,
    pub layer: String,
    pub points: Vec<P>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RoutedVia {
    pub net: String,
    pub at: P,
    pub via: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Unrouted {
    pub net: String,
    pub from: P,
    pub to: P,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RouteResult {
    pub connections: usize,
    pub routed: usize,
    pub tracks: Vec<RoutedTrack>,
    pub vias: Vec<RoutedVia>,
    pub failed: Vec<Unrouted>,
}

#[derive(Clone)]
enum Shape {
    Poly(Vec<P>),
    Seg(P, P, f64),
    Circle(P, f64),
}

impl Shape {
    fn dist(&self, p: P) -> f64 {
        match self {
            Shape::Poly(v) => {
                if geom::point_in_polygon(p, v) {
                    return 0.0;
                }
                (0..v.len())
                    .map(|i| geom::point_segment_distance(p, v[i], v[(i + 1) % v.len()]))
                    .fold(f64::MAX, f64::min)
            }
            Shape::Seg(a, b, r) => (geom::point_segment_distance(p, *a, *b) - r).max(0.0),
            Shape::Circle(c, r) => (geom::dist(p, *c) - r).max(0.0),
        }
    }

    fn bounds(&self) -> (P, P) {
        match self {
            Shape::Poly(v) => {
                let mut lo = [f64::MAX; 2];
                let mut hi = [f64::MIN; 2];
                for q in v {
                    lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                    hi = [hi[0].max(q[0]), hi[1].max(q[1])];
                }
                (lo, hi)
            }
            Shape::Seg(a, b, r) => {
                ([a[0].min(b[0]) - r, a[1].min(b[1]) - r], [a[0].max(b[0]) + r, a[1].max(b[1]) + r])
            }
            Shape::Circle(c, r) => ([c[0] - r, c[1] - r], [c[0] + r, c[1] + r]),
        }
    }
}

#[derive(Clone)]
struct Obstacle {
    net: Option<usize>,
    layers: Vec<usize>,
    shape: Shape,
    clearance: f64,
}

const FREE: u16 = 0;
const BLOCK: u16 = u16::MAX;

struct Grid {
    x0: f64,
    y0: f64,
    g: f64,
    w: usize,
    h: usize,
    track: Vec<u16>,
    via: Vec<u16>,
    rt: Vec<u16>,
    rv: Vec<u16>,
    hist: Vec<f32>,
}

impl Grid {
    fn add_drill(&mut self, c: P, reach: f64) {
        let shape = Shape::Circle(c, 0.0);
        for (x, y) in self.cells_near(&shape, reach + self.g * 0.6) {
            for l in 0..self.via.len() / (self.w * self.h) {
                let i = self.idx(l, x, y);
                self.via[i] = BLOCK;
            }
        }
    }

    fn clear_routed(&mut self) {
        self.rt.iter_mut().for_each(|v| *v = FREE);
        self.rv.iter_mut().for_each(|v| *v = FREE);
    }

    fn mark_routed(&mut self, c: &Conn, ctx: &Ctx) {
        let value = c.net as u16 + 1;
        let slack = self.g * 0.6;
        let mut shapes: Vec<(Vec<usize>, Shape)> = Vec::new();
        for (l, pts) in &c.tracks {
            for s in pts.windows(2) {
                shapes.push((vec![*l], Shape::Seg(s[0], s[1], ctx.widths[*l] / 2.0)));
            }
        }
        for v in &c.vias {
            shapes.push((ctx.via_layers.to_vec(), Shape::Circle(*v, ctx.via_r)));
            let shape = Shape::Circle(*v, 0.0);
            let reach = 2.0 * ctx.drill_r + ctx.hole_gap + self.g * 0.6;
            for (x, y) in self.cells_near(&shape, reach) {
                for l in 0..self.rv.len() / (self.w * self.h) {
                    let i = self.idx(l, x, y);
                    self.rv[i] = BLOCK;
                }
            }
        }
        let widest = ctx.widths.iter().cloned().fold(0.0, f64::max);
        for (layers, shape) in shapes {
            for is_via in [false, true] {
                let reach = |l: usize| {
                    (if is_via { ctx.via_r } else { ctx.widths[l] / 2.0 }) + ctx.clearance + slack
                };
                let most = (if is_via { ctx.via_r } else { widest / 2.0 }) + ctx.clearance + slack;
                for (x, y) in self.cells_near(&shape, most) {
                    let d = shape.dist(self.center(x, y));
                    for &l in layers.iter().filter(|&&l| d <= reach(l)) {
                        let i = self.idx(l, x, y);
                        if is_via {
                            Self::mark(&mut self.rv, i, value);
                        } else {
                            Self::mark(&mut self.rt, i, value);
                        }
                    }
                }
            }
        }
    }

    fn crowd(&self, net: usize, p: P, routing: &[usize]) -> usize {
        let r = (0.6 / self.g).ceil() as i64;
        let (cx, cy) = self.cell(p);
        let mut n = 0;
        for &l in routing {
            for y in (cy - r).max(0)..=(cy + r).min(self.h as i64 - 1) {
                for x in (cx - r).max(0)..=(cx + r).min(self.w as i64 - 1) {
                    if !Self::free(&self.track, self.idx(l, x as usize, y as usize), net) {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    fn ok(&self, i: usize, net: usize, via: bool, soft: bool) -> (bool, bool) {
        let (fixed, routed) = if via { (&self.via, &self.rv) } else { (&self.track, &self.rt) };
        if !Self::free(fixed, i, net) {
            return (false, false);
        }
        let clash = !Self::free(routed, i, net);
        (soft || !clash, clash)
    }

    fn idx(&self, l: usize, x: usize, y: usize) -> usize {
        (l * self.h + y) * self.w + x
    }

    fn center(&self, x: usize, y: usize) -> P {
        [self.x0 + (x as f64 + 0.5) * self.g, self.y0 + (y as f64 + 0.5) * self.g]
    }

    fn cell(&self, p: P) -> (i64, i64) {
        (((p[0] - self.x0) / self.g).floor() as i64, ((p[1] - self.y0) / self.g).floor() as i64)
    }

    fn cells_near(&self, shape: &Shape, reach: f64) -> Vec<(usize, usize)> {
        let (lo, hi) = shape.bounds();
        let (x0, y0) = self.cell([lo[0] - reach, lo[1] - reach]);
        let (x1, y1) = self.cell([hi[0] + reach, hi[1] + reach]);
        let mut out = Vec::new();
        for y in y0.max(0)..=y1.min(self.h as i64 - 1) {
            for x in x0.max(0)..=x1.min(self.w as i64 - 1) {
                let (x, y) = (x as usize, y as usize);
                if shape.dist(self.center(x, y)) <= reach {
                    out.push((x, y));
                }
            }
        }
        out
    }

    fn mark(map: &mut [u16], i: usize, value: u16) {
        let c = map[i];
        if c == FREE {
            map[i] = value;
        } else if c != value {
            map[i] = BLOCK;
        }
    }

    fn add(&mut self, o: &Obstacle, half_widths: &[f64], via_radius: f64, clearance: f64) {
        let value = o.net.map(|n| n as u16 + 1).unwrap_or(BLOCK);
        let c = clearance.max(o.clearance);
        let slack = self.g * 0.6;
        let widest = o.layers.iter().map(|&l| half_widths[l]).fold(0.0, f64::max);
        for is_via in [false, true] {
            let most = (if is_via { via_radius } else { widest }) + c + slack;
            for (x, y) in self.cells_near(&o.shape, most) {
                let d = o.shape.dist(self.center(x, y));
                for &l in o.layers.iter().filter(|&&l| {
                    d <= (if is_via { via_radius } else { half_widths[l] }) + c + slack
                }) {
                    let i = self.idx(l, x, y);
                    if is_via {
                        Self::mark(&mut self.via, i, value);
                    } else {
                        Self::mark(&mut self.track, i, value);
                    }
                }
            }
        }
    }

    fn free(map: &[u16], i: usize, net: usize) -> bool {
        let c = map[i];
        c == FREE || c == net as u16 + 1
    }
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

const DIRS: [(i64, i64); 8] =
    [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];

pub fn route(layout: &Layout, board: &Board, opts: &RouteOptions) -> Result<RouteResult, String> {
    let copper = &layout.copper;
    let layer_of = |name: &str| copper.iter().position(|c| c == name);
    let mut routing = Vec::new();
    for l in &opts.layers {
        routing.push(layer_of(l).ok_or_else(|| format!("`{l}` is not a copper layer"))?);
    }
    if routing.is_empty() {
        routing = (0..copper.len()).collect();
    }
    let targets: Vec<usize> = (0..layout.nets.len())
        .filter(|&n| opts.nets.iter().any(|g| glob(g, &layout.nets[n].name)))
        .collect();
    if targets.is_empty() {
        return Err("no net matches".into());
    }

    let mut obstacles = Vec::new();
    for part in &layout.parts {
        for pad in &part.pads {
            let layers: Vec<usize> = pad.copper.iter().filter_map(|c| layer_of(c)).collect();
            if !layers.is_empty() {
                for o in &pad.outlines {
                    obstacles.push(Obstacle {
                        net: pad.net,
                        layers: layers.clone(),
                        shape: Shape::Poly(o.clone()),
                        clearance: pad.net.map(|n| layout.nets[n].clearance).unwrap_or(0.0),
                    });
                }
            }
            if let Some((c, s, _)) = pad.drill
                && (pad.kind == PadKind::Npth || layers.is_empty())
            {
                obstacles.push(Obstacle {
                    net: None,
                    layers: (0..copper.len()).collect(),
                    shape: Shape::Circle(c, s[0].max(s[1]) / 2.0),
                    clearance: 0.0,
                });
            }
        }
    }
    for t in &layout.tracks {
        let Some(l) = layer_of(&t.layer) else { continue };
        for w in t.points.windows(2) {
            obstacles.push(Obstacle {
                net: Some(t.net),
                layers: vec![l],
                shape: Shape::Seg(w[0], w[1], t.width / 2.0),
                clearance: layout.nets[t.net].clearance,
            });
        }
    }
    for v in &layout.vias {
        obstacles.push(Obstacle {
            net: Some(v.net),
            layers: v.layers.iter().filter_map(|c| layer_of(c)).collect(),
            shape: Shape::Circle(v.at, v.diameter / 2.0),
            clearance: layout.nets[v.net].clearance,
        });
    }

    let mut drills: Vec<(P, f64)> = layout.vias.iter().map(|v| (v.at, v.drill / 2.0)).collect();
    for part in &layout.parts {
        for pad in &part.pads {
            if let Some((c, s, _)) = pad.drill {
                drills.push((c, s[0].min(s[1]) / 2.0));
            }
        }
    }

    let mut by_class: Vec<(String, Vec<usize>)> = Vec::new();
    for &n in &targets {
        let c = &layout.nets[n].class;
        match by_class.iter_mut().find(|(k, _)| k == c) {
            Some((_, v)) => v.push(n),
            None => by_class.push((c.clone(), vec![n])),
        }
    }
    let partner = |n: usize| {
        layout.pairs.iter().find_map(|p| {
            if p.p == n {
                Some(p.n)
            } else if p.n == n {
                Some(p.p)
            } else {
                None
            }
        })
    };

    let mut out = RouteResult::default();
    let edge = board.rules.min_copper_to_edge.to_mm();
    for (class, nets) in by_class {
        let nc = board.netclasses.iter().find(|c| c.name == class);
        let spec_name = opts.via.clone().or_else(|| nc.and_then(|c| c.via.clone()));
        let spec = spec_name
            .as_ref()
            .and_then(|n| board.vias.iter().find(|v| &v.name == n))
            .or(board.vias.first())
            .ok_or("the board defines no [[vias]]")?;
        let width = layout.nets[nets[0]].width;
        let widths: Vec<f64> =
            copper.iter().map(|l| nc.map(|c| c.width_on(l).to_mm()).unwrap_or(width)).collect();
        let halves: Vec<f64> = widths.iter().map(|w| w / 2.0).collect();
        let clearance = layout.nets[nets[0]].clearance;
        let gap = nc.and_then(|c| c.diff_gap).map(|g| g.to_mm());
        let via_r = spec.diameter.to_mm() / 2.0;
        let via_layers: Vec<usize> = {
            let a = layer_of(&spec.from).unwrap_or(0);
            let b = layer_of(&spec.to).unwrap_or(copper.len() - 1);
            (a.min(b)..=a.max(b)).collect()
        };
        let class_routing: Vec<usize> = match nc.filter(|c| !c.layers.is_empty()) {
            Some(c) => routing
                .iter()
                .copied()
                .filter(|&l| c.layers.iter().any(|x| x == &copper[l]))
                .collect(),
            None => routing.clone(),
        };
        if class_routing.is_empty() {
            for &n in &nets {
                out.failed.push(Unrouted {
                    net: layout.nets[n].name.clone(),
                    from: [0.0, 0.0],
                    to: [0.0, 0.0],
                    reason: format!("class {class} allows none of the routing layers"),
                });
            }
            continue;
        }
        let routing = &class_routing;
        let ctx = Ctx {
            drill_r: spec.drill.to_mm() / 2.0,
            hole_gap: board.rules.min_hole_to_hole.to_mm(),
            widths: widths.clone(),
            clearance,
            via_r,
            via_layers: &via_layers,
            routing,
            opts,
        };

        let mut grid = build_grid(layout, opts.grid, &halves, via_r, edge);
        for o in &obstacles {
            grid.add(o, &halves, via_r, clearance);
        }
        for &(c, r) in &drills {
            grid.add_drill(c, r + ctx.drill_r + ctx.hole_gap);
        }

        let conns: Vec<(P, P, usize)> =
            layout.ratsnest.iter().filter(|(_, _, n)| nets.contains(n)).cloned().collect();
        let crowd: Vec<usize> = conns
            .iter()
            .map(|c| grid.crowd(c.2, c.0, routing) + grid.crowd(c.2, c.1, routing))
            .collect();
        let mut order: Vec<usize> = (0..conns.len()).collect();
        order.sort_by(|&i, &j| {
            crowd[j].cmp(&crowd[i]).then(
                geom::dist(conns[i].0, conns[i].1)
                    .partial_cmp(&geom::dist(conns[j].0, conns[j].1))
                    .unwrap_or(Ordering::Equal),
            )
        });
        let conns: Vec<(P, P, usize)> = order.into_iter().map(|i| conns[i]).collect();
        out.connections += conns.len();
        let mut routed: Vec<Option<Conn>> = vec![None; conns.len()];
        let mut locked = vec![false; conns.len()];
        if let Some(gap) = gap.filter(|_| opts.pairs) {
            for pair in &layout.pairs {
                let find = |n: usize| {
                    let v: Vec<usize> = (0..conns.len()).filter(|&i| conns[i].2 == n).collect();
                    (v.len() == 1).then(|| v[0])
                };
                let (Some(cp), Some(cn)) = (find(pair.p), find(pair.n)) else { continue };
                match route_pair(&mut grid, &obstacles, &ctx, gap, conns[cp], conns[cn]) {
                    Some((pc, nc)) => {
                        routed[cp] = Some(pc);
                        routed[cn] = Some(nc);
                        locked[cp] = true;
                        locked[cn] = true;
                    }
                    None => {
                        grid.clear_routed();
                        for r in routed.iter().flatten() {
                            grid.mark_routed(r, &ctx);
                        }
                    }
                }
            }
        }
        let mut ripped = vec![0usize; conns.len()];
        let mut queue: std::collections::VecDeque<usize> =
            (0..conns.len()).filter(|&i| routed[i].is_none()).collect();
        let mut failed: Vec<(usize, String)> = Vec::new();
        let mut budget = conns.len() * 40 + 100;
        while let Some(ci) = queue.pop_front() {
            let (a, b, net) = conns[ci];
            let attract = gap.and_then(|gap| {
                let other = partner(net)?;
                let geo: Vec<&Conn> = routed.iter().flatten().filter(|c| c.net == other).collect();
                (!geo.is_empty()).then(|| attraction(&grid, &geo, gap, &widths))
            });
            let hard = search(&grid, &obstacles, net, a, b, &ctx, false, attract.as_ref());
            let path = match hard {
                Ok(p) => p,
                Err(reason) if budget == 0 || ripped[ci] > 30 => {
                    failed.push((ci, reason));
                    continue;
                }
                Err(reason) => {
                    budget -= 1;
                    match search(&grid, &obstacles, net, a, b, &ctx, true, attract.as_ref()) {
                        Ok(p) => {
                            for &i in &p {
                                if grid.ok(i, net, false, true).1 {
                                    grid.hist[i] += (grid.g * 10.0) as f32;
                                }
                            }
                            let conn = conn_of(&grid, &p, a, b, net);
                            let mut hits: Vec<usize> = (0..routed.len())
                                .filter(|&k| {
                                    routed[k]
                                        .as_ref()
                                        .is_some_and(|r| r.net != net && conflicts(r, &conn, &ctx))
                                })
                                .collect();
                            let partners: Vec<usize> = hits
                                .iter()
                                .filter(|&&k| locked[k])
                                .filter_map(|&k| {
                                    let other = partner(conns[k].2)?;
                                    (0..conns.len()).find(|&j| conns[j].2 == other && locked[j])
                                })
                                .collect();
                            hits.extend(partners);
                            hits.sort_unstable();
                            hits.dedup();
                            for &k in &hits {
                                locked[k] = false;
                            }
                            if std::env::var("AGENTEE_ROUTE_DEBUG").is_ok() {
                                eprintln!(
                                    "{} rips {:?}",
                                    layout.nets[net].name,
                                    hits.iter()
                                        .map(|&k| &layout.nets[conns[k].2].name)
                                        .collect::<Vec<_>>()
                                );
                            }
                            for &k in &hits {
                                routed[k] = None;
                                ripped[k] += 1;
                                queue.push_back(k);
                            }
                            grid.clear_routed();
                            for r in routed.iter().flatten() {
                                grid.mark_routed(r, &ctx);
                            }
                            if !p.iter().all(|&i| grid.ok(i, net, false, false).0) {
                                ripped[ci] += 1;
                                queue.push_back(ci);
                                continue;
                            }
                            p
                        }
                        Err(e) => {
                            if std::env::var("AGENTEE_ROUTE_DEBUG").is_ok() {
                                eprintln!("{} soft failed: {e}", layout.nets[net].name);
                            }
                            failed.push((ci, reason));
                            continue;
                        }
                    }
                }
            };
            let conn = conn_of(&grid, &path, a, b, net);
            grid.mark_routed(&conn, &ctx);
            routed[ci] = Some(conn);
        }

        for (ci, reason) in failed {
            if routed[ci].is_none() {
                let (a, b, net) = conns[ci];
                out.failed.push(Unrouted {
                    net: layout.nets[net].name.clone(),
                    from: a,
                    to: b,
                    reason,
                });
            }
        }
        for conn in routed.into_iter().flatten() {
            out.routed += 1;
            let name = layout.nets[conn.net].name.clone();
            for (l, pts) in conn.tracks {
                for w in pts.windows(2) {
                    obstacles.push(Obstacle {
                        net: Some(conn.net),
                        layers: vec![l],
                        shape: Shape::Seg(w[0], w[1], widths[l] / 2.0),
                        clearance,
                    });
                }
                out.tracks.push(RoutedTrack {
                    net: name.clone(),
                    layer: copper[l].clone(),
                    points: pts,
                });
            }
            for at in conn.vias {
                drills.push((at, spec.drill.to_mm() / 2.0));
                obstacles.push(Obstacle {
                    net: Some(conn.net),
                    layers: via_layers.clone(),
                    shape: Shape::Circle(at, via_r),
                    clearance,
                });
                out.vias.push(RoutedVia { net: name.clone(), at, via: spec.name.clone() });
            }
        }
    }
    Ok(out)
}

struct Ctx<'a> {
    drill_r: f64,
    hole_gap: f64,
    widths: Vec<f64>,
    clearance: f64,
    via_r: f64,
    via_layers: &'a [usize],
    routing: &'a [usize],
    opts: &'a RouteOptions,
}

#[derive(Clone)]
struct Conn {
    net: usize,
    tracks: Vec<(usize, Vec<P>)>,
    vias: Vec<P>,
}

fn conn_of(grid: &Grid, path: &[usize], a: P, b: P, net: usize) -> Conn {
    let (tracks, vias) = geometry(grid, path, a, b, net);
    Conn { net, tracks, vias }
}

fn conflicts(r: &Conn, c: &Conn, ctx: &Ctx) -> bool {
    let need = |l: usize| ctx.widths[l] + ctx.clearance - 1e-6;
    let via_need = |l: usize| ctx.via_r + ctx.widths[l] / 2.0 + ctx.clearance - 1e-6;
    let vv_need = 2.0 * ctx.via_r + ctx.clearance - 1e-6;
    for (la, pa) in &r.tracks {
        for (lb, pb) in &c.tracks {
            if la != lb {
                continue;
            }
            for s in pa.windows(2) {
                for t in pb.windows(2) {
                    if geom::segment_segment_distance(s[0], s[1], t[0], t[1]) < need(*la) {
                        return true;
                    }
                }
            }
        }
        for v in &c.vias {
            if ctx.via_layers.contains(la)
                && pa
                    .windows(2)
                    .any(|s| geom::point_segment_distance(*v, s[0], s[1]) < via_need(*la))
            {
                return true;
            }
        }
    }
    for v in &r.vias {
        for (lb, pb) in &c.tracks {
            if ctx.via_layers.contains(lb)
                && pb
                    .windows(2)
                    .any(|s| geom::point_segment_distance(*v, s[0], s[1]) < via_need(*lb))
            {
                return true;
            }
        }
        if c.vias.iter().any(|w| geom::dist(*v, *w) < vv_need) {
            return true;
        }
    }
    false
}

fn attraction(
    grid: &Grid,
    geo: &[&Conn],
    gap: f64,
    widths: &[f64],
) -> std::collections::HashSet<usize> {
    let mut out = std::collections::HashSet::new();
    let band = grid.g * 0.75;
    for c in geo {
        for (l, pts) in &c.tracks {
            let (width, pitch) = (widths[*l], widths[*l] + gap);
            for s in pts.windows(2) {
                let shape = Shape::Seg(s[0], s[1], 0.0);
                for (x, y) in grid.cells_near(&shape, pitch + band) {
                    let d = shape.dist(grid.center(x, y));
                    if (d - pitch).abs() <= band && d > width {
                        out.insert(grid.idx(*l, x, y));
                    }
                }
            }
        }
    }
    out
}

fn build_grid(layout: &Layout, g: f64, halves: &[f64], via_r: f64, edge: f64) -> Grid {
    let layers = halves.len();
    let b = layout.bounds();
    let (x0, y0) = (b.min[0] - g, b.min[1] - g);
    let w = ((b.max[0] - x0) / g).ceil() as usize + 2;
    let h = ((b.max[1] - y0) / g).ceil() as usize + 2;
    let mut grid = Grid {
        x0,
        y0,
        g,
        w,
        h,
        track: vec![FREE; layers * w * h],
        via: vec![FREE; layers * w * h],
        rt: vec![FREE; layers * w * h],
        rv: vec![FREE; layers * w * h],
        hist: vec![0.0; layers * w * h],
    };
    let outline = &layout.outline;
    let n = outline.len();
    for y in 0..h {
        let cy = grid.center(0, y)[1];
        let mut xs: Vec<f64> = (0..n)
            .filter_map(|i| {
                let (a, b) = (outline[i], outline[(i + 1) % n]);
                ((a[1] <= cy) != (b[1] <= cy))
                    .then(|| a[0] + (cy - a[1]) / (b[1] - a[1]) * (b[0] - a[0]))
            })
            .collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        for x in 0..w {
            let cx = grid.center(x, y)[0];
            let inside = xs.iter().filter(|&&e| e < cx).count() % 2 == 1;
            if !inside {
                for l in 0..layers {
                    let i = grid.idx(l, x, y);
                    grid.track[i] = BLOCK;
                    grid.via[i] = BLOCK;
                }
            }
        }
    }
    for i in 0..n {
        let seg = Shape::Seg(outline[i], outline[(i + 1) % n], 0.0);
        let widest = halves.iter().cloned().fold(0.0, f64::max);
        for (reach, via) in [(edge + widest, false), (edge + via_r, true)] {
            for (x, y) in grid.cells_near(&seg, reach) {
                let d = seg.dist(grid.center(x, y));
                for l in (0..layers).filter(|&l| via || d <= edge + halves[l]) {
                    let k = grid.idx(l, x, y);
                    if via {
                        grid.via[k] = BLOCK;
                    } else {
                        grid.track[k] = BLOCK;
                    }
                }
            }
        }
    }
    grid
}

fn anchors(grid: &Grid, obstacles: &[Obstacle], net: usize, p: P, routing: &[usize]) -> Vec<usize> {
    let own: Vec<&Obstacle> = obstacles.iter().filter(|o| o.net == Some(net)).collect();
    let mut seen = vec![false; own.len()];
    let mut stack: Vec<usize> = (0..own.len()).filter(|&i| own[i].shape.dist(p) < 1e-6).collect();
    stack.iter().for_each(|&i| seen[i] = true);
    let mut group = Vec::new();
    while let Some(i) = stack.pop() {
        group.push(i);
        for j in 0..own.len() {
            if !seen[j]
                && own[i].layers.iter().any(|l| own[j].layers.contains(l))
                && touch(&own[i].shape, &own[j].shape)
            {
                seen[j] = true;
                stack.push(j);
            }
        }
    }
    let mut out = Vec::new();
    for i in group {
        let o = own[i];
        for (x, y) in grid.cells_near(&o.shape, 0.0) {
            for &l in o.layers.iter().filter(|l| routing.contains(l)) {
                out.push(grid.idx(l, x, y));
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn touch(a: &Shape, b: &Shape) -> bool {
    let edges = |v: &Vec<P>| -> Vec<Shape> {
        (0..v.len()).map(|i| Shape::Seg(v[i], v[(i + 1) % v.len()], 0.0)).collect()
    };
    let point = |s: &Shape| match s {
        Shape::Poly(v) => v[0],
        Shape::Seg(a, _, _) => *a,
        Shape::Circle(c, _) => *c,
    };
    match (a, b) {
        (Shape::Poly(v), other) | (other, Shape::Poly(v)) => {
            geom::point_in_polygon(point(other), v)
                || matches!(other, Shape::Poly(w) if geom::point_in_polygon(w[0], v))
                || edges(v).iter().any(|e| touch(e, other))
        }
        (Shape::Circle(c, r), Shape::Circle(d, q)) => geom::dist(*c, *d) <= r + q + 1e-6,
        (Shape::Seg(a0, a1, r), Shape::Circle(c, q))
        | (Shape::Circle(c, q), Shape::Seg(a0, a1, r)) => {
            geom::point_segment_distance(*c, *a0, *a1) <= r + q + 1e-6
        }
        (Shape::Seg(a0, a1, r), Shape::Seg(b0, b1, q)) => {
            geom::segment_segment_distance(*a0, *a1, *b0, *b1) <= r + q + 1e-6
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn search(
    grid: &Grid,
    obstacles: &[Obstacle],
    net: usize,
    a: P,
    b: P,
    ctx: &Ctx,
    soft: bool,
    attract: Option<&std::collections::HashSet<usize>>,
) -> Result<Vec<usize>, String> {
    let sources = anchors(grid, obstacles, net, a, ctx.routing);
    let goals = anchors(grid, obstacles, net, b, ctx.routing);
    search_between(grid, net, a, b, &sources, &goals, ctx, soft, attract, None)
}

#[allow(clippy::too_many_arguments)]
fn search_between(
    grid: &Grid,
    net: usize,
    a: P,
    b: P,
    sources: &[usize],
    goals: &[usize],
    ctx: &Ctx,
    soft: bool,
    attract: Option<&std::collections::HashSet<usize>>,
    repel: Option<&std::collections::HashSet<usize>>,
) -> Result<Vec<usize>, String> {
    let (routing, via_layers, opts) = (ctx.routing, ctx.via_layers, ctx.opts);
    let penalty = 20.0 * grid.g;
    if sources.is_empty() || goals.is_empty() {
        return Err("an end has no copper on the routing layers".into());
    }
    let legal = |v: &[usize]| -> Vec<usize> {
        v.iter().copied().filter(|&i| grid.ok(i, net, false, soft).0).collect()
    };
    let (sources, goals) = (legal(sources), legal(goals));
    if sources.is_empty() || goals.is_empty() {
        return Err("the class width does not fit into an end pad, neck it down by hand".into());
    }
    let (sources, goals) = (sources.as_slice(), goals.as_slice());
    let plane = grid.w * grid.h;
    let unpack = |i: usize| (i / plane, (i % plane) % grid.w, (i % plane) / grid.w);
    let mut margin = opts.margin;
    loop {
        let lo = grid.cell([a[0].min(b[0]) - margin, a[1].min(b[1]) - margin]);
        let hi = grid.cell([a[0].max(b[0]) + margin, a[1].max(b[1]) + margin]);
        let (wx0, wy0) = (lo.0.max(0) as usize, lo.1.max(0) as usize);
        let (wx1, wy1) =
            ((hi.0.max(0) as usize).min(grid.w - 1), (hi.1.max(0) as usize).min(grid.h - 1));
        let (ww, wh) = (wx1 - wx0 + 1, wy1 - wy0 + 1);
        let local = |i: usize| -> Option<usize> {
            let (l, x, y) = unpack(i);
            let rl = routing.iter().position(|&r| r == l)?;
            (x >= wx0 && x <= wx1 && y >= wy0 && y <= wy1)
                .then(|| (rl * wh + (y - wy0)) * ww + (x - wx0))
        };
        let n = routing.len() * ww * wh;
        let mut cost = vec![f32::MAX; n];
        let mut from = vec![usize::MAX; n];
        let mut dir = vec![8u8; n];
        let mut goal = vec![false; n];
        for &gi in goals {
            if let Some(k) = local(gi) {
                goal[k] = true;
            }
        }
        let (gx, gy) = {
            let c = grid.cell(b);
            (c.0 as f64, c.1 as f64)
        };
        let heur = |x: usize, y: usize| {
            let (dx, dy) = ((x as f64 - gx).abs(), (y as f64 - gy).abs());
            (dx.max(dy) + (std::f64::consts::SQRT_2 - 1.0) * dx.min(dy)) * grid.g
        };
        let mut heap = BinaryHeap::new();
        for &s in sources {
            if let Some(k) = local(s) {
                cost[k] = 0.0;
                let (_, x, y) = unpack(s);
                heap.push(Node { f: heur(x, y), i: s });
            }
        }
        let mut hit = None;
        while let Some(Node { i, .. }) = heap.pop() {
            let k = local(i).unwrap();
            if goal[k] {
                hit = Some(i);
                break;
            }
            let (l, x, y) = unpack(i);
            let here = cost[k] as f64;
            for (d, (dx, dy)) in DIRS.iter().enumerate() {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx < wx0 as i64 || ny < wy0 as i64 || nx > wx1 as i64 || ny > wy1 as i64 {
                    continue;
                }
                let (nx, ny) = (nx as usize, ny as usize);
                let j = grid.idx(l, nx, ny);
                let (ok, clash) = grid.ok(j, net, false, soft);
                if !ok {
                    continue;
                }
                if dx.abs() + dy.abs() == 2
                    && (!grid.ok(grid.idx(l, nx, y), net, false, soft).0
                        || !grid.ok(grid.idx(l, x, ny), net, false, soft).0)
                {
                    continue;
                }
                let step = if dx.abs() + dy.abs() == 2 { std::f64::consts::SQRT_2 } else { 1.0 };
                let turn = if dir[k] != 8 && dir[k] as usize != d { 0.5 } else { 0.0 };
                let pull = if attract.is_some_and(|a| a.contains(&j)) { 0.5 } else { 1.0 };
                let push = if repel.is_some_and(|r| r.contains(&j)) { 8.0 } else { 0.0 };
                let c = here
                    + (step * pull + turn + push) * grid.g
                    + grid.hist[j] as f64
                    + if clash { penalty } else { 0.0 };
                let kj = local(j).unwrap();
                if (c as f32) < cost[kj] {
                    cost[kj] = c as f32;
                    from[kj] = i;
                    dir[kj] = d as u8;
                    heap.push(Node { f: c + heur(nx, ny), i: j });
                }
            }
            let via_checks: Vec<(bool, bool)> =
                via_layers.iter().map(|&vl| grid.ok(grid.idx(vl, x, y), net, true, soft)).collect();
            if via_checks.iter().all(|c| c.0) {
                let via_clash = via_checks.iter().any(|c| c.1);
                for &nl in routing.iter().filter(|&&nl| nl != l && via_layers.contains(&nl)) {
                    let j = grid.idx(nl, x, y);
                    let (ok, clash) = grid.ok(j, net, false, soft);
                    if !ok {
                        continue;
                    }
                    let kj = local(j).unwrap();
                    let c = here + opts.via_cost + if clash || via_clash { penalty } else { 0.0 };
                    if (c as f32) < cost[kj] {
                        cost[kj] = c as f32;
                        from[kj] = i;
                        dir[kj] = 8;
                        heap.push(Node { f: c + heur(x, y), i: j });
                    }
                }
            }
        }
        if let Some(end) = hit {
            let mut path = vec![end];
            let mut cur = end;
            while let Some(k) = local(cur) {
                let p = from[k];
                if p == usize::MAX {
                    break;
                }
                path.push(p);
                cur = p;
            }
            path.reverse();
            return Ok(path);
        }
        let whole = wx0 == 0 && wy0 == 0 && wx1 == grid.w - 1 && wy1 == grid.h - 1;
        if whole || (!soft && margin > opts.margin) {
            return Err("no path within the rules".into());
        }
        margin *= 3.0;
    }
}

type Geometry = (Vec<(usize, Vec<P>)>, Vec<P>);

fn geometry(grid: &Grid, path: &[usize], a: P, b: P, net: usize) -> Geometry {
    let plane = grid.w * grid.h;
    let unpack = |i: usize| (i / plane, (i % plane) % grid.w, (i % plane) / grid.w);
    let mut tracks: Vec<(usize, Vec<P>)> = Vec::new();
    let mut vias = Vec::new();
    let near = |i: usize, q: P| {
        let (_, x, y) = unpack(i);
        geom::dist(grid.center(x, y), q) < 0.35
    };
    let mut run: Vec<P> = if near(path[0], a) { vec![a] } else { Vec::new() };
    let mut layer = unpack(path[0]).0;
    for &i in path {
        let (l, x, y) = unpack(i);
        let p = grid.center(x, y);
        if l != layer {
            run.push(p);
            tracks.push((layer, std::mem::take(&mut run)));
            vias.push(p);
            layer = l;
        }
        run.push(p);
    }
    if near(*path.last().unwrap(), b) {
        run.push(b);
    }
    tracks.push((layer, run));
    let tracks = tracks
        .into_iter()
        .map(|(l, pts)| (l, simplify(&pull_tight(grid, l, &simplify(&pts), net))))
        .filter(|(_, pts)| pts.len() >= 2)
        .collect();
    (tracks, vias)
}

fn clear_line(grid: &Grid, l: usize, p: P, q: P, net: usize) -> bool {
    let n = (geom::dist(p, q) / (grid.g * 0.25)).ceil().max(1.0) as usize;
    (0..=n).all(|k| {
        let t = k as f64 / n as f64;
        let (x, y) = grid.cell([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
        x >= 0
            && y >= 0
            && (x as usize) < grid.w
            && (y as usize) < grid.h
            && grid.ok(grid.idx(l, x as usize, y as usize), net, false, false).0
    })
}

fn pull_tight(grid: &Grid, l: usize, pts: &[P], net: usize) -> Vec<P> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    let mut i = 0;
    while i < pts.len() - 1 {
        let mut j = (i + 60).min(pts.len() - 1);
        while j > i + 1 && !clear_line(grid, l, pts[i], pts[j], net) {
            j -= 1;
        }
        out.push(pts[j]);
        i = j;
    }
    out
}

fn simplify(pts: &[P]) -> Vec<P> {
    let mut v: Vec<P> = Vec::new();
    for &p in pts {
        if v.last().is_some_and(|q| geom::dist(*q, p) < 1e-9) {
            continue;
        }
        v.push(p);
    }
    let mut out: Vec<P> = Vec::new();
    for &p in &v {
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let cross = (b[0] - a[0]) * (p[1] - b[1]) - (b[1] - a[1]) * (p[0] - b[0]);
            let dot = (b[0] - a[0]) * (p[0] - b[0]) + (b[1] - a[1]) * (p[1] - b[1]);
            if cross.abs() < 1e-9 && dot > 0.0 {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

fn route_pair(
    grid: &mut Grid,
    obstacles: &[Obstacle],
    ctx: &Ctx,
    gap: f64,
    (ap, bp, p): (P, P, usize),
    (an, bn, n): (P, P, usize),
) -> Option<(Conn, Conn)> {
    let mid = |a: P, b: P| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let (ma, mb) = (mid(ap, an), mid(bp, bn));
    if geom::dist(ma, mb) < 1e-9 {
        return None;
    }
    for &l in ctx.routing {
        let pitch = ctx.widths[l] + gap;
        let Some(cells) = search_center(grid, l, p, n, ma, mb, pitch / 2.0, ctx) else {
            if std::env::var("AGENTEE_ROUTE_DEBUG").is_ok() {
                eprintln!("pair {p}/{n}: no centreline on layer {l}");
            }
            continue;
        };
        let pts = simplify(&cells.iter().map(|&(x, y)| grid.center(x, y)).collect::<Vec<_>>());
        if pts.len() < 2 {
            continue;
        }
        let left = offset(&pts, pitch / 2.0);
        let right = offset(&pts, -pitch / 2.0);
        let straight = geom::dist(left[0], ap) + geom::dist(right[0], an);
        let swapped = geom::dist(left[0], an) + geom::dist(right[0], ap);
        let (pp, np) = if straight <= swapped { (left, right) } else { (right, left) };
        let pc = Conn { net: p, tracks: vec![(l, pp.clone())], vias: Vec::new() };
        let nc = Conn { net: n, tracks: vec![(l, np.clone())], vias: Vec::new() };
        grid.mark_routed(&pc, ctx);
        grid.mark_routed(&nc, ctx);
        let mut parts: Vec<Conn> = Vec::new();
        let ends = [(p, ap, pp[0]), (n, an, np[0]), (p, bp, *pp.last()?), (n, bn, *np.last()?)];
        let mut ok = true;
        for (net, pad, end) in ends {
            let sources = anchors(grid, obstacles, net, pad, ctx.routing);
            let (x, y) = grid.cell(end);
            let goals: Vec<usize> = (-1..=1)
                .flat_map(|dy| (-1..=1).map(move |dx| (x + dx, y + dy)))
                .filter(|&(x, y)| {
                    x >= 0 && y >= 0 && (x as usize) < grid.w && (y as usize) < grid.h
                })
                .map(|(x, y)| grid.idx(l, x as usize, y as usize))
                .collect();
            let partner = if net == p { n } else { p };
            let geo: Vec<Conn> = parts
                .iter()
                .filter(|c| c.net == partner)
                .cloned()
                .chain([if partner == p { pc.clone() } else { nc.clone() }])
                .collect();
            let refs: Vec<&Conn> = geo.iter().collect();
            let repel = band(grid, &refs, gap, ctx.widths.as_slice());
            match search_between(
                grid,
                net,
                pad,
                end,
                &sources,
                &goals,
                ctx,
                false,
                None,
                Some(&repel),
            ) {
                Ok(path) => {
                    let c = conn_of(grid, &path, pad, end, net);
                    grid.mark_routed(&c, ctx);
                    parts.push(c);
                }
                Err(e) => {
                    if std::env::var("AGENTEE_ROUTE_DEBUG").is_ok() {
                        eprintln!("pair {p}/{n}: breakout of {net} from {pad:?} to {end:?}: {e}");
                    }
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            return None;
        }
        let join = |net: usize, main: Vec<P>, a: &Conn, b: &Conn| Conn {
            net,
            tracks: a.tracks.iter().chain(b.tracks.iter()).cloned().chain([(l, main)]).collect(),
            vias: a.vias.iter().chain(b.vias.iter()).cloned().collect(),
        };
        return Some((join(p, pp, &parts[0], &parts[2]), join(n, np, &parts[1], &parts[3])));
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn search_center(
    grid: &Grid,
    l: usize,
    p: usize,
    n: usize,
    ma: P,
    mb: P,
    half: f64,
    ctx: &Ctx,
) -> Option<Vec<(usize, usize)>> {
    let r = (half / grid.g).ceil() as i64;
    let disc: Vec<(i64, i64)> = (-r..=r)
        .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
        .filter(|&(dx, dy)| ((dx * dx + dy * dy) as f64).sqrt() * grid.g <= half + 1e-9)
        .collect();
    let okv = |v: u16| v == FREE || v == p as u16 + 1 || v == n as u16 + 1;
    let near_end = |x: i64, y: i64| {
        let q = grid.center(x as usize, y as usize);
        geom::dist(q, ma) < 1.2 || geom::dist(q, mb) < 1.2
    };
    let legal = |x: i64, y: i64| {
        let own = near_end(x, y);
        disc.iter().all(|&(dx, dy)| {
            let (cx, cy) = (x + dx, y + dy);
            if cx < 0 || cy < 0 || cx as usize >= grid.w || cy as usize >= grid.h {
                return false;
            }
            let i = grid.idx(l, cx as usize, cy as usize);
            let fixed = grid.track[i];
            (fixed == FREE || (own && okv(fixed))) && okv(grid.rt[i])
        })
    };
    let reach = 3.0;
    let margin = ctx.opts.margin + reach;
    let lo = grid.cell([ma[0].min(mb[0]) - margin, ma[1].min(mb[1]) - margin]);
    let hi = grid.cell([ma[0].max(mb[0]) + margin, ma[1].max(mb[1]) + margin]);
    let (x0, y0) = (lo.0.max(0), lo.1.max(0));
    let (x1, y1) = (hi.0.min(grid.w as i64 - 1), hi.1.min(grid.h as i64 - 1));
    let (ww, wh) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
    let loc = |x: i64, y: i64| (y - y0) as usize * ww + (x - x0) as usize;
    let mut cost = vec![f32::MAX; ww * wh];
    let mut from = vec![usize::MAX; ww * wh];
    let mut dir = vec![8u8; ww * wh];
    let heur = |x: i64, y: i64| {
        let q = grid.center(x as usize, y as usize);
        let (dx, dy) = ((q[0] - mb[0]).abs(), (q[1] - mb[1]).abs());
        dx.max(dy) + (std::f64::consts::SQRT_2 - 1.0) * dx.min(dy)
    };
    let mut heap = BinaryHeap::new();
    let (sx0, sy0) = grid.cell([ma[0] - reach, ma[1] - reach]);
    let (sx1, sy1) = grid.cell([ma[0] + reach, ma[1] + reach]);
    for y in sy0.max(y0)..=sy1.min(y1) {
        for x in sx0.max(x0)..=sx1.min(x1) {
            let d = geom::dist(grid.center(x as usize, y as usize), ma);
            if d <= reach && legal(x, y) {
                let k = loc(x, y);
                let c0 = (d - 1.5).abs() * 2.0;
                cost[k] = c0 as f32;
                heap.push(Node { f: c0 + heur(x, y), i: k });
            }
        }
    }
    let goal = |x: i64, y: i64| geom::dist(grid.center(x as usize, y as usize), mb) <= reach;
    let mut hit = None;
    while let Some(Node { i: k, .. }) = heap.pop() {
        let (x, y) = ((k % ww) as i64 + x0, (k / ww) as i64 + y0);
        if goal(x, y) {
            hit = Some(k);
            break;
        }
        let here = cost[k] as f64;
        for (d, (dx, dy)) in DIRS.iter().enumerate() {
            let (nx, ny) = (x + dx, y + dy);
            if nx < x0 || ny < y0 || nx > x1 || ny > y1 || !legal(nx, ny) {
                continue;
            }
            if dx.abs() + dy.abs() == 2 && (!legal(nx, y) || !legal(x, ny)) {
                continue;
            }
            let step = if dx.abs() + dy.abs() == 2 { std::f64::consts::SQRT_2 } else { 1.0 };
            let turn = if dir[k] != 8 && dir[k] as usize != d { 2.0 } else { 0.0 };
            let j = loc(nx, ny);
            let c = here + (step + turn) * grid.g;
            if (c as f32) < cost[j] {
                cost[j] = c as f32;
                from[j] = k;
                dir[j] = d as u8;
                let g = geom::dist(grid.center(nx as usize, ny as usize), mb);
                let finish = if g <= reach { (g - 1.5).abs() * 2.0 } else { 0.0 };
                heap.push(Node { f: c + heur(nx, ny) + finish, i: j });
            }
        }
    }
    let mut k = hit?;
    let mut out = vec![((k % ww) + x0 as usize, (k / ww) + y0 as usize)];
    while from[k] != usize::MAX {
        k = from[k];
        out.push(((k % ww) + x0 as usize, (k / ww) + y0 as usize));
    }
    out.reverse();
    Some(out)
}

fn offset(pts: &[P], d: f64) -> Vec<P> {
    let normal = |a: P, b: P| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        [-dy / len, dx / len]
    };
    let n = pts.len();
    (0..n)
        .map(|i| {
            let m = if i == 0 {
                normal(pts[0], pts[1])
            } else if i == n - 1 {
                normal(pts[n - 2], pts[n - 1])
            } else {
                let (a, b) = (normal(pts[i - 1], pts[i]), normal(pts[i], pts[i + 1]));
                let k = 1.0 + a[0] * b[0] + a[1] * b[1];
                [(a[0] + b[0]) / k, (a[1] + b[1]) / k]
            };
            [pts[i][0] + m[0] * d, pts[i][1] + m[1] * d]
        })
        .collect()
}

fn band(grid: &Grid, geo: &[&Conn], gap: f64, widths: &[f64]) -> std::collections::HashSet<usize> {
    let mut out = std::collections::HashSet::new();
    for c in geo {
        for (l, pts) in &c.tracks {
            let pitch = widths[*l] + gap;
            let (lo, hi) = (pitch + 0.1 * gap + 0.01, pitch + 2.5 * gap);
            for s in pts.windows(2) {
                let shape = Shape::Seg(s[0], s[1], 0.0);
                for (x, y) in grid.cells_near(&shape, hi) {
                    let d = shape.dist(grid.center(x, y));
                    if d >= lo {
                        out.insert(grid.idx(*l, x, y));
                    }
                }
            }
        }
    }
    out
}
