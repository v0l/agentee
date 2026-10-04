mod conn;
mod grid;
mod rules;
mod search;
mod shape;
mod soft;
mod spread;

use agentee_core::board::Board;
use agentee_core::geom::{self, P};
use agentee_core::layout::{Layout, glob};
use agentee_core::route::{RouteResult, RoutedTrack, RoutedVia, Unrouted};
use conn::{Islands, Item, NetCopper};
pub use grid::{Fence, Grid, Shape};
pub use rules::{Need, NetRule, Rules, um};
use search::{Query, Source, Window, seg_cells};
pub use shape::{Access, Ctx, PadRef};
pub use spread::spread;

const MAX_TARGETS: usize = 3_000_000;
use soft::{Copper, Piece, Soft};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Options {
    pub nets: Vec<String>,
    pub grid: f64,
    pub via_cost: f64,
    pub bend_cost: f64,
    pub margin: f64,
    pub rounds: usize,
    pub via_in_pad: bool,
    pub fences: bool,
    pub criticality: BTreeMap<String, f64>,
    pub zone_cost: f64,
    pub entry: f64,
    pub escalate: usize,
    pub corridor_cost: f64,
    pub prefer_gain: f64,
    pub stall: usize,
    pub verbose: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            nets: Vec::new(),
            grid: 0.05,
            via_cost: 1.0,
            bend_cost: 0.1,
            margin: 3.0,
            rounds: 30,
            via_in_pad: false,
            fences: true,
            criticality: BTreeMap::new(),
            zone_cost: 1.0,
            entry: 1.0,
            escalate: 1,
            corridor_cost: 1.0,
            prefer_gain: 0.5,
            stall: 3,
            verbose: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Guide {
    pub tile: f64,
    pub origin: P,
    pub corridors: HashMap<usize, HashSet<(usize, i64, i64)>>,
    pub prefer: HashMap<usize, Vec<(usize, P, P)>>,
    pub prefer_vias: HashMap<usize, Vec<P>>,
}

impl Guide {
    fn tile_of(&self, p: P) -> (i64, i64) {
        (
            ((p[0] - self.origin[0]) / self.tile).floor() as i64,
            ((p[1] - self.origin[1]) / self.tile).floor() as i64,
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub result: RouteResult,
    pub overlap: Vec<(usize, usize, P)>,
    pub rounds: usize,
    pub overlap_left: usize,
}

type Footprint = (Vec<Vec<u32>>, Vec<Vec<u32>>);
pub type Terminals = Vec<(P, Vec<usize>)>;

struct NetState {
    net: usize,
    reach: f64,
    own: HashSet<u32>,
    copper: NetCopper,
    pads: Vec<PadRef>,
    pieces: Vec<Piece>,
    fp: Option<Footprint>,
    failed: Vec<Unrouted>,
    needed: usize,
    joined: usize,
    span: f64,
}

struct Env<'a> {
    layout: &'a Layout,
    grid: &'a Grid,
    soft: &'a Soft,
    rules: &'a Rules,
    islands: &'a Islands,
    opts: &'a Options,
    zone: &'a [u16],
    guide: &'a Guide,
}

pub struct Base {
    pub rules: Rules,
    pub grid: Grid,
    islands: Islands,
    zone: Vec<u16>,
    pub nets: Vec<usize>,
}

fn pads_of(copper: &NetCopper) -> Vec<PadRef> {
    copper
        .items
        .iter()
        .filter_map(|it| match it {
            Item::Pad { shapes, layers, centre, pitch } => {
                Some(shapes.iter().filter_map(move |s| match s {
                    Shape::Poly(v) => Some(PadRef {
                        layers: layers.clone(),
                        outline: v.clone(),
                        centre: *centre,
                        pitch: *pitch,
                    }),
                    _ => None,
                }))
            }
            _ => None,
        })
        .flatten()
        .collect()
}

impl Base {
    pub fn new(layout: &Layout, board: &Board, opts: &Options) -> Result<Base, String> {
        let wanted: Vec<usize> = (0..layout.nets.len())
            .filter(|&n| {
                let name = &layout.nets[n].name;
                opts.nets.is_empty() || opts.nets.iter().any(|g| glob(g, name))
            })
            .collect();
        if wanted.is_empty() {
            return Err("no net matches".into());
        }
        let islands = Islands::new(layout, &wanted);
        let nets: Vec<usize> = wanted
            .iter()
            .copied()
            .filter(|&n| {
                let copper = conn::net_copper(layout, &islands, n);
                (0..copper.groups).filter(|&g| copper.has_pad(g)).count() >= 2
            })
            .collect();
        let rules = Rules::new(layout, board, opts, &wanted)?;
        let fences: Vec<Fence> = if opts.fences {
            layout
                .parts
                .iter()
                .filter(|p| crate::escape::is_bga(p))
                .map(|p| {
                    let mut lo = [f64::MAX; 2];
                    let mut hi = [f64::MIN; 2];
                    for q in p.pads.iter().flat_map(|q| q.outlines.iter().flatten()) {
                        lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                        hi = [hi[0].max(q[0]), hi[1].max(q[1])];
                    }
                    let mut nets: Vec<usize> = p.pads.iter().filter_map(|q| q.net).collect();
                    nets.sort_unstable();
                    nets.dedup();
                    Fence { nets, lo, hi }
                })
                .collect()
        } else {
            Vec::new()
        };
        let grid = Grid::build(layout, &rules, opts.grid, &fences);
        let zone = zone_map(layout, &grid);
        Ok(Base { rules, grid, islands, zone, nets })
    }

    pub fn terminals(&self, layout: &Layout, net: usize) -> (Vec<Terminals>, bool) {
        let c = conn::net_copper(layout, &self.islands, net);
        let mut out: Vec<Terminals> = vec![Vec::new(); c.groups];
        let mut island = false;
        for (it, &g) in c.items.iter().zip(&c.group) {
            match it {
                Item::Pad { centre, layers, .. } => out[g].push((*centre, layers.clone())),
                Item::Island { .. } => island = true,
                _ => {}
            }
        }
        (out.into_iter().filter(|g| !g.is_empty()).collect(), island)
    }

    pub fn pads(&self, layout: &Layout, net: usize) -> Vec<PadRef> {
        pads_of(&conn::net_copper(layout, &self.islands, net))
    }
}

pub fn route(layout: &Layout, board: &Board, opts: &Options) -> Result<RouteResult, String> {
    let base = Base::new(layout, board, opts)?;
    Ok(route_on(layout, &base, opts, &Guide::default()).result)
}

pub fn route_on(layout: &Layout, base: &Base, opts: &Options, guide: &Guide) -> Outcome {
    let t0 = Instant::now();
    let log = |m: String| {
        if opts.verbose || std::env::var("AGENTEE_ROUTE_DEBUG").is_ok() {
            eprintln!("[{:>6.1}s] {m}", t0.elapsed().as_secs_f64());
        }
    };
    let (rules, grid, islands, zone) = (&base.rules, &base.grid, &base.islands, &base.zone);
    let mut states: Vec<NetState> = Vec::new();
    for &n in &base.nets {
        let copper = conn::net_copper(layout, islands, n);
        let pad_groups = (0..copper.groups).filter(|&g| copper.has_pad(g)).count();
        if pad_groups < 2 {
            continue;
        }
        let pads = pads_of(&copper);
        let mut lo = [f64::MAX; 2];
        let mut hi = [f64::MIN; 2];
        for p in &pads {
            lo = [lo[0].min(p.centre[0]), lo[1].min(p.centre[1])];
            hi = [hi[0].max(p.centre[0]), hi[1].max(p.centre[1])];
        }
        let mut own = HashSet::new();
        let h = rules.rule(n).width.iter().map(|w| w / 2.0).fold(0.0, f64::max) + rules.slack;
        for p in &pads {
            grid.near(&Shape::Poly(p.outline.clone()), 0.0, |x, y, d| {
                if d < h {
                    for &l in &p.layers {
                        own.insert(grid.idx(l, x, y) as u32);
                    }
                }
            });
        }
        states.push(NetState {
            net: n,
            reach: 1.0,
            own,
            copper,
            pads,
            pieces: Vec::new(),
            fp: None,
            failed: Vec::new(),
            needed: pad_groups - 1,
            joined: 0,
            span: (hi[0] - lo[0]) + (hi[1] - lo[1]),
        });
    }
    let mut soft = Soft::new(grid, rules);
    log(format!(
        "grid {}x{}x{}, {} nets, {} connections, {} track buckets, {} via buckets, {} corridors",
        grid.w,
        grid.h,
        grid.nl,
        states.len(),
        states.iter().map(|s| s.needed).sum::<usize>(),
        rules.buckets.len(),
        rules.via_buckets.len(),
        guide.corridors.len()
    ));
    let mut order: Vec<usize> = (0..states.len()).collect();
    order.sort_by(|&a, &b| {
        let (ra, rb) = (rules.rule(states[a].net), rules.rule(states[b].net));
        rb.crit.total_cmp(&ra.crit).then(states[a].span.total_cmp(&states[b].span))
    });
    let plane = grid.plane();
    let mut pres = 0.5f32;
    let mut todo = order.clone();
    let mut conflicted: Vec<usize> = Vec::new();
    let mut overlap_cells: Vec<(usize, usize, P)> = Vec::new();
    let mut best = usize::MAX;
    let mut since = 0;
    let mut rounds = 0;
    for round in 0..opts.rounds.max(1) {
        rounds = round + 1;
        for &si in &todo {
            if let Some(fp) = states[si].fp.take() {
                soft.apply(&fp, false);
            }
            let env = Env { layout, grid, soft: &soft, rules, islands, opts, zone, guide };
            let (pieces, failed, joined) = route_net(&states[si], &env, pres, false);
            let st = &mut states[si];
            st.pieces = pieces;
            st.failed = failed;
            st.joined = joined;
            let rule = rules.rule(st.net);
            let fp = Soft::footprint(grid, rules, &Copper::of(&st.pieces, rule.clearance));
            soft.apply(&fp, true);
            st.fp = Some(fp);
        }
        conflicted.clear();
        overlap_cells.clear();
        let mut overlap = 0usize;
        for &si in &order {
            let cells = conflicts(&states[si], grid, &mut soft, rules);
            if !cells.is_empty() {
                overlap += cells.len();
                conflicted.push(si);
                for &(l, c) in &cells {
                    soft.hist[l * plane + c] += 0.4;
                    overlap_cells.push((states[si].net, l, grid.center(c % grid.w, c / grid.w)));
                }
            }
        }
        let joined: usize = states.iter().map(|s| s.joined).sum();
        let needed: usize = states.iter().map(|s| s.needed).sum();
        log(format!(
            "round {round}: rerouted {}, {joined} of {needed} joined, {} nets overlap ({overlap} cells), pres {pres:.2}",
            todo.len(),
            conflicted.len()
        ));
        if conflicted.is_empty() {
            break;
        }
        if (overlap as f64) < best as f64 * 0.95 {
            best = overlap;
            since = 0;
        } else {
            since += 1;
            if since >= opts.stall.max(1) {
                break;
            }
        }
        if round >= 3 {
            for &si in &conflicted {
                states[si].reach = (states[si].reach * 1.5).min(4.0);
            }
        }
        pres = (pres * 1.8).min(100.0);
        todo = conflicted.clone();
    }
    let overlap_left = conflicted.len();
    if !conflicted.is_empty() {
        for &si in &conflicted {
            if let Some(fp) = states[si].fp.take() {
                soft.apply(&fp, false);
            }
            states[si].pieces.clear();
        }
        for &si in &conflicted {
            let env = Env { layout, grid, soft: &soft, rules, islands, opts, zone, guide };
            let (pieces, failed, joined) = route_net(&states[si], &env, pres, true);
            let st = &mut states[si];
            st.pieces = pieces;
            st.failed = failed;
            st.joined = joined;
            let rule = rules.rule(st.net);
            let fp = Soft::footprint(grid, rules, &Copper::of(&st.pieces, rule.clearance));
            soft.apply(&fp, true);
            st.fp = Some(fp);
        }
        let joined: usize = states.iter().map(|s| s.joined).sum();
        let lost: Vec<&str> = conflicted
            .iter()
            .filter(|&&si| states[si].joined < states[si].needed)
            .map(|&si| layout.nets[states[si].net].name.as_str())
            .collect();
        log(format!(
            "hard pass over {} nets, {joined} joined, {} nets short of a route: {}",
            conflicted.len(),
            lost.len(),
            lost.join(" ")
        ));
    }
    let mut out = RouteResult::default();
    for st in &states {
        let name = layout.nets[st.net].name.clone();
        out.connections += st.needed;
        out.routed += st.joined;
        for p in &st.pieces {
            for r in &p.tracks {
                if r.points.len() < 2 {
                    continue;
                }
                out.tracks.push(RoutedTrack {
                    net: name.clone(),
                    layer: layout.copper[r.layer].clone(),
                    width: r.neck.then_some(r.width),
                    points: r.points.clone(),
                });
            }
            for &(at, k) in &p.vias {
                out.vias.push(RoutedVia { net: name.clone(), at, via: rules.vias[k].name.clone() });
            }
        }
        out.failed.extend(st.failed.iter().cloned());
    }
    log(format!("{} of {} connections", out.routed, out.connections));
    Outcome { result: out, overlap: overlap_cells, rounds, overlap_left }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct PadAccess {
    pub net: String,
    pub at: P,
    pub exits: usize,
    pub vias: usize,
}

pub fn access_report(layout: &Layout, base: &Base, opts: &Options) -> Vec<PadAccess> {
    let soft = Soft::new(&base.grid, &base.rules);
    let own = HashSet::new();
    let mut out = Vec::new();
    for &n in &base.nets {
        let rule = base.rules.rule(n);
        let pads = base.pads(layout, n);
        let entry: Vec<P> = pads
            .iter()
            .filter(|p| p.layers.iter().any(|&l| !rule.track[l]))
            .map(|p| p.centre)
            .collect();
        let ctx = Ctx {
            grid: &base.grid,
            soft: &soft,
            rules: &base.rules,
            rule,
            net: n as u16,
            entry: &entry,
            entry_r: opts.entry,
            own: &own,
        };
        for p in &pads {
            let mut exits = 0;
            let mut vias = 0;
            for &l in &p.layers {
                if rule.track[l] || !entry.is_empty() {
                    exits += ctx.necks(p, l).len();
                }
                vias += ctx
                    .stubs(p, l, false, opts.via_cost, 0.0)
                    .iter()
                    .filter(|a| a.via.is_some())
                    .count();
            }
            out.push(PadAccess { net: layout.nets[n].name.clone(), at: p.centre, exits, vias });
        }
    }
    out
}

fn lean_mask(env: &Env, net: usize, win: &Window) -> Option<Vec<i8>> {
    let g = env.guide;
    let corridor = g.corridors.get(&net);
    let prefer = g.prefer.get(&net);
    let vias = g.prefer_vias.get(&net);
    if corridor.is_none() && prefer.is_none() && vias.is_none() {
        return None;
    }
    let grid = env.grid;
    let (ww, wh) = (win.ww(), win.wh());
    let area = ww * wh;
    let mut out = vec![0i8; area * grid.nl];
    if let Some(cor) = corridor.filter(|_| g.tile > 0.0) {
        let lo = g.tile_of(grid.center(win.x0, win.y0));
        let hi = g.tile_of(grid.center(win.x1, win.y1));
        let tw = (hi.0 - lo.0 + 1) as usize;
        let th = (hi.1 - lo.1 + 1) as usize;
        for l in 0..grid.nl {
            let mut near = vec![false; tw * th];
            for ty in 0..th {
                for tx in 0..tw {
                    let (x, y) = (lo.0 + tx as i64, lo.1 + ty as i64);
                    near[ty * tw + tx] =
                        (-1..=1).any(|dy| (-1..=1).any(|dx| cor.contains(&(l, x + dx, y + dy))));
                }
            }
            for y in 0..wh {
                for x in 0..ww {
                    let t = g.tile_of(grid.center(x + win.x0, y + win.y0));
                    let k = (t.1 - lo.1) as usize * tw + (t.0 - lo.0) as usize;
                    if !near[k] {
                        out[l * area + y * ww + x] = 1;
                    }
                }
            }
        }
    }
    let mut mark = |l: usize, x: usize, y: usize| {
        if win.contains(x, y) {
            out[l * area + (y - win.y0) * ww + (x - win.x0)] = -1;
        }
    };
    for &(l, a, b) in prefer.into_iter().flatten() {
        for (x, y) in seg_cells(grid, a, b) {
            mark(l, x, y);
        }
    }
    for &at in vias.into_iter().flatten() {
        let (x, y) = grid.cell(at);
        for l in 0..grid.nl {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if grid.inside(x + dx, y + dy) {
                        mark(l, (x + dx) as usize, (y + dy) as usize);
                    }
                }
            }
        }
    }
    Some(out)
}

fn zone_map(layout: &Layout, grid: &Grid) -> Vec<u16> {
    let plane = grid.plane();
    let mut zone = vec![u16::MAX; plane * grid.nl];
    let nl = grid.nl;
    for z in &layout.zones {
        let Some(l) = layout.copper.iter().position(|c| *c == z.layer) else { continue };
        if l == 0 || l + 1 == nl {
            continue;
        }
        let lo = z.origin;
        let hi = [lo[0] + z.width as f64 * z.cell, lo[1] + z.height as f64 * z.cell];
        let (x0, y0, x1, y1) = grid.span(lo, hi, 0.0);
        if x1 == usize::MAX || y1 == usize::MAX {
            continue;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                if z.filled(grid.center(x, y)) {
                    zone[l * plane + y * grid.w + x] = z.net as u16;
                }
            }
        }
    }
    zone
}

fn conflicts(st: &NetState, grid: &Grid, soft: &mut Soft, rules: &Rules) -> Vec<(usize, usize)> {
    let Some(fp) = st.fp.as_ref() else { return Vec::new() };
    soft.apply(fp, false);
    let rule = rules.rule(st.net);
    let mut out = Vec::new();
    for p in &st.pieces {
        for r in &p.tracks {
            let Some(b) = rule.bucket[r.layer] else { continue };
            for w in r.points.windows(2) {
                for (x, y) in seg_cells(grid, w[0], w[1]) {
                    let c = y * grid.w + x;
                    if soft.tracks[b][c] > 0
                        && !st.own.contains(&((r.layer * grid.plane() + c) as u32))
                    {
                        out.push((r.layer, c));
                    }
                }
            }
        }
        for &(at, k) in &p.vias {
            let Some(vi) = rule.vias.iter().position(|&v| v == k) else { continue };
            let (x, y) = grid.cell(at);
            if !grid.inside(x, y) {
                continue;
            }
            let c = y as usize * grid.w + x as usize;
            if soft.vias[rule.via_bucket[vi]][c] > 0 {
                for &l in &rules.vias[k].layers {
                    out.push((l, c));
                }
            }
        }
    }
    soft.apply(fp, true);
    out.sort_unstable();
    out.dedup();
    out
}

fn piece_points(p: &Piece) -> Vec<P> {
    let mut out: Vec<P> = p.vias.iter().map(|v| v.0).collect();
    for r in &p.tracks {
        out.extend(r.points.iter().copied());
    }
    out
}

fn route_net(
    st: &NetState,
    env: &Env,
    pres: f32,
    hard: bool,
) -> (Vec<Piece>, Vec<Unrouted>, usize) {
    let nc = &st.copper;
    let name = &env.layout.nets[st.net].name;
    let pad_groups: Vec<usize> = (0..nc.groups).filter(|&g| nc.has_pad(g)).collect();
    let size = |g: usize| nc.group.iter().filter(|&&k| k == g).count();
    let Some(&start) =
        pad_groups.iter().max_by_key(|&&g| (!nc.islands(g).is_empty(), size(g), usize::MAX - g))
    else {
        return (Vec::new(), Vec::new(), 0);
    };
    let points: Vec<Vec<P>> = (0..nc.groups).map(|g| nc.points(g)).collect();
    let mut tree = vec![start];
    let mut rest: Vec<usize> = pad_groups.iter().copied().filter(|&g| g != start).collect();
    let mut best: Vec<(f64, P, P)> = rest
        .iter()
        .map(|&g| {
            conn::group_distance(
                env.layout,
                env.islands,
                nc,
                &points[g],
                &points[start],
                &nc.islands(start),
            )
        })
        .collect();
    let mut pieces: Vec<Piece> = Vec::new();
    let mut failed = Vec::new();
    let mut joined = 0;
    while !rest.is_empty() {
        let k = (0..rest.len()).min_by(|&i, &j| best[i].0.total_cmp(&best[j].0)).unwrap();
        let g = rest.remove(k);
        let (_, a, b) = best.remove(k);
        match connect(st, env, g, &tree, &pieces, a, b, pres, hard) {
            Ok(piece) => {
                let new_pts: Vec<P> =
                    points[g].iter().copied().chain(piece_points(&piece)).collect();
                let isl = nc.islands(g);
                for (i, &r) in rest.iter().enumerate() {
                    let d = conn::group_distance(
                        env.layout,
                        env.islands,
                        nc,
                        &points[r],
                        &new_pts,
                        &isl,
                    );
                    if d.0 < best[i].0 {
                        best[i] = d;
                    }
                }
                pieces.push(piece);
                tree.push(g);
                joined += 1;
            }
            Err(reason) => failed.push(Unrouted { net: name.clone(), from: a, to: b, reason }),
        }
    }
    (pieces, failed, joined)
}

#[allow(clippy::too_many_arguments)]
fn connect(
    st: &NetState,
    env: &Env,
    g: usize,
    tree: &[usize],
    pieces: &[Piece],
    a: P,
    b: P,
    pres: f32,
    hard: bool,
) -> Result<Piece, String> {
    let grid = env.grid;
    let rule = env.rules.rule(st.net);
    let net = st.net as u16;
    let nc = &st.copper;
    let lo = [a[0].min(b[0]), a[1].min(b[1])];
    let hi = [a[0].max(b[0]), a[1].max(b[1])];
    let guided = env.guide.corridors.get(&st.net).is_some_and(|c| !c.is_empty());
    let mut margin = if guided {
        (2.0 * env.guide.tile).max(env.opts.margin * 0.5) * st.reach
    } else {
        env.opts.margin * st.reach
    };
    let mut tries = 0;
    loop {
        let win = Window::around(grid, lo, hi, margin);
        if !win.fits() {
            if win.is_whole(grid) {
                return Err("the window is too large for the grid".into());
            }
            margin *= 3.0;
            continue;
        }
        let entry: Vec<P> = st
            .pads
            .iter()
            .filter(|p| p.layers.iter().any(|&l| !rule.track[l]))
            .map(|p| p.centre)
            .filter(|c| {
                let (x, y) = grid.cell(*c);
                grid.inside(x, y) && win.contains(x as usize, y as usize)
            })
            .collect();
        let ctx = Ctx {
            grid,
            soft: env.soft,
            rules: env.rules,
            rule,
            net,
            entry: &entry,
            entry_r: env.opts.entry,
            own: &st.own,
        };
        let mut access: Vec<Access> = Vec::new();
        let mut sources: Vec<Source> = Vec::new();
        let mut targets: Vec<(usize, u32)> = Vec::new();
        let legal = |i3: usize| -> bool {
            let l = i3 / grid.plane();
            let c2 = i3 % grid.plane();
            let at = grid.center(c2 % grid.w, c2 / grid.w);
            let planar =
                rule.track[l] || entry.iter().any(|e| geom::dist(*e, at) <= env.opts.entry);
            planar
                && grid.track_ok(i3, net, rule.need[l])
                && (!hard
                    || st.own.contains(&(i3 as u32))
                    || rule.bucket[l].is_none_or(|bk| env.soft.tracks[bk][c2] == 0))
        };
        for (i, it) in nc.items.iter().enumerate() {
            let mine = nc.group[i] == g;
            let theirs = tree.contains(&nc.group[i]);
            if !mine && !theirs {
                continue;
            }
            let cells = item_cells(env, it, &win);
            if let Item::Pad { shapes, layers, centre, pitch } = it {
                let (x, y) = grid.cell(*centre);
                let seen = grid.inside(x, y) && win.contains(x as usize, y as usize);
                for &l in layers.iter().filter(|_| seen) {
                    let any = cells.iter().any(|&c| c / grid.plane() == l && legal(c));
                    for s in shapes {
                        let Shape::Poly(v) = s else { continue };
                        let wide = rule.width[l] > geom::min_extent(v) + 1e-9;
                        if any && !wide && pitch.is_none() && rule.track[l] {
                            continue;
                        }
                        let pad = PadRef {
                            layers: layers.clone(),
                            outline: v.clone(),
                            centre: *centre,
                            pitch: *pitch,
                        };
                        let mut found: Vec<(Access, Vec<usize>)> = Vec::new();
                        if !any && (rule.track[l] || !entry.is_empty()) {
                            for nk in ctx.necks(&pad, l) {
                                let (x, y) = grid.cell(nk.to);
                                if !grid.inside(x, y) || !win.contains(x as usize, y as usize) {
                                    continue;
                                }
                                let i3 = grid.idx(l, x as usize, y as usize);
                                let cost = geom::dist(nk.from, nk.to) as f32;
                                found.push((Access { neck: Some(nk), via: None, cost }, vec![i3]));
                            }
                        }
                        for a in ctx.stubs(&pad, l, hard, env.opts.via_cost, pres) {
                            let Some((at, k)) = a.via else { continue };
                            let (x, y) = grid.cell(at);
                            if !grid.inside(x, y) || !win.contains(x as usize, y as usize) {
                                continue;
                            }
                            let cells: Vec<usize> = env.rules.vias[k]
                                .layers
                                .iter()
                                .filter(|&&m| m != l)
                                .map(|&m| grid.idx(m, x as usize, y as usize))
                                .collect();
                            found.push((a, cells));
                        }
                        for (a, cells) in found {
                            let cost = a.cost;
                            let landed = a.via.is_some();
                            access.push(a);
                            let tag = access.len() as u32;
                            for i3 in cells {
                                if mine {
                                    sources.push(Source {
                                        at: i3,
                                        cost,
                                        tag,
                                        via_only: false,
                                        landed,
                                    });
                                } else {
                                    targets.push((i3, tag));
                                }
                            }
                        }
                    }
                }
            }
            for c in cells {
                if mine {
                    sources.push(Source {
                        at: c,
                        cost: 0.0,
                        tag: 0,
                        via_only: !legal(c),
                        landed: false,
                    });
                } else {
                    targets.push((c, 0));
                }
            }
        }
        for p in pieces {
            for c in piece_cells(env, p, &win) {
                targets.push((c, 0));
                if targets.len() > MAX_TARGETS {
                    break;
                }
            }
            if targets.len() > MAX_TARGETS {
                break;
            }
        }
        if targets.len() > MAX_TARGETS {
            let keep = (targets.len() / MAX_TARGETS).max(1);
            let thin: Vec<(usize, u32)> = targets
                .into_iter()
                .enumerate()
                .filter(|(i, _)| i % keep == 0)
                .map(|(_, t)| t)
                .collect();
            targets = thin;
        }
        let source_cells: std::collections::HashSet<usize> = sources.iter().map(|s| s.at).collect();
        targets.retain(|(c, tag)| *tag != 0 || !source_cells.contains(c));
        if sources.is_empty() {
            if win.is_whole(grid) {
                return Err("an end has no copper on the routing layers".into());
            }
            margin *= 3.0;
            continue;
        }
        let lean = lean_mask(env, st.net, &win);
        let q = Query {
            grid,
            soft: env.soft,
            rules: env.rules,
            rule,
            net,
            window: win,
            sources,
            targets,
            entry: entry.clone(),
            entry_r: env.opts.entry,
            pres,
            hard,
            via_cost: env.opts.via_cost,
            bend_cost: env.opts.bend_cost,
            zone: env.zone,
            zone_cost: env.opts.zone_cost,
            extra: lean.as_deref(),
            outside: env.opts.corridor_cost as f32,
            gain: env.opts.prefer_gain as f32,
            own: &st.own,
        };
        tries += 1;
        if let Some(found) = q.run() {
            if found.cells.len() <= 1 {
                margin *= 3.0;
                if win.is_whole(grid) || tries > env.opts.escalate {
                    return Err("the groups already touch".into());
                }
                continue;
            }
            return Ok(ctx.piece(&found, &st.pads, &access));
        }
        if win.is_whole(grid) || tries > env.opts.escalate {
            return Err("no path within the rules".into());
        }
        margin *= 3.0;
    }
}

fn shape_cells(grid: &Grid, shape: &Shape, layers: &[usize], win: &Window, out: &mut Vec<usize>) {
    let before = out.len();
    grid.near(shape, 0.0, |x, y, _| {
        if win.contains(x, y) {
            for &l in layers {
                out.push(grid.idx(l, x, y));
            }
        }
    });
    if out.len() == before {
        let (lo, hi) = shape.bounds();
        let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        let (x, y) = grid.cell(c);
        if grid.inside(x, y) && win.contains(x as usize, y as usize) {
            for &l in layers {
                out.push(grid.idx(l, x as usize, y as usize));
            }
        }
    }
}

fn item_cells(env: &Env, it: &Item, win: &Window) -> Vec<usize> {
    let grid = env.grid;
    let mut out = Vec::new();
    match it {
        Item::Pad { shapes, layers, .. } => {
            for s in shapes {
                shape_cells(grid, s, layers, win, &mut out);
            }
        }
        Item::Seg { layer, shape } => shape_cells(grid, shape, &[*layer], win, &mut out),
        Item::Via { at, r, layers } => {
            shape_cells(grid, &Shape::Circle(*at, *r), layers, win, &mut out)
        }
        Item::Island { zone, label, layer } => {
            for y in win.y0..=win.y1 {
                for x in win.x0..=win.x1 {
                    if env.islands.at(env.layout, *zone, grid.center(x, y)) == Some(*label) {
                        out.push(grid.idx(*layer, x, y));
                    }
                }
            }
            let last = out.len().saturating_sub(1);
            if out.len() > MAX_TARGETS / 8 {
                let keep = (out.len() / (MAX_TARGETS / 8)).max(1);
                let thin: Vec<usize> = out
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| i % keep == 0 || *i == last)
                    .map(|(_, c)| c)
                    .collect();
                return thin;
            }
        }
    }
    out
}

fn piece_cells(env: &Env, p: &Piece, win: &Window) -> Vec<usize> {
    let grid = env.grid;
    let mut out = Vec::new();
    for r in &p.tracks {
        for w in r.points.windows(2) {
            shape_cells(grid, &Shape::Seg(w[0], w[1], r.width / 2.0), &[r.layer], win, &mut out);
        }
    }
    for &(at, k) in &p.vias {
        let o = &env.rules.vias[k];
        shape_cells(grid, &Shape::Circle(at, o.r), &o.layers, win, &mut out);
    }
    out
}
