use super::grid::{Grid, Shape, edge_dist};
use super::rules::Rules;
use super::shape::PadRef;
use agentee_core::geom::{self, P};
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

#[derive(Clone, Debug, Default)]
pub struct Piece {
    pub tracks: Vec<Run>,
    pub vias: Vec<(P, usize)>,
    pub trimmed: Option<Vec<Run>>,
    pub ms: f64,
    pub coupled: bool,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub layer: usize,
    pub points: Vec<P>,
    pub width: f64,
    pub neck: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Runs(Vec<(u32, u32)>);

impl Runs {
    fn push(&mut self, c: u32) {
        match self.0.last_mut() {
            Some(r) if r.0 + r.1 == c => r.1 += 1,
            Some(r) if c >= r.0 && c < r.0 + r.1 => {}
            _ => self.0.push((c, 1)),
        }
    }

    fn settle(&mut self) {
        self.0.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::with_capacity(self.0.len());
        for &(s, n) in &self.0 {
            match out.last_mut() {
                Some(r) if s <= r.0 + r.1 => r.1 = r.1.max(s + n - r.0),
                _ => out.push((s, n)),
            }
        }
        out.shrink_to_fit();
        self.0 = out;
    }

    pub fn contains(&self, c: u32) -> bool {
        let k = self.0.partition_point(|r| r.0 <= c);
        k > 0 && c < self.0[k - 1].0 + self.0[k - 1].1
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().flat_map(|&(s, n)| s..s + n)
    }
}

pub type Footprint = (Vec<Runs>, Vec<Runs>);

pub struct Soft {
    tracks: Vec<Vec<AtomicU8>>,
    vias: Vec<Vec<AtomicU8>>,
    pub hist: Vec<f32>,
}

pub struct Copper {
    pub segs: Vec<(usize, P, P, f64)>,
    pub vias: Vec<(P, usize)>,
    pub clearance: f64,
    pub shadow: bool,
    pub only_shadow: bool,
    pub domain: Option<usize>,
    pub zone: Option<usize>,
}

impl Copper {
    pub fn of(pieces: &[Piece], rule: &super::rules::NetRule, pads: &[PadRef]) -> Copper {
        let (clearance, shadow) = (rule.band, rule.shadows);
        let mut segs = Vec::new();
        let mut vias = Vec::new();
        for p in pieces {
            for r in &p.tracks {
                let h = r.width / 2.0;
                for w in r.points.windows(2) {
                    let (lo, hi) = Shape::Seg(w[0], w[1], 0.0).bounds();
                    let near: Vec<&PadRef> = pads
                        .iter()
                        .filter(|pd| {
                            pd.layers.contains(&r.layer)
                                && pd.outline.iter().any(|q| {
                                    q[0] >= lo[0] - 2.0
                                        && q[0] <= hi[0] + 2.0
                                        && q[1] >= lo[1] - 2.0
                                        && q[1] <= hi[1] + 2.0
                                })
                        })
                        .collect();
                    if near.is_empty() {
                        segs.push((r.layer, w[0], w[1], h));
                        continue;
                    }
                    let covered = |q: P| {
                        near.iter().any(|pd| {
                            geom::point_in_polygon(q, &pd.outline) && edge_dist(&pd.outline, q) >= h
                        })
                    };
                    for (a, b) in exposed(w[0], w[1], h, &covered) {
                        segs.push((r.layer, a, b, h));
                    }
                }
                if r.points.len() == 1 {
                    segs.push((r.layer, r.points[0], r.points[0], r.width / 2.0));
                }
            }
            vias.extend(p.vias.iter().copied());
        }
        Copper {
            segs,
            vias,
            clearance,
            shadow,
            only_shadow: false,
            domain: rule.domain,
            zone: rule.zone,
        }
    }
}

fn exposed(a: P, b: P, h: f64, covered: &impl Fn(P) -> bool) -> Vec<(P, P)> {
    let len = geom::dist(a, b);
    let n = ((len / (h.max(0.02) * 0.5)).ceil() as usize).max(1);
    let at = |k: usize| {
        let t = k as f64 / n as f64;
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    };
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for k in 0..=n {
        let hidden = covered(at(k));
        match (hidden, start) {
            (false, None) => start = Some(k.saturating_sub(1)),
            (true, Some(s)) => {
                out.push((at(s), at(k)));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((at(s), at(n)));
    }
    out
}

impl Soft {
    pub fn new(grid: &Grid, rules: &Rules) -> Soft {
        let plane = grid.plane();
        Soft {
            tracks: rules.buckets.iter().map(|_| zeroed(plane)).collect(),
            vias: rules.via_buckets.iter().map(|_| zeroed(plane)).collect(),
            hist: vec![0.0; plane * grid.nl],
        }
    }

    #[inline]
    pub fn track(&self, bucket: usize, c: usize) -> u8 {
        self.tracks[bucket][c].load(Relaxed)
    }

    #[inline]
    pub fn via(&self, bucket: usize, c: usize) -> u8 {
        self.vias[bucket][c].load(Relaxed)
    }

    pub fn track_less(&self, bucket: usize, c: usize, old: Option<&Footprint>) -> u8 {
        let n = self.track(bucket, c);
        match old {
            Some(fp) if n > 0 && fp.0[bucket].contains(c as u32) => n - 1,
            _ => n,
        }
    }

    pub fn via_less(&self, bucket: usize, c: usize, old: Option<&Footprint>) -> u8 {
        let n = self.via(bucket, c);
        match old {
            Some(fp) if n > 0 && fp.1[bucket].contains(c as u32) => n - 1,
            _ => n,
        }
    }

    pub fn footprint(grid: &Grid, rules: &Rules, c: &Copper) -> Footprint {
        let slack = rules.slack + grid.g * std::f64::consts::FRAC_1_SQRT_2;
        let mut tracks: Vec<Runs> = vec![Runs::default(); rules.buckets.len()];
        let mut vias: Vec<Runs> = vec![Runs::default(); rules.via_buckets.len()];
        type Reach = (bool, usize, f64, Option<(usize, f64)>);
        let mut reach: Vec<Reach> = Vec::new();
        let stamp = |shape: &Shape,
                     reach: &[Reach],
                     tracks: &mut Vec<Runs>,
                     vias: &mut Vec<Runs>| {
            let far = reach.first().map_or(0.0, |r| r.2);
            grid.near(shape, far, |x, y, d| {
                let cell = (y * grid.w + x) as u32;
                for &(via, k, r, local) in reach {
                    if d > r {
                        break;
                    }
                    let r = match local {
                        Some((z, near)) if d > near && rules.zones[z].holds(grid.center(x, y)) => {
                            near
                        }
                        _ => r,
                    };
                    if d <= r {
                        if via { vias[k].push(cell) } else { tracks[k].push(cell) }
                    }
                }
            });
        };
        for &(l, a, b, h) in &c.segs {
            reach.clear();
            let own = !c.only_shadow;
            for (bi, bk) in rules.buckets.iter().enumerate().filter(|(_, bk)| own && bk.layer == l)
            {
                let apart = rules.iso.apart(c.domain, bk.domain, l);
                let gap = c.clearance.max(bk.c).max(apart);
                let local = c
                    .zone
                    .filter(|z| bk.zone == Some(*z))
                    .map(|z| (z, h + bk.h + rules.zones[z].own.max(apart) + slack));
                reach.push((false, bi, h + bk.h + gap + slack, local));
            }
            for (vi, vb) in
                rules.via_buckets.iter().enumerate().filter(|(_, v)| own && v.layers.contains(&l))
            {
                let gap = c.clearance.max(vb.c).max(rules.iso.apart(c.domain, vb.domain, l));
                let r = (h + vb.r + gap).max(h + vb.dr + rules.hole_cu) + slack;
                reach.push((true, vi, r, None));
            }
            let (across, crit_only) =
                if c.shadow { (&rules.shadow[l], false) } else { (&rules.cut[l], true) };
            for &m in across {
                for (bi, bk) in rules
                    .buckets
                    .iter()
                    .enumerate()
                    .filter(|(_, bk)| bk.layer == m && (bk.crit || !crit_only))
                {
                    reach.push((false, bi, h + bk.h + super::rules::SHADOW_GAP + slack, None));
                }
            }
            reach.sort_by(|x, y| y.2.total_cmp(&x.2));
            stamp(&Shape::Seg(a, b, 0.0), &reach, &mut tracks, &mut vias);
        }
        for &(at, k) in &c.vias {
            let o = &rules.vias[k];
            reach.clear();
            for (bi, bk) in
                rules.buckets.iter().enumerate().filter(|(_, bk)| o.layers.contains(&bk.layer))
            {
                let gap = c.clearance.max(bk.c).max(rules.iso.apart(c.domain, bk.domain, bk.layer));
                let r = (o.r + bk.h + gap).max(o.dr + rules.hole_cu + bk.h);
                reach.push((false, bi, r + slack, None));
            }
            for (vi, vb) in rules.via_buckets.iter().enumerate() {
                let shared = vb.layers.iter().filter(|l| o.layers.contains(l));
                let apart = shared
                    .clone()
                    .map(|&l| rules.iso.apart(c.domain, vb.domain, l))
                    .fold(0.0, f64::max);
                let shares = shared.count() > 0;
                let copper = if shares {
                    (o.r + vb.r + c.clearance.max(vb.c).max(apart))
                        .max(o.dr + rules.hole_cu + vb.r)
                        .max(vb.dr + rules.hole_cu + o.r)
                } else {
                    0.0
                };
                reach.push((true, vi, copper.max(o.dr + vb.dr + rules.hole_gap) + slack, None));
            }
            reach.sort_by(|x, y| y.2.total_cmp(&x.2));
            stamp(&Shape::Circle(at, 0.0), &reach, &mut tracks, &mut vias);
        }
        for v in tracks.iter_mut().chain(vias.iter_mut()) {
            v.settle();
        }
        (tracks, vias)
    }

    pub fn apply(&self, fp: &Footprint, add: bool) {
        for (map, cells) in self.tracks.iter().chain(&self.vias).zip(fp.0.iter().chain(&fp.1)) {
            for c in cells.iter() {
                if add {
                    map[c as usize].fetch_add(1, Relaxed);
                } else {
                    map[c as usize].fetch_sub(1, Relaxed);
                }
            }
        }
    }
}

fn zeroed(n: usize) -> Vec<AtomicU8> {
    (0..n).map(|_| AtomicU8::new(0)).collect()
}
