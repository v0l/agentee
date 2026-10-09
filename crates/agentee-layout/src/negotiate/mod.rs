mod conn;
mod grid;
mod pair;
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
pub use grid::{Fence, Grid, Shape, ball_phase};
pub use rules::{Need, NetRule, Rules, um};
use search::{Query, Source, Window, seg_cells};
pub use shape::{Access, Ctx, PadRef, Seg};
pub use spread::{illegal, spread};

const MAX_TARGETS: usize = 3_000_000;
const DEFERRED: &str = "deferred";
const MAX_PRES: f32 = 100.0;
const RESERVE_COST: f32 = 2.0;
const BATCH: usize = 32;
const RIP_PASSES: usize = 3;
const RIP_BLOCKERS: usize = 3;
const CLASH_HALO: i64 = 0;
const LOST_SHOWN: usize = 6;
const HIST_STEP: f32 = 0.4;
const SETTLED: usize = 2;
const VIA_PASSES: usize = 2;
const VIA_DEARER: f64 = 4.0;
const DEAD: &str = "no path within the rules in an earlier round";
use soft::{Copper, Footprint, Piece, Soft};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Options {
    pub nets: Vec<String>,
    pub layers: Vec<String>,
    pub via: Vec<String>,
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
    pub pour_joins: bool,
    pub couple: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            nets: Vec::new(),
            layers: Vec::new(),
            via: Vec::new(),
            grid: 0.05,
            via_cost: 3.0,
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
            pour_joins: true,
            couple: true,
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
    pub hist: Option<Vec<f32>>,
    pub warm: Option<Warm>,
}

type WarmNet = (Vec<Piece>, Vec<Unrouted>, HashSet<usize>);

#[derive(Clone, Debug, Default)]
pub struct Warm {
    nets: HashMap<usize, WarmNet>,
}

#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub warm: Warm,
    pub spend: Spend,
    pub hist: Vec<f32>,
    pub result: RouteResult,
    pub overlap: Vec<(usize, usize, P)>,
    pub rounds: usize,
    pub overlap_left: usize,
}

type Conflicts = (Vec<(usize, usize)>, Vec<bool>);
pub type Terminals = Vec<(P, Vec<usize>)>;

struct NetState {
    net: usize,
    fence: Option<Vec<bool>>,
    dead: HashSet<usize>,
    bad: Option<Vec<bool>>,
    clash: Vec<u64>,
    wide: bool,
    pops: u64,
    spent_ms: f64,
    tries: usize,
    own: HashSet<u32>,
    copper: NetCopper,
    pads: Vec<PadRef>,
    pieces: Vec<Piece>,
    fp: Option<Footprint>,
    failed: Vec<Unrouted>,
    needed: usize,
    joined: usize,
    span: f64,
    mate: Option<(usize, usize)>,
}

struct Env<'a> {
    layout: &'a Layout,
    grid: &'a Grid,
    soft: &'a Soft,
    rules: &'a Rules,
    islands: &'a Islands,
    opts: &'a Options,
    zone: &'a [u16],
    reserve: &'a [u16],
    guide: &'a Guide,
    escalate: usize,
    tiles: &'a Tiles,
    fenced: bool,
    jacobi: bool,
}

struct Tiles {
    size: f64,
    w: usize,
    h: usize,
    tx: Vec<usize>,
    ty: Vec<usize>,
    lo: (i64, i64),
    origin: P,
}

impl Tiles {
    fn new(grid: &Grid, guide: &Guide) -> Tiles {
        let (size, origin) =
            if guide.tile > 0.0 { (guide.tile, guide.origin) } else { (1.0, [grid.x0, grid.y0]) };
        let t = |v: f64, o: f64| ((v - o) / size).floor() as i64;
        let lo = (t(grid.x0, origin[0]), t(grid.y0, origin[1]));
        let tx: Vec<usize> =
            (0..grid.w).map(|x| (t(grid.center(x, 0)[0], origin[0]) - lo.0) as usize).collect();
        let ty: Vec<usize> =
            (0..grid.h).map(|y| (t(grid.center(0, y)[1], origin[1]) - lo.1) as usize).collect();
        let w = tx.last().map_or(1, |v| v + 1);
        let h = ty.last().map_or(1, |v| v + 1);
        Tiles { size, w, h, tx, ty, lo, origin }
    }

    fn of(&self, p: P) -> (i64, i64) {
        (
            ((p[0] - self.origin[0]) / self.size).floor() as i64 - self.lo.0,
            ((p[1] - self.origin[1]) / self.size).floor() as i64 - self.lo.1,
        )
    }

    fn mark(&self, out: &mut [bool], lo: P, hi: P) {
        let (a, b) = (self.of(lo), self.of(hi));
        for y in a.1.max(0)..=b.1.min(self.h as i64 - 1) {
            for x in a.0.max(0)..=b.0.min(self.w as i64 - 1) {
                out[y as usize * self.w + x as usize] = true;
            }
        }
    }

    fn grow(&self, v: &[bool], d: i64) -> Vec<bool> {
        let mut out = v.to_vec();
        for y in 0..self.h as i64 {
            for x in 0..self.w as i64 {
                if !v[y as usize * self.w + x as usize] {
                    continue;
                }
                for yy in (y - d).max(0)..=(y + d).min(self.h as i64 - 1) {
                    for xx in (x - d).max(0)..=(x + d).min(self.w as i64 - 1) {
                        out[yy as usize * self.w + xx as usize] = true;
                    }
                }
            }
        }
        out
    }
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
        let planes = agentee_core::tie::plane_nets(layout);
        let wanted: Vec<usize> = (0..layout.nets.len())
            .filter(|&n| {
                let name = &layout.nets[n].name;
                opts.nets.is_empty()
                    || opts.nets.iter().any(|g| g == name)
                    || (!planes.contains(&n) && opts.nets.iter().any(|g| glob(g, name)))
            })
            .collect();
        if wanted.is_empty() {
            return Err("no net matches".into());
        }
        let islands = Islands::new(layout, &wanted, opts.pour_joins);
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
        let grid = Grid::build(layout, board, &rules, opts.grid, &fences);
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
            fence: None,
            dead: HashSet::new(),
            bad: None,
            clash: Vec::new(),
            wide: false,
            pops: 0,
            spent_ms: 0.0,
            tries: 0,
            own,
            copper,
            pads,
            pieces: Vec::new(),
            fp: None,
            failed: Vec::new(),
            needed: pad_groups - 1,
            joined: 0,
            span: (hi[0] - lo[0]) + (hi[1] - lo[1]),
            mate: None,
        });
    }
    let mut soft = Soft::new(grid, rules);
    for shadow in [true, false] {
        let segs: Vec<(usize, P, P, f64)> = layout
            .tracks
            .iter()
            .filter(|t| rules.casts.get(t.net).copied().unwrap_or(false) == shadow)
            .filter_map(|t| {
                let l = layout.copper.iter().position(|c| *c == t.layer)?;
                Some(t.points.windows(2).map(move |w| (l, w[0], w[1], t.width / 2.0)))
            })
            .flatten()
            .filter(|s| shadow || !rules.cut[s.0].is_empty())
            .collect();
        if !segs.is_empty() {
            let c = Copper {
                segs,
                vias: Vec::new(),
                clearance: 0.0,
                shadow,
                only_shadow: true,
                domain: None,
            };
            soft.apply(&Soft::footprint(grid, rules, &c), true);
        }
    }
    if let Some(h) = guide.hist.as_ref().filter(|h| h.len() == soft.hist.len()) {
        soft.hist.clone_from(h);
    }
    let mut cold: Vec<bool> = vec![true; states.len()];
    let mut couples = pair::Couples::new(&mut states, rules);
    if let Some(w) = &guide.warm {
        for (si, st) in states.iter_mut().enumerate() {
            let Some((pieces, failed, dead)) = w.nets.get(&st.net) else { continue };
            st.pieces = pieces.iter().map(|p| Piece { ms: 0.0, ..p.clone() }).collect();
            st.failed = failed.clone();
            st.dead = dead.clone();
            st.joined = st.needed.saturating_sub(st.failed.len());
            let rule = rules.rule(st.net);
            let fp = Soft::footprint(grid, rules, &Copper::of(&st.pieces, rule, &st.pads));
            soft.apply(&fp, true);
            st.fp = Some(fp);
            cold[si] = !st.failed.is_empty();
        }
    }
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
    let hist_step = HIST_STEP;
    let tiles = Tiles::new(grid, guide);
    let reserve = reserve_map(grid, rules, guide);
    let mut seen: HashMap<usize, (u64, usize)> = HashMap::new();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(64);
    let mut todo: Vec<usize> = order.iter().copied().filter(|&si| cold[si]).collect();
    let mut conflicted: Vec<usize> = Vec::new();
    let mut overlap_cells: Vec<(usize, usize, P)> = Vec::new();
    let mut best = usize::MAX;
    let mut since = 0;
    let mut rounds = 0;
    let mut spend = Spend { threads, setup_ms: ms(t0), ..Default::default() };
    let mut last_gain = 0;
    for round in 0..opts.rounds.max(1) {
        rounds = round + 1;
        let tr = Instant::now();
        let env = Env {
            layout,
            grid,
            soft: &soft,
            rules,
            islands,
            opts,
            zone,
            reserve: &reserve,
            guide,
            escalate: opts.escalate,
            tiles: &tiles,
            fenced: false,
            jacobi: false,
        };
        couples.place(&mut states, &mut todo, &env, pres, false);
        let many = route_many(&mut states, &todo, &env, pres, false, threads);
        spend.add(&many, layout);
        conflicted.clear();
        overlap_cells.clear();
        let mut overlap = 0usize;
        let mut settled = Vec::new();
        let found = all_conflicts(&states, &order, grid, &soft, rules, threads);
        for (&si, (cells, bad)) in order.iter().zip(found) {
            states[si].bad = Some(bad);
            states[si].clash = clash_tiles(&tiles, grid, &cells);
            if !cells.is_empty() {
                let sig = {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    cells.hash(&mut h);
                    h.finish()
                };
                let e = seen.entry(si).or_insert((0, 0));
                e.1 = if e.0 == sig { e.1 + 1 } else { 0 };
                e.0 = sig;
                if pres >= MAX_PRES && e.1 >= SETTLED {
                    settled.push(si);
                }
                overlap += cells.len();
                conflicted.push(si);
                for &(l, c) in &cells {
                    soft.hist[l * plane + c] += hist_step;
                    overlap_cells.push((states[si].net, l, grid.center(c % grid.w, c / grid.w)));
                }
            }
        }
        let joined: usize = states.iter().map(|s| s.joined).sum();
        let needed: usize = states.iter().map(|s| s.needed).sum();
        log(format!(
            "round {round}: rerouted {}, {joined} of {needed} joined, {} nets overlap ({overlap} cells), pres {pres:.2}, {:.0} ms, {:.0} ms of work, {:.0} ms on the longest chain, {} groups, {} redone outside their fence",
            todo.len(),
            conflicted.len(),
            ms(tr),
            many.work_ms,
            many.critical_ms,
            many.groups,
            many.again
        ));
        spend.rounds.push(RoundSpend {
            nets: todo.len(),
            wall_ms: ms(tr),
            work_ms: many.work_ms,
            critical_ms: many.critical_ms,
            joined,
            overlap_nets: conflicted.len(),
            overlap_cells: overlap,
        });
        if conflicted.is_empty() {
            last_gain = round;
            break;
        }
        if (overlap as f64) < best as f64 * 0.95 {
            best = overlap;
            since = 0;
            last_gain = round;
        } else if pres >= MAX_PRES {
            since += 1;
            if since >= opts.stall.max(1) {
                break;
            }
        }
        pres = (pres * 1.8).min(MAX_PRES);
        todo = conflicted.iter().copied().filter(|si| !settled.contains(si)).collect();
        if todo.is_empty() {
            break;
        }
    }
    let overlap_left = conflicted.len();
    spend.tail_ms = spend.rounds.iter().skip(last_gain + 1).map(|r| r.wall_ms).sum();
    spend.stuck_ms = conflicted.iter().map(|&si| states[si].spent_ms).sum();
    for st in &states {
        for f in &st.failed {
            log(format!("  soft failure {} at {:?}: {}", f.net, f.from, f.reason));
        }
    }
    if std::env::var("AGENTEE_ROUTE_DEBUG").is_ok_and(|v| v == "3") {
        let mut top: Vec<&NetState> = states.iter().collect();
        top.sort_by(|a, b| b.spent_ms.total_cmp(&a.spent_ms));
        for st in top.iter().take(25) {
            log(format!(
                "  {}: {:.0} ms in {} routes, {} pops in the last, {} pads, fence {} tiles",
                layout.nets[st.net].name,
                st.spent_ms,
                st.tries,
                st.pops,
                st.pads.len(),
                st.fence.as_ref().map(|f| f.iter().filter(|v| **v).count()).unwrap_or(0)
            ));
        }
    }
    if std::env::var("AGENTEE_ROUTE_DEBUG").is_ok_and(|v| v == "2") {
        for &si in &conflicted {
            let (cells, _) = conflicts(&states, si, grid, &soft, rules);
            let rule = rules.rule(states[si].net);
            let mut with: BTreeMap<&str, usize> = BTreeMap::new();
            for &(l, c) in &cells {
                let Some(b) = rule.bucket[l] else { continue };
                for (sj, o) in states.iter().enumerate() {
                    if sj == si {
                        continue;
                    }
                    if o.fp.as_ref().is_some_and(|fp| fp.0[b].binary_search(&(c as u32)).is_ok()) {
                        *with.entry(layout.nets[o.net].name.as_str()).or_default() += 1;
                    }
                }
            }
            let at = cells.first().map(|&(l, c)| (l, grid.center(c % grid.w, c / grid.w)));
            for p in &states[si].pieces {
                for r in &p.tracks {
                    let hit = r.points.windows(2).any(|w| {
                        seg_cells(grid, w[0], w[1])
                            .iter()
                            .any(|&(x, y)| cells.contains(&(r.layer, y * grid.w + x)))
                    });
                    if hit {
                        eprintln!("    run neck={} w={} {:?}", r.neck, r.width, r.points);
                    }
                }
            }
            eprintln!(
                "  {} {} cells at {:?} with {:?}",
                layout.nets[states[si].net].name,
                cells.len(),
                at,
                with
            );
        }
    }
    let th = Instant::now();
    if !conflicted.is_empty() && std::env::var("AGENTEE_ROUTE_SOFT").is_err() {
        for &si in &conflicted {
            if let Some(fp) = states[si].fp.take() {
                soft.apply(&fp, false);
            }
            let keep = kept_footprint(&states[si], rules, grid);
            if let Some(fp) = &keep {
                soft.apply(fp, true);
            } else {
                states[si].bad = None;
            }
            states[si].fp = keep;
        }
        let env = Env {
            layout,
            grid,
            soft: &soft,
            rules,
            islands,
            opts,
            zone,
            reserve: &reserve,
            guide,
            escalate: opts.escalate,
            tiles: &tiles,
            fenced: false,
            jacobi: false,
        };
        couples.place(&mut states, &mut conflicted, &env, MAX_PRES, true);
        let many = route_many(&mut states, &conflicted, &env, pres, true, threads);
        spend.add(&many, layout);
        let (mut dropped, mut dropped_ms) =
            drop_overlaps(&mut states, &order, grid, &soft, rules, layout);
        if std::env::var("AGENTEE_NORIP").is_err() {
            let tr = Instant::now();
            let (mut tried, mut won) = (0, 0);
            let mut lost = HashSet::new();
            for _ in 0..RIP_PASSES {
                let (t, w) = rip_and_retry(&mut states, &order, &env, MAX_PRES, threads, &mut lost);
                tried += t;
                won += w;
                if w == 0 {
                    break;
                }
            }
            let (d2, m2) = drop_overlaps(&mut states, &order, grid, &soft, rules, layout);
            dropped += d2;
            dropped_ms += m2;
            let uncoupled = pair::uncouple_short(&mut states, &couples, &env);
            if uncoupled > 0 {
                let (d3, m3) = drop_overlaps(&mut states, &order, grid, &soft, rules, layout);
                dropped += d3;
                dropped_ms += m3;
                log(format!("{uncoupled} connections won by routing short pairs uncoupled"));
            }
            log(format!(
                "rip-up and retry: {tried} short nets tried, {won} connections won, {:.0} ms",
                ms(tr)
            ));
        }
        spend.dropped_ms = dropped_ms;
        if dropped > 0 {
            log(format!("{dropped} pieces dropped, still overlapping after the hard pass"));
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
    spend.hard_ms = ms(th);
    if std::env::var("AGENTEE_ROUTE_SOFT").is_err() {
        let tv = Instant::now();
        let dear = Options {
            via_cost: opts.via_cost * VIA_DEARER,
            zone_cost: opts.zone_cost * VIA_DEARER,
            ..opts.clone()
        };
        let env = Env {
            layout,
            grid,
            soft: &soft,
            rules,
            islands,
            opts: &dear,
            zone,
            reserve: &reserve,
            guide,
            escalate: opts.escalate,
            tiles: &tiles,
            fenced: false,
            jacobi: false,
        };
        let mut saved = 0;
        for _ in 0..VIA_PASSES {
            let won = fewer_vias(&mut states, &order, &env);
            saved += won;
            if won == 0 {
                break;
            }
        }
        log(format!("{saved} vias saved by rerouting with dearer vias, {:.0} ms", ms(tv)));
    }
    spend.final_ms = states.iter().flat_map(|s| &s.pieces).map(|p| p.ms).sum();
    spend.wall_ms = ms(t0);
    spend.lost.sort_by(|a, b| b.1.total_cmp(&a.1));
    spend.lost.truncate(LOST_SHOWN);
    log(spend.summary());

    for st in states.iter_mut() {
        pair::trim_runs(st);
    }
    let mut out = RouteResult::default();
    for st in &states {
        let name = layout.nets[st.net].name.clone();
        out.connections += st.needed;
        out.routed += st.joined;
        if st.joined > 0 {
            *out.per_net.entry(name.clone()).or_default() += st.joined;
        }
        for p in &st.pieces {
            for r in p.trimmed.as_ref().unwrap_or(&p.tracks) {
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
                let stacked = out.vias.iter().any(|v| {
                    v.net == name
                        && geom::dist(v.at, at) < rules.vias[k].r
                        && v.via == rules.vias[k].name
                });
                if !stacked {
                    out.vias.push(RoutedVia {
                        net: name.clone(),
                        at,
                        via: rules.vias[k].name.clone(),
                    });
                }
            }
        }
        out.failed.extend(st.failed.iter().cloned());
    }
    log(format!("{} of {} connections", out.routed, out.connections));
    for f in &out.failed {
        log(format!("  failed {} at {:?}: {}", f.net, f.from, f.reason));
    }
    let warm = Warm {
        nets: states
            .iter()
            .map(|st| (st.net, (st.pieces.clone(), st.failed.clone(), st.dead.clone())))
            .collect(),
    };
    Outcome {
        warm,
        spend,
        hist: soft.hist,
        result: out,
        overlap: overlap_cells,
        rounds,
        overlap_left,
    }
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
            holes: &[],
            old: None,
            partner: None,
            pads: &pads,
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

#[derive(Default)]
struct Routed {
    pieces: Vec<Piece>,
    failed: Vec<Unrouted>,
    joined: usize,
    dead: Vec<usize>,
    failed_ms: f64,
    ripped_ms: f64,
    ms: f64,
    pops: u64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Spend {
    pub threads: usize,
    pub wall_ms: f64,
    pub setup_ms: f64,
    pub hard_ms: f64,
    pub tail_ms: f64,
    pub work_ms: f64,
    pub final_ms: f64,
    pub ripped_ms: f64,
    pub failed_ms: f64,
    pub deferred_ms: f64,
    pub dropped_ms: f64,
    pub stuck_ms: f64,
    pub rounds: Vec<RoundSpend>,
    pub lost: Vec<(String, f64)>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct RoundSpend {
    pub nets: usize,
    pub wall_ms: f64,
    pub work_ms: f64,
    pub critical_ms: f64,
    pub joined: usize,
    pub overlap_nets: usize,
    pub overlap_cells: usize,
}

impl Spend {
    fn add(&mut self, m: &Many, layout: &Layout) {
        for &(n, t) in &m.lost {
            let name = &layout.nets[n].name;
            match self.lost.iter_mut().find(|x| x.0 == *name) {
                Some(x) => x.1 += t,
                None => self.lost.push((name.clone(), t)),
            }
        }
        self.work_ms += m.work_ms;
        self.deferred_ms += m.deferred_ms;
        self.failed_ms += m.failed_ms;
        self.ripped_ms += m.ripped_ms;
    }

    pub fn absorb(&mut self, o: &Spend) {
        self.threads = self.threads.max(o.threads);
        self.wall_ms += o.wall_ms;
        self.setup_ms += o.setup_ms;
        self.hard_ms += o.hard_ms;
        self.tail_ms += o.tail_ms;
        self.work_ms += o.work_ms;
        self.final_ms += o.final_ms;
        self.ripped_ms += o.ripped_ms;
        self.failed_ms += o.failed_ms;
        self.deferred_ms += o.deferred_ms;
        self.dropped_ms += o.dropped_ms;
        self.stuck_ms += o.stuck_ms;
        self.rounds.extend(o.rounds.iter().cloned());
        for (net, t) in &o.lost {
            match self.lost.iter_mut().find(|x| x.0 == *net) {
                Some(x) => x.1 += t,
                None => self.lost.push((net.clone(), *t)),
            }
        }
        self.lost.sort_by(|a, b| b.1.total_cmp(&a.1));
        self.lost.truncate(LOST_SHOWN);
    }

    pub fn other_ms(&self) -> f64 {
        (self.work_ms
            - self.final_ms
            - self.ripped_ms
            - self.failed_ms
            - self.deferred_ms
            - self.dropped_ms)
            .max(0.0)
    }

    pub fn round_wall_ms(&self) -> f64 {
        self.rounds.iter().map(|r| r.wall_ms).sum()
    }

    pub fn critical_ms(&self) -> f64 {
        self.rounds.iter().map(|r| r.critical_ms).sum()
    }

    pub fn summary(&self) -> String {
        let pct = |v: f64| if self.work_ms > 0.0 { 100.0 * v / self.work_ms } else { 0.0 };
        let s = |v: f64| v / 1000.0;
        format!(
            "time {:.1} s: setup {:.1} s, {} rounds {:.1} s ({:.1} s after the last round that cut the overlap), hard pass {:.1} s\n\
             search work {:.1} s on {} threads, {:.1} s on the longest chains: kept {:.1} s ({:.0}%), ripped up later {:.1} s ({:.0}%), found nothing {:.1} s ({:.0}%), redone outside the fence {:.1} s ({:.0}%), dropped as clashes {:.1} s ({:.0}%), bookkeeping {:.1} s ({:.0}%)\n\
             {:.1} s went to nets that still overlapped when negotiation stopped\n\
             searches that found nothing, by net: {}",
            s(self.wall_ms),
            s(self.setup_ms),
            self.rounds.len(),
            s(self.round_wall_ms()),
            s(self.tail_ms),
            s(self.hard_ms),
            s(self.work_ms),
            self.threads,
            s(self.critical_ms()),
            s(self.final_ms),
            pct(self.final_ms),
            s(self.ripped_ms),
            pct(self.ripped_ms),
            s(self.failed_ms),
            pct(self.failed_ms),
            s(self.deferred_ms),
            pct(self.deferred_ms),
            s(self.dropped_ms),
            pct(self.dropped_ms),
            s(self.other_ms()),
            pct(self.other_ms()),
            s(self.stuck_ms),
            self.lost
                .iter()
                .map(|(n, t)| format!("{n} {:.1} s", t / 1000.0))
                .collect::<Vec<_>>()
                .join(", "),
        )
    }
}

#[derive(Default)]
struct Many {
    work_ms: f64,
    critical_ms: f64,
    deferred_ms: f64,
    failed_ms: f64,
    ripped_ms: f64,
    again: usize,
    groups: usize,
    lost: Vec<(usize, f64)>,
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn base_margin(env: &Env, st: &NetState) -> f64 {
    let guided = env.guide.corridors.get(&st.net).is_some_and(|c| !c.is_empty());
    if guided { (2.0 * env.guide.tile).max(env.opts.margin * 0.5) } else { env.opts.margin }
}

fn fence_of(env: &Env, st: &NetState) -> Vec<bool> {
    let t = env.tiles;
    let mut out = vec![false; t.w * t.h];
    if kept(st).is_some() {
        let m = base_margin(env, st);
        for (p, _) in st.pieces.iter().zip(st.bad.iter().flatten()).filter(|(_, b)| **b) {
            let pts = piece_points(p);
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for q in &pts {
                lo = [lo[0].min(q[0] - m), lo[1].min(q[1] - m)];
                hi = [hi[0].max(q[0] + m), hi[1].max(q[1] + m)];
            }
            t.mark(&mut out, lo, hi);
        }
        return out;
    }
    let pts: Vec<P> = (0..st.copper.groups).flat_map(|g| st.copper.points(g)).collect();
    match env.guide.corridors.get(&st.net).filter(|c| !c.is_empty()) {
        Some(cor) => {
            for &(_, x, y) in cor {
                let (x, y) = (x - t.lo.0, y - t.lo.1);
                if x >= 0 && y >= 0 && (x as usize) < t.w && (y as usize) < t.h {
                    out[y as usize * t.w + x as usize] = true;
                }
            }
            let mut out = t.grow(&out, 1);
            let mut ends = vec![false; t.w * t.h];
            for p in &pts {
                t.mark(&mut ends, *p, *p);
            }
            for (o, e) in out.iter_mut().zip(t.grow(&ends, 1)) {
                *o |= e;
            }
            out
        }
        None => {
            let m = base_margin(env, st);
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for p in &pts {
                lo = [lo[0].min(p[0] - m), lo[1].min(p[1] - m)];
                hi = [hi[0].max(p[0] + m), hi[1].max(p[1] + m)];
            }
            t.mark(&mut out, lo, hi);
            out
        }
    }
}

fn route_one(
    st: &NetState,
    env: &Env,
    pres: f32,
    hard: bool,
    old: Option<&Footprint>,
    partner: Option<&pair::Partner>,
) -> (Routed, Footprint) {
    let t = Instant::now();
    let pops = search::pops();
    if let Some(fp) = old {
        env.soft.apply(fp, false);
    }
    let before: f64 = st.pieces.iter().map(|p| p.ms).sum();
    let mut r = match kept(st) {
        Some(keep) => {
            let mut runs = 0..0;
            let mut items = Vec::new();
            for p in &keep {
                let mine = piece_items(env.rules, std::slice::from_ref(p));
                if p.coupled {
                    let at = st.copper.solid() + items.len();
                    runs = at..at + mine.len();
                }
                items.extend(mine);
            }
            let nc = st.copper.with(env.layout, env.islands, items);
            let mut r = route_net(st, &nc, env, pres, hard, partner, runs);
            let mut all: Vec<Piece> = keep.into_iter().cloned().collect();
            r.ripped_ms = before - all.iter().map(|p| p.ms).sum::<f64>();
            all.append(&mut r.pieces);
            r.pieces = all;
            r
        }
        None => {
            let mut r = route_net(st, &st.copper, env, pres, hard, partner, 0..0);
            r.ripped_ms = before;
            r
        }
    };
    r.joined = st.needed.saturating_sub(r.failed.len());
    let rule = env.rules.rule(st.net);
    let fp = Soft::footprint(env.grid, env.rules, &Copper::of(&r.pieces, rule, &st.pads));
    if !env.jacobi {
        env.soft.apply(&fp, true);
    }
    r.ms = ms(t);
    r.pops = search::pops() - pops;
    (r, fp)
}

fn route_unit(
    states: &[NetState],
    lead: usize,
    follow: usize,
    env: &Env,
    pres: f32,
    hard: bool,
    old: [Option<&Footprint>; 2],
) -> [(Routed, Footprint); 2] {
    let (sl, sf) = (&states[lead], &states[follow]);
    let pr = &env.rules.pairs[sl.mate.expect("a unit is a pair").1];
    if let Some(fp) = old[1] {
        env.soft.apply(fp, false);
    }
    let held: Vec<Piece> = kept(sf).map(|k| k.into_iter().cloned().collect()).unwrap_or_default();
    let theirs = if env.jacobi { sf.fp.as_ref() } else { None };
    let pl = pair::Partner::of(&held, theirs, env.rules, pr);
    let a = route_one(sl, env, pres, hard, old[0], Some(&pl));
    let mine = if env.jacobi { sl.fp.as_ref() } else { Some(&a.1) };
    let pf = pair::Partner::of(&a.0.pieces, mine, env.rules, pr);
    let b = route_one(sf, env, pres, hard, None, Some(&pf));
    [a, b]
}

fn mates_in(states: &[NetState], picks: &[usize]) -> Vec<Option<usize>> {
    picks
        .iter()
        .map(|&si| states[si].mate.and_then(|(m, _)| picks.iter().position(|&x| x == m)))
        .collect()
}

fn route_many(
    states: &mut [NetState],
    todo: &[usize],
    env: &Env,
    pres: f32,
    hard: bool,
    threads: usize,
) -> Many {
    let n = todo.len();
    let mates = mates_in(states, todo);
    let halo = (env.rules.reach / env.tiles.size).ceil() as i64;
    let mut bits: Vec<Vec<u64>> = Vec::with_capacity(n);
    for &si in todo {
        let fence = fence_of(env, &states[si]);
        let wide = env.tiles.grow(&fence, halo);
        let mut b = vec![0u64; wide.len().div_ceil(64)];
        for (k, _) in wide.iter().enumerate().filter(|(_, v)| **v) {
            b[k / 64] |= 1 << (k % 64);
        }
        bits.push(b);
        states[si].fence = Some(fence);
    }
    let hit = |a: &[u64], b: &[u64]| a.iter().zip(b).any(|(x, y)| x & y != 0);
    let jacobi = !hard;
    let follower = |i: usize| mates[i].is_some_and(|j| j < i);
    let mut groups: Vec<Vec<usize>> = Vec::new();
    if jacobi {
        let mut seen: Vec<Vec<u64>> = Vec::new();
        let mut by_cost: Vec<usize> = (0..n).filter(|&i| !follower(i)).collect();
        by_cost.sort_by_key(|&i| std::cmp::Reverse(states[todo[i]].pops));
        for i in by_cost {
            let si = todo[i];
            let mut c = states[si].clash.clone();
            if let Some(j) = mates[i] {
                let o = &states[todo[j]].clash;
                if c.len() < o.len() {
                    c.resize(o.len(), 0);
                }
                for (a, b) in c.iter_mut().zip(o) {
                    *a |= b;
                }
            }
            let free =
                (0..groups.len()).find(|&k| groups[k].len() < batch_size() && !hit(&seen[k], &c));
            let k = free.unwrap_or_else(|| {
                groups.push(Vec::new());
                seen.push(Vec::new());
                groups.len() - 1
            });
            groups[k].push(i);
            groups[k].extend(mates[i]);
            if seen[k].len() < c.len() {
                seen[k].resize(c.len(), 0);
            }
            for (a, b) in seen[k].iter_mut().zip(&c) {
                *a |= b;
            }
        }
    }
    for g in &mut groups {
        g.sort_unstable();
    }
    let deps: Vec<Vec<usize>> = if jacobi {
        vec![Vec::new(); n]
    } else {
        (0..n)
            .map(|i| {
                let mut d: Vec<usize> = (0..i).filter(|&j| hit(&bits[i], &bits[j])).collect();
                if let Some(m) = mates[i].filter(|&m| m > i) {
                    d.extend((0..i).filter(|&j| hit(&bits[m], &bits[j])));
                    d.sort_unstable();
                    d.dedup();
                }
                d
            })
            .collect()
    };
    let old: Vec<Option<Footprint>> =
        if jacobi { vec![None; n] } else { todo.iter().map(|&si| states[si].fp.take()).collect() };
    let quick = Env { escalate: 0, fenced: true, jacobi, ..*env };
    let whole = Env { jacobi, ..*env };
    let results: Vec<std::sync::Mutex<Option<(Routed, Footprint)>>> =
        (0..n).map(|_| std::sync::Mutex::new(None)).collect();
    struct Queue {
        started: Vec<bool>,
        done: Vec<bool>,
        next: usize,
    }
    let queue =
        std::sync::Mutex::new(Queue { started: vec![false; n], done: vec![false; n], next: 0 });
    if jacobi {
        for (slot, r) in results.iter().zip(route_batches(
            states,
            todo,
            &groups,
            [&quick, &whole],
            pres,
            hard,
            threads,
        )) {
            *slot.lock().expect("result") = Some(r);
        }
    } else {
        let wake = std::sync::Condvar::new();
        let shared: &[NetState] = states;
        let work = || loop {
            let i = {
                let mut q = queue.lock().expect("queue");
                loop {
                    if q.next >= n {
                        break None;
                    }
                    let start = q.next;
                    let pick =
                        (start..n).find(|&i| !q.started[i] && deps[i].iter().all(|&j| q.done[j]));
                    if let Some(i) = pick {
                        q.started[i] = true;
                        if let Some(m) = mates[i] {
                            q.started[m] = true;
                        }
                        while q.next < n && q.started[q.next] {
                            q.next += 1;
                        }
                        break Some(i);
                    }
                    q = wake.wait(q).expect("queue");
                }
            };
            let Some(i) = i else { break };
            match mates[i] {
                Some(m) => {
                    let [a, b] = route_unit(
                        shared,
                        todo[i],
                        todo[m],
                        &quick,
                        pres,
                        hard,
                        [old[i].as_ref(), old[m].as_ref()],
                    );
                    *results[i].lock().expect("result") = Some(a);
                    *results[m].lock().expect("result") = Some(b);
                    let mut q = queue.lock().expect("queue");
                    q.done[i] = true;
                    q.done[m] = true;
                }
                None => {
                    let pa = pair::partner_of(shared, todo[i], env.rules);
                    let out = route_one(
                        &shared[todo[i]],
                        &quick,
                        pres,
                        hard,
                        old[i].as_ref(),
                        pa.as_ref(),
                    );
                    *results[i].lock().expect("result") = Some(out);
                    queue.lock().expect("queue").done[i] = true;
                }
            }
            wake.notify_all();
        };
        std::thread::scope(|s| {
            for _ in 0..threads.min(n).max(1) {
                s.spawn(work);
            }
        });
    }
    let deferred = |r: &Routed| r.failed.iter().any(|f| f.reason == DEFERRED);
    let mut redone: HashMap<usize, (Routed, Footprint)> = HashMap::new();
    if jacobi {
        let mut redo: Vec<usize> = (0..n)
            .filter(|&i| {
                results[i].lock().expect("result").as_ref().is_some_and(|x| deferred(&x.0))
            })
            .collect();
        for i in redo.clone() {
            if let Some(m) = mates[i].filter(|m| !redo.contains(m)) {
                redo.push(m);
            }
        }
        redo.sort_unstable();
        for &i in &redo {
            states[todo[i]].fp = results[i].lock().expect("result").as_ref().map(|x| x.1.clone());
        }
        let picks: Vec<usize> = redo.iter().map(|&i| todo[i]).collect();
        for &si in &picks {
            states[si].wide = true;
        }
        let pick_mates = mates_in(states, &picks);
        let mut chunks: Vec<Vec<usize>> = Vec::new();
        for (k, mate) in pick_mates.iter().enumerate() {
            if mate.is_some_and(|m| m < k) {
                continue;
            }
            if chunks.last().is_none_or(|c| c.len() >= BATCH) {
                chunks.push(Vec::new());
            }
            let c = chunks.last_mut().expect("chunk");
            c.push(k);
            c.extend(*mate);
        }
        redone = redo
            .into_iter()
            .zip(route_batches(states, &picks, &chunks, [&whole, &whole], pres, hard, threads))
            .collect();
    }
    let mut out = Many::default();
    let mut ef = vec![0.0f64; n];
    for (i, slot) in results.into_iter().enumerate() {
        let (mut r, mut fp) = slot.into_inner().expect("result").expect("routed");
        ef[i] = r.ms + deps[i].iter().map(|&j| ef[j]).fold(0.0, f64::max);
        out.work_ms += r.ms;
        if deferred(&r) || redone.contains_key(&i) {
            out.deferred_ms += r.ms;
            (r, fp) = match redone.remove(&i) {
                Some(x) => x,
                None => {
                    env.soft.apply(&fp, false);
                    let pa = pair::partner_of(states, todo[i], env.rules);
                    route_one(&states[todo[i]], env, pres, hard, None, pa.as_ref())
                }
            };
            out.work_ms += r.ms;
            out.again += 1;
        }
        let st = &mut states[todo[i]];
        if !hard {
            st.dead.extend(std::mem::take(&mut r.dead));
        }
        out.failed_ms += r.failed_ms;
        if r.failed_ms > 0.0 {
            out.lost.push((st.net, r.failed_ms));
        }
        out.ripped_ms += r.ripped_ms;
        st.spent_ms += r.ms;
        st.tries += 1;
        st.pops = r.pops;
        st.pieces = r.pieces;
        st.failed = r.failed;
        st.joined = r.joined;
        st.fp = Some(fp);
    }
    out.critical_ms = if jacobi {
        groups.iter().map(|g| g.iter().map(|&i| ef[i]).fold(0.0, f64::max)).sum()
    } else {
        ef.iter().copied().fold(0.0, f64::max)
    };
    out.groups = groups.len();
    out
}

fn lean_mask(env: &Env, net: usize, win: &Window, fence: Option<&[bool]>) -> Option<Vec<i8>> {
    let g = env.guide;
    let corridor = g.corridors.get(&net);
    let prefer = g.prefer.get(&net);
    let vias = g.prefer_vias.get(&net);
    if corridor.is_none() && prefer.is_none() && vias.is_none() && fence.is_none() {
        return None;
    }
    let grid = env.grid;
    let (ww, wh) = (win.ww(), win.wh());
    let area = ww * wh;
    let mut out = vec![0i8; area * grid.nl];
    if let Some(cor) = corridor.filter(|_| g.tile > 0.0) {
        let t = env.tiles;
        let (tx0, ty0) = (t.tx[win.x0], t.ty[win.y0]);
        let tw = t.tx[win.x1] - tx0 + 1;
        let th = t.ty[win.y1] - ty0 + 1;
        let cols: Vec<usize> = (0..ww).map(|x| t.tx[x + win.x0] - tx0).collect();
        for l in 0..grid.nl {
            let mut near = vec![false; tw * th];
            for ty in 0..th {
                for tx in 0..tw {
                    let (x, y) = ((tx + tx0) as i64 + t.lo.0, (ty + ty0) as i64 + t.lo.1);
                    near[ty * tw + tx] =
                        (-1..=1).any(|dy| (-1..=1).any(|dx| cor.contains(&(l, x + dx, y + dy))));
                }
            }
            if near.iter().all(|v| *v) {
                continue;
            }
            for y in 0..wh {
                let row = (t.ty[y + win.y0] - ty0) * tw;
                let base = l * area + y * ww;
                for x in 0..ww {
                    if !near[row + cols[x]] {
                        out[base + x] = 1;
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
    if let Some(f) = fence {
        let t = env.tiles;
        let cols: Vec<usize> = (0..ww).map(|x| t.tx[x + win.x0]).collect();
        for y in 0..wh {
            let row = t.ty[y + win.y0] * t.w;
            for x in 0..ww {
                if !f[row + cols[x]] {
                    for l in 0..grid.nl {
                        out[l * area + y * ww + x] = search::BLOCKED;
                    }
                }
            }
        }
    }
    Some(out)
}

fn reserve_map(grid: &Grid, rules: &Rules, guide: &Guide) -> Vec<u16> {
    if guide.prefer.is_empty() && guide.prefer_vias.is_empty() {
        return Vec::new();
    }
    let plane = grid.plane();
    let mut out = vec![u16::MAX; plane * grid.nl];
    let mut claim = |net: usize, l: usize, shape: &Shape, reach: f64| {
        grid.near(shape, reach, |x, y, _| {
            let c = &mut out[l * plane + y * grid.w + x];
            *c = if *c == u16::MAX || *c == net as u16 { net as u16 } else { u16::MAX - 1 };
        });
    };
    for (&n, segs) in &guide.prefer {
        let Some(rule) = rules.nets.get(n).and_then(|r| r.as_ref()) else { continue };
        for &(l, a, b) in segs {
            claim(n, l, &Shape::Seg(a, b, 0.0), rule.width[l] / 2.0 + rule.clearance);
        }
    }
    for (&n, vias) in &guide.prefer_vias {
        let Some(rule) = rules.nets.get(n).and_then(|r| r.as_ref()) else { continue };
        let Some(&k) = rule.vias.first() else { continue };
        let o = &rules.vias[k];
        for &at in vias {
            for &l in &o.layers {
                claim(n, l, &Shape::Circle(at, 0.0), o.r + rule.clearance);
            }
        }
    }
    out
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

fn entry_mask(grid: &Grid, rule: &NetRule, pads: &[PadRef], win: &Window) -> Option<Vec<bool>> {
    if rule.band <= rule.clearance + 1e-9 {
        return None;
    }
    let ww = win.ww();
    let mut out = vec![false; ww * win.wh()];
    for pad in pads {
        let (lo, hi) = Shape::Poly(pad.outline.clone()).bounds();
        let (x0, y0, x1, y1) = grid.span(lo, hi, rule.neck);
        if x1 == usize::MAX || y1 == usize::MAX {
            continue;
        }
        for y in y0.max(win.y0)..=y1.min(win.y1) {
            for x in x0.max(win.x0)..=x1.min(win.x1) {
                let c = grid.center(x, y);
                let inside = geom::point_in_polygon(c, &pad.outline);
                if inside || grid::edge_dist(&pad.outline, c) <= rule.neck {
                    out[(y - win.y0) * ww + (x - win.x0)] = true;
                }
            }
        }
    }
    Some(out)
}

fn clash_tiles(tiles: &Tiles, grid: &Grid, cells: &[(usize, usize)]) -> Vec<u64> {
    if cells.is_empty() {
        return Vec::new();
    }
    let mut v = vec![false; tiles.w * tiles.h];
    for &(_, c) in cells {
        v[tiles.ty[c / grid.w] * tiles.w + tiles.tx[c % grid.w]] = true;
    }
    let v = tiles.grow(&v, CLASH_HALO);
    let mut b = vec![0u64; v.len().div_ceil(64)];
    for (k, _) in v.iter().enumerate().filter(|(_, x)| **x) {
        b[k / 64] |= 1 << (k % 64);
    }
    b
}

fn route_batches(
    states: &mut [NetState],
    picks: &[usize],
    groups: &[Vec<usize>],
    envs: [&Env; 2],
    pres: f32,
    hard: bool,
    threads: usize,
) -> Vec<(Routed, Footprint)> {
    let results: Vec<std::sync::Mutex<Option<(Routed, Footprint)>>> =
        picks.iter().map(|_| std::sync::Mutex::new(None)).collect();
    let mates = mates_in(states, picks);
    for group in groups {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let shared: &[NetState] = states;
        std::thread::scope(|s| {
            for _ in 0..threads.min(group.len()).max(1) {
                s.spawn(|| {
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&i) = group.get(k) else { break };
                        let st = &shared[picks[i]];
                        let env = envs[st.wide as usize];
                        match mates[i] {
                            Some(m) if m < i => {}
                            Some(m) => {
                                let [a, b] = route_unit(
                                    shared,
                                    picks[i],
                                    picks[m],
                                    env,
                                    pres,
                                    hard,
                                    [None, None],
                                );
                                *results[i].lock().expect("result") = Some(a);
                                *results[m].lock().expect("result") = Some(b);
                            }
                            None => {
                                let pa = pair::partner_of(shared, picks[i], env.rules);
                                let out = route_one(st, env, pres, hard, None, pa.as_ref());
                                *results[i].lock().expect("result") = Some(out);
                            }
                        }
                    }
                });
            }
        });
        let olds: Vec<Option<Footprint>> =
            group.iter().map(|&i| states[picks[i]].fp.take()).collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..threads.min(group.len()).max(1) {
                s.spawn(|| {
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&i) = group.get(k) else { break };
                        if let Some(o) = &olds[k] {
                            envs[0].soft.apply(o, false);
                        }
                        let r = results[i].lock().expect("result");
                        envs[0].soft.apply(&r.as_ref().expect("routed").1, true);
                    }
                });
            }
        });
    }
    results.into_iter().map(|m| m.into_inner().expect("result").expect("routed")).collect()
}

fn all_conflicts(
    states: &[NetState],
    order: &[usize],
    grid: &Grid,
    soft: &Soft,
    rules: &Rules,
    threads: usize,
) -> Vec<Conflicts> {
    let out: Vec<std::sync::Mutex<Conflicts>> =
        order.iter().map(|_| std::sync::Mutex::new(Default::default())).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads.min(order.len()).max(1) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&si) = order.get(i) else { break };
                    *out[i].lock().expect("conflicts") = conflicts(states, si, grid, soft, rules);
                }
            });
        }
    });
    out.into_iter().map(|m| m.into_inner().expect("conflicts")).collect()
}

fn conflicts(states: &[NetState], si: usize, grid: &Grid, soft: &Soft, rules: &Rules) -> Conflicts {
    let st = &states[si];
    let Some(fp) = st.fp.as_ref() else { return (Vec::new(), Vec::new()) };
    let rule = rules.rule(st.net);
    let partner = pair::partner_of(states, si, rules);
    let theirs = |b: usize, c: usize| u8::from(partner.as_ref().is_some_and(|p| p.has(b, c)));
    let mut out = Vec::new();
    let mut bad = vec![false; st.pieces.len()];
    for (pi, p) in st.pieces.iter().enumerate() {
        let before = out.len();
        let close = partner.as_ref().is_some_and(|pa| pa.clashes(p, rules));
        for r in &p.tracks {
            let Some(b) = rule.bucket[r.layer] else { continue };
            for w in r.points.windows(2) {
                for (x, y) in seg_cells(grid, w[0], w[1]) {
                    let c = y * grid.w + x;
                    if (close || soft.track_less(b, c, Some(fp)) > theirs(b, c))
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
            let vb = rule.via_bucket[vi];
            let mine = u8::from(partner.as_ref().is_some_and(|p| p.has_via(vb, c)));
            if close || soft.via_less(vb, c, Some(fp)) > mine {
                for &l in &rules.vias[k].layers {
                    out.push((l, c));
                }
            }
        }
        bad[pi] = out.len() > before;
    }
    out.sort_unstable();
    out.dedup();
    (out, bad)
}

struct Saved {
    pieces: Vec<Piece>,
    fp: Option<Footprint>,
    failed: Vec<Unrouted>,
    joined: usize,
    bad: Option<Vec<bool>>,
}

fn save(st: &NetState) -> Saved {
    Saved {
        pieces: st.pieces.clone(),
        fp: st.fp.clone(),
        failed: st.failed.clone(),
        joined: st.joined,
        bad: st.bad.clone(),
    }
}

fn restore(st: &mut NetState, s: Saved, soft: &Soft) {
    if let Some(fp) = st.fp.take() {
        soft.apply(&fp, false);
    }
    if let Some(fp) = &s.fp {
        soft.apply(fp, true);
    }
    st.pieces = s.pieces;
    st.fp = s.fp;
    st.failed = s.failed;
    st.joined = s.joined;
    st.bad = s.bad;
}

fn reroute(states: &mut [NetState], si: usize, env: &Env, pres: f32, hard: bool) {
    let partner = pair::partner_of(states, si, env.rules);
    let st = &mut states[si];
    let old = st.fp.take();
    let (r, fp) = route_one(st, env, pres, hard, old.as_ref(), partner.as_ref());
    st.pieces = r.pieces;
    st.failed = r.failed;
    st.joined = r.joined;
    st.fp = Some(fp);
}

fn track_cells(st: &NetState, grid: &Grid, rules: &Rules, pieces: &[Piece]) -> Vec<(usize, u32)> {
    let rule = rules.rule(st.net);
    let mut out = Vec::new();
    for p in pieces {
        for r in &p.tracks {
            let Some(b) = rule.bucket[r.layer] else { continue };
            for w in r.points.windows(2) {
                for (x, y) in seg_cells(grid, w[0], w[1]) {
                    out.push((b, (y * grid.w + x) as u32));
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn rip_and_retry(
    states: &mut [NetState],
    order: &[usize],
    env: &Env,
    pres: f32,
    threads: usize,
    lost: &mut HashSet<(usize, Vec<usize>)>,
) -> (usize, usize) {
    let soft = env.soft;
    let losers: Vec<usize> = order
        .iter()
        .copied()
        .filter(|&si| {
            let st = &states[si];
            st.joined < st.needed && st.failed.iter().any(|f| f.reason != DEAD)
        })
        .collect();
    let probe_env = Env { jacobi: true, ..*env };
    let held_bad: Vec<Option<Vec<bool>>> =
        losers.iter().map(|&si| states[si].bad.clone()).collect();
    for &si in &losers {
        states[si].bad = Some(vec![false; states[si].pieces.len()]);
    }
    let paths: Vec<std::sync::Mutex<Vec<(usize, u32)>>> =
        losers.iter().map(|_| std::sync::Mutex::new(Vec::new())).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let shared: &[NetState] = states;
    std::thread::scope(|s| {
        for _ in 0..threads.min(losers.len()).max(1) {
            s.spawn(|| {
                loop {
                    let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&si) = losers.get(k) else { break };
                    let st = &shared[si];
                    let pa = pair::partner_of(shared, si, env.rules);
                    let (probe, _) = route_one(st, &probe_env, pres, false, None, pa.as_ref());
                    let fresh = probe.pieces.get(st.pieces.len()..).unwrap_or_default();
                    *paths[k].lock().expect("path") = track_cells(st, env.grid, env.rules, fresh);
                }
            });
        }
    });
    for (&si, bad) in losers.iter().zip(held_bad) {
        states[si].bad = bad;
    }
    let (mut tried, mut won) = (0, 0);
    for (k, si) in losers.into_iter().enumerate() {
        let path = std::mem::take(&mut *paths[k].lock().expect("path"));
        if path.is_empty() {
            continue;
        }
        let held = save(&states[si]);
        if let Some(fp) = states[si].fp.take() {
            soft.apply(&fp, false);
        }
        states[si].bad = Some(vec![false; states[si].pieces.len()]);
        let blockers: Vec<usize> = (0..states.len())
            .filter(|&sj| sj != si)
            .filter(|&sj| {
                states[sj]
                    .fp
                    .as_ref()
                    .is_some_and(|fp| path.iter().any(|&(b, c)| fp.0[b].binary_search(&c).is_ok()))
            })
            .collect();
        let key = (si, blockers.clone());
        if blockers.len() > RIP_BLOCKERS || lost.contains(&key) {
            restore(&mut states[si], held, soft);
            continue;
        }
        tried += 1;
        let before: usize = std::iter::once(si)
            .chain(blockers.iter().copied())
            .map(|k| states[k].needed - states[k].joined)
            .sum();
        let kept: Vec<Saved> = blockers.iter().map(|&sj| save(&states[sj])).collect();
        for &sj in &blockers {
            let st = &mut states[sj];
            if let Some(fp) = st.fp.take() {
                soft.apply(&fp, false);
            }
            let rule = env.rules.rule(st.net);
            let bad: Vec<bool> = st
                .pieces
                .iter()
                .map(|p| {
                    let fp = Soft::footprint(
                        env.grid,
                        env.rules,
                        &Copper::of(std::slice::from_ref(p), rule, &st.pads),
                    );
                    path.iter().any(|&(b, c)| fp.0[b].binary_search(&c).is_ok())
                })
                .collect();
            st.bad = Some(bad);
            st.fp = kept_footprint(st, env.rules, env.grid);
            if let Some(fp) = &st.fp {
                soft.apply(fp, true);
            }
        }
        reroute(states, si, env, pres, true);
        for &sj in &blockers {
            reroute(states, sj, env, pres, true);
        }
        let after: usize = std::iter::once(si)
            .chain(blockers.iter().copied())
            .map(|k| states[k].needed - states[k].joined)
            .sum();
        if after < before {
            won += before - after;
        } else {
            for (&sj, k) in blockers.iter().zip(kept).rev() {
                restore(&mut states[sj], k, soft);
            }
            restore(&mut states[si], held, soft);
            lost.insert(key);
        }
    }
    (tried, won)
}

fn via_total(st: &NetState) -> usize {
    st.pieces.iter().map(|p| p.vias.len()).sum()
}

fn fewer_vias(states: &mut [NetState], order: &[usize], env: &Env) -> usize {
    let planes = agentee_core::tie::plane_nets(env.layout);
    let mut saved = 0;
    for &si in order {
        let before = (states[si].joined, via_total(&states[si]));
        if before.1 == 0 || states[si].fp.is_none() || planes.contains(&states[si].net) {
            continue;
        }
        let held = save(&states[si]);
        let coupled = states[si].pieces.iter().any(|p| p.coupled);
        states[si].bad = coupled.then(|| states[si].pieces.iter().map(|p| !p.coupled).collect());
        reroute(states, si, env, MAX_PRES, true);
        let after = (states[si].joined, via_total(&states[si]));
        if after.0 >= before.0 && after.1 < before.1 {
            saved += before.1 - after.1;
        } else {
            restore(&mut states[si], held, env.soft);
        }
    }
    saved
}

fn drop_overlaps(
    states: &mut [NetState],
    order: &[usize],
    grid: &Grid,
    soft: &Soft,
    rules: &Rules,
    layout: &Layout,
) -> (usize, f64) {
    let mut dropped = 0;
    let mut dropped_ms = 0.0;
    for &si in order {
        let (cells, bad) = conflicts(states, si, grid, soft, rules);
        if cells.is_empty() {
            continue;
        }
        let real: Vec<bool> = states[si]
            .pieces
            .iter()
            .zip(&bad)
            .map(|(p, &b)| b && clashes(p, si, states, rules))
            .collect();
        if !real.iter().any(|b| *b) {
            continue;
        }
        let st = &mut states[si];
        if let Some(fp) = st.fp.take() {
            soft.apply(&fp, false);
        }
        let name = layout.nets[st.net].name.clone();
        let mut keep = Vec::new();
        for (p, b) in std::mem::take(&mut st.pieces).into_iter().zip(real) {
            if b {
                let pts = piece_points(&p);
                let from = pts.first().copied().unwrap_or_default();
                let to = pts.last().copied().unwrap_or_default();
                st.failed.push(Unrouted {
                    net: name.clone(),
                    from,
                    to,
                    reason: "overlapped another net".into(),
                });
                dropped += 1;
                dropped_ms += p.ms;
            } else {
                keep.push(p);
            }
        }
        st.pieces = keep;
        st.joined = st.needed.saturating_sub(st.failed.len());
        let fp =
            Soft::footprint(grid, rules, &Copper::of(&st.pieces, rules.rule(st.net), &st.pads));
        soft.apply(&fp, true);
        st.fp = Some(fp);
    }
    (dropped, dropped_ms)
}

fn clashes(p: &Piece, si: usize, states: &[NetState], rules: &Rules) -> bool {
    let me = rules.rule(states[si].net);
    let segs = |p: &Piece| -> Vec<(usize, P, P, f64)> {
        let mut out = Vec::new();
        for r in &p.tracks {
            for w in r.points.windows(2) {
                out.push((r.layer, w[0], w[1], r.width / 2.0));
            }
        }
        out
    };
    let mine = segs(p);
    let mate = states[si].mate;
    states.iter().enumerate().filter(|(j, _)| *j != si).any(|(j, o)| {
        let them = rules.rule(o.net);
        let base = match mate {
            Some((m, ci)) if m == j => rules.pairs[ci].clear,
            _ => me.clearance.max(them.clearance),
        };
        let gap = |l: usize| base.max(rules.iso.apart(me.domain, them.domain, l)) - 1e-4;
        o.pieces.iter().any(|q| {
            let theirs = segs(q);
            let tt = mine.iter().any(|&(l, a, b, h)| {
                theirs.iter().any(|&(m, x, y, k)| {
                    l == m && geom::segment_segment_distance(a, b, x, y) - h - k < gap(l)
                })
            });
            let via_seg = |vias: &[(P, usize)], segs: &[(usize, P, P, f64)]| {
                vias.iter().any(|&(v, kv)| {
                    let o = &rules.vias[kv];
                    segs.iter().any(|&(l, a, b, h)| {
                        let d = geom::point_segment_distance(v, a, b) - h;
                        o.layers.contains(&l)
                            && (d - o.r < gap(l) || d - o.dr < rules.hole_cu - 1e-4)
                    })
                })
            };
            let vv = p.vias.iter().any(|&(v, kv)| {
                q.vias.iter().any(|&(w, kw)| {
                    let (a, b) = (&rules.vias[kv], &rules.vias[kw]);
                    let d = geom::dist(v, w);
                    let c = a
                        .layers
                        .iter()
                        .filter(|l| b.layers.contains(l))
                        .map(|&l| gap(l))
                        .reduce(f64::max);
                    let holes = d - a.dr - b.r < rules.hole_cu - 1e-4
                        || d - b.dr - a.r < rules.hole_cu - 1e-4;
                    c.is_some_and(|c| d - a.r - b.r < c || holes)
                        || d - a.dr - b.dr < rules.hole_gap - 1e-4
                })
            });
            tt || vv || via_seg(&p.vias, &theirs) || via_seg(&q.vias, &mine)
        })
    })
}

fn piece_items(rules: &Rules, pieces: &[&Piece]) -> Vec<Item> {
    let mut out = Vec::new();
    for p in pieces {
        for r in &p.tracks {
            let h = r.width / 2.0;
            if r.points.len() == 1 {
                out.push(Item::Seg {
                    layer: r.layer,
                    shape: Shape::Seg(r.points[0], r.points[0], h),
                });
            }
            for w in r.points.windows(2) {
                out.push(Item::Seg { layer: r.layer, shape: Shape::Seg(w[0], w[1], h) });
            }
        }
        for &(at, k) in &p.vias {
            let o = &rules.vias[k];
            out.push(Item::Via { at, r: o.r, drill: o.dr, layers: o.layers.clone() });
        }
    }
    out
}

fn kept(st: &NetState) -> Option<Vec<&Piece>> {
    let bad = st.bad.as_ref()?;
    if bad.len() != st.pieces.len() || !bad.iter().any(|b| !b) {
        return None;
    }
    Some(st.pieces.iter().zip(bad).filter(|(_, b)| !**b).map(|(p, _)| p).collect())
}

fn kept_footprint(st: &NetState, rules: &Rules, grid: &Grid) -> Option<Footprint> {
    let keep: Vec<Piece> = kept(st)?.into_iter().cloned().collect();
    Some(Soft::footprint(grid, rules, &Copper::of(&keep, rules.rule(st.net), &st.pads)))
}

fn within_fence(env: &Env, st: &NetState, a: &Access) -> bool {
    let Some(f) = st.fence.as_deref().filter(|_| env.fenced) else { return true };
    let t = env.tiles;
    let inside = |p: P| {
        let (x, y) = t.of(p);
        x >= 0
            && y >= 0
            && (x as usize) < t.w
            && (y as usize) < t.h
            && f[y as usize * t.w + x as usize]
    };
    a.neck.as_ref().is_none_or(|n| inside(n.from) && inside(n.to))
        && a.via.is_none_or(|(at, _)| inside(at))
}

fn seg_of(it: &Item) -> Option<Seg> {
    match it {
        Item::Seg { layer, shape: Shape::Seg(a, b, h) } => Some((*layer, *a, *b, *h)),
        _ => None,
    }
}

fn piece_points(p: &Piece) -> Vec<P> {
    let mut out: Vec<P> = p.vias.iter().map(|v| v.0).collect();
    for r in &p.tracks {
        out.extend(r.points.iter().copied());
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn route_net(
    st: &NetState,
    nc: &NetCopper,
    env: &Env,
    pres: f32,
    hard: bool,
    partner: Option<&pair::Partner>,
    runs: std::ops::Range<usize>,
) -> Routed {
    let name = &env.layout.nets[st.net].name;
    let pad_groups: Vec<usize> =
        (0..nc.groups).filter(|&g| nc.has_pad(g) || nc.holds(g, runs.clone())).collect();
    let size = |g: usize| nc.group.iter().filter(|&&k| k == g).count();
    let Some(&start) =
        pad_groups.iter().max_by_key(|&&g| (!nc.islands(g).is_empty(), size(g), usize::MAX - g))
    else {
        return Routed::default();
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
    let mut dead = Vec::new();
    let mut joined = 0;
    let mut failed_ms = 0.0;
    while !rest.is_empty() {
        let k = (0..rest.len()).min_by(|&i, &j| best[i].0.total_cmp(&best[j].0)).unwrap();
        let g = rest.remove(k);
        let (_, a, b) = best.remove(k);
        let key = nc.group.iter().position(|&k| k == g).unwrap_or(usize::MAX);
        if st.dead.contains(&key) {
            failed.push(Unrouted { net: name.clone(), from: a, to: b, reason: DEAD.into() });
            continue;
        }
        let t = Instant::now();
        let used: usize =
            nc.items.iter().skip(nc.solid()).filter(|it| matches!(it, Item::Via { .. })).count()
                + pieces.iter().map(|p| p.vias.len()).sum::<usize>();
        let spare = env.rules.budget[st.net].as_ref().map(|b| b.0.saturating_sub(used));
        match connect(st, nc, env, g, &tree, &pieces, a, b, pres, hard, partner, spare) {
            Ok(mut piece) => {
                piece.ms = ms(t);
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
            Err(reason) => {
                if reason != DEFERRED {
                    dead.push(key);
                    failed_ms += ms(t);
                }
                failed.push(Unrouted { net: name.clone(), from: a, to: b, reason })
            }
        }
    }
    Routed { pieces, failed, joined, dead, failed_ms, ..Default::default() }
}

#[allow(clippy::too_many_arguments)]
fn connect(
    st: &NetState,
    nc: &NetCopper,
    env: &Env,
    g: usize,
    tree: &[usize],
    pieces: &[Piece],
    a: P,
    b: P,
    pres: f32,
    hard: bool,
    partner: Option<&pair::Partner>,
    spare: Option<usize>,
) -> Result<Piece, String> {
    let first = search_piece(st, nc, env, g, tree, pieces, a, b, pres, hard, partner, 1.0, true);
    let Some(left) = spare.filter(|_| env.rules.budget[st.net].is_some()) else { return first };
    let over = |p: &Piece| p.vias.len() > left;
    if !first.as_ref().is_ok_and(over) {
        return first;
    }
    let mul = if left == 0 { VIA_DEARER * VIA_DEARER } else { VIA_DEARER };
    match search_piece(st, nc, env, g, tree, pieces, a, b, pres, hard, partner, mul, true) {
        Ok(p) if !over(&p) => Ok(p),
        _ => first,
    }
}

#[allow(clippy::too_many_arguments)]
fn search_piece(
    st: &NetState,
    nc: &NetCopper,
    env: &Env,
    g: usize,
    tree: &[usize],
    pieces: &[Piece],
    a: P,
    b: P,
    pres: f32,
    hard: bool,
    partner: Option<&pair::Partner>,
    via_mul: f64,
    vias: bool,
) -> Result<Piece, String> {
    let grid = env.grid;
    let rule = env.rules.rule(st.net);
    let net = st.net as u16;
    let lo = [a[0].min(b[0]), a[1].min(b[1])];
    let hi = [a[0].max(b[0]), a[1].max(b[1])];
    let mut margin = base_margin(env, st);
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
        let holes: Vec<(P, f64)> = nc
            .items
            .iter()
            .filter_map(|it| match it {
                Item::Via { at, drill, .. } => Some((*at, *drill)),
                _ => None,
            })
            .chain(
                pieces
                    .iter()
                    .flat_map(|p| p.vias.iter().map(|&(at, k)| (at, env.rules.vias[k].dr))),
            )
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
            holes: &holes,
            old: env.jacobi.then_some(st.fp.as_ref()).flatten(),
            partner,
            pads: &st.pads,
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
                && partner.is_none_or(|p| !p.near_track(l, at, rule.width[l] / 2.0, 0.0))
                && (!hard
                    || st.own.contains(&(i3 as u32))
                    || rule.bucket[l].is_none_or(|bk| {
                        env.soft.track(bk, c2) <= u8::from(partner.is_some_and(|p| p.has(bk, c2)))
                    }))
        };
        for (i, it) in nc.items.iter().enumerate() {
            let mine = nc.group[i] == g;
            let theirs = tree.contains(&nc.group[i]);
            if !mine && !theirs {
                continue;
            }
            let cells = item_cells(env, it, &win);
            let mut narrow: Vec<usize> = Vec::new();
            if let Item::Pad { shapes, layers, centre, pitch } = it {
                let (x, y) = grid.cell(*centre);
                let seen = grid.inside(x, y) && win.contains(x as usize, y as usize);
                for &l in layers.iter().filter(|_| seen) {
                    let any = cells.iter().any(|&c| c / grid.plane() == l && legal(c));
                    for s in shapes {
                        let Shape::Poly(v) = s else { continue };
                        let wide = rule.width[l] > geom::min_extent(v) + 1e-9;
                        if wide && rule.track[l] {
                            narrow.push(l);
                        }
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
                        if (!any || wide) && (rule.track[l] || !entry.is_empty()) {
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
                            let Some((at, k)) = a.via.filter(|_| vias) else { continue };
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
                            if !within_fence(env, st, &a) {
                                continue;
                            }
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
                let necked = narrow.contains(&(c / grid.plane()));
                if mine {
                    sources.push(Source {
                        at: c,
                        cost: 0.0,
                        tag: 0,
                        via_only: necked || !legal(c),
                        landed: false,
                    });
                } else if !necked {
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
        let source_cells: std::collections::HashSet<usize> =
            sources.iter().filter(|s| s.tag == 0).map(|s| s.at).collect();
        targets.retain(|(c, tag)| *tag != 0 || !source_cells.contains(c));
        let source_vias: Vec<(P, usize)> = sources
            .iter()
            .filter(|s| s.tag > 0)
            .filter_map(|s| access[s.tag as usize - 1].via)
            .collect();
        targets.retain(|&(_, tag)| {
            let Some((at, k)) = (tag > 0).then(|| access[tag as usize - 1].via).flatten() else {
                return true;
            };
            source_vias.iter().all(|&(v, kv)| {
                let d = geom::dist(v, at);
                d < 1e-6 || d >= env.rules.vias[k].dr + env.rules.vias[kv].dr + env.rules.hole_gap
            })
        });
        if sources.is_empty() {
            if win.is_whole(grid) {
                return Err("an end has no copper on the routing layers".into());
            }
            margin *= 3.0;
            continue;
        }
        let fence = st.fence.as_deref().filter(|_| env.fenced);
        let mut lean = lean_mask(env, st.net, &win, fence);
        if let Some(p) = partner {
            let m = lean.get_or_insert_with(|| vec![0i8; win.ww() * win.wh() * grid.nl]);
            p.mask(grid, &win, &rule.width, m);
        }
        let entering = entry_mask(grid, rule, &st.pads, &win);
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
            via_cost: env.opts.via_cost * via_mul,
            vias,
            bend_cost: env.opts.bend_cost,
            zone: env.zone,
            reserve: env.reserve,
            reserve_cost: RESERVE_COST,
            zone_cost: env.opts.zone_cost,
            extra: lean.as_deref(),
            outside: env.opts.corridor_cost as f32,
            gain: env.opts.prefer_gain as f32,
            own: &st.own,
            holes: &holes,
            old: env.jacobi.then_some(st.fp.as_ref()).flatten(),
            partner,
            entering: entering.as_deref(),
        };
        tries += 1;
        let traced: Vec<P> = std::env::var("AGENTEE_TRACE")
            .ok()
            .map(|v| {
                v.split(';')
                    .filter_map(|p| {
                        let xy: Vec<f64> = p.split(',').filter_map(|t| t.parse().ok()).collect();
                        (xy.len() == 2).then(|| [xy[0], xy[1]])
                    })
                    .collect()
            })
            .unwrap_or_default();
        let trace = traced.iter().any(|t| geom::dist(*t, a) < 0.05 || geom::dist(*t, b) < 0.05);
        if trace {
            let planar = q.sources.iter().filter(|s| !s.via_only).count();
            eprintln!(
                "trace {} {:?}->{:?} hard {hard} try {tries} margin {margin:.1} win {}x{} sources {} ({} planar) targets {}",
                env.layout.nets[st.net].name,
                a,
                b,
                win.ww(),
                win.wh(),
                q.sources.len(),
                planar,
                q.targets.len()
            );
        }
        if let Some(found) = q.run() {
            if found.cells.len() <= 1 && found.tag == 0 {
                margin *= 3.0;
                if win.is_whole(grid) || tries > env.opts.escalate {
                    return Err("the groups already touch".into());
                }
                if tries > env.escalate {
                    return Err(DEFERRED.into());
                }
                continue;
            }
            let src: Vec<Seg> = nc
                .items
                .iter()
                .zip(&nc.group)
                .filter(|(_, k)| **k == g)
                .filter_map(|(it, _)| seg_of(it))
                .collect();
            let dst: Vec<Seg> = nc
                .items
                .iter()
                .zip(&nc.group)
                .filter(|(_, k)| tree.contains(k))
                .filter_map(|(it, _)| seg_of(it))
                .chain(pieces.iter().flat_map(|p| {
                    p.tracks.iter().flat_map(|r| {
                        r.points.windows(2).map(move |w| (r.layer, w[0], w[1], r.width / 2.0))
                    })
                }))
                .collect();
            return Ok(ctx.piece(&found, &st.pads, &access, (&src, &dst)));
        }
        if env.fenced {
            return Err(DEFERRED.into());
        }
        if win.is_whole(grid) || tries > env.opts.escalate {
            return Err("no path within the rules".into());
        }
        if tries > env.escalate {
            return Err(DEFERRED.into());
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
        Item::Via { at, r, layers, .. } => {
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

fn batch_size() -> usize {
    std::env::var("AGENTEE_BATCH").ok().and_then(|v| v.parse().ok()).unwrap_or(BATCH)
}
