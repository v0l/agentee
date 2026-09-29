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
    fn clear_routed(&mut self) {
        self.rt.iter_mut().for_each(|v| *v = FREE);
        self.rv.iter_mut().for_each(|v| *v = FREE);
    }

    fn mark_routed(&mut self, c: &Conn, ctx: &Ctx) {
        let value = c.net as u16 + 1;
        let slack = self.g * 0.25;
        let hw = ctx.width / 2.0;
        let reach_t = hw + ctx.clearance + slack;
        let reach_v = ctx.via_r + ctx.clearance + slack;
        let mut shapes: Vec<(Vec<usize>, Shape)> = Vec::new();
        for (l, pts) in &c.tracks {
            for s in pts.windows(2) {
                shapes.push((vec![*l], Shape::Seg(s[0], s[1], hw)));
            }
        }
        for v in &c.vias {
            shapes.push((ctx.via_layers.to_vec(), Shape::Circle(*v, ctx.via_r)));
        }
        for (layers, shape) in shapes {
            for (reach, is_via) in [(reach_t, false), (reach_v, true)] {
                for (x, y) in self.cells_near(&shape, reach) {
                    for &l in &layers {
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

    fn add(&mut self, o: &Obstacle, half_width: f64, via_radius: f64, clearance: f64) {
        let value = o.net.map(|n| n as u16 + 1).unwrap_or(BLOCK);
        let c = clearance.max(o.clearance);
        let slack = self.g * 0.25;
        for (reach, is_via) in [(half_width + c + slack, false), (via_radius + c + slack, true)] {
            for (x, y) in self.cells_near(&o.shape, reach) {
                for &l in &o.layers {
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
        let clearance = layout.nets[nets[0]].clearance;
        let pitch = nc.and_then(|c| c.diff_gap).map(|g| g.to_mm() + width);
        let via_r = spec.diameter.to_mm() / 2.0;
        let via_layers: Vec<usize> = {
            let a = layer_of(&spec.from).unwrap_or(0);
            let b = layer_of(&spec.to).unwrap_or(copper.len() - 1);
            (a.min(b)..=a.max(b)).collect()
        };
        let ctx = Ctx { width, clearance, via_r, via_layers: &via_layers, routing: &routing, opts };

        let mut grid = build_grid(layout, opts.grid, copper.len(), width / 2.0, via_r, edge);
        for o in &obstacles {
            grid.add(o, width / 2.0, via_r, clearance);
        }

        let conns: Vec<(P, P, usize)> =
            layout.ratsnest.iter().filter(|(_, _, n)| nets.contains(n)).cloned().collect();
        let crowd: Vec<usize> = conns
            .iter()
            .map(|c| grid.crowd(c.2, c.0, &routing) + grid.crowd(c.2, c.1, &routing))
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
        let mut ripped = vec![0usize; conns.len()];
        let mut queue: std::collections::VecDeque<usize> = (0..conns.len()).collect();
        let mut failed: Vec<(usize, String)> = Vec::new();
        let mut budget = conns.len() * 20 + 50;
        while let Some(ci) = queue.pop_front() {
            let (a, b, net) = conns[ci];
            let attract = pitch.and_then(|pitch| {
                let other = partner(net)?;
                let geo: Vec<&Conn> = routed.iter().flatten().filter(|c| c.net == other).collect();
                (!geo.is_empty()).then(|| attraction(&grid, &geo, pitch, width))
            });
            let hard = search(&grid, &obstacles, net, a, b, &ctx, false, attract.as_ref());
            let path = match hard {
                Ok(p) => p,
                Err(reason) if budget == 0 || ripped[ci] > 12 => {
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
                            let hits: Vec<usize> = (0..routed.len())
                                .filter(|&k| {
                                    routed[k]
                                        .as_ref()
                                        .is_some_and(|r| r.net != net && conflicts(r, &conn, &ctx))
                                })
                                .collect();
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
                        shape: Shape::Seg(w[0], w[1], width / 2.0),
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
    width: f64,
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
    let need = ctx.width + ctx.clearance - 1e-6;
    let via_need = ctx.via_r + ctx.width / 2.0 + ctx.clearance - 1e-6;
    let vv_need = 2.0 * ctx.via_r + ctx.clearance - 1e-6;
    for (la, pa) in &r.tracks {
        for (lb, pb) in &c.tracks {
            if la != lb {
                continue;
            }
            for s in pa.windows(2) {
                for t in pb.windows(2) {
                    if geom::segment_segment_distance(s[0], s[1], t[0], t[1]) < need {
                        return true;
                    }
                }
            }
        }
        for v in &c.vias {
            if ctx.via_layers.contains(la)
                && pa.windows(2).any(|s| geom::point_segment_distance(*v, s[0], s[1]) < via_need)
            {
                return true;
            }
        }
    }
    for v in &r.vias {
        for (lb, pb) in &c.tracks {
            if ctx.via_layers.contains(lb)
                && pb.windows(2).any(|s| geom::point_segment_distance(*v, s[0], s[1]) < via_need)
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
    pitch: f64,
    width: f64,
) -> std::collections::HashSet<usize> {
    let mut out = std::collections::HashSet::new();
    let band = grid.g * 0.75;
    for c in geo {
        for (l, pts) in &c.tracks {
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

fn build_grid(
    layout: &Layout,
    g: f64,
    layers: usize,
    half_width: f64,
    via_r: f64,
    edge: f64,
) -> Grid {
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
        for (reach, via) in [(edge + half_width, false), (edge + via_r, true)] {
            for (x, y) in grid.cells_near(&seg, reach) {
                for l in 0..layers {
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
    let mut out = Vec::new();
    for o in obstacles.iter().filter(|o| o.net == Some(net) && o.shape.dist(p) < 1e-6) {
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
    let (routing, via_layers, opts) = (ctx.routing, ctx.via_layers, ctx.opts);
    let penalty = 20.0 * grid.g;
    let sources = anchors(grid, obstacles, net, a, routing);
    let goals = anchors(grid, obstacles, net, b, routing);
    if sources.is_empty() || goals.is_empty() {
        return Err("an end has no copper on the routing layers".into());
    }
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
        for &gi in &goals {
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
        for &s in &sources {
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
                let c = here
                    + (step * pull + turn) * grid.g
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
    let mut run: Vec<P> = vec![a];
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
    run.push(b);
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
