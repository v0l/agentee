use super::grid::{Grid, Shape};
use super::rules::Rules;
use agentee_core::geom::P;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

#[derive(Clone, Debug, Default)]
pub struct Piece {
    pub tracks: Vec<Run>,
    pub vias: Vec<(P, usize)>,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub layer: usize,
    pub points: Vec<P>,
    pub width: f64,
    pub neck: bool,
}

pub struct Soft {
    tracks: Vec<Vec<AtomicU8>>,
    vias: Vec<Vec<AtomicU8>>,
    pub hist: Vec<f32>,
}

pub struct Copper {
    pub segs: Vec<(usize, P, P, f64)>,
    pub vias: Vec<(P, usize)>,
    pub clearance: f64,
}

impl Copper {
    pub fn of(pieces: &[Piece], clearance: f64) -> Copper {
        let mut segs = Vec::new();
        let mut vias = Vec::new();
        for p in pieces {
            for r in &p.tracks {
                for w in r.points.windows(2) {
                    segs.push((r.layer, w[0], w[1], r.width / 2.0));
                }
                if r.points.len() == 1 {
                    segs.push((r.layer, r.points[0], r.points[0], r.width / 2.0));
                }
            }
            vias.extend(p.vias.iter().copied());
        }
        Copper { segs, vias, clearance }
    }
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

    fn cells(grid: &Grid, shape: &Shape, reach: f64, out: &mut Vec<u32>) {
        grid.near(shape, reach, |x, y, _| out.push((y * grid.w + x) as u32));
    }

    pub fn footprint(grid: &Grid, rules: &Rules, c: &Copper) -> (Vec<Vec<u32>>, Vec<Vec<u32>>) {
        let slack = rules.slack + grid.g * std::f64::consts::FRAC_1_SQRT_2;
        let mut tracks: Vec<Vec<u32>> = vec![Vec::new(); rules.buckets.len()];
        let mut vias: Vec<Vec<u32>> = vec![Vec::new(); rules.via_buckets.len()];
        for &(l, a, b, h) in &c.segs {
            let shape = Shape::Seg(a, b, 0.0);
            for (bi, bk) in rules.buckets.iter().enumerate().filter(|(_, bk)| bk.layer == l) {
                let reach = h + bk.h + c.clearance.max(bk.c) + slack;
                Self::cells(grid, &shape, reach, &mut tracks[bi]);
            }
            for (vi, vb) in
                rules.via_buckets.iter().enumerate().filter(|(_, v)| v.layers.contains(&l))
            {
                let reach =
                    (h + vb.r + c.clearance.max(vb.c)).max(h + vb.dr + rules.hole_cu) + slack;
                Self::cells(grid, &shape, reach, &mut vias[vi]);
            }
        }
        for &(at, k) in &c.vias {
            let o = &rules.vias[k];
            let shape = Shape::Circle(at, 0.0);
            for (bi, bk) in
                rules.buckets.iter().enumerate().filter(|(_, bk)| o.layers.contains(&bk.layer))
            {
                let reach =
                    (o.r + bk.h + c.clearance.max(bk.c)).max(o.dr + rules.hole_cu + bk.h) + slack;
                Self::cells(grid, &shape, reach, &mut tracks[bi]);
            }
            for (vi, vb) in rules.via_buckets.iter().enumerate() {
                let shares = vb.layers.iter().any(|l| o.layers.contains(l));
                let copper = if shares { o.r + vb.r + c.clearance.max(vb.c) } else { 0.0 };
                let reach = copper.max(o.dr + vb.dr + rules.hole_gap) + slack;
                Self::cells(grid, &shape, reach, &mut vias[vi]);
            }
        }
        for v in tracks.iter_mut().chain(vias.iter_mut()) {
            v.sort_unstable();
            v.dedup();
        }
        (tracks, vias)
    }

    pub fn apply(&self, fp: &(Vec<Vec<u32>>, Vec<Vec<u32>>), add: bool) {
        let step = |v: u8| if add { v.saturating_add(1) } else { v.saturating_sub(1) };
        for (map, cells) in self.tracks.iter().chain(&self.vias).zip(fp.0.iter().chain(&fp.1)) {
            for &c in cells {
                let _ = map[c as usize].fetch_update(Relaxed, Relaxed, |v| Some(step(v)));
            }
        }
    }
}

fn zeroed(n: usize) -> Vec<AtomicU8> {
    (0..n).map(|_| AtomicU8::new(0)).collect()
}
