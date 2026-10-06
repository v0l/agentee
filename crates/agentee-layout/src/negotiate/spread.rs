use super::Base;
use super::search::seg_cells;
use agentee_core::geom::{self, P};
use agentee_core::layout::Layout;
use agentee_core::route::RoutedTrack;

const STEPS: usize = 10;

fn intersect(p: P, d: P, q: P, e: P) -> Option<P> {
    let den = d[0] * e[1] - d[1] * e[0];
    if den.abs() < 1e-9 {
        return None;
    }
    let t = ((q[0] - p[0]) * e[1] - (q[1] - p[1]) * e[0]) / den;
    Some([p[0] + d[0] * t, p[1] + d[1] * t])
}

fn exact_clear(base: &Base, l: usize, net: u16, a: P, b: P, half: f64, clr: f64) -> bool {
    let grid = &base.grid;
    let n = (geom::dist(a, b) / (grid.g * 0.25)).ceil().max(1.0) as usize;
    (0..=n).all(|k| {
        let t = k as f64 / n as f64;
        let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let (d, q) = grid.exact(l, p, net);
        d > half + clr + 1e-3 && q > half + 1e-3
    })
}

fn dir(a: P, b: P) -> Option<P> {
    let l = geom::dist(a, b);
    (l > 1e-9).then(|| [(b[0] - a[0]) / l, (b[1] - a[1]) / l])
}

fn same_way(a: P, b: P, c: P, d: P) -> bool {
    match (dir(a, b), dir(c, d)) {
        (Some(u), Some(v)) => u[0] * v[0] + u[1] * v[1] > 0.999,
        _ => false,
    }
}

pub fn spread(layout: &Layout, base: &Base, tracks: &mut [RoutedTrack]) -> Vec<(usize, Vec<P>)> {
    let grid = &base.grid;
    let g = grid.g;
    let layer_of = |n: &str| layout.copper.iter().position(|c| c == n);
    let net_of = |n: &str| layout.nets.iter().position(|x| x.name == n);
    let mut anchors: Vec<(usize, P, f64)> = Vec::new();
    for t in tracks.iter() {
        if let (Some(n), Some(a), Some(b)) = (net_of(&t.net), t.points.first(), t.points.last()) {
            let h = t.width.unwrap_or(layout.nets[n].width) / 2.0;
            anchors.push((n, *a, h));
            anchors.push((n, *b, h));
        }
    }
    for v in &layout.vias {
        anchors.push((v.net, v.at, v.diameter / 2.0));
    }
    let planes: Vec<usize> = base.rules.shadow.iter().flatten().copied().collect();
    let mut undo = Vec::new();
    for (ti, track) in tracks.iter_mut().enumerate() {
        let old = track.points.clone();
        let (Some(l), Some(n)) = (layer_of(&track.layer), net_of(&track.net)) else {
            continue;
        };
        if planes.contains(&l) {
            continue;
        }
        let Some(rule) = base.rules.nets.get(n).and_then(|r| r.as_ref()) else { continue };
        if rule.band > rule.clearance + 1e-9 {
            continue;
        }
        let need = rule.need[l];
        let net = n as u16;
        let half = track.width.unwrap_or(rule.width[l]) / 2.0;
        let legal = |a: P, b: P| {
            seg_cells(grid, a, b)
                .into_iter()
                .all(|(x, y)| grid.track_ok(grid.idx(l, x, y), net, need))
                && exact_clear(base, l, net, a, b, half, rule.clearance)
        };
        let len = track.points.len();
        if len < 4 {
            continue;
        }
        for k in 1..len - 2 {
            let pts = &track.points;
            let (p0, p1, p2, p3) = (pts[k - 1], pts[k], pts[k + 1], pts[k + 2]);
            let Some(u) = dir(p1, p2) else { continue };
            let normal = [-u[1], u[0]];
            let tied = anchors.iter().any(|&(an, q, r)| {
                an == n
                    && q != pts[0]
                    && q != pts[len - 1]
                    && [(p0, p1), (p1, p2), (p2, p3)]
                        .iter()
                        .any(|&(a, b)| geom::point_segment_distance(q, a, b) < half + r + g)
            });
            if tied {
                continue;
            }
            let shifted = |s: f64| -> Option<(P, P)> {
                let a = [p1[0] + normal[0] * s, p1[1] + normal[1] * s];
                let d0 = dir(p0, p1)?;
                let d1 = dir(p2, p3)?;
                let q1 = intersect(a, u, p0, d0)?;
                let q2 = intersect(a, u, p3, d1)?;
                let keeps = same_way(p0, q1, p0, p1)
                    && same_way(q1, q2, p1, p2)
                    && same_way(q2, p3, p2, p3)
                    && geom::dist(p0, q1) >= g
                    && geom::dist(q2, p3) >= g;
                (keeps && legal(p0, q1) && legal(q1, q2) && legal(q2, p3)).then_some((q1, q2))
            };
            let mut room = [0.0f64; 2];
            for (side, sign) in [(0, -1.0), (1, 1.0)] {
                for s in 1..=STEPS {
                    if shifted(sign * s as f64 * g).is_none() {
                        break;
                    }
                    room[side] = s as f64 * g;
                }
            }
            let target = (room[1] - room[0]) / 2.0;
            let target = (target / g).trunc() * g;
            if target.abs() < g {
                continue;
            }
            if let Some((q1, q2)) = shifted(target) {
                let pts = &mut track.points;
                pts[k] = q1;
                pts[k + 1] = q2;
            }
        }
        if track.points != old {
            undo.push((ti, old));
        }
    }
    undo
}

pub fn illegal(
    layout: &Layout,
    base: &Base,
    tracks: &[RoutedTrack],
    which: &[usize],
) -> Vec<usize> {
    let grid = &base.grid;
    which
        .iter()
        .copied()
        .filter(|&ti| {
            let t = &tracks[ti];
            let (Some(l), Some(n)) = (
                layout.copper.iter().position(|c| *c == t.layer),
                layout.nets.iter().position(|x| x.name == t.net),
            ) else {
                return false;
            };
            let Some(rule) = base.rules.nets.get(n).and_then(|r| r.as_ref()) else { return false };
            let half = t.width.unwrap_or(rule.width[l]) / 2.0;
            t.points.windows(2).any(|w| {
                seg_cells(grid, w[0], w[1])
                    .into_iter()
                    .any(|(x, y)| !grid.track_ok(grid.idx(l, x, y), n as u16, rule.need[l]))
                    || !exact_clear(base, l, n as u16, w[0], w[1], half, rule.clearance)
            })
        })
        .collect()
}
