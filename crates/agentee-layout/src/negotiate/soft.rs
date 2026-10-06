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
}

#[derive(Clone, Debug)]
pub struct Run {
    pub layer: usize,
    pub points: Vec<P>,
    pub width: f64,
    pub neck: bool,
}

pub type Footprint = (Vec<Vec<u32>>, Vec<Vec<u32>>);

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
        Copper { segs, vias, clearance, shadow, only_shadow: false, domain: rule.domain }
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
            Some(fp) if n > 0 && fp.0[bucket].binary_search(&(c as u32)).is_ok() => n - 1,
            _ => n,
        }
    }

    pub fn via_less(&self, bucket: usize, c: usize, old: Option<&Footprint>) -> u8 {
        let n = self.via(bucket, c);
        match old {
            Some(fp) if n > 0 && fp.1[bucket].binary_search(&(c as u32)).is_ok() => n - 1,
            _ => n,
        }
    }

    pub fn footprint(grid: &Grid, rules: &Rules, c: &Copper) -> (Vec<Vec<u32>>, Vec<Vec<u32>>) {
        let slack = rules.slack + grid.g * std::f64::consts::FRAC_1_SQRT_2;
        let mut tracks: Vec<Vec<u32>> = vec![Vec::new(); rules.buckets.len()];
        let mut vias: Vec<Vec<u32>> = vec![Vec::new(); rules.via_buckets.len()];
        let mut reach: Vec<(bool, usize, f64)> = Vec::new();
        let stamp = |shape: &Shape,
                     reach: &[(bool, usize, f64)],
                     tracks: &mut Vec<Vec<u32>>,
                     vias: &mut Vec<Vec<u32>>| {
            let far = reach.iter().map(|r| r.2).fold(0.0, f64::max);
            grid.near(shape, far, |x, y, d| {
                let cell = (y * grid.w + x) as u32;
                for &(via, k, r) in reach {
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
                let gap = c.clearance.max(bk.c).max(rules.iso.apart(c.domain, bk.domain, l));
                reach.push((false, bi, h + bk.h + gap + slack));
            }
            for (vi, vb) in
                rules.via_buckets.iter().enumerate().filter(|(_, v)| own && v.layers.contains(&l))
            {
                let gap = c.clearance.max(vb.c).max(rules.iso.apart(c.domain, vb.domain, l));
                let r = (h + vb.r + gap).max(h + vb.dr + rules.hole_cu) + slack;
                reach.push((true, vi, r));
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
                    reach.push((false, bi, h + bk.h + super::rules::SHADOW_GAP + slack));
                }
            }
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
                reach.push((false, bi, r + slack));
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
                reach.push((true, vi, copper.max(o.dr + vb.dr + rules.hole_gap) + slack));
            }
            stamp(&Shape::Circle(at, 0.0), &reach, &mut tracks, &mut vias);
        }
        for v in tracks.iter_mut().chain(vias.iter_mut()) {
            v.sort_unstable();
            v.dedup();
        }
        (tracks, vias)
    }

    pub fn apply(&self, fp: &(Vec<Vec<u32>>, Vec<Vec<u32>>), add: bool) {
        for (map, cells) in self.tracks.iter().chain(&self.vias).zip(fp.0.iter().chain(&fp.1)) {
            for &c in cells {
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
