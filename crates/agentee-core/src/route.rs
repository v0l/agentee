use crate::board::Board;
use crate::footprint::PadKind;
use crate::geom::{self, P};
use crate::layout::{Layout, NECKDOWN, glob};
use crate::units::Length;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub struct RouteOptions {
    pub nets: Vec<String>,
    pub layers: Vec<String>,
    pub grid: f64,
    pub via: Vec<String>,
    pub via_cost: f64,
    pub bend_cost: f64,
    pub margin: f64,
    pub pairs: bool,
    pub via_in_pad: bool,
}

impl Default for RouteOptions {
    fn default() -> Self {
        RouteOptions {
            nets: Vec::new(),
            layers: Vec::new(),
            grid: 0.05,
            via: Vec::new(),
            via_cost: 3.0,
            bend_cost: 0.1,
            margin: 5.0,
            pairs: false,
            via_in_pad: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RoutedTrack {
    pub net: String,
    pub layer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
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

    fn seg_dist(&self, p: P, q: P) -> f64 {
        match self {
            Shape::Poly(v) => geom::polyline_polygon_distance(&[p, q], v),
            Shape::Seg(a, b, r) => (geom::segment_segment_distance(p, q, *a, *b) - r).max(0.0),
            Shape::Circle(c, r) => (geom::point_segment_distance(*c, p, q) - r).max(0.0),
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

struct SmdPad {
    layers: Vec<usize>,
    outlines: Vec<Vec<P>>,
    lo: P,
    hi: P,
}

impl SmdPad {
    fn clears(&self, at: P, ctx: &Ctx) -> bool {
        let reach = ctx.via_r.max(ctx.drill_r + ctx.hole_smd) + 1e-3;
        if at[0] < self.lo[0] - reach
            || at[1] < self.lo[1] - reach
            || at[0] > self.hi[0] + reach
            || at[1] > self.hi[1] + reach
            || !self.layers.iter().any(|l| ctx.via_layers.contains(l))
        {
            return true;
        }
        if self.outlines.iter().any(|o| pad_gap(o, at) <= -(ctx.via_r + 1e-3)) {
            return ctx.in_pad;
        }
        self.outlines.iter().all(|o| pad_gap(o, at) >= reach)
    }
}

fn via_clears_smd(at: P, ctx: &Ctx) -> bool {
    ctx.smd.iter().all(|p| p.clears(at, ctx))
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

    fn block_smd(&mut self, ctx: &Ctx) {
        let reach = ctx.via_r.max(ctx.drill_r + ctx.hole_smd) + 1e-3;
        let layers = self.via.len() / (self.w * self.h);
        for pad in ctx.smd {
            if !pad.layers.iter().any(|l| ctx.via_layers.contains(l)) {
                continue;
            }
            for o in &pad.outlines {
                for (x, y) in self.cells_near(&Shape::Poly(o.clone()), reach) {
                    if !pad.clears(self.center(x, y), ctx) {
                        for l in 0..layers {
                            let i = self.idx(l, x, y);
                            self.via[i] = BLOCK;
                        }
                    }
                }
            }
        }
    }

    fn block_silk(&mut self, silk: &[crate::layout::SilkBox], via_r: f64) {
        let layers = self.via.len() / (self.w * self.h);
        let reach = via_r + self.g / 2.0;
        for b in silk.iter().filter(|b| b.outline.len() >= 3) {
            for (x, y) in self.cells_near(&Shape::Poly(b.outline.clone()), reach) {
                for l in 0..layers {
                    let i = self.idx(l, x, y);
                    self.via[i] = BLOCK;
                }
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
        for n in &c.necks {
            shapes.push((vec![n.layer], Shape::Seg(n.from, n.to, n.width / 2.0)));
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
        let keep = ctx.via_keep();
        for (layers, shape) in shapes {
            let c = if matches!(shape, Shape::Circle(..)) { keep } else { ctx.clearance };
            for is_via in [false, true] {
                let reach = |l: usize| {
                    (if is_via { ctx.via_r + keep } else { ctx.widths[l] / 2.0 + c }) + slack
                };
                let most = if is_via { ctx.via_r + keep } else { widest / 2.0 + c } + slack;
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

    fn add(
        &mut self,
        o: &Obstacle,
        half_widths: &[f64],
        via_radius: f64,
        clearance: f64,
        hole_reach: f64,
    ) {
        let value = o.net.map(|n| n as u16 + 1).unwrap_or(BLOCK);
        let c = clearance.max(o.clearance);
        let slack = self.g * 0.6;
        let widest = o.layers.iter().map(|&l| half_widths[l]).fold(0.0, f64::max);
        let via_reach = (via_radius + c).max(hole_reach) + slack;
        for is_via in [false, true] {
            let most = if is_via { via_reach } else { widest + c + slack };
            for (x, y) in self.cells_near(&o.shape, most) {
                let d = o.shape.dist(self.center(x, y));
                for &l in o
                    .layers
                    .iter()
                    .filter(|&&l| d <= if is_via { via_reach } else { half_widths[l] + c + slack })
                {
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

    let hole_cu = board.rules.min_via_hole_to_copper.to_mm();
    let mut obstacles = Vec::new();
    let mut smd = Vec::new();
    for part in &layout.parts {
        for pad in &part.pads {
            let layers: Vec<usize> = pad.copper.iter().filter_map(|c| layer_of(c)).collect();
            if pad.drill.is_none() && !layers.is_empty() {
                let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
                for q in pad.outlines.iter().flatten() {
                    lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                    hi = [hi[0].max(q[0]), hi[1].max(q[1])];
                }
                smd.push(SmdPad { layers: layers.clone(), outlines: pad.outlines.clone(), lo, hi });
            }
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
            clearance: layout.nets[v.net].clearance.max(v.drill / 2.0 + hole_cu - v.diameter / 2.0),
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

    let mut lanes: Vec<(Vec<usize>, usize, String)> = Vec::new();
    for iface in &layout.interfaces {
        let Some(m) = iface.spec.max_vias else { continue };
        for lane in &iface.lanes {
            let nets: Vec<usize> = lane
                .nets
                .iter()
                .filter_map(|n| layout.nets.iter().position(|x| &x.name == n))
                .collect();
            lanes.push((nets, m as usize, iface.name.clone()));
        }
    }

    let mut out = RouteResult::default();
    let edge = board.rules.min_copper_to_edge.to_mm();
    for (class, nets) in by_class {
        let nc = board.netclasses.iter().find(|c| c.name == class);
        if let Some(n) = opts.via.iter().find(|n| !board.vias.iter().any(|v| &v.name == *n)) {
            return Err(format!("the board has no via `{n}`"));
        }
        let names: Vec<String> = if opts.via.is_empty() {
            nc.map(|c| c.via.clone()).unwrap_or_default()
        } else {
            opts.via.clone()
        };
        let mut specs: Vec<&crate::board::Via> =
            names.iter().filter_map(|n| board.vias.iter().find(|v| &v.name == n)).collect();
        if specs.is_empty() {
            specs.extend(board.vias.first());
        }
        if specs.is_empty() {
            return Err("the board defines no [[vias]]".into());
        }
        let options: Vec<ViaOption> = specs
            .iter()
            .map(|s| ViaOption {
                name: s.name.clone(),
                layers: s.copper_layers(copper).iter().filter_map(|c| layer_of(c)).collect(),
                via_r: s.diameter.to_mm() / 2.0,
                drill_r: s.drill.to_mm() / 2.0,
                cost: s.cost,
            })
            .collect();
        let width = layout.nets[nets[0]].width;
        let widths: Vec<f64> =
            copper.iter().map(|l| nc.map(|c| c.width_on(l).to_mm()).unwrap_or(width)).collect();
        let halves: Vec<f64> = widths.iter().map(|w| w / 2.0).collect();
        let clearance = layout.nets[nets[0]].clearance;
        let gap = nc.and_then(|c| c.diff_gap).map(|g| g.to_mm());
        let via_r = options.iter().map(|o| o.via_r).fold(0.0, f64::max);
        let drill_r = options.iter().map(|o| o.drill_r).fold(0.0, f64::max);
        let mut via_layers: Vec<usize> = options.iter().flat_map(|o| o.layers.clone()).collect();
        via_layers.sort_unstable();
        via_layers.dedup();
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
        let necking = Necking {
            length: nc.and_then(|c| c.neckdown).map(Length::to_mm).unwrap_or(NECKDOWN),
            min_width: board.rules.min_track_width.to_mm(),
            edge,
            board: layout.edge(),
        };
        let ctx = Ctx {
            drill_r,
            hole_gap: board.rules.min_hole_to_hole.to_mm(),
            hole_cu,
            hole_smd: board.rules.min_hole_to_smd_pad.to_mm(),
            in_pad: opts.via_in_pad
                && 2.0 * drill_r <= board.rules.max_filled_via_drill.to_mm() + 1e-6,
            smd: &smd,
            widths: widths.clone(),
            clearance,
            via_r,
            via_layers: &via_layers,
            vias: &options,
            stack_vias: board.rules.stacked_microvias,
            routing,
            opts,
            necking: Some(&necking),
        };

        let mut grid = build_grid(layout, opts.grid, &halves, via_r, edge);
        for o in &obstacles {
            grid.add(o, &halves, via_r, clearance, ctx.drill_r + hole_cu);
        }
        for &(c, r) in &drills {
            grid.add_drill(c, r + ctx.drill_r + ctx.hole_gap);
        }
        grid.block_smd(&ctx);
        grid.block_silk(&layout.silk, via_r);

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
        let spent: Vec<usize> = lanes
            .iter()
            .map(|(ns, _, _)| {
                layout.vias.iter().filter(|v| ns.contains(&v.net)).count()
                    + out
                        .vias
                        .iter()
                        .filter(|v| ns.iter().any(|&n| layout.nets[n].name == v.net))
                        .count()
            })
            .collect();
        let room = |net: usize, routed: &[Option<Conn>], skip: &[usize]| -> Option<(usize, &str)> {
            lanes
                .iter()
                .zip(&spent)
                .filter(|((ns, _, _), _)| ns.contains(&net))
                .map(|((ns, m, name), used)| {
                    let fresh: usize = routed
                        .iter()
                        .enumerate()
                        .filter(|(k, _)| !skip.contains(k))
                        .filter_map(|(_, r)| r.as_ref())
                        .filter(|r| ns.contains(&r.net))
                        .map(|r| r.vias.len())
                        .sum();
                    (m.saturating_sub(used + fresh), name.as_str())
                })
                .min()
        };
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
                    Some((pc, nc))
                        if [(pc.net, pc.vias.len()), (nc.net, nc.vias.len())].iter().all(
                            |&(n, v)| room(n, &routed, &[]).is_none_or(|(left, _)| v <= left),
                        ) =>
                    {
                        routed[cp] = Some(pc);
                        routed[cn] = Some(nc);
                        locked[cp] = true;
                        locked[cn] = true;
                    }
                    _ => {
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
            let fresh: Vec<Obstacle> = routed
                .iter()
                .flatten()
                .filter(|c| c.net == net)
                .flat_map(|c| copper_of(c, &ctx))
                .collect();
            let hard = search(&grid, &obstacles, &fresh, net, a, b, &ctx, false, attract.as_ref());
            let path = match hard {
                Ok(p) => p,
                Err(reason) if budget == 0 || ripped[ci] > 30 => {
                    failed.push((ci, reason));
                    continue;
                }
                Err(reason) => {
                    budget -= 1;
                    match search(&grid, &obstacles, &fresh, net, a, b, &ctx, true, attract.as_ref())
                    {
                        Ok(p) => {
                            for &i in &p.cells {
                                if grid.ok(i, net, false, true).1 {
                                    grid.hist[i] += (grid.g * 10.0) as f32;
                                }
                            }
                            let conn = conn_found(&grid, &p, net);
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
                            if !p.cells.iter().all(|&i| grid.ok(i, net, false, false).0) {
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
            let mut conn = conn_found(&grid, &path, net);
            if let Some((left, iface)) = room(net, &routed, &[ci])
                && conn.vias.len() > left
            {
                match fewer_vias(&grid, &obstacles, &fresh, net, a, b, &ctx, left) {
                    Some(c) => conn = c,
                    None => {
                        failed.push((
                            ci,
                            format!(
                                "needs {} vias, interface {iface} leaves room for {left}",
                                conn.vias.len()
                            ),
                        ));
                        continue;
                    }
                }
            }
            grid.mark_routed(&conn, &ctx);
            routed[ci] = Some(conn);
        }

        for ci in 0..conns.len() {
            let Some(old) = routed[ci].clone() else { continue };
            let others: Vec<&Conn> = routed
                .iter()
                .enumerate()
                .filter(|&(k, _)| k != ci)
                .filter_map(|(_, r)| r.as_ref())
                .collect();
            if locked[ci]
                || old.vias.is_empty()
                || others.iter().any(|r| r.net == old.net && joined(r, &old, &ctx))
            {
                continue;
            }
            grid.clear_routed();
            for r in &others {
                grid.mark_routed(r, &ctx);
            }
            let (a, b, net) = conns[ci];
            let fresh: Vec<Obstacle> =
                others.iter().filter(|c| c.net == net).flat_map(|c| copper_of(c, &ctx)).collect();
            if let Some(c) = one_layer(&grid, &obstacles, &fresh, net, a, b, &ctx)
                && track_length(&c) <= track_length(&old) * 1.25 + 1.0
            {
                routed[ci] = Some(c);
            }
        }

        grid.clear_routed();
        snap_vias(&grid, &obstacles, &mut routed, &locked, &ctx);

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
            let placed = placed_vias(&conn, &options);
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
                    width: None,
                    points: pts,
                });
            }
            for n in conn.necks {
                obstacles.push(Obstacle {
                    net: Some(conn.net),
                    layers: vec![n.layer],
                    shape: Shape::Seg(n.from, n.to, n.width / 2.0),
                    clearance,
                });
                out.tracks.push(RoutedTrack {
                    net: name.clone(),
                    layer: copper[n.layer].clone(),
                    width: Some(n.width),
                    points: vec![n.from, n.to],
                });
            }
            for (at, o) in placed {
                drills.push((at, o.drill_r));
                obstacles.push(Obstacle {
                    net: Some(conn.net),
                    layers: o.layers.clone(),
                    shape: Shape::Circle(at, o.via_r),
                    clearance: ctx.via_keep(),
                });
                out.vias.push(RoutedVia { net: name.clone(), at, via: o.name.clone() });
            }
        }
    }
    Ok(out)
}

struct ViaOption {
    name: String,
    layers: Vec<usize>,
    via_r: f64,
    drill_r: f64,
    cost: f64,
}

impl ViaOption {
    fn joins(&self, a: usize, b: usize) -> bool {
        self.layers.contains(&a) && self.layers.contains(&b)
    }
}

fn cheapest(options: &[ViaOption], a: usize, b: usize) -> Option<&ViaOption> {
    options
        .iter()
        .filter(|o| o.joins(a, b))
        .min_by(|x, y| x.cost.total_cmp(&y.cost).then(x.layers.len().cmp(&y.layers.len())))
}

fn placed_vias<'o>(c: &Conn, options: &'o [ViaOption]) -> Vec<(P, &'o ViaOption)> {
    let mut out: Vec<(P, &ViaOption, usize, usize)> = Vec::new();
    for (k, &at) in c.vias.iter().enumerate() {
        let (a, b) = c.hops.get(k).copied().unwrap_or((usize::MAX, usize::MAX));
        if let Some(last) = out.last_mut()
            && geom::dist(last.0, at) < 1e-6
            && let Some(o) = cheapest(options, last.2.min(a).min(b), last.3.max(a).max(b))
        {
            *last = (at, o, last.2.min(a).min(b), last.3.max(a).max(b));
            continue;
        }
        let o = cheapest(options, a, b).unwrap_or(&options[0]);
        out.push((at, o, a.min(b), a.max(b)));
    }
    out.into_iter().map(|(at, o, _, _)| (at, o)).collect()
}

struct Ctx<'a> {
    drill_r: f64,
    hole_gap: f64,
    hole_cu: f64,
    hole_smd: f64,
    in_pad: bool,
    smd: &'a [SmdPad],
    widths: Vec<f64>,
    clearance: f64,
    via_r: f64,
    via_layers: &'a [usize],
    vias: &'a [ViaOption],
    stack_vias: bool,
    routing: &'a [usize],
    opts: &'a RouteOptions,
    necking: Option<&'a Necking<'a>>,
}

struct Necking<'a> {
    length: f64,
    min_width: f64,
    edge: f64,
    board: geom::BoardEdge<'a>,
}

impl Ctx<'_> {
    fn via_keep(&self) -> f64 {
        self.clearance.max(self.drill_r + self.hole_cu - self.via_r)
    }
}

#[derive(Clone)]
struct Conn {
    net: usize,
    tracks: Vec<(usize, Vec<P>)>,
    vias: Vec<P>,
    hops: Vec<(usize, usize)>,
    necks: Vec<Neck>,
}

#[derive(Clone, Debug, PartialEq)]
struct Neck {
    layer: usize,
    from: P,
    to: P,
    width: f64,
}

struct Found {
    cells: Vec<usize>,
    a: P,
    b: P,
    necks: Vec<Neck>,
}

fn conn_of(grid: &Grid, path: &[usize], a: P, b: P, net: usize) -> Conn {
    let (tracks, vias) = geometry(grid, path, a, b, net);
    Conn { net, tracks, vias, hops: hops_of(grid, path), necks: Vec::new() }
}

fn conn_found(grid: &Grid, f: &Found, net: usize) -> Conn {
    Conn { necks: f.necks.clone(), ..conn_of(grid, &f.cells, f.a, f.b, net) }
}

fn wires(c: &Conn, ctx: &Ctx) -> Vec<(usize, P, P, f64)> {
    let mut out: Vec<(usize, P, P, f64)> = c
        .tracks
        .iter()
        .flat_map(|(l, pts)| pts.windows(2).map(|w| (*l, w[0], w[1], ctx.widths[*l])))
        .collect();
    out.extend(c.necks.iter().map(|n| (n.layer, n.from, n.to, n.width)));
    out
}

fn track_length(c: &Conn) -> f64 {
    c.tracks.iter().flat_map(|(_, p)| p.windows(2)).map(|w| geom::dist(w[0], w[1])).sum()
}

fn joined(a: &Conn, b: &Conn, ctx: &Ctx) -> bool {
    let (pa, pb) = (copper_of(a, ctx), copper_of(b, ctx));
    pa.iter().any(|x| {
        pb.iter()
            .any(|y| x.layers.iter().any(|l| y.layers.contains(l)) && touch(&x.shape, &y.shape))
    })
}

fn one_layer(
    grid: &Grid,
    obstacles: &[Obstacle],
    fresh: &[Obstacle],
    net: usize,
    a: P,
    b: P,
    ctx: &Ctx,
) -> Option<Conn> {
    let mut best: Option<Conn> = None;
    for &l in ctx.routing {
        let layer = [l];
        let one = Ctx { widths: ctx.widths.clone(), routing: &layer, ..*ctx };
        if let Ok(p) = search(grid, obstacles, fresh, net, a, b, &one, false, None) {
            let c = conn_found(grid, &p, net);
            if c.vias.is_empty() && best.as_ref().is_none_or(|x| track_length(&c) < track_length(x))
            {
                best = Some(c);
            }
        }
    }
    best
}

#[allow(clippy::too_many_arguments)]
fn fewer_vias(
    grid: &Grid,
    obstacles: &[Obstacle],
    fresh: &[Obstacle],
    net: usize,
    a: P,
    b: P,
    ctx: &Ctx,
    left: usize,
) -> Option<Conn> {
    if left > 0 {
        let dear = RouteOptions {
            nets: Vec::new(),
            layers: Vec::new(),
            via: Vec::new(),
            via_cost: ctx.opts.via_cost * 10.0 + 10.0,
            ..*ctx.opts
        };
        let wary = Ctx { widths: ctx.widths.clone(), opts: &dear, ..*ctx };
        if let Ok(p) = search(grid, obstacles, fresh, net, a, b, &wary, false, None) {
            let c = conn_found(grid, &p, net);
            if c.vias.len() <= left {
                return Some(c);
            }
        }
    }
    one_layer(grid, obstacles, fresh, net, a, b, ctx)
}

struct Slide {
    track: usize,
    path: Vec<P>,
}

impl Slide {
    fn cross(&self, axis: usize, target: f64) -> Option<(usize, P)> {
        self.path.windows(2).enumerate().find_map(|(k, w)| {
            let d = w[1][axis] - w[0][axis];
            if d.abs() < 1e-9 {
                return None;
            }
            let t = (target - w[0][axis]) / d;
            (t > 1e-9 && t <= 1.0 + 1e-9).then(|| {
                (k, [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t])
            })
        })
    }
}

fn slides(c: &Conn, vi: usize, ctx: &Ctx, reach: f64) -> Vec<Slide> {
    let v = c.vias[vi];
    let mut out = Vec::new();
    for (j, (_, pts)) in c.tracks.iter().enumerate() {
        let mut back: Vec<P> = if geom::dist(pts[0], v) < 1e-6 {
            pts.clone()
        } else if geom::dist(pts[pts.len() - 1], v) < 1e-6 {
            pts.iter().rev().cloned().collect()
        } else {
            continue;
        };
        let keep = 2.0 * ctx.via_r + ctx.clearance;
        let n = back.len();
        let Some(dir) = heading(back[n - 1], back[n - 2]) else { continue };
        let last = geom::dist(back[n - 2], back[n - 1]);
        if last <= keep {
            back.pop();
        } else {
            back[n - 1] = [back[n - 1][0] + dir[0] * keep, back[n - 1][1] + dir[1] * keep];
        }
        let mut path = vec![back[0]];
        let mut walked = 0.0;
        for w in back.windows(2) {
            let l = geom::dist(w[0], w[1]);
            if walked + l >= reach {
                let t = (reach - walked) / l;
                path.push([w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]);
                break;
            }
            walked += l;
            path.push(w[1]);
        }
        if path.len() >= 2 {
            out.push(Slide { track: j, path });
        }
    }
    out
}

fn despike(pts: &[P]) -> Vec<P> {
    let mut out: Vec<P> = Vec::new();
    for &p in &simplify(pts) {
        while out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let cross = (b[0] - a[0]) * (p[1] - b[1]) - (b[1] - a[1]) * (p[0] - b[0]);
            let dot = (b[0] - a[0]) * (p[0] - b[0]) + (b[1] - a[1]) * (p[1] - b[1]);
            if cross.abs() < 1e-9 && dot < 0.0 {
                out.pop();
            } else {
                break;
            }
        }
        if out.last().is_none_or(|q| geom::dist(*q, p) > 1e-9) {
            out.push(p);
        }
    }
    simplify(&out)
}

fn move_via(c: &Conn, vi: usize, slide: &Slide, k: usize, to: P) -> Conn {
    let v = c.vias[vi];
    let mut out = c.clone();
    for (j, (_, pts)) in out.tracks.iter_mut().enumerate() {
        let n = pts.len();
        let at_start = geom::dist(pts[0], v) < 1e-6;
        if !at_start && geom::dist(pts[n - 1], v) >= 1e-6 {
            continue;
        }
        if at_start {
            pts.reverse();
        }
        if j == slide.track {
            pts.truncate(n - 1 - k);
            pts.push(to);
        } else {
            pts.extend_from_slice(&slide.path[1..=k]);
            pts.push(to);
        }
        *pts = despike(pts);
        if at_start {
            pts.reverse();
        }
    }
    out.tracks.retain(|(_, pts)| pts.len() >= 2);
    out.vias[vi] = to;
    out
}

fn via_fits(
    grid: &Grid,
    c: &Conn,
    vi: usize,
    run: &[P],
    run_layers: &[usize],
    others: &[&Conn],
    ctx: &Ctx,
) -> bool {
    let to = c.vias[vi];
    let drills = 2.0 * ctx.drill_r + ctx.hole_gap - 1e-6;
    let (x, y) = grid.cell(to);
    if x < 0 || y < 0 || x as usize >= grid.w || y as usize >= grid.h {
        return false;
    }
    let cell_ok = ctx
        .via_layers
        .iter()
        .all(|&l| grid.ok(grid.idx(l, x as usize, y as usize), c.net, true, false).0);
    let own_ok = c.vias.iter().enumerate().all(|(k, w)| k == vi || geom::dist(*w, to) >= drills);
    let lines_ok =
        run_layers.iter().all(|&l| run.windows(2).all(|w| clear_line(grid, l, w[0], w[1], c.net)));
    cell_ok
        && via_clears_smd(to, ctx)
        && own_ok
        && lines_ok
        && others
            .iter()
            .all(|r| !conflicts(r, c, ctx) && r.vias.iter().all(|w| geom::dist(*w, to) >= drills))
}

fn pad_gap(pad: &[P], p: P) -> f64 {
    let edge = (0..pad.len())
        .map(|i| geom::point_segment_distance(p, pad[i], pad[(i + 1) % pad.len()]))
        .fold(f64::MAX, f64::min);
    if geom::point_in_polygon(p, pad) { -edge } else { edge }
}

fn snap_vias(
    grid: &Grid,
    obstacles: &[Obstacle],
    routed: &mut [Option<Conn>],
    locked: &[bool],
    ctx: &Ctx,
) {
    let reach = (10.0 * ctx.via_r).max(2.5);
    let pads: Vec<&Vec<P>> = obstacles
        .iter()
        .filter(|o| o.layers.iter().any(|l| ctx.via_layers.contains(l)))
        .filter_map(|o| match &o.shape {
            Shape::Poly(v) => Some(v),
            _ => None,
        })
        .collect();
    let clear_of_pads = |from: P, to: P| {
        pads.iter().all(|pad| {
            let after = pad_gap(pad, to);
            after >= ctx.via_r || after >= pad_gap(pad, from) - 1e-6
        })
    };
    let lone = |ci: usize, routed: &[Option<Conn>]| {
        let Some(c) = &routed[ci] else { return false };
        !locked[ci] && routed.iter().flatten().filter(|r| r.net == c.net).count() == 1
    };
    let mut items: Vec<(usize, usize, P, Vec<P>)> = Vec::new();
    for ci in 0..routed.len() {
        if !lone(ci, routed) {
            continue;
        }
        let c = routed[ci].as_ref().unwrap();
        for vi in 0..c.vias.len() {
            let dirs: Vec<P> = slides(c, vi, ctx, reach)
                .iter()
                .filter_map(|s| heading(s.path[0], s.path[1]))
                .collect();
            if !dirs.is_empty() {
                items.push((ci, vi, c.vias[vi], dirs));
            }
        }
    }
    let mut root: Vec<usize> = (0..items.len()).collect();
    fn find(root: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while root[i] != i {
            root[i] = root[root[i]];
            i = root[i];
        }
        i
    }
    for i in 0..items.len() {
        for j in i + 1..items.len() {
            let (a, b) = (&items[i], &items[j]);
            let parallel =
                a.3.iter().any(|u| b.3.iter().any(|v| (u[0] * v[0] + u[1] * v[1]).abs() > 0.99));
            if a.0 != b.0 && parallel && geom::dist(a.2, b.2) <= reach {
                let (ri, rj) = (find(&mut root, i), find(&mut root, j));
                root[ri] = rj;
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for i in 0..items.len() {
        let r = find(&mut root, i);
        match groups.iter_mut().find(|g| find(&mut root, g[0]) == r) {
            Some(g) => g.push(i),
            None => groups.push(vec![i]),
        }
    }
    for group in groups.into_iter().filter(|g| g.len() >= 2) {
        let mut best: Option<(usize, Vec<Option<Conn>>)> = None;
        for axis in 0..2 {
            let at = |m: usize, routed: &[Option<Conn>]| {
                routed[items[m].0].as_ref().map(|c| c.vias[items[m].1][axis]).unwrap_or(f64::NAN)
            };
            let lined = |c: f64, routed: &[Option<Conn>]| {
                group.iter().filter(|&&m| (at(m, routed) - c).abs() < 1e-6).count()
            };
            let now = group.iter().map(|&m| lined(at(m, routed), routed)).max().unwrap_or(0);
            let mut targets: Vec<f64> = group.iter().map(|&m| at(m, routed)).collect();
            let origin = if axis == 0 { grid.x0 } else { grid.y0 };
            let lo = targets.iter().cloned().fold(f64::MAX, f64::min);
            let hi = targets.iter().cloned().fold(f64::MIN, f64::max);
            let first = ((lo - origin) / grid.g - 0.5).ceil() as i64;
            let last = ((hi - origin) / grid.g - 0.5).floor() as i64;
            targets.extend((first..=last).map(|k| origin + (k as f64 + 0.5) * grid.g));
            targets.sort_by(|a, b| a.total_cmp(b));
            targets.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
            for target in targets {
                let mut trial: Vec<Option<Conn>> = routed.to_vec();
                let mut moves: Vec<(f64, usize)> = group
                    .iter()
                    .map(|&m| ((at(m, &trial) - target).abs(), m))
                    .filter(|(d, _)| *d > 1e-6)
                    .collect();
                moves.sort_by(|a, b| a.0.total_cmp(&b.0));
                for (_, m) in moves {
                    let (ci, vi) = (items[m].0, items[m].1);
                    let old = trial[ci].clone().unwrap();
                    let v = old.vias[vi];
                    let Some((slide, k, to)) =
                        slides(&old, vi, ctx, reach).into_iter().find_map(|s| {
                            let (k, to) = s.cross(axis, target)?;
                            Some((s, k, to))
                        })
                    else {
                        continue;
                    };
                    let moved = move_via(&old, vi, &slide, k, to);
                    let mut run = slide.path[..=k].to_vec();
                    run.push(to);
                    let run_layers: Vec<usize> = old
                        .tracks
                        .iter()
                        .enumerate()
                        .filter(|(j, (_, pts))| {
                            *j != slide.track
                                && (geom::dist(pts[0], v) < 1e-6
                                    || geom::dist(pts[pts.len() - 1], v) < 1e-6)
                        })
                        .map(|(_, (l, _))| *l)
                        .collect();
                    let others: Vec<&Conn> = trial
                        .iter()
                        .enumerate()
                        .filter(|&(k, _)| k != ci)
                        .filter_map(|(_, r)| r.as_ref())
                        .collect();
                    if track_length(&moved) <= track_length(&old) + 1e-6
                        && clear_of_pads(v, to)
                        && via_fits(grid, &moved, vi, &run, &run_layers, &others, ctx)
                    {
                        trial[ci] = Some(moved);
                    }
                }
                let got = lined(target, &trial);
                if got >= 2 && got > now && best.as_ref().is_none_or(|b| got > b.0) {
                    best = Some((got, trial));
                }
            }
        }
        if let Some((_, trial)) = best {
            routed.clone_from_slice(&trial);
        }
    }
}

fn copper_of(c: &Conn, ctx: &Ctx) -> Vec<Obstacle> {
    let mut out = Vec::new();
    for (l, a, b, w) in wires(c, ctx) {
        out.push(Obstacle {
            net: Some(c.net),
            layers: vec![l],
            shape: Shape::Seg(a, b, w / 2.0),
            clearance: ctx.clearance,
        });
    }
    for v in &c.vias {
        out.push(Obstacle {
            net: Some(c.net),
            layers: ctx.via_layers.to_vec(),
            shape: Shape::Circle(*v, ctx.via_r),
            clearance: ctx.via_keep(),
        });
    }
    out
}

fn conflicts(r: &Conn, c: &Conn, ctx: &Ctx) -> bool {
    let keep = ctx.via_keep();
    let via_need = |w: f64| ctx.via_r + w / 2.0 + keep - 1e-6;
    let vv_need = 2.0 * ctx.via_r + keep - 1e-6;
    let (rw, cw) = (wires(r, ctx), wires(c, ctx));
    for &(la, a0, a1, wa) in &rw {
        for &(lb, b0, b1, wb) in &cw {
            if la == lb
                && geom::segment_segment_distance(a0, a1, b0, b1)
                    < (wa + wb) / 2.0 + ctx.clearance - 1e-6
            {
                return true;
            }
        }
        if ctx.via_layers.contains(&la)
            && c.vias.iter().any(|v| geom::point_segment_distance(*v, a0, a1) < via_need(wa))
        {
            return true;
        }
    }
    for v in &r.vias {
        if cw.iter().any(|&(lb, b0, b1, wb)| {
            ctx.via_layers.contains(&lb) && geom::point_segment_distance(*v, b0, b1) < via_need(wb)
        }) {
            return true;
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
    let snap = |v: f64| ((v / g).floor() - 1.5) * g;
    let (x0, y0) = (snap(b.min[0]), snap(b.min[1]));
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
    let board = layout.edge();
    let edges: Vec<(P, P)> =
        if board.is_closed() { board.segments().collect() } else { Vec::new() };
    for y in 0..h {
        let cy = grid.center(0, y)[1];
        let mut xs: Vec<f64> = edges
            .iter()
            .filter(|(a, b)| (a[1] <= cy) != (b[1] <= cy))
            .map(|&(a, b)| a[0] + (cy - a[1]) / (b[1] - a[1]) * (b[0] - a[0]))
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
    for &(a, b) in &edges {
        let seg = Shape::Seg(a, b, 0.0);
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

fn anchors(
    grid: &Grid,
    obstacles: &[Obstacle],
    fresh: &[Obstacle],
    net: usize,
    p: P,
    routing: &[usize],
) -> Vec<usize> {
    let own: Vec<&Obstacle> =
        obstacles.iter().chain(fresh).filter(|o| o.net == Some(net)).collect();
    let mut seen = vec![false; own.len()];
    let at: Vec<usize> = (0..own.len()).filter(|&i| own[i].shape.dist(p) < 1e-6).collect();
    let pads: Vec<usize> =
        at.iter().copied().filter(|&i| matches!(own[i].shape, Shape::Poly(_))).collect();
    let mut stack = if pads.is_empty() { at } else { pads };
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
    fresh: &[Obstacle],
    net: usize,
    a: P,
    b: P,
    ctx: &Ctx,
    soft: bool,
    attract: Option<&std::collections::HashSet<usize>>,
) -> Result<Found, String> {
    let from = end_of(grid, obstacles, fresh, net, a, ctx, soft);
    let to = end_of(grid, obstacles, fresh, net, b, ctx, soft);
    let cells = search_between(grid, net, a, b, &from.cells, &to.cells, ctx, soft, attract, None)?;
    let (a, na) = from.start(cells[0], a);
    let (b, nb) = to.start(cells[cells.len() - 1], b);
    Ok(Found { cells, a, b, necks: na.into_iter().chain(nb).collect() })
}

struct End {
    cells: Vec<usize>,
    stubs: Vec<(usize, Neck)>,
}

impl End {
    fn start(&self, cell: usize, p: P) -> (P, Option<Neck>) {
        match self.stubs.iter().find(|(c, _)| *c == cell) {
            Some((_, n)) => (n.to, Some(n.clone())),
            None => (p, None),
        }
    }
}

fn end_of(
    grid: &Grid,
    obstacles: &[Obstacle],
    fresh: &[Obstacle],
    net: usize,
    p: P,
    ctx: &Ctx,
    soft: bool,
) -> End {
    let cells = anchors(grid, obstacles, fresh, net, p, ctx.routing);
    let pad = obstacles.iter().find_map(|o| match &o.shape {
        Shape::Poly(v) if o.net == Some(net) && geom::point_in_polygon(p, v) => Some((v, o)),
        _ => None,
    });
    let (Some(nk), Some((pad, o))) = (ctx.necking, pad) else {
        return End { cells, stubs: Vec::new() };
    };
    let plane = grid.w * grid.h;
    let mut ring = pad.clone();
    ring.push(pad[0]);
    let joined = obstacles.iter().any(|q| match &q.shape {
        Shape::Poly(v) => {
            q.net == Some(net)
                && !std::ptr::eq(v, pad)
                && q.layers.iter().any(|l| o.layers.contains(l))
                && geom::polyline_polygon_distance(&ring, v) < 1e-6
        }
        _ => false,
    });
    let pad_w = if joined { f64::INFINITY } else { geom::min_extent(pad) };
    let layers: Vec<usize> = o
        .layers
        .iter()
        .copied()
        .filter(|l| ctx.routing.contains(l))
        .filter(|&l| {
            ctx.widths[l] > pad_w + 1e-6
                || !cells.iter().any(|&i| i / plane == l && grid.ok(i, net, false, soft).0)
        })
        .collect();
    let stubs: Vec<(usize, Neck)> = layers
        .iter()
        .flat_map(|&l| stubs(grid, obstacles, net, p, pad, pad_w, l, ctx, nk, soft))
        .collect();
    if stubs.is_empty() {
        return End { cells, stubs };
    }
    let shape = Shape::Poly(pad.clone());
    let mut cells: Vec<usize> = cells
        .into_iter()
        .filter(|&i| {
            let (l, k) = (i / plane, i % plane);
            !layers.contains(&l)
                || shape.dist(grid.center(k % grid.w, k / grid.w)) >= ctx.widths[l] / 2.0
        })
        .chain(stubs.iter().map(|s| s.0))
        .collect();
    cells.sort_unstable();
    cells.dedup();
    End { cells, stubs }
}

#[allow(clippy::too_many_arguments)]
fn stubs(
    grid: &Grid,
    obstacles: &[Obstacle],
    net: usize,
    p: P,
    pad: &[P],
    pad_w: f64,
    l: usize,
    ctx: &Ctx,
    nk: &Necking,
    soft: bool,
) -> Vec<(usize, Neck)> {
    let wide = ctx.widths[l];
    let overhangs = wide > pad_w + 1e-6;
    let reach = nk.length + wide + ctx.clearance + 1.0;
    let near: Vec<(&Shape, f64)> = obstacles
        .iter()
        .filter(|o| o.net != Some(net) && o.layers.contains(&l))
        .filter(|o| {
            let (lo, hi) = o.shape.bounds();
            lo[0] <= p[0] + reach
                && lo[1] <= p[1] + reach
                && hi[0] >= p[0] - reach
                && hi[1] >= p[1] - reach
        })
        .map(|o| (&o.shape, o.clearance.max(ctx.clearance)))
        .collect();
    let edges: Vec<(P, P)> = nk.board.segments().collect();
    let cell = |q: P| {
        let (x, y) = grid.cell(q);
        (x >= 0 && y >= 0 && (x as usize) < grid.w && (y as usize) < grid.h)
            .then(|| grid.idx(l, x as usize, y as usize))
    };
    let mut out = Vec::new();
    for (dx, dy) in DIRS {
        let norm = ((dx * dx + dy * dy) as f64).sqrt();
        let u = [dx as f64 / norm, dy as f64 / norm];
        let mut k = 1;
        while k as f64 * grid.g <= nk.length + 1e-9 {
            let len = k as f64 * grid.g;
            k += 1;
            let q = [p[0] + u[0] * len, p[1] + u[1] * len];
            let gap = near
                .iter()
                .map(|(s, c)| s.seg_dist(p, q) - c)
                .chain(
                    edges.iter().map(|e| geom::segment_segment_distance(p, q, e.0, e.1) - nk.edge),
                )
                .fold(f64::MAX, f64::min);
            let width = crate::neck::neck_width(wide.min(pad_w).min(2.0 * gap), nk.min_width);
            if width < nk.min_width - 1e-9 {
                break;
            }
            let cap = !(overhangs && geom::point_in_polygon(q, pad))
                && near.iter().all(|(s, c)| s.dist(q) >= wide / 2.0 + c - 1e-9)
                && edges
                    .iter()
                    .all(|e| geom::point_segment_distance(q, e.0, e.1) >= nk.edge + wide / 2.0);
            let Some(i) = cell(q).filter(|&i| cap && grid.ok(i, net, false, soft).0) else {
                continue;
            };
            let steps = (len / (grid.g * 0.25)).ceil() as usize;
            let free = (0..=steps).all(|s| {
                let t = s as f64 / steps as f64;
                cell([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t])
                    .is_some_and(|j| Grid::free(&grid.rt, j, net))
            });
            if free {
                out.push((i, Neck { layer: l, from: p, to: q, width }));
            }
            break;
        }
    }
    out
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
        return Err(
            "the class width does not fit into an end pad and no neck-down within the class neckdown length keeps clearance"
                .into(),
        );
    }
    let (sources, goals) = (sources.as_slice(), goals.as_slice());
    let plane = grid.w * grid.h;
    let unpack = |i: usize| (i / plane, (i % plane) % grid.w, (i % plane) / grid.w);
    let via_here = |x: usize, y: usize, layers: &[usize]| -> Option<bool> {
        let mut clash = false;
        for &vl in layers {
            let (ok, c) = grid.ok(grid.idx(vl, x, y), net, true, soft);
            if !ok {
                return None;
            }
            clash |= c;
        }
        Some(clash)
    };
    let hop = |x: usize, y: usize, l: usize, ul: usize, clash: bool| -> Option<f64> {
        let price = |cost: f64, layers: &[usize]| {
            let via_clash = via_here(x, y, layers)?;
            Some(
                opts.via_cost * cost
                    + layers.len() as f64 * 1e-4
                    + if clash || via_clash { penalty } else { 0.0 },
            )
        };
        if ctx.vias.is_empty() {
            return price(1.0, via_layers);
        }
        ctx.vias
            .iter()
            .filter(|o| o.joins(l, ul))
            .filter_map(|o| price(o.cost, &o.layers))
            .min_by(f64::total_cmp)
    };
    let enter = |j: usize, diagonal: bool| -> Option<f64> {
        let (ok, clash) = grid.ok(j, net, false, soft);
        if !ok {
            return None;
        }
        let step = if diagonal { std::f64::consts::SQRT_2 } else { 1.0 };
        let pull = if attract.is_some_and(|a| a.contains(&j)) { 0.5 } else { 1.0 };
        let push = if repel.is_some_and(|r| r.contains(&j)) { 8.0 } else { 0.0 };
        Some(
            (step * pull + push) * grid.g + grid.hist[j] as f64 + if clash { penalty } else { 0.0 },
        )
    };
    let corner_ok = |l: usize, x: usize, y: usize, nx: usize, ny: usize| {
        (nx == x || ny == y)
            || (grid.ok(grid.idx(l, nx, y), net, false, soft).0
                && grid.ok(grid.idx(l, x, ny), net, false, soft).0)
    };
    let mut margin = opts.margin;
    loop {
        let lo = grid.cell([a[0].min(b[0]) - margin, a[1].min(b[1]) - margin]);
        let hi = grid.cell([a[0].max(b[0]) + margin, a[1].max(b[1]) + margin]);
        let (wx0, wy0) = (lo.0.max(0) as usize, lo.1.max(0) as usize);
        let (wx1, wy1) =
            ((hi.0.max(0) as usize).min(grid.w - 1), (hi.1.max(0) as usize).min(grid.h - 1));
        let (ww, wh) = (wx1 - wx0 + 1, wy1 - wy0 + 1);
        let inside = |x: i64, y: i64| {
            x >= wx0 as i64 && y >= wy0 as i64 && x <= wx1 as i64 && y <= wy1 as i64
        };
        let local = |i: usize| -> Option<usize> {
            let (l, x, y) = unpack(i);
            let rl = routing.iter().position(|&r| r == l)?;
            inside(x as i64, y as i64).then(|| (rl * wh + (y - wy0)) * ww + (x - wx0))
        };
        let n = routing.len() * ww * wh;
        let mut rest = vec![f32::MAX; n];
        let mut is_source = vec![false; n];
        for &s in sources {
            if let Some(k) = local(s) {
                is_source[k] = true;
            }
        }
        let mut heap = BinaryHeap::new();
        for &gi in goals {
            if let Some(k) = local(gi) {
                rest[k] = 0.0;
                heap.push(Node { f: 0.0, i: gi });
            }
        }
        let mut stop = f64::MAX;
        let mut bound = f64::MAX;
        while let Some(Node { i, f }) = heap.pop() {
            let k = local(i).unwrap();
            if f > rest[k] as f64 + 1e-6 {
                continue;
            }
            if f > stop {
                bound = f;
                break;
            }
            if is_source[k] && stop == f64::MAX {
                stop = f * 1.25 + 30.0 * opts.bend_cost + 1.0;
            }
            let (l, x, y) = unpack(i);
            let mut arrive: [Option<f64>; 2] = [None, None];
            for (dx, dy) in DIRS {
                let (px, py) = (x as i64 - dx, y as i64 - dy);
                if !inside(px, py) {
                    continue;
                }
                let (px, py) = (px as usize, py as usize);
                let u = grid.idx(l, px, py);
                if !grid.ok(u, net, false, soft).0 || !corner_ok(l, px, py, x, y) {
                    continue;
                }
                let diagonal = dx != 0 && dy != 0;
                let slot = &mut arrive[diagonal as usize];
                if slot.is_none() {
                    *slot = enter(i, diagonal);
                }
                let Some(c) = *slot else { continue };
                let ku = local(u).unwrap();
                let c = f + c;
                if (c as f32) < rest[ku] {
                    rest[ku] = c as f32;
                    heap.push(Node { f: c, i: u });
                }
            }
            if !via_layers.contains(&l) {
                continue;
            }
            let clash = grid.ok(i, net, false, soft).1;
            for &ul in routing.iter().filter(|&&ul| ul != l && via_layers.contains(&ul)) {
                let u = grid.idx(ul, x, y);
                if !grid.ok(u, net, false, soft).0 {
                    continue;
                }
                let Some(cost) = hop(x, y, ul, l, clash) else { continue };
                let c = f + cost;
                let ku = local(u).unwrap();
                if (c as f32) < rest[ku] {
                    rest[ku] = c as f32;
                    heap.push(Node { f: c, i: u });
                }
            }
        }
        if stop < f64::MAX {
            let heur = |k: usize| (rest[k] as f64).min(bound);
            let mut is_goal = vec![false; n];
            for &gi in goals {
                if let Some(k) = local(gi) {
                    is_goal[k] = true;
                }
            }
            let mut states = States::new(n);
            let mut heap = BinaryHeap::new();
            for &s in sources {
                if let Some(k) = local(s)
                    && rest[k] < f32::MAX
                {
                    states.set(k, NODIR, 0.0, usize::MAX);
                    heap.push(Node { f: heur(k), i: s * 9 + NODIR });
                }
            }
            let mut hit = None;
            while let Some(Node { i: state, f }) = heap.pop() {
                let (i, d0) = (state / 9, state % 9);
                let k = local(i).unwrap();
                let here = states.cost(k, d0) as f64;
                if f > here + heur(k) + 1e-3 {
                    continue;
                }
                if is_goal[k] {
                    hit = Some(state);
                    break;
                }
                let (l, x, y) = unpack(i);
                for (d, (dx, dy)) in DIRS.iter().enumerate() {
                    let Some(turn) = bend(d0, d, opts.bend_cost) else { continue };
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if !inside(nx, ny) {
                        continue;
                    }
                    let (nx, ny) = (nx as usize, ny as usize);
                    let j = grid.idx(l, nx, ny);
                    let kj = local(j).unwrap();
                    let h = heur(kj);
                    if h == f32::MAX as f64 || !corner_ok(l, x, y, nx, ny) {
                        continue;
                    }
                    let Some(step) = enter(j, *dx != 0 && *dy != 0) else { continue };
                    let c = here + step + turn;
                    if (c as f32) < states.cost(kj, d) {
                        states.set(kj, d, c as f32, state);
                        heap.push(Node { f: c + h, i: j * 9 + d });
                    }
                }
                if !via_layers.contains(&l)
                    || (!ctx.stack_vias && d0 == NODIR && states.from(k, NODIR) != usize::MAX)
                {
                    continue;
                }
                for &nl in routing.iter().filter(|&&nl| nl != l && via_layers.contains(&nl)) {
                    let j = grid.idx(nl, x, y);
                    let (ok, clash) = grid.ok(j, net, false, soft);
                    let kj = local(j).unwrap();
                    let h = heur(kj);
                    if !ok || h == f32::MAX as f64 {
                        continue;
                    }
                    let Some(cost) = hop(x, y, l, nl, clash) else { continue };
                    let c = here + cost;
                    if (c as f32) < states.cost(kj, NODIR) {
                        states.set(kj, NODIR, c as f32, state);
                        heap.push(Node { f: c + h, i: j * 9 + NODIR });
                    }
                }
            }
            if let Some(end) = hit {
                let mut path = vec![end / 9];
                let mut cur = end;
                loop {
                    let p = states.from(local(cur / 9).unwrap(), cur % 9);
                    if p == usize::MAX {
                        break;
                    }
                    path.push(p / 9);
                    cur = p;
                }
                path.reverse();
                path.dedup();
                return Ok(path);
            }
        }
        let whole = wx0 == 0 && wy0 == 0 && wx1 == grid.w - 1 && wy1 == grid.h - 1;
        if whole || (!soft && margin > opts.margin) {
            return Err("no path within the rules".into());
        }
        margin *= 3.0;
    }
}

const NODIR: usize = 8;

fn bend(d0: usize, d1: usize, cost: f64) -> Option<f64> {
    if d0 == NODIR {
        return Some(0.0);
    }
    match (d0 as i64 - d1 as i64).rem_euclid(8).min((d1 as i64 - d0 as i64).rem_euclid(8)) {
        0 => Some(0.0),
        1 => Some(cost),
        2 => Some(3.0 * cost),
        _ => None,
    }
}

struct States {
    slot: Vec<u32>,
    blocks: Vec<([f32; 9], [usize; 9])>,
}

impl States {
    fn new(n: usize) -> Self {
        States { slot: vec![u32::MAX; n], blocks: Vec::new() }
    }

    fn cost(&self, k: usize, d: usize) -> f32 {
        match self.slot[k] {
            u32::MAX => f32::MAX,
            s => self.blocks[s as usize].0[d],
        }
    }

    fn from(&self, k: usize, d: usize) -> usize {
        self.blocks[self.slot[k] as usize].1[d]
    }

    fn set(&mut self, k: usize, d: usize, cost: f32, from: usize) {
        if self.slot[k] == u32::MAX {
            self.slot[k] = self.blocks.len() as u32;
            self.blocks.push(([f32::MAX; 9], [usize::MAX; 9]));
        }
        let b = &mut self.blocks[self.slot[k] as usize];
        b.0[d] = cost;
        b.1[d] = from;
    }
}

type Geometry = (Vec<(usize, Vec<P>)>, Vec<P>);

fn hops_of(grid: &Grid, path: &[usize]) -> Vec<(usize, usize)> {
    let plane = grid.w * grid.h;
    path.windows(2).map(|w| (w[0] / plane, w[1] / plane)).filter(|(a, b)| a != b).collect()
}

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
        .map(|(l, pts)| {
            (l, simplify(&chamfer(grid, l, &octilinear(grid, l, &simplify(&pts), net), net)))
        })
        .filter(|(_, pts)| pts.len() >= 2)
        .collect();
    (tracks, vias)
}

fn clear_line(grid: &Grid, l: usize, p: P, q: P, net: usize) -> bool {
    let n = (geom::dist(p, q) / (grid.g * 0.25)).ceil().max(1.0) as usize;
    let ok = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && (x as usize) < grid.w
            && (y as usize) < grid.h
            && grid.ok(grid.idx(l, x as usize, y as usize), net, false, false).0
    };
    (0..=n).all(|k| {
        let t = k as f64 / n as f64;
        let at = [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
        let (x, y) = grid.cell(at);
        if ok(x, y) {
            return true;
        }
        let (fx, fy) = ((at[0] - grid.x0) / grid.g - 0.5, (at[1] - grid.y0) / grid.g - 0.5);
        let (cx, cy) = (fx.floor() as i64, fy.floor() as i64);
        [(cx, cy), (cx + 1, cy), (cx, cy + 1), (cx + 1, cy + 1)].into_iter().any(|(x, y)| {
            let c = [grid.x0 + (x as f64 + 0.5) * grid.g, grid.y0 + (y as f64 + 0.5) * grid.g];
            geom::dist(c, at) <= grid.g * 0.6 && ok(x, y)
        })
    })
}

fn dogleg(p: P, q: P, diagonal_first: bool) -> Vec<P> {
    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
    let d = dx.abs().min(dy.abs());
    if d < 1e-9 || (dx.abs() - dy.abs()).abs() < 1e-9 {
        return vec![p, q];
    }
    let diag = [d * dx.signum(), d * dy.signum()];
    let m = if diagonal_first {
        [p[0] + diag[0], p[1] + diag[1]]
    } else {
        [q[0] - diag[0], q[1] - diag[1]]
    };
    vec![p, m, q]
}

fn heading(a: P, b: P) -> Option<P> {
    let l = geom::dist(a, b);
    (l > 1e-9).then(|| [(b[0] - a[0]) / l, (b[1] - a[1]) / l])
}

fn octilinear(grid: &Grid, l: usize, pts: &[P], net: usize) -> Vec<P> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let n = pts.len();
    let mut out = vec![pts[0]];
    let mut i = 0;
    while i < n - 1 {
        let before =
            (out.len() >= 2).then(|| heading(out[out.len() - 2], out[out.len() - 1])).flatten();
        let mut found = None;
        for j in (i + 1..n.min(i + 120)).rev() {
            let mut variants = [dogleg(pts[i], pts[j], true), dogleg(pts[i], pts[j], false)];
            if let Some(h) = before {
                let keeps = |v: &Vec<P>| {
                    heading(v[0], v[1]).is_some_and(|g| (g[0] * h[0] + g[1] * h[1]) > 0.99)
                };
                if keeps(&variants[1]) && !keeps(&variants[0]) {
                    variants.swap(0, 1);
                }
            }
            if let Some(v) = variants
                .into_iter()
                .find(|v| v.windows(2).all(|w| clear_line(grid, l, w[0], w[1], net)))
            {
                found = Some((j, v));
                break;
            }
        }
        match found {
            Some((j, v)) => {
                out.extend_from_slice(&v[1..]);
                i = j;
            }
            None => {
                out.push(pts[i + 1]);
                i += 1;
            }
        }
    }
    simplify(&out)
}

fn chamfer(grid: &Grid, l: usize, pts: &[P], net: usize) -> Vec<P> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    for k in 1..pts.len() - 1 {
        let (a, b, c) = (*out.last().unwrap(), pts[k], pts[k + 1]);
        let (Some(u), Some(v)) = (heading(a, b), heading(b, c)) else {
            out.push(b);
            continue;
        };
        if (u[0] * v[0] + u[1] * v[1]).abs() > 1e-6 {
            out.push(b);
            continue;
        }
        let reach = (geom::dist(a, b).min(geom::dist(b, c)) * 0.5).min(1.0);
        let mut done = false;
        let mut cut = reach;
        while cut >= grid.g {
            let p = [b[0] - u[0] * cut, b[1] - u[1] * cut];
            let q = [b[0] + v[0] * cut, b[1] + v[1] * cut];
            if clear_line(grid, l, p, q, net) {
                out.push(p);
                out.push(q);
                done = true;
                break;
            }
            cut *= 0.5;
        }
        if !done {
            out.push(b);
        }
    }
    out.push(*pts.last().unwrap());
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
        let pc = Conn {
            net: p,
            tracks: vec![(l, pp.clone())],
            vias: Vec::new(),
            hops: Vec::new(),
            necks: Vec::new(),
        };
        let nc = Conn {
            net: n,
            tracks: vec![(l, np.clone())],
            vias: Vec::new(),
            hops: Vec::new(),
            necks: Vec::new(),
        };
        grid.mark_routed(&pc, ctx);
        grid.mark_routed(&nc, ctx);
        let mut parts: Vec<Conn> = Vec::new();
        let ends = [(p, ap, pp[0]), (n, an, np[0]), (p, bp, *pp.last()?), (n, bn, *np.last()?)];
        let mut ok = true;
        for (net, pad, end) in ends {
            let sources = anchors(grid, obstacles, &[], net, pad, ctx.routing);
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
            hops: a.hops.iter().chain(b.hops.iter()).cloned().collect(),
            necks: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn open_grid(w: usize, h: usize, layers: usize) -> Grid {
        Grid {
            x0: 0.0,
            y0: 0.0,
            g: 0.1,
            w,
            h,
            track: vec![FREE; layers * w * h],
            via: vec![FREE; layers * w * h],
            rt: vec![FREE; layers * w * h],
            rv: vec![FREE; layers * w * h],
            hist: vec![0.0; layers * w * h],
        }
    }

    fn block(grid: &mut Grid, l: usize, x: std::ops::Range<usize>, y: std::ops::Range<usize>) {
        for yy in y {
            for xx in x.clone() {
                let i = grid.idx(l, xx, yy);
                grid.track[i] = BLOCK;
                grid.via[i] = BLOCK;
            }
        }
    }

    fn route_one(grid: &Grid, layers: &[usize], a: (usize, usize), b: (usize, usize)) -> Geometry {
        let opts = RouteOptions { margin: 10.0, ..Default::default() };
        let ctx = Ctx {
            drill_r: 0.1,
            hole_gap: 0.2,
            hole_cu: 0.0,
            hole_smd: 0.0,
            in_pad: false,
            smd: &[],
            widths: vec![0.1; 2],
            clearance: 0.1,
            via_r: 0.2,
            via_layers: layers,
            vias: &[],
            stack_vias: true,
            routing: layers,
            opts: &opts,
            necking: None,
        };
        let (pa, pb) = (grid.center(a.0, a.1), grid.center(b.0, b.1));
        let path = search_between(
            grid,
            0,
            pa,
            pb,
            &[grid.idx(0, a.0, a.1)],
            &[grid.idx(0, b.0, b.1)],
            &ctx,
            false,
            None,
            None,
        )
        .unwrap();
        geometry(grid, &path, pa, pb, 0)
    }

    fn octilinear_and_forward(pts: &[P]) {
        for w in pts.windows(2) {
            let a = (w[1][1] - w[0][1]).atan2(w[1][0] - w[0][0]).to_degrees().rem_euclid(45.0);
            assert!(a.min(45.0 - a) < 1e-6, "{:?} -> {:?} is off 45", w[0], w[1]);
        }
        for w in pts.windows(3) {
            let (u, v) = (heading(w[0], w[1]).unwrap(), heading(w[1], w[2]).unwrap());
            assert!(u[0] * v[0] + u[1] * v[1] > 0.7, "{:?} turns more than 45", w[1]);
        }
    }

    #[test]
    fn routes_around_a_wall_in_45_degree_steps() {
        let mut grid = open_grid(60, 40, 1);
        block(&mut grid, 0, 25..30, 12..40);
        let (tracks, vias) = route_one(&grid, &[0], (5, 30), (55, 32));
        assert!(vias.is_empty());
        assert_eq!(tracks.len(), 1);
        let pts = &tracks[0].1;
        octilinear_and_forward(pts);
        assert!(pts.len() <= 8, "{} corners: {pts:?}", pts.len());
    }

    #[test]
    fn a_blocked_layer_takes_a_via() {
        let mut grid = open_grid(40, 20, 2);
        block(&mut grid, 0, 20..21, 0..20);
        let (tracks, vias) = route_one(&grid, &[0, 1], (5, 10), (35, 10));
        assert_eq!(vias.len(), 2);
        assert!(tracks.iter().any(|t| t.0 == 1));
        for t in &tracks {
            octilinear_and_forward(&t.1);
        }
    }

    #[test]
    fn a_via_budget_of_zero_keeps_one_layer() {
        let mut grid = open_grid(60, 40, 2);
        block(&mut grid, 0, 30..31, 0..36);
        let opts = RouteOptions { via_cost: 0.1, ..Default::default() };
        let ctx = Ctx {
            drill_r: 0.1,
            hole_gap: 0.2,
            hole_cu: 0.0,
            hole_smd: 0.0,
            in_pad: false,
            smd: &[],
            widths: vec![0.1; 2],
            clearance: 0.1,
            via_r: 0.2,
            via_layers: &[0, 1],
            vias: &[],
            stack_vias: true,
            routing: &[0, 1],
            opts: &opts,
            necking: None,
        };
        let (a, b) = (grid.center(5, 5), grid.center(55, 5));
        let pad = |at: P| Obstacle {
            net: Some(0),
            layers: vec![0],
            shape: Shape::Circle(at, 0.05),
            clearance: 0.1,
        };
        let pads = [pad(a), pad(b)];
        let path = search(&grid, &pads, &[], 0, a, b, &ctx, false, None).unwrap();
        assert_eq!(conn_found(&grid, &path, 0).vias.len(), 2);
        let c = fewer_vias(&grid, &pads, &[], 0, a, b, &ctx, 0).unwrap();
        assert!(c.vias.is_empty() && c.tracks.iter().all(|t| t.0 == 0));
        assert!(fewer_vias(&grid, &pads, &[], 0, a, b, &ctx, 2).unwrap().vias.len() <= 2);
    }

    #[test]
    fn staggered_vias_of_parallel_connections_line_up() {
        let grid = open_grid(80, 40, 2);
        let opts = RouteOptions::default();
        let ctx = Ctx {
            drill_r: 0.1,
            hole_gap: 0.2,
            hole_cu: 0.0,
            hole_smd: 0.0,
            in_pad: false,
            smd: &[],
            widths: vec![0.1; 2],
            clearance: 0.1,
            via_r: 0.2,
            via_layers: &[0, 1],
            vias: &[],
            stack_vias: true,
            routing: &[0, 1],
            opts: &opts,
            necking: None,
        };
        let conn = |net: usize, y: f64, x: f64| Conn {
            net,
            tracks: vec![(0, vec![[1.05, y], [x, y]]), (1, vec![[x, y], [6.05, y]])],
            vias: vec![[x, y]],
            hops: Vec::new(),
            necks: Vec::new(),
        };
        let mut routed = vec![Some(conn(0, 1.05, 3.05)), Some(conn(1, 2.05, 4.05))];
        let before: Vec<f64> = routed.iter().flatten().map(track_length).collect();
        snap_vias(&grid, &[], &mut routed, &[false, false], &ctx);
        let got: Vec<&Conn> = routed.iter().flatten().collect();
        assert!((got[0].vias[0][0] - got[1].vias[0][0]).abs() < 1e-9, "{:?}", got[0].vias);
        for (c, len) in got.iter().zip(before) {
            assert!((track_length(c) - len).abs() < 1e-9);
            let v = c.vias[0];
            assert!(c.tracks.iter().all(|(_, p)| p.contains(&v)));
        }
        let mut locked = vec![Some(conn(0, 1.05, 3.05)), Some(conn(1, 2.05, 4.05))];
        snap_vias(&grid, &[], &mut locked, &[true, false], &ctx);
        assert_eq!(locked[0].as_ref().unwrap().vias[0], [3.05, 1.05]);
        assert_eq!(locked[1].as_ref().unwrap().vias[0], [4.05, 2.05]);
    }

    #[test]
    fn via_cells_keep_off_smd_pads_unless_via_in_pad() {
        let square = |c: P, h: f64| {
            vec![
                [c[0] - h, c[1] - h],
                [c[0] + h, c[1] - h],
                [c[0] + h, c[1] + h],
                [c[0] - h, c[1] + h],
            ]
        };
        let pad = SmdPad {
            layers: vec![0],
            outlines: vec![square([2.05, 2.05], 0.3)],
            lo: [1.75, 1.75],
            hi: [2.35, 2.35],
        };
        let smd = [pad];
        let opts = RouteOptions::default();
        for in_pad in [false, true] {
            let ctx = Ctx {
                drill_r: 0.1,
                hole_gap: 0.2,
                hole_cu: 0.0,
                hole_smd: 0.2,
                in_pad,
                smd: &smd,
                widths: vec![0.1; 2],
                clearance: 0.1,
                via_r: 0.2,
                via_layers: &[0, 1],
                vias: &[],
                stack_vias: true,
                routing: &[0, 1],
                opts: &opts,
                necking: None,
            };
            let mut grid = open_grid(50, 50, 2);
            grid.block_smd(&ctx);
            let via_ok = |x: usize| {
                let at = grid.center(x, 20);
                let cell = [0, 1].iter().all(|&l| grid.ok(grid.idx(l, x, 20), 0, true, false).0);
                assert_eq!(cell, via_clears_smd(at, &ctx), "{at:?}");
                cell
            };
            assert_eq!(via_ok(20), in_pad, "centred in the pad");
            assert!(!via_ok(24), "cuts the pad edge");
            assert!(!via_ok(25), "hole 0.1 mm from the pad");
            assert!(via_ok(27), "hole 0.3 mm from the pad");
            assert!(!via_ok(22), "annulus past the pad edge");
        }
    }

    #[test]
    fn snapped_vias_keep_off_smd_pads() {
        let grid = open_grid(80, 40, 2);
        let opts = RouteOptions::default();
        let pad = |c: P| SmdPad {
            layers: vec![0],
            outlines: vec![vec![
                [c[0] - 0.1, c[1] - 0.1],
                [c[0] + 0.1, c[1] - 0.1],
                [c[0] + 0.1, c[1] + 0.1],
                [c[0] - 0.1, c[1] + 0.1],
            ]],
            lo: [c[0] - 0.1, c[1] - 0.1],
            hi: [c[0] + 0.1, c[1] + 0.1],
        };
        let smd = [pad([3.05, 2.35]), pad([4.05, 0.75])];
        let ctx = Ctx {
            drill_r: 0.1,
            hole_gap: 0.2,
            hole_cu: 0.0,
            hole_smd: 0.2,
            in_pad: false,
            smd: &smd,
            widths: vec![0.1; 2],
            clearance: 0.1,
            via_r: 0.2,
            via_layers: &[0, 1],
            vias: &[],
            stack_vias: true,
            routing: &[0, 1],
            opts: &opts,
            necking: None,
        };
        let conn = |net: usize, y: f64, x: f64| Conn {
            net,
            tracks: vec![(0, vec![[1.05, y], [x, y]]), (1, vec![[x, y], [6.05, y]])],
            vias: vec![[x, y]],
            hops: Vec::new(),
            necks: Vec::new(),
        };
        let mut routed = vec![Some(conn(0, 1.05, 3.05)), Some(conn(1, 2.05, 4.05))];
        snap_vias(&grid, &[], &mut routed, &[false, false], &ctx);
        for c in routed.iter().flatten() {
            assert!(via_clears_smd(c.vias[0], &ctx), "{:?}", c.vias[0]);
        }
    }

    #[test]
    fn vias_keep_hole_to_copper_from_other_nets() {
        let opts = RouteOptions::default();
        let ctx = Ctx {
            drill_r: 0.1,
            hole_gap: 0.2,
            hole_cu: 0.4,
            hole_smd: 0.0,
            in_pad: false,
            smd: &[],
            widths: vec![0.1],
            clearance: 0.1,
            via_r: 0.2,
            via_layers: &[0],
            vias: &[],
            stack_vias: true,
            routing: &[0],
            opts: &opts,
            necking: None,
        };
        let mut grid = open_grid(40, 40, 1);
        let track = Obstacle {
            net: Some(1),
            layers: vec![0],
            shape: Shape::Seg([1.0, 0.5], [1.0, 3.5], 0.0),
            clearance: 0.1,
        };
        grid.add(&track, &[0.05], ctx.via_r, ctx.clearance, ctx.drill_r + ctx.hole_cu);
        assert!(!grid.ok(grid.idx(0, 14, 20), 0, true, false).0, "hole 0.35 mm from the track");
        assert!(grid.ok(grid.idx(0, 17, 20), 0, true, false).0, "hole 0.65 mm from the track");
        let via = Conn {
            net: 0,
            tracks: Vec::new(),
            vias: vec![[1.45, 2.05]],
            hops: Vec::new(),
            necks: Vec::new(),
        };
        let near = Conn {
            net: 1,
            tracks: vec![(0, vec![[1.0, 0.5], [1.0, 3.5]])],
            vias: Vec::new(),
            hops: Vec::new(),
            necks: Vec::new(),
        };
        assert!(conflicts(&near, &via, &ctx) && conflicts(&via, &near, &ctx));
        let far = Conn {
            net: 0,
            tracks: Vec::new(),
            vias: vec![[1.65, 2.05]],
            hops: Vec::new(),
            necks: Vec::new(),
        };
        assert!(!conflicts(&near, &far, &ctx));
    }

    #[test]
    fn no_path_fails_fast() {
        let mut grid = open_grid(40, 20, 1);
        block(&mut grid, 0, 20..21, 0..20);
        let opts = RouteOptions::default();
        let ctx = Ctx {
            drill_r: 0.1,
            hole_gap: 0.2,
            hole_cu: 0.0,
            hole_smd: 0.0,
            in_pad: false,
            smd: &[],
            widths: vec![0.1],
            clearance: 0.1,
            via_r: 0.2,
            via_layers: &[0],
            vias: &[],
            stack_vias: true,
            routing: &[0],
            opts: &opts,
            necking: None,
        };
        let (a, b) = (grid.center(5, 10), grid.center(35, 10));
        let got = search_between(
            &grid,
            0,
            a,
            b,
            &[grid.idx(0, 5, 10)],
            &[grid.idx(0, 35, 10)],
            &ctx,
            false,
            None,
            None,
        );
        assert!(got.is_err());
    }

    fn square(c: P, h: f64) -> Vec<P> {
        vec![[c[0] - h, c[1] - h], [c[0] + h, c[1] - h], [c[0] + h, c[1] + h], [c[0] - h, c[1] + h]]
    }

    fn necked_route(pitch: f64, necking: Option<&Necking>) -> Result<(Found, Conn), String> {
        let opts = RouteOptions { margin: 10.0, ..Default::default() };
        let ctx = Ctx {
            drill_r: 0.1,
            hole_gap: 0.2,
            hole_cu: 0.0,
            hole_smd: 0.0,
            in_pad: false,
            smd: &[],
            widths: vec![0.4],
            clearance: 0.15,
            via_r: 0.2,
            via_layers: &[0],
            vias: &[],
            stack_vias: true,
            routing: &[0],
            opts: &opts,
            necking,
        };
        let (a, b) = ([1.025, 2.025], [5.025, 2.025]);
        let pad = |net: usize, c: P, h: f64| Obstacle {
            net: Some(net),
            layers: vec![0],
            shape: Shape::Poly(square(c, h)),
            clearance: 0.15,
        };
        let obstacles = [
            pad(0, a, 0.15),
            pad(1, [a[0], a[1] + pitch], 0.15),
            pad(2, [a[0], a[1] - pitch], 0.15),
            pad(0, b, 0.5),
        ];
        let mut grid = open_grid(60, 40, 1);
        for o in &obstacles {
            grid.add(o, &[0.2], ctx.via_r, ctx.clearance, 0.0);
        }
        let found = search(&grid, &obstacles, &[], 0, a, b, &ctx, false, None)?;
        let conn = conn_found(&grid, &found, 0);
        Ok((found, conn))
    }

    #[test]
    fn a_wide_track_necks_down_into_a_small_pad() {
        let outline = square([3.0, 2.0], 3.0);
        let necking = Necking {
            length: 0.6,
            min_width: 0.1,
            edge: 0.0,
            board: geom::BoardEdge::new(&outline, &[]),
        };
        assert!(necked_route(0.5, None).is_err());
        let (found, conn) = necked_route(0.5, Some(&necking)).unwrap();
        assert_eq!(found.necks.len(), 1);
        let n = &found.necks[0];
        assert_eq!(n.from, [1.025, 2.025]);
        assert!((n.width - 0.3).abs() < 1e-9, "neck width {}", n.width);
        let len = geom::dist(n.from, n.to);
        assert!(len <= 0.6 + 1e-9 && n.to[0] > 1.175, "neck {n:?}");
        assert_eq!(conn.tracks[0].1[0], n.to);
        assert_eq!(conn.necks, found.necks);
        let (found, _) = necked_route(0.425, Some(&necking)).unwrap();
        let n = &found.necks[0];
        assert!(n.width <= 0.25 + 1e-9 && n.width >= 0.24 - 1e-9, "neck width {}", n.width);
        let tight = Necking { length: 0.2, ..necking };
        assert!(necked_route(0.5, Some(&tight)).is_err());
        let fab = Necking { min_width: 0.35, ..necking };
        assert!(necked_route(0.5, Some(&fab)).is_err());
    }

    #[test]
    fn touching_pads_of_one_net_count_as_one_wide_pad() {
        let outline = square([3.0, 2.0], 3.0);
        let necking = Necking {
            length: 0.6,
            min_width: 0.1,
            edge: 0.0,
            board: geom::BoardEdge::new(&outline, &[]),
        };
        let opts = RouteOptions { margin: 10.0, ..Default::default() };
        let ctx = Ctx {
            drill_r: 0.1,
            hole_gap: 0.2,
            hole_cu: 0.0,
            hole_smd: 0.0,
            in_pad: false,
            smd: &[],
            widths: vec![0.4],
            clearance: 0.15,
            via_r: 0.2,
            via_layers: &[0],
            vias: &[],
            stack_vias: true,
            routing: &[0],
            opts: &opts,
            necking: Some(&necking),
        };
        let (a, b) = ([1.025, 2.025], [5.025, 2.025]);
        let pad = |c: P, h: f64| Obstacle {
            net: Some(0),
            layers: vec![0],
            shape: Shape::Poly(square(c, h)),
            clearance: 0.15,
        };
        let single = [pad(a, 0.15), pad(b, 0.5)];
        let joined = [pad(a, 0.15), pad([a[0], a[1] + 0.3], 0.15), pad(b, 0.5)];
        let mut grid = open_grid(60, 40, 1);
        for o in &joined {
            grid.add(o, &[0.2], ctx.via_r, ctx.clearance, 0.0);
        }
        let found = search(&grid, &single, &[], 0, a, b, &ctx, false, None).unwrap();
        assert_eq!(found.necks.len(), 1);
        let found = search(&grid, &joined, &[], 0, a, b, &ctx, false, None).unwrap();
        assert!(found.necks.is_empty(), "{:?}", found.necks);
    }

    #[test]
    fn necks_count_at_their_own_width_in_conflicts() {
        let opts = RouteOptions::default();
        let ctx = Ctx {
            drill_r: 0.1,
            hole_gap: 0.2,
            hole_cu: 0.0,
            hole_smd: 0.0,
            in_pad: false,
            smd: &[],
            widths: vec![0.4],
            clearance: 0.15,
            via_r: 0.2,
            via_layers: &[0],
            vias: &[],
            stack_vias: true,
            routing: &[0],
            opts: &opts,
            necking: None,
        };
        let neck = |net: usize, y: f64| Conn {
            net,
            tracks: Vec::new(),
            vias: Vec::new(),
            hops: Vec::new(),
            necks: vec![Neck { layer: 0, from: [0.0, y], to: [0.5, y], width: 0.2 }],
        };
        assert!(!conflicts(&neck(0, 0.0), &neck(1, 0.36), &ctx));
        assert!(conflicts(&neck(0, 0.0), &neck(1, 0.34), &ctx));
        let wide = Conn {
            net: 1,
            tracks: vec![(0, vec![[0.0, 0.46], [0.5, 0.46]])],
            vias: Vec::new(),
            hops: Vec::new(),
            necks: Vec::new(),
        };
        assert!(!conflicts(&neck(0, 0.0), &wide, &ctx));
        assert!(conflicts(&neck(0, 0.1), &wide, &ctx));
    }
}
