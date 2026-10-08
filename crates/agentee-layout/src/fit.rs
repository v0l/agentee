use crate::Model;
use crate::placement::{self, Board};
use agentee_core::geom::P;
use agentee_core::graphic::Bounds;
use agentee_core::place;
use agentee_core::project::LayoutInputs;
use serde::Serialize;
use std::collections::HashMap;

const GRID: f64 = 0.5;
const STEPS: usize = 12;
const GROWS: usize = 6;
const GROW: f64 = 1.3;
const FLOOR: f64 = 0.2;
const TILE: f64 = 1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Strategy {
    Tight,
    #[default]
    Balanced,
    Spread,
}

impl Strategy {
    pub fn parse(s: &str) -> Result<Strategy, String> {
        match s {
            "tight" => Ok(Strategy::Tight),
            "balanced" => Ok(Strategy::Balanced),
            "spread" => Ok(Strategy::Spread),
            _ => Err(format!("unknown strategy `{s}`, use tight, balanced or spread")),
        }
    }

    fn overflow_share(self) -> f64 {
        match self {
            Strategy::Tight => 0.03,
            Strategy::Balanced => 0.01,
            Strategy::Spread => 0.002,
        }
    }

    fn spacing(self) -> f64 {
        match self {
            Strategy::Tight => 0.1,
            Strategy::Balanced => 0.2,
            Strategy::Spread => 0.4,
        }
    }

    fn fill(self) -> f64 {
        match self {
            Strategy::Tight => 0.9,
            Strategy::Balanced => 0.7,
            Strategy::Spread => 0.5,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Ask {
    pub strategy: Strategy,
    pub verify: usize,
    pub aspects: Vec<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Trial {
    pub width: f64,
    pub height: f64,
    pub fits: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    pub congestion: f64,
    pub hpwl: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routed: Option<Routed>,
    #[serde(skip)]
    pub moves: Vec<placement::Move>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Routed {
    pub overflow: f64,
    pub wirelength: f64,
    pub stuck: usize,
    pub ok: bool,
    pub ms: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Shape {
    pub aspect: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best: Option<usize>,
    pub trials: Vec<Trial>,
}

impl Shape {
    pub fn best_trial(&self) -> Option<&Trial> {
        self.best.map(|i| &self.trials[i])
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FitReport {
    pub strategy: Strategy,
    pub current: [f64; 2],
    pub origin: [f64; 2],
    pub parts_area: f64,
    pub baseline: Trial,
    pub shapes: Vec<Shape>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best: Option<[f64; 2]>,
    pub trials: usize,
    pub ms: f64,
}

impl FitReport {
    pub fn best_trial(&self) -> Option<&Trial> {
        let [w, h] = self.best?;
        self.shapes.iter().filter_map(Shape::best_trial).find(|t| t.width == w && t.height == h)
    }

    pub fn table(&self) -> String {
        let mut t = format!(
            "strategy {:?}: parts {:.0} mm2, now {:.1} x {:.1} mm, congestion {:.2}{}\n",
            self.strategy,
            self.parts_area,
            self.current[0],
            self.current[1],
            self.baseline.congestion,
            self.baseline.why.as_ref().map(|w| format!(", does not fit: {w}")).unwrap_or_default()
        );
        t += &format!(
            "{:>7}{:>16}{:>10}{:>8}{:>10}{:>12}\n",
            "aspect", "size mm", "area mm2", "tries", "congest", "overflow"
        );
        for s in &self.shapes {
            let (size, area, cong, routed) = match s.best_trial() {
                Some(b) => (
                    format!("{:.1} x {:.1}", b.width, b.height),
                    format!("{:.0}", b.width * b.height),
                    format!("{:.2}", b.congestion),
                    b.routed.as_ref().map_or("-".into(), |r| {
                        format!("{:.1} mm{}", r.overflow, if r.ok { "" } else { " no" })
                    }),
                ),
                None => ("none fits".into(), "-".into(), "-".into(), "-".into()),
            };
            t += &format!(
                "{:>7.2}{size:>16}{area:>10}{:>8}{cong:>10}{routed:>12}",
                s.aspect,
                s.trials.len()
            );
            if let Some(l) = &s.limit {
                t += &format!("   smaller: {l}");
            }
            t += "\n";
        }
        match self.best {
            Some([w, h]) => {
                t += &format!(
                    "smallest {w:.1} x {h:.1} mm, {} tries in {:.0} ms\n",
                    self.trials, self.ms
                )
            }
            None => t += "no size fits\n",
        }
        t
    }
}

struct World<'a> {
    model: Model<'a>,
    seed: Board,
    turned: Board,
    old: Bounds,
    origin: P,
    anchored: Vec<bool>,
    hole: Vec<bool>,
    planes: Vec<bool>,
    layers: f64,
    pitch: f64,
    strategy: Strategy,
}

impl World<'_> {
    fn place(&self, w: f64, h: f64) -> Result<Board, String> {
        let [ow, oh] = self.old.size();
        let flip = (w > h) != (ow > oh) && (w - h).abs() > 1e-9 && (ow - oh).abs() > 1e-9;
        let mut bd = if flip { self.turned.clone() } else { self.seed.clone() };
        bd.outline = vec![
            self.origin,
            [self.origin[0] + w, self.origin[1]],
            [self.origin[0] + w, self.origin[1] + h],
            [self.origin[0], self.origin[1] + h],
        ];
        bd.bounds = Bounds::EMPTY;
        bd.outline.iter().for_each(|q| bd.bounds.add(*q));
        bd.keepouts.clear();
        let o = if flip {
            let c = self.old.center();
            let mut b = Bounds::EMPTY;
            b.add([c[0] - oh / 2.0, c[1] - ow / 2.0]);
            b.add([c[0] + oh / 2.0, c[1] + ow / 2.0]);
            b
        } else {
            self.old
        };
        let n = bd.bounds;
        let [ow, oh] = o.size();
        for (i, c) in bd.cells.iter_mut().enumerate() {
            let u = [(c.at[0] - o.min[0]) / ow, (c.at[1] - o.min[1]) / oh];
            let scaled = [n.min[0] + u[0] * w, n.min[1] + u[1] * h];
            let kept = |k: usize| {
                if u[k] < 0.5 {
                    n.min[k] + (c.at[k] - o.min[k])
                } else {
                    n.max[k] - (o.max[k] - c.at[k])
                }
            };
            c.at = if self.hole[i] {
                [kept(0), kept(1)]
            } else if self.anchored[i] {
                let dx = (c.at[0] - o.min[0]).min(o.max[0] - c.at[0]);
                let dy = (c.at[1] - o.min[1]).min(o.max[1] - c.at[1]);
                if dx < dy { [kept(0), scaled[1]] } else { [scaled[0], kept(1)] }
            } else {
                scaled
            };
        }
        let edge = self.model.board.rules.min_copper_to_edge.to_mm().max(0.3);
        let mut held: Vec<usize> = (0..bd.cells.len()).filter(|&i| self.anchored[i]).collect();
        held.sort_by_key(|&i| (!self.hole[i], std::cmp::Reverse((bd.cells[i].area * 1e3) as i64)));
        let mut taken: Vec<([f64; 4], bool, bool)> = Vec::new();
        for &i in &held {
            let c = bd.cells[i].clone();
            let dx = (c.at[0] - n.min[0]).min(n.max[0] - c.at[0]);
            let dy = (c.at[1] - n.min[1]).min(n.max[1] - c.at[1]);
            let along = if dx < dy { 1 } else { 0 };
            let free = |at: P| {
                let r = placement::rect_of(&c, at, 0.0);
                let within = self.hole[i]
                    || (r[along] >= n.min[along] + edge && r[along + 2] <= n.max[along] - edge);
                within
                    && !taken.iter().any(|(o, through, bottom)| {
                        (c.through || *through || c.bottom == *bottom) && placement::overlaps(r, *o)
                    })
            };
            let slide = if self.hole[i] { 0 } else { (n.size()[along] / 0.1) as i64 };
            let spot = (0..=slide)
                .flat_map(|k| [k, -k])
                .map(|k| {
                    let mut at = c.at;
                    at[along] += k as f64 * 0.1;
                    at
                })
                .find(|&at| free(at));
            let Some(at) = spot else {
                return Err(format!("{} has no room on its edge", c.reference));
            };
            bd.cells[i].at = at;
            taken.push((placement::rect_of(&bd.cells[i], at, 0.0), c.through, c.bottom));
        }
        let (_, failed) = placement::legalise(
            &self.model,
            &mut bd,
            &HashMap::new(),
            self.strategy.spacing(),
            true,
        );
        match failed.first() {
            Some(f) => Err(format!("placement: {f}")),
            None => Ok(bd),
        }
    }

    fn congestion(&self, bd: &Board, w: f64, h: f64) -> f64 {
        let (nx, ny) = ((w / TILE).ceil() as usize, (h / TILE).ceil() as usize);
        let mut demand = vec![0.0f64; nx * ny];
        let n0 = bd.bounds.min;
        let l = &self.model.layout;
        let mut pins: HashMap<usize, Vec<P>> = HashMap::new();
        for c in &bd.cells {
            for &(net, off) in &c.pins {
                if !self.planes[net] {
                    pins.entry(net).or_default().push([c.at[0] + off[0], c.at[1] + off[1]]);
                }
            }
        }
        for (net, ps) in pins {
            if ps.len() < 2 || net >= l.nets.len() {
                continue;
            }
            let mut b = Bounds::EMPTY;
            ps.iter().for_each(|q| b.add(*q));
            let [bw, bh] = b.size();
            let steiner = 1.0 + 0.15 * (ps.len() as f64 - 3.0).max(0.0).sqrt();
            let length = (bw + bh) * steiner;
            let (x0, x1) =
                (((b.min[0] - n0[0]) / TILE) as usize, ((b.max[0] - n0[0]) / TILE) as usize);
            let (y0, y1) =
                (((b.min[1] - n0[1]) / TILE) as usize, ((b.max[1] - n0[1]) / TILE) as usize);
            let (x1, y1) = (x1.min(nx - 1), y1.min(ny - 1));
            let (x0, y0) = (x0.min(x1), y0.min(y1));
            let tiles = ((x1 - x0 + 1) * (y1 - y0 + 1)) as f64;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    demand[y * nx + x] += length / tiles;
                }
            }
        }
        let cap = self.layers * TILE * TILE / self.pitch * self.strategy.fill();
        let total: f64 = demand.iter().sum();
        if total <= 0.0 {
            return 0.0;
        }
        let mut sorted = demand.clone();
        sorted.sort_by(|a, b| b.total_cmp(a));
        let top = (sorted.len() / 20).max(1);
        sorted[..top].iter().sum::<f64>() / top as f64 / cap
    }

    fn try_size(&self, w: f64, h: f64) -> Trial {
        let mut t = Trial {
            width: w,
            height: h,
            fits: false,
            why: None,
            congestion: 0.0,
            hpwl: 0.0,
            routed: None,
            moves: Vec::new(),
        };
        match self.place(w, h) {
            Err(e) => t.why = Some(e),
            Ok(bd) => {
                t.congestion = self.congestion(&bd, w, h);
                t.hpwl = placement::hpwl(&bd);
                if t.congestion > 1.0 {
                    t.why = Some(format!(
                        "the busiest twentieth of the board wants {:.0}% of its routing room",
                        t.congestion * 100.0
                    ));
                } else {
                    t.fits = true;
                    t.moves = placement::moves_of(&bd);
                }
            }
        }
        t
    }
}

fn route_check(
    inputs: &LayoutInputs,
    text: &str,
    origin: P,
    strategy: Strategy,
    stuck: Option<usize>,
    t: &Trial,
) -> Result<Routed, String> {
    let t0 = std::time::Instant::now();
    let mut inp = inputs.clone();
    inp.board.outline = Some(agentee_core::board::Outline::Rect {
        origin: agentee_core::units::Point::mm(origin[0], origin[1]),
        size: agentee_core::units::Point::mm(t.width, t.height),
        corner_radius: match inputs.board.outline.as_ref() {
            Some(agentee_core::board::Outline::Rect { corner_radius, .. }) => *corner_radius,
            _ => agentee_core::units::Length::ZERO,
        },
    });
    let text =
        crate::start::reset(text, &crate::start::Reset { routing: true, ..Default::default() })?;
    let text = crate::write_moves(&text, &t.moves)?;
    let run = crate::Run {
        from: Some("access".into()),
        to: Some("global".into()),
        only: None,
        watch: None,
        stop: None,
    };
    let r = crate::run_text(&inp, &text, &run)?;
    let term = |k: &str| r.score.terms.get(k).map_or(0.0, |t| t.raw);
    let (overflow, wirelength, stuck_now) =
        (term("overflow"), term("wirelength"), term("access") as usize);
    Ok(Routed {
        overflow,
        wirelength,
        stuck: stuck_now,
        ok: overflow <= strategy.overflow_share() * wirelength.max(1.0)
            && stuck.is_none_or(|s| stuck_now <= s),
        ms: t0.elapsed().as_secs_f64() * 1000.0,
    })
}

fn snap(v: f64) -> f64 {
    (v / GRID).ceil() * GRID
}

fn search_shape(
    world: &World,
    aspect: f64,
    fixed: (Option<f64>, Option<f64>),
    from: f64,
    start: f64,
) -> Shape {
    let size = |area: f64| -> (f64, f64) {
        match fixed {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, snap(area / w)),
            (None, Some(h)) => (snap(area / h), h),
            (None, None) => (snap((area * aspect).sqrt()), snap((area / aspect).sqrt())),
        }
    };
    let mut out = Shape { aspect, limit: None, best: None, trials: Vec::new() };
    let check = |area: f64, out: &mut Shape| -> bool {
        let (w, h) = size(area);
        if let Some(k) = out.trials.iter().position(|t| t.width == w && t.height == h) {
            return out.trials[k].fits;
        }
        let t = world.try_size(w, h);
        let fits = t.fits;
        out.trials.push(t);
        if fits
            && out.best.is_none_or(|b| {
                let o = &out.trials[b];
                w * h < o.width * o.height
            })
        {
            out.best = Some(out.trials.len() - 1);
        }
        fits
    };
    if matches!(fixed, (Some(_), Some(_))) {
        check(0.0, &mut out);
        return out;
    }
    let mut hi = start;
    let mut grown = 0;
    while !check(hi, &mut out) {
        grown += 1;
        if grown > GROWS {
            return out;
        }
        hi *= GROW * GROW;
    }
    let mut lo = from.min(hi);
    for _ in 0..STEPS {
        let mid = ((lo.sqrt() + hi.sqrt()) / 2.0).powi(2);
        let (w0, h0) = size(hi);
        let (w1, h1) = size(mid);
        if (w0 - w1).abs() < GRID / 2.0 && (h0 - h1).abs() < GRID / 2.0 {
            break;
        }
        if check(mid, &mut out) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let best = out.best_trial().map(|b| b.width * b.height).unwrap_or(f64::MAX);
    out.limit = out
        .trials
        .iter()
        .filter(|t| !t.fits && t.width * t.height < best)
        .max_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)))
        .and_then(|t| t.why.clone());
    for t in &mut out.trials {
        t.moves.clear();
    }
    if let Some(b) = out.best {
        let (w, h) = (out.trials[b].width, out.trials[b].height);
        out.trials[b].moves = world.try_size(w, h).moves;
    }
    out
}

fn parts_area(bd: &Board) -> f64 {
    bd.cells.iter().map(|c| c.area).sum()
}

pub fn fit(inputs: &LayoutInputs, text: &str, ask: &Ask) -> Result<FitReport, String> {
    let t0 = std::time::Instant::now();
    let loaded = crate::load(inputs, text)?;
    if loaded.layout.outline.len() < 3 {
        return Err("the board has no outline to start from".into());
    }
    let model = loaded.model();
    let mut seed = placement::build(&model);
    for c in &mut seed.cells {
        c.fixed = false;
    }
    let l = &model.layout;
    let anchored: Vec<bool> = l
        .parts
        .iter()
        .map(|p| {
            place::edge_mount(&p.footprint)
                || place::role_of(&p.reference, &p.footprint_name, &p.footprint)
                    == place::Role::Hole
        })
        .collect();
    let hole: Vec<bool> = l
        .parts
        .iter()
        .map(|p| place::role_of(&p.reference, &p.footprint_name, &p.footprint) == place::Role::Hole)
        .collect();
    let full: Vec<(&str, &Vec<String>)> = model
        .file
        .zones
        .iter()
        .filter(|z| z.outline.as_ref().is_none_or(|o| o.is_empty()))
        .map(|z| (z.net.as_str(), &z.layers))
        .collect();
    let planes: Vec<bool> = l
        .nets
        .iter()
        .map(|n| {
            place::is_ground(&n.name)
                || full.iter().any(|(z, _)| *z == n.name)
                || place::is_power_net(model.board, &n.name, &n.class)
        })
        .collect();
    let plane_layers: Vec<&String> = full.iter().flat_map(|(_, ls)| ls.iter()).collect();
    let copper = &l.copper;
    let layers = copper
        .iter()
        .enumerate()
        .filter(|(i, c)| *i == 0 || *i + 1 == copper.len() || !plane_layers.contains(c))
        .count() as f64;
    let default = model.board.netclasses.iter().find(|c| c.name == "Default");
    let pitch = default.map_or(0.4, |c| c.track_width.to_mm() + c.clearance.to_mm());
    let mut old = Bounds::EMPTY;
    l.outline.iter().for_each(|q| old.add(*q));
    let current = old.size();
    let mut turned = seed.clone();
    let mid = old.center();
    for c in &mut turned.cells {
        let rel = [c.at[0] - mid[0], c.at[1] - mid[1]];
        c.at = [mid[0] - rel[1], mid[1] + rel[0]];
        placement::rotate_cell(c, 90.0);
    }
    let world = World {
        turned,
        old,
        origin: old.min,
        anchored,
        hole,
        planes,
        layers,
        pitch,
        strategy: ask.strategy,
        seed,
        model,
    };
    let parts = parts_area(&world.seed);
    let baseline = world.try_size(current[0], current[1]);
    let mut aspects: Vec<f64> = if ask.aspects.is_empty() {
        let mut v = vec![current[0] / current[1]];
        for k in 0..=12 {
            let a = 2f64.powf(-1.25 + k as f64 * 2.5 / 12.0);
            v.push((a * 100.0).round() / 100.0);
        }
        v
    } else {
        ask.aspects.clone()
    };
    if ask.width.is_some() || ask.height.is_some() {
        aspects = vec![match (ask.width, ask.height) {
            (Some(w), Some(h)) => w / h,
            (Some(w), None) => w / current[1],
            (None, Some(h)) => current[0] / h,
            (None, None) => unreachable!(),
        }];
    }
    let mut seen: Vec<f64> = Vec::new();
    aspects.retain(|a| {
        let keep = a.is_finite() && *a > 0.0 && !seen.iter().any(|s| (s - a).abs() < 0.02);
        seen.push(*a);
        keep
    });
    let start = current[0] * current[1];
    let from = (parts * FLOOR).max(1.0);
    let fixed = (ask.width, ask.height);
    let shapes: Vec<Shape> =
        aspects.iter().map(|&a| search_shape(&world, a, fixed, from, start)).collect();
    let mut shapes = shapes;
    let mut baseline = baseline;
    let mut stuck = None;
    if ask.verify > 0 && baseline.fits {
        let r = route_check(inputs, text, world.origin, ask.strategy, None, &baseline)?;
        stuck = Some(r.stuck);
        baseline.routed = Some(r);
    }
    let mut order: Vec<usize> = (0..shapes.len()).filter(|&s| shapes[s].best.is_some()).collect();
    let area = |s: &Shape| s.best_trial().map_or(f64::MAX, |t| t.width * t.height);
    order.sort_by(|&a, &b| area(&shapes[a]).total_cmp(&area(&shapes[b])));
    let mut best = None;
    for &s in order.iter().take(ask.verify) {
        let b = shapes[s].best.unwrap();
        let r = route_check(inputs, text, world.origin, ask.strategy, stuck, &shapes[s].trials[b])?;
        let ok = r.ok;
        shapes[s].trials[b].routed = Some(r);
        if ok {
            best = Some(b).map(|b| [shapes[s].trials[b].width, shapes[s].trials[b].height]);
            break;
        }
    }
    if ask.verify == 0 {
        best = shapes
            .iter()
            .filter_map(Shape::best_trial)
            .min_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)))
            .map(|t| [t.width, t.height]);
    }
    let trials = shapes.iter().map(|s| s.trials.len()).sum::<usize>() + 1;
    Ok(FitReport {
        strategy: ask.strategy,
        current,
        origin: world.origin,
        parts_area: parts,
        baseline,
        shapes,
        best,
        trials,
        ms: t0.elapsed().as_secs_f64() * 1000.0,
    })
}
