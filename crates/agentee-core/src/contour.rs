use crate::geom::P;
use std::collections::HashMap;

pub fn loops(mask: &[u8], width: usize, height: usize, origin: P, cell: f64) -> Vec<Vec<P>> {
    let on = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && (x as usize) < width
            && (y as usize) < height
            && mask[y as usize * width + x as usize] != 0
    };
    let mut next: HashMap<(i64, i64), Vec<(i64, i64)>> = HashMap::new();
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            if !on(x, y) {
                continue;
            }
            let mut add = |a: (i64, i64), b: (i64, i64)| next.entry(a).or_default().push(b);
            if !on(x, y - 1) {
                add((x, y), (x + 1, y));
            }
            if !on(x + 1, y) {
                add((x + 1, y), (x + 1, y + 1));
            }
            if !on(x, y + 1) {
                add((x + 1, y + 1), (x, y + 1));
            }
            if !on(x - 1, y) {
                add((x, y + 1), (x, y));
            }
        }
    }
    let mut starts: Vec<(i64, i64)> = next.keys().copied().collect();
    starts.sort();
    let mut out = Vec::new();
    for start in starts {
        while next.get(&start).is_some_and(|v| !v.is_empty()) {
            let mut ring = vec![start];
            let mut prev = start;
            let mut cur = next.get_mut(&start).and_then(|v| v.pop()).unwrap();
            while cur != start {
                ring.push(cur);
                let din = (cur.0 - prev.0, cur.1 - prev.1);
                let Some(cands) = next.get_mut(&cur) else { break };
                if cands.is_empty() {
                    break;
                }
                let pick = (0..cands.len())
                    .max_by_key(|&i| {
                        let d = (cands[i].0 - cur.0, cands[i].1 - cur.1);
                        din.0 * d.1 - din.1 * d.0
                    })
                    .unwrap();
                let nxt = cands.swap_remove(pick);
                prev = cur;
                cur = nxt;
            }
            out.push(
                corners(&ring)
                    .into_iter()
                    .map(|(x, y)| [origin[0] + x as f64 * cell, origin[1] + y as f64 * cell])
                    .collect(),
            );
        }
    }
    out
}

fn corners(ring: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let n = ring.len();
    (0..n)
        .filter(|&i| {
            let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
            (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0) != 0
        })
        .map(|i| ring[i])
        .collect()
}

pub fn area(ring: &[P]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| ring[i][0] * ring[(i + 1) % n][1] - ring[(i + 1) % n][0] * ring[i][1])
        .sum::<f64>()
        / 2.0
}

fn perpendicular(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l = (dx * dx + dy * dy).sqrt();
    if l < 1e-15 {
        return ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt();
    }
    ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / l
}

fn dp(pts: &[P], tol: f64, out: &mut Vec<P>) {
    let n = pts.len();
    if n < 3 {
        out.push(pts[0]);
        return;
    }
    let (a, b) = (pts[0], pts[n - 1]);
    let (k, d) = (1..n - 1)
        .map(|i| (i, perpendicular(pts[i], a, b)))
        .fold((0, -1.0), |m, x| if x.1 > m.1 { x } else { m });
    if d > tol {
        dp(&pts[..=k], tol, out);
        dp(&pts[k..], tol, out);
    } else {
        out.push(a);
    }
}

pub fn simplify(ring: &[P], tol: f64) -> Vec<P> {
    let n = ring.len();
    if n < 8 {
        return ring.to_vec();
    }
    let far = (1..n)
        .max_by(|a, b| {
            let d =
                |i: usize| (ring[i][0] - ring[0][0]).powi(2) + (ring[i][1] - ring[0][1]).powi(2);
            d(*a).total_cmp(&d(*b))
        })
        .unwrap();
    let mut out = Vec::new();
    dp(&ring[..=far], tol, &mut out);
    let mut back: Vec<P> = ring[far..].to_vec();
    back.push(ring[0]);
    dp(&back, tol, &mut out);
    if out.len() < 3 { ring.to_vec() } else { out }
}

pub fn triangles(rings: &[Vec<P>]) -> Vec<[P; 3]> {
    let mut edges: Vec<(P, P)> = Vec::new();
    for r in rings {
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            if (a[1] - b[1]).abs() > 1e-12 {
                edges.push(if a[1] < b[1] { (a, b) } else { (b, a) });
            }
        }
    }
    if edges.is_empty() {
        return Vec::new();
    }
    edges.sort_by(|a, b| a.0[1].total_cmp(&b.0[1]));
    let mut ys: Vec<f64> = edges.iter().flat_map(|e| [e.0[1], e.1[1]]).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    let x_at = |e: &(P, P), y: f64| {
        let t = (y - e.0[1]) / (e.1[1] - e.0[1]);
        e.0[0] + t * (e.1[0] - e.0[0])
    };
    let mut out = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    let mut next = 0;
    let mut open: HashMap<(usize, usize), f64> = HashMap::new();
    let emit = |out: &mut Vec<[P; 3]>, l: usize, r: usize, y0: f64, y1: f64| {
        let (el, er) = (&edges[l], &edges[r]);
        let (a, b) = ([x_at(el, y0), y0], [x_at(er, y0), y0]);
        let (c, d) = ([x_at(er, y1), y1], [x_at(el, y1), y1]);
        out.push([a, b, c]);
        out.push([a, c, d]);
    };
    for w in ys.windows(2) {
        let (y0, y1) = (w[0], w[1]);
        active.retain(|&i| edges[i].1[1] > y0 + 1e-12);
        while next < edges.len() && edges[next].0[1] <= y0 + 1e-12 {
            if edges[next].1[1] > y0 + 1e-12 {
                active.push(next);
            }
            next += 1;
        }
        let ym = 0.5 * (y0 + y1);
        let mut order: Vec<usize> = active.clone();
        order.sort_by(|a, b| x_at(&edges[*a], ym).total_cmp(&x_at(&edges[*b], ym)));
        let pairs: Vec<(usize, usize)> = order.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let closing: Vec<(usize, usize)> =
            open.keys().filter(|k| !pairs.contains(k)).copied().collect();
        for k in closing {
            let start = open.remove(&k).unwrap();
            emit(&mut out, k.0, k.1, start, y0);
        }
        for p in pairs {
            open.entry(p).or_insert(y0);
        }
    }
    let end = *ys.last().unwrap();
    for (k, start) in open {
        emit(&mut out, k.0, k.1, start, end);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_with_a_hole_gives_an_outer_and_a_hole() {
        let (w, h) = (5, 5);
        let mut m = vec![1u8; w * h];
        m[2 * w + 2] = 0;
        let rings = loops(&m, w, h, [0.0, 0.0], 1.0);
        assert_eq!(rings.len(), 2);
        let mut areas: Vec<f64> = rings.iter().map(|r| area(r)).collect();
        areas.sort_by(f64::total_cmp);
        assert_eq!(areas, vec![-1.0, 25.0]);
        let tri: f64 = triangles(&rings).iter().map(|t| area(t.as_ref()).abs()).sum();
        assert!((tri - 24.0).abs() < 1e-9, "{tri}");
    }

    #[test]
    fn diagonal_pixels_stay_apart() {
        let m = vec![1u8, 0, 0, 1];
        let rings = loops(&m, 2, 2, [0.0, 0.0], 1.0);
        assert_eq!(rings.len(), 2);
        assert!(rings.iter().all(|r| (area(r) - 1.0).abs() < 1e-12));
    }

    #[test]
    fn a_staircase_disc_simplifies_to_a_smooth_ring_of_the_same_area() {
        let (n, r) = (200usize, 80.0);
        let m: Vec<u8> = (0..n * n)
            .map(|i| {
                let (x, y) = ((i % n) as f64 + 0.5 - 100.0, (i / n) as f64 + 0.5 - 100.0);
                (x * x + y * y < r * r) as u8
            })
            .collect();
        let rings = loops(&m, n, n, [0.0, 0.0], 1.0);
        let smooth = simplify(&rings[0], 1.0);
        assert!(smooth.len() < rings[0].len() / 3, "{} {}", smooth.len(), rings[0].len());
        let want = std::f64::consts::PI * r * r;
        assert!((area(&smooth) - want).abs() / want < 0.01);
        let tri: f64 =
            triangles(std::slice::from_ref(&smooth)).iter().map(|t| area(t.as_ref()).abs()).sum();
        assert!((tri - area(&smooth).abs()).abs() < 1e-6);
    }
}
