pub type P = [f64; 2];

pub fn rotate(p: P, deg: f64) -> P {
    if deg == 0.0 {
        return p;
    }
    let (s, c) = deg.to_radians().sin_cos();
    [p[0] * c + p[1] * s, -p[0] * s + p[1] * c]
}

pub fn point_segment_distance(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    };
    let (x, y) = (a[0] + t * dx - p[0], a[1] + t * dy - p[1]);
    (x * x + y * y).sqrt()
}

fn cross(o: P, a: P, b: P) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

pub fn segments_intersect(a: P, b: P, c: P, d: P) -> bool {
    let (d1, d2) = (cross(c, d, a), cross(c, d, b));
    let (d3, d4) = (cross(a, b, c), cross(a, b, d));
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

pub fn segment_segment_distance(a: P, b: P, c: P, d: P) -> f64 {
    if segments_intersect(a, b, c, d) {
        return 0.0;
    }
    point_segment_distance(a, c, d)
        .min(point_segment_distance(b, c, d))
        .min(point_segment_distance(c, a, b))
        .min(point_segment_distance(d, a, b))
}

pub fn point_in_polygon(p: P, poly: &[P]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
    }
    inside
}

fn edges(poly: &[P]) -> impl Iterator<Item = (P, P)> + '_ {
    (0..poly.len()).map(move |i| (poly[i], poly[(i + 1) % poly.len()]))
}

pub fn polygon_distance(a: &[P], b: &[P]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return f64::MAX;
    }
    if point_in_polygon(a[0], b) || point_in_polygon(b[0], a) {
        return 0.0;
    }
    let mut best = f64::MAX;
    for (p, q) in edges(a) {
        for (r, s) in edges(b) {
            best = best.min(segment_segment_distance(p, q, r, s));
            if best == 0.0 {
                return 0.0;
            }
        }
    }
    best
}

pub fn polyline_polygon_distance(line: &[P], poly: &[P]) -> f64 {
    if line.iter().any(|p| point_in_polygon(*p, poly)) {
        return 0.0;
    }
    let mut best = f64::MAX;
    for w in line.windows(2) {
        for (r, s) in edges(poly) {
            best = best.min(segment_segment_distance(w[0], w[1], r, s));
        }
    }
    best
}

pub fn circle(c: P, r: f64, n: usize) -> Vec<P> {
    (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect()
}

pub fn rounded_rect(w: f64, h: f64, r: f64, per_corner: usize) -> Vec<P> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let (hx, hy) = (w / 2.0 - r, h / 2.0 - r);
    if r <= 0.0 {
        return vec![
            [-w / 2.0, -h / 2.0],
            [w / 2.0, -h / 2.0],
            [w / 2.0, h / 2.0],
            [-w / 2.0, h / 2.0],
        ];
    }
    let corners = [([hx, hy], 0.0), ([-hx, hy], 90.0), ([-hx, -hy], 180.0), ([hx, -hy], 270.0)];
    let mut out = Vec::with_capacity(4 * (per_corner + 1));
    for (c, start) in corners {
        for i in 0..=per_corner {
            let a = (start + 90.0 * i as f64 / per_corner as f64).to_radians();
            out.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
        }
    }
    out
}

fn signed_area(poly: &[P]) -> f64 {
    edges(poly).map(|(a, b)| a[0] * b[1] - b[0] * a[1]).sum::<f64>() / 2.0
}

pub fn is_convex(poly: &[P]) -> bool {
    let n = poly.len();
    let mut sign = 0.0;
    for i in 0..n {
        let c = cross(poly[i], poly[(i + 1) % n], poly[(i + 2) % n]);
        if c.abs() < 1e-12 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    true
}

fn in_triangle(p: P, a: P, b: P, c: P) -> bool {
    let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}

pub fn triangulate(poly: &[P]) -> Vec<[usize; 3]> {
    let mut idx: Vec<usize> = (0..poly.len()).collect();
    if signed_area(poly) < 0.0 {
        idx.reverse();
    }
    let mut out = Vec::new();
    let mut guard = 0;
    while idx.len() > 3 && guard < poly.len() * poly.len() {
        guard += 1;
        let n = idx.len();
        let ear = (0..n).find(|&i| {
            let (a, b, c) = (idx[(i + n - 1) % n], idx[i], idx[(i + 1) % n]);
            cross(poly[a], poly[b], poly[c]) > 0.0
                && !idx.iter().any(|&k| {
                    k != a && k != b && k != c && in_triangle(poly[k], poly[a], poly[b], poly[c])
                })
        });
        let Some(i) = ear else { break };
        out.push([idx[(i + n - 1) % n], idx[i], idx[(i + 1) % n]]);
        idx.remove(i);
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_distance() {
        let a = rounded_rect(1.0, 1.0, 0.0, 0);
        let b: Vec<P> = a.iter().map(|p| [p[0] + 1.5, p[1]]).collect();
        assert!((polygon_distance(&a, &b) - 0.5).abs() < 1e-9);
        let c: Vec<P> = a.iter().map(|p| [p[0] + 0.5, p[1]]).collect();
        assert_eq!(polygon_distance(&a, &c), 0.0);
    }

    #[test]
    fn concave_polygons_triangulate_to_their_area() {
        let l = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]];
        assert!(!is_convex(&l));
        let tris = triangulate(&l);
        assert_eq!(tris.len(), 4);
        let area: f64 = tris.iter().map(|t| signed_area(&[l[t[0]], l[t[1]], l[t[2]]]).abs()).sum();
        assert!((area - 3.0).abs() < 1e-9);
    }

    #[test]
    fn rotation_is_counter_clockwise_on_screen() {
        let p = rotate([1.0, 0.0], 90.0);
        assert!((p[0]).abs() < 1e-12 && (p[1] + 1.0).abs() < 1e-12);
    }
}
