use super::grid::Grid;
use super::rules::{NetRule, Rules};
use super::soft::Soft;
use agentee_core::geom::{self, P};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub const DIRS: [(i64, i64); 8] =
    [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
const START: u8 = 8;
const LANDED: u8 = 9;
const VIA_ONLY: u8 = 10;
pub const BLOCKED: i8 = i8::MIN;

#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

pub const MAX_WINDOW_CELLS: usize = 2_000_000;

impl Window {
    pub fn around(grid: &Grid, lo: P, hi: P, margin: f64) -> Window {
        let (x0, y0, x1, y1) = grid.span(lo, hi, margin);
        Window { x0, y0, x1: x1.max(x0), y1: y1.max(y0) }.cap()
    }

    pub fn is_whole(&self, grid: &Grid) -> bool {
        self.x0 == 0 && self.y0 == 0 && self.x1 >= grid.w - 1 && self.y1 >= grid.h - 1
    }

    fn cap(mut self) -> Window {
        while self.ww() * self.wh() > MAX_WINDOW_CELLS {
            let wide = self.ww() > self.wh();
            if (wide && self.x0 == self.x1) || (!wide && self.y0 == self.y1) {
                break;
            }
            if wide {
                let cx = (self.x0 + self.x1) / 2;
                self.x0 = (self.x0 + 1).min(cx);
                self.x1 = self.x1.saturating_sub(1).max(self.x0);
            } else {
                let cy = (self.y0 + self.y1) / 2;
                self.y0 = (self.y0 + 1).min(cy);
                self.y1 = self.y1.saturating_sub(1).max(self.y0);
            }
        }
        self
    }

    pub fn fits(&self) -> bool {
        self.ww() * self.wh() <= MAX_WINDOW_CELLS
    }

    pub fn ww(&self) -> usize {
        self.x1 - self.x0 + 1
    }

    pub fn wh(&self) -> usize {
        self.y1 - self.y0 + 1
    }

    pub fn contains(&self, x: usize, y: usize) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

pub struct Query<'a> {
    pub grid: &'a Grid,
    pub soft: &'a Soft,
    pub rules: &'a Rules,
    pub rule: &'a NetRule,
    pub net: u16,
    pub window: Window,
    pub sources: Vec<Source>,
    pub targets: Vec<(usize, u32)>,
    pub entry: Vec<P>,
    pub entry_r: f64,
    pub pres: f32,
    pub hard: bool,
    pub via_cost: f64,
    pub bend_cost: f64,
    pub zone: &'a [u16],
    pub zone_cost: f64,
    pub extra: Option<&'a [i8]>,
    pub outside: f32,
    pub gain: f32,
    pub own: &'a std::collections::HashSet<u32>,
    pub holes: &'a [(P, f64)],
}

#[derive(Clone, Copy, Debug)]
pub struct Source {
    pub at: usize,
    pub cost: f32,
    pub tag: u32,
    pub via_only: bool,
    pub landed: bool,
}

#[derive(Clone, Copy)]
struct Slot {
    cost: f32,
    parent: u32,
    stamp: u32,
    dir: u8,
    since: u8,
}

const FRESH: Slot =
    Slot { cost: f32::INFINITY, parent: u32::MAX, stamp: 0, dir: START, since: u8::MAX };
const TARGET: u8 = 1;
const BLOCK: usize = 4;
const MINE: u8 = 2;

#[derive(Default)]
struct Scratch {
    slots: Vec<Slot>,
    flags: Vec<(u32, u8)>,
    heur: Vec<f32>,
    entry: Vec<bool>,
    epoch: u32,
}

impl Scratch {
    fn start(&mut self, n: usize) {
        if self.epoch == u32::MAX {
            self.slots.clear();
            self.flags.clear();
            self.epoch = 0;
        }
        self.epoch += 1;
        if self.slots.len() < n {
            self.slots.resize(n, FRESH);
            self.flags.resize(n, (0, 0));
        }
    }
}

#[inline]
fn slot(v: &[Slot], epoch: u32, k: usize) -> Slot {
    let s = v[k];
    if s.stamp == epoch { s } else { FRESH }
}

#[inline]
fn flag(v: &[(u32, u8)], epoch: u32, k: usize) -> u8 {
    let f = v[k];
    if f.0 == epoch { f.1 } else { 0 }
}

#[inline]
fn set_flag(v: &mut [(u32, u8)], epoch: u32, k: usize, bit: u8) {
    if v[k].0 != epoch {
        v[k] = (epoch, 0);
    }
    v[k].1 |= bit;
}

thread_local! {
    static SCRATCH: std::cell::RefCell<Scratch> = std::cell::RefCell::new(Scratch::default());
}

fn reset<T: Copy>(v: &mut Vec<T>, n: usize, value: T) {
    v.clear();
    v.resize(n, value);
}

pub struct Found {
    pub cells: Vec<(usize, usize, usize)>,
    pub vias: Vec<Option<usize>>,
    pub tag: u32,
    pub end_tag: u32,
}

#[derive(Clone, Copy, PartialEq)]
struct Node {
    f: f32,
    i: u32,
}

impl Eq for Node {}

impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.total_cmp(&self.f).then(o.i.cmp(&self.i))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

fn bend(d0: u8, d1: usize, cost: f64) -> Option<f64> {
    if d0 >= START {
        return Some(0.0);
    }
    let a = d0 as i64;
    let b = d1 as i64;
    match (a - b).rem_euclid(8).min((b - a).rem_euclid(8)) {
        0 => Some(0.0),
        1 => Some(cost),
        2 => Some(3.0 * cost),
        _ => None,
    }
}

impl Query<'_> {
    #[inline]
    fn lean(&self, j: usize) -> f32 {
        match self.extra.map(|x| x[j]) {
            Some(1) => self.outside,
            Some(-1) => -self.gain,
            _ => 0.0,
        }
    }

    #[inline]
    fn blocked(&self, j: usize) -> bool {
        self.extra.is_some_and(|x| x[j] == BLOCKED)
    }

    pub fn congestion(&self) -> f32 {
        (1.0 - 0.9 * self.rule.crit as f32).max(0.05)
    }

    pub fn run(&self) -> Option<Found> {
        let grid = self.grid;
        let w = self.window;
        let (ww, wh) = (w.ww(), w.wh());
        let nl = grid.nl;
        let area = ww * wh;
        let n = area * nl;
        let plane = grid.plane();
        let local = |i3: usize| -> Option<usize> {
            let l = i3 / plane;
            let c = i3 % plane;
            let (x, y) = (c % grid.w, c / grid.w);
            w.contains(x, y).then(|| l * area + (y - w.y0) * ww + (x - w.x0))
        };
        SCRATCH.with(|cell| {
            let mut s = cell.borrow_mut();
            self.search(&mut s, local, n, area, ww, wh, plane)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn search(
        &self,
        scratch: &mut Scratch,
        local: impl Fn(usize) -> Option<usize>,
        n: usize,
        area: usize,
        ww: usize,
        wh: usize,
        plane: usize,
    ) -> Option<Found> {
        let grid = self.grid;
        let w = self.window;
        let nl = grid.nl;
        scratch.start(n);
        let epoch = scratch.epoch;
        let Scratch { slots, flags, heur, entry, .. } = scratch;
        let g = grid.g as f32;
        let (bw, bh) = (ww.div_ceil(BLOCK), wh.div_ceil(BLOCK));
        reset(heur, bw * bh, f32::MAX);
        let mut end_tags = std::collections::HashMap::new();
        let mut any_target = false;
        for &(t, tg) in &self.targets {
            if let Some(k) = local(t) {
                set_flag(flags, epoch, k, TARGET);
                let k2 = k % area;
                heur[(k2 / ww / BLOCK) * bw + (k2 % ww) / BLOCK] = 0.0;
                any_target = true;
                if tg != 0 {
                    end_tags.insert(k, tg);
                }
            }
        }
        if !any_target {
            return None;
        }
        let diag = g * std::f32::consts::SQRT_2;
        let (step, step_d) = (g * BLOCK as f32, diag * BLOCK as f32);
        for y in 0..bh {
            for x in 0..bw {
                let i = y * bw + x;
                let mut v = heur[i];
                if x > 0 {
                    v = v.min(heur[i - 1] + step);
                }
                if y > 0 {
                    v = v.min(heur[i - bw] + step);
                    if x > 0 {
                        v = v.min(heur[i - bw - 1] + step_d);
                    }
                    if x + 1 < bw {
                        v = v.min(heur[i - bw + 1] + step_d);
                    }
                }
                heur[i] = v;
            }
        }
        for y in (0..bh).rev() {
            for x in (0..bw).rev() {
                let i = y * bw + x;
                let mut v = heur[i];
                if x + 1 < bw {
                    v = v.min(heur[i + 1] + step);
                }
                if y + 1 < bh {
                    v = v.min(heur[i + bw] + step);
                    if x + 1 < bw {
                        v = v.min(heur[i + bw + 1] + step_d);
                    }
                    if x > 0 {
                        v = v.min(heur[i + bw - 1] + step_d);
                    }
                }
                heur[i] = v;
            }
        }
        let loose = (BLOCK - 1) as f32 * diag;
        let heur = &*heur;
        let h = |k2: usize| (heur[(k2 / ww / BLOCK) * bw + (k2 % ww) / BLOCK] - loose).max(0.0);
        let entry_r2 = self.entry_r * self.entry_r;
        let has_entry = !self.entry.is_empty();
        if has_entry {
            reset(entry, area, false);
        }
        for p in &self.entry {
            let (x0, y0, x1, y1) = grid.span(*p, *p, self.entry_r);
            if x1 == usize::MAX || y1 == usize::MAX {
                continue;
            }
            for y in y0.max(w.y0)..=y1.min(w.y1) {
                for x in x0.max(w.x0)..=x1.min(w.x1) {
                    let c = grid.center(x, y);
                    let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
                    if dx * dx + dy * dy <= entry_r2 {
                        entry[(y - w.y0) * ww + (x - w.x0)] = true;
                    }
                }
            }
        }
        let entry = &*entry;
        let planar = |l: usize, k2: usize| self.rule.track[l] || (has_entry && entry[k2]);
        let spacing = self
            .rule
            .vias
            .iter()
            .map(|&k| 2.0 * self.rules.vias[k].dr + self.rules.hole_gap)
            .fold(0.0, f64::max);
        let gap_cells = (spacing / grid.g).ceil().min(250.0) as u8;
        let mut tag = std::collections::HashMap::new();
        let mut heap = BinaryHeap::new();
        let cong = self.congestion();
        let hard = self.hard;
        let rule = self.rule;
        let net = self.net;
        for &c in self.own {
            if let Some(k) = local(c as usize) {
                set_flag(flags, epoch, k, MINE);
            }
        }
        let flags = &*flags;
        let at = |l: usize, x: usize, y: usize| l * area + (y - w.y0) * ww + (x - w.x0);
        let ok = |l: usize, x: usize, y: usize| -> bool {
            let i3 = grid.idx(l, x, y);
            if !grid.track_ok(i3, net, rule.need[l]) {
                return false;
            }
            !hard
                || flag(flags, epoch, at(l, x, y)) & MINE != 0
                || rule.bucket[l].is_none_or(|b| self.soft.track(b, y * grid.w + x) == 0)
        };
        for s in &self.sources {
            let Some(k) = local(s.at) else { continue };
            if s.cost < slot(slots, epoch, k).cost {
                let dir = if s.landed {
                    LANDED
                } else if s.via_only {
                    VIA_ONLY
                } else {
                    START
                };
                slots[k] = Slot {
                    cost: s.cost,
                    parent: u32::MAX,
                    stamp: epoch,
                    dir,
                    since: if s.landed { 0 } else { u8::MAX },
                };
                tag.insert(k, s.tag);
                heap.push(Node { f: s.cost + h(k % area), i: k as u32 });
            }
        }
        let mut no_via = vec![false; if self.holes.is_empty() { 0 } else { area }];
        let widest = rule.vias.iter().map(|&k| self.rules.vias[k].dr).fold(0.0, f64::max);
        for &(c, r) in self.holes {
            let reach = widest + r + self.rules.hole_gap + 0.002;
            let (x0, y0, x1, y1) = grid.span(c, c, reach);
            if x1 == usize::MAX || y1 == usize::MAX {
                continue;
            }
            for y in y0.max(w.y0)..=y1.min(w.y1) {
                for x in x0.max(w.x0)..=x1.min(w.x1) {
                    let d = geom::dist(grid.center(x, y), c);
                    if d > 1e-6 && d < reach {
                        no_via[(y - w.y0) * ww + (x - w.x0)] = true;
                    }
                }
            }
        }
        let bend_cost = self.bend_cost;
        let mut via_kind: std::collections::HashMap<usize, usize> =
            std::collections::HashMap::new();
        let mut hit = None;
        while let Some(Node { f, i }) = heap.pop() {
            let k = i as usize;
            let me = slot(slots, epoch, k);
            let here = me.cost;
            if f > here + h(k % area) + 1e-4 {
                continue;
            }
            if flag(flags, epoch, k) & TARGET != 0 {
                hit = Some(k);
                break;
            }
            let l = k / area;
            let k2 = k % area;
            let (lx, ly) = (k2 % ww, k2 / ww);
            let (x, y) = (lx + w.x0, ly + w.y0);
            let d0 = me.dir;
            if planar(l, k2) && d0 != VIA_ONLY {
                let mut open = [false; 8];
                for (d, (dx, dy)) in DIRS.iter().enumerate() {
                    let (nx, ny) = (lx as i64 + dx, ly as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= ww as i64 || ny >= wh as i64 {
                        continue;
                    }
                    let (nx, ny) = (nx as usize, ny as usize);
                    let j2 = ny * ww + nx;
                    open[d] = planar(l, j2)
                        && !self.blocked(l * area + j2)
                        && ok(l, nx + w.x0, ny + w.y0);
                }
                for (d, (dx, dy)) in DIRS.iter().enumerate() {
                    if !open[d] {
                        continue;
                    }
                    let diagonal = d % 2 == 1;
                    if diagonal && !(open[d - 1] && open[(d + 1) % 8]) {
                        continue;
                    }
                    let Some(turn) = bend(d0, d, bend_cost) else { continue };
                    let (nx, ny) = ((lx as i64 + dx) as usize, (ly as i64 + dy) as usize);
                    let j2 = ny * ww + nx;
                    let (gx, gy) = (nx + w.x0, ny + w.y0);
                    let len = if diagonal { diag } else { g };
                    let c2 = gy * grid.w + gx;
                    let count = if flag(flags, epoch, l * area + j2) & MINE != 0 {
                        0.0
                    } else {
                        rule.bucket[l].map(|b| self.soft.track(b, c2) as f32).unwrap_or(0.0)
                    };
                    let hist = self.soft.hist[l * plane + c2];
                    let zn = self.zone[l * plane + c2];
                    let zone =
                        if zn != u16::MAX && zn != net { self.zone_cost as f32 } else { 0.0 };
                    let j = l * area + j2;
                    let lean = self.lean(j);
                    let step = len
                        * ((1.0 + zone + lean).max(0.4) + cong * (hist + self.pres * count))
                        + turn as f32;
                    let c = here + step;
                    if c < slot(slots, epoch, j).cost {
                        slots[j] = Slot {
                            cost: c,
                            parent: k as u32,
                            stamp: epoch,
                            dir: d as u8,
                            since: me.since.saturating_add(1),
                        };
                        heap.push(Node { f: c + h(j2), i: j as u32 });
                    }
                }
            }
            if d0 == LANDED || me.since < gap_cells || no_via.get(k2).copied().unwrap_or(false) {
                continue;
            }
            let c2 = y * grid.w + x;
            for l2 in 0..nl {
                if l2 == l {
                    continue;
                }
                let j = l2 * area + k2;
                if !(planar(l2, k2) || flag(flags, epoch, j) & TARGET != 0) || self.blocked(j) {
                    continue;
                }
                let mut best: Option<(f32, usize)> = None;
                for (vi, &vk) in rule.vias.iter().enumerate().take(rule.class_vias) {
                    let o = &self.rules.vias[vk];
                    if !o.joins(l, l2) {
                        continue;
                    }
                    if !grid.via_ok(c2, &o.layers, vk, net, rule.via_need[vi]) {
                        continue;
                    }
                    let vb = rule.via_bucket[vi];
                    let count = self.soft.via(vb, c2) as f32;
                    if hard && count > 0.0 {
                        continue;
                    }
                    let hist = self.soft.hist[l * plane + c2];
                    let c = (self.via_cost * o.cost) as f32
                        * ((1.0 + self.lean(j)).max(0.4) + cong * (hist + self.pres * count));
                    if best.is_none_or(|b| c < b.0) {
                        best = Some((c, vk));
                    }
                }
                let Some((step, vk)) = best else { continue };
                let c = here + step;
                if c < slot(slots, epoch, j).cost {
                    slots[j] =
                        Slot { cost: c, parent: k as u32, stamp: epoch, dir: LANDED, since: 0 };
                    via_kind.insert(j, vk);
                    heap.push(Node { f: c + h(k2), i: j as u32 });
                }
            }
        }
        let end = hit?;
        let mut cells = Vec::new();
        let mut vias = Vec::new();
        let mut c = end;
        loop {
            let l = c / area;
            let k2 = c % area;
            cells.push((l, k2 % ww + w.x0, k2 / ww + w.y0));
            let sc = slot(slots, epoch, c);
            vias.push(if sc.dir == LANDED && sc.parent != u32::MAX {
                via_kind.get(&c).copied()
            } else {
                None
            });
            if sc.parent == u32::MAX {
                break;
            }
            c = sc.parent as usize;
        }
        cells.reverse();
        vias.reverse();
        Some(Found {
            cells,
            vias,
            tag: tag.get(&c).copied().unwrap_or(0),
            end_tag: end_tags.get(&end).copied().unwrap_or(0),
        })
    }
}

pub fn seg_cells(grid: &Grid, p: P, q: P) -> Vec<(usize, usize)> {
    let n = (geom::dist(p, q) / (grid.g * 0.25)).ceil().max(1.0) as usize;
    let mut out: Vec<(usize, usize)> = Vec::new();
    for k in 0..=n {
        let t = k as f64 / n as f64;
        let at = [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
        let (x, y) = grid.cell(at);
        if grid.inside(x, y) {
            let c = (x as usize, y as usize);
            if out.last() != Some(&c) {
                out.push(c);
            }
        }
    }
    out
}
