use super::edges;
use crate::geom::P;
use crate::graphic::Bounds;

pub(super) struct Winding {
    y0: f64,
    band: f64,
    rows: Vec<Vec<(P, P)>>,
}

impl Winding {
    pub fn new(rings: &[Vec<P>]) -> Winding {
        let mut b = Bounds::EMPTY;
        rings.iter().flatten().for_each(|p| b.add(*p));
        let count: usize = rings.iter().map(Vec::len).sum();
        if b.is_empty() || count == 0 {
            return Winding { y0: 0.0, band: 1.0, rows: Vec::new() };
        }
        let height = (b.max[1] - b.min[1]).max(1e-9);
        let n = (count / 16).clamp(1, 4096);
        let band = height / n as f64;
        let mut rows = vec![Vec::new(); n];
        let row = |y: f64| (((y - b.min[1]) / band).floor().max(0.0) as usize).min(n - 1);
        for r in rings {
            for (a, c) in edges(r) {
                if a[1] == c[1] {
                    continue;
                }
                let (lo, hi) = (row(a[1].min(c[1])), row(a[1].max(c[1])));
                rows[lo..=hi].iter_mut().for_each(|r| r.push((a, c)));
            }
        }
        Winding { y0: b.min[1], band, rows }
    }

    pub fn winding(&self, p: P) -> i32 {
        if self.rows.is_empty() {
            return 0;
        }
        let k = (p[1] - self.y0) / self.band;
        if k < 0.0 || k as usize >= self.rows.len() {
            return 0;
        }
        let mut w = 0;
        for (a, c) in &self.rows[k as usize] {
            if (a[1] > p[1]) != (c[1] > p[1]) {
                let x = a[0] + (p[1] - a[1]) / (c[1] - a[1]) * (c[0] - a[0]);
                if x > p[0] {
                    w += if c[1] > a[1] { 1 } else { -1 };
                }
            }
        }
        w
    }

    pub fn inside(&self, p: P) -> bool {
        self.winding(p) > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winding_matches_rings_with_a_hole() {
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = vec![[4.0, 4.0], [4.0, 6.0], [6.0, 6.0], [6.0, 4.0]];
        let w = Winding::new(&[outer, hole]);
        assert!(w.inside([1.0, 1.0]));
        assert!(!w.inside([5.0, 5.0]));
        assert!(!w.inside([11.0, 5.0]));
        assert!(w.inside([9.0, 5.0]));
    }
}
