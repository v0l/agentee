use agentee_core::geom::P;
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
    let mut edges = 0usize;
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            if !on(x, y) {
                continue;
            }
            let mut add = |a: (i64, i64), b: (i64, i64)| {
                next.entry(a).or_default().push(b);
                edges += 1;
            };
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
    let mut out = Vec::new();
    while let Some((&start, _)) = next.iter().find(|(_, v)| !v.is_empty()) {
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
            simplify(&ring)
                .into_iter()
                .map(|(x, y)| [origin[0] + x as f64 * cell, origin[1] + y as f64 * cell])
                .collect(),
        );
    }
    let _ = edges;
    out
}

fn simplify(ring: &[(i64, i64)]) -> Vec<(i64, i64)> {
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
    }

    #[test]
    fn diagonal_pixels_stay_apart() {
        let m = vec![1u8, 0, 0, 1];
        let rings = loops(&m, 2, 2, [0.0, 0.0], 1.0);
        assert_eq!(rings.len(), 2);
        assert!(rings.iter().all(|r| (area(r) - 1.0).abs() < 1e-12));
    }
}
