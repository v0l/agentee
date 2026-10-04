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
}

#[derive(Clone, Copy, Debug)]
pub struct Source {
    pub at: usize,
    pub cost: f32,
    pub tag: u32,
    pub via_only: bool,
    pub landed: bool,
}

pub struct Found {
    pub cells: Vec<(usize, usize, usize)>,
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
        let mut target = vec![false; n];
        let mut end_tags = std::collections::HashMap::new();
        let mut any_target = false;
        for &(t, tg) in &self.targets {
            if let Some(k) = local(t) {
                target[k] = true;
                any_target = true;
                if tg != 0 {
                    end_tags.insert(k, tg);
                }
            }
        }
        if !any_target {
            return None;
        }
        let g = grid.g as f32;
        let mut heur = vec![f32::MAX; area];
        for k in 0..n {
            if target[k] {
                heur[k % area] = 0.0;
            }
        }
        let diag = g * std::f32::consts::SQRT_2;
        for y in 0..wh {
            for x in 0..ww {
                let i = y * ww + x;
                let mut v = heur[i];
                if x > 0 {
                    v = v.min(heur[i - 1] + g);
                }
                if y > 0 {
                    v = v.min(heur[i - ww] + g);
                    if x > 0 {
                        v = v.min(heur[i - ww - 1] + diag);
                    }
                    if x + 1 < ww {
                        v = v.min(heur[i - ww + 1] + diag);
                    }
                }
                heur[i] = v;
            }
        }
        for y in (0..wh).rev() {
            for x in (0..ww).rev() {
                let i = y * ww + x;
                let mut v = heur[i];
                if x + 1 < ww {
                    v = v.min(heur[i + 1] + g);
                }
                if y + 1 < wh {
                    v = v.min(heur[i + ww] + g);
                    if x + 1 < ww {
                        v = v.min(heur[i + ww + 1] + diag);
                    }
                    if x > 0 {
                        v = v.min(heur[i + ww - 1] + diag);
                    }
                }
                heur[i] = v;
            }
        }
        let entry_r2 = self.entry_r * self.entry_r;
        let mut entry = vec![false; area];
        if !self.entry.is_empty() {
            for y in 0..wh {
                for x in 0..ww {
                    let c = grid.center(x + w.x0, y + w.y0);
                    entry[y * ww + x] = self.entry.iter().any(|p| {
                        let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
                        dx * dx + dy * dy <= entry_r2
                    });
                }
            }
        }
        let planar = |l: usize, k2: usize| self.rule.track[l] || entry[k2];
        let mut cost = vec![f32::INFINITY; n];
        let mut parent = vec![u32::MAX; n];
        let mut dir = vec![START; n];
        let mut tag = std::collections::HashMap::new();
        let mut heap = BinaryHeap::new();
        let cong = self.congestion();
        let hard = self.hard;
        let rule = self.rule;
        let net = self.net;
        let mut mine = vec![false; n];
        for &c in self.own {
            if let Some(k) = local(c as usize) {
                mine[k] = true;
            }
        }
        let at = |l: usize, x: usize, y: usize| l * area + (y - w.y0) * ww + (x - w.x0);
        let ok = |l: usize, x: usize, y: usize| -> bool {
            let i3 = grid.idx(l, x, y);
            if !grid.track_ok(i3, net, rule.need[l]) {
                return false;
            }
            !hard
                || mine[at(l, x, y)]
                || rule.bucket[l].is_none_or(|b| self.soft.tracks[b][y * grid.w + x] == 0)
        };
        for s in &self.sources {
            let Some(k) = local(s.at) else { continue };
            if s.cost < cost[k] {
                cost[k] = s.cost;
                parent[k] = u32::MAX;
                dir[k] = if s.landed {
                    LANDED
                } else if s.via_only {
                    VIA_ONLY
                } else {
                    START
                };
                tag.insert(k, s.tag);
                heap.push(Node { f: s.cost + heur[k % area], i: k as u32 });
            }
        }
        let bend_cost = self.bend_cost;
        let mut hit = None;
        while let Some(Node { f, i }) = heap.pop() {
            let k = i as usize;
            let here = cost[k];
            if f > here + heur[k % area] + 1e-4 {
                continue;
            }
            if target[k] {
                hit = Some(k);
                break;
            }
            let l = k / area;
            let k2 = k % area;
            let (lx, ly) = (k2 % ww, k2 / ww);
            let (x, y) = (lx + w.x0, ly + w.y0);
            let d0 = dir[k];
            if planar(l, k2) && d0 != VIA_ONLY {
                for (d, (dx, dy)) in DIRS.iter().enumerate() {
                    let Some(turn) = bend(d0, d, bend_cost) else { continue };
                    let (nx, ny) = (lx as i64 + dx, ly as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= ww as i64 || ny >= wh as i64 {
                        continue;
                    }
                    let (nx, ny) = (nx as usize, ny as usize);
                    let j2 = ny * ww + nx;
                    if !planar(l, j2) {
                        continue;
                    }
                    let (gx, gy) = (nx + w.x0, ny + w.y0);
                    if !ok(l, gx, gy) {
                        continue;
                    }
                    let diagonal = *dx != 0 && *dy != 0;
                    if diagonal && !(ok(l, gx, y) && ok(l, x, gy)) {
                        continue;
                    }
                    let len = if diagonal { diag } else { g };
                    let c2 = gy * grid.w + gx;
                    let count = if mine[l * area + j2] {
                        0.0
                    } else {
                        rule.bucket[l].map(|b| self.soft.tracks[b][c2] as f32).unwrap_or(0.0)
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
                    if c < cost[j] {
                        cost[j] = c;
                        parent[j] = k as u32;
                        dir[j] = d as u8;
                        heap.push(Node { f: c + heur[j2], i: j as u32 });
                    }
                }
            }
            if d0 == LANDED {
                continue;
            }
            let c2 = y * grid.w + x;
            for l2 in 0..nl {
                if l2 == l {
                    continue;
                }
                let j = l2 * area + k2;
                if !(planar(l2, k2) || target[j]) {
                    continue;
                }
                let mut best: Option<f32> = None;
                for (vi, &vk) in rule.vias.iter().enumerate().take(rule.class_vias) {
                    let o = &self.rules.vias[vk];
                    if !o.joins(l, l2) {
                        continue;
                    }
                    if !grid.via_ok(c2, &o.layers, vk, net, rule.via_need[vi]) {
                        continue;
                    }
                    let vb = rule.via_bucket[vi];
                    let count = self.soft.vias[vb][c2] as f32;
                    if hard && count > 0.0 {
                        continue;
                    }
                    let hist = self.soft.hist[l * plane + c2];
                    let c = (self.via_cost * o.cost) as f32
                        * ((1.0 + self.lean(j)).max(0.4) + cong * (hist + self.pres * count));
                    if best.is_none_or(|b| c < b) {
                        best = Some(c);
                    }
                }
                let Some(step) = best else { continue };
                let c = here + step;
                if c < cost[j] {
                    cost[j] = c;
                    parent[j] = k as u32;
                    dir[j] = LANDED;
                    heap.push(Node { f: c + heur[k2], i: j as u32 });
                }
            }
        }
        let end = hit?;
        let mut cells = Vec::new();
        let mut c = end;
        loop {
            let l = c / area;
            let k2 = c % area;
            cells.push((l, k2 % ww + w.x0, k2 / ww + w.y0));
            if parent[c] == u32::MAX {
                break;
            }
            c = parent[c] as usize;
        }
        cells.reverse();
        Some(Found {
            cells,
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
