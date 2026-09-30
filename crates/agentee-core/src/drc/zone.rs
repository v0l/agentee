use super::{Category, Ctx, Report, Rule, Setup, every, recorded};
use crate::diag::Severity;
use crate::geom::{self, P};
use crate::units::Length;
use std::collections::HashMap;

pub static RULES: &[Rule] = &[
    Rule {
        id: "zone-overlap",
        category: Category::Zone,
        severity: Severity::Error,
        summary: "the fills of two zones of different nets on one layer overlap, a short",
        when: "zones",
        applies: with_zones,
        check: recorded,
    },
    Rule {
        id: "zone-to-zone",
        category: Category::Zone,
        severity: Severity::Error,
        summary: "the fills of two zones of different nets closer than their clearance",
        when: "zones",
        applies: with_zones,
        check: recorded,
    },
    Rule {
        id: "zone-clearance",
        category: Category::Zone,
        severity: Severity::Error,
        summary: "a fill that covers or comes closer than the clearance to copper of another net",
        when: "zones",
        applies: with_zones,
        check: recorded,
    },
    Rule {
        id: "zone-tips",
        category: Category::Zone,
        severity: Severity::Warning,
        summary: "sharp fill tips under 30 degrees, which etch unevenly and can lift",
        when: "zones",
        applies: with_zones,
        check: recorded,
    },
    Rule {
        id: "copper-neck",
        category: Category::Zone,
        severity: Severity::Warning,
        summary: "necks in a fill narrower than the zone's min_width, left by the fill",
        when: "zones",
        applies: with_zones,
        check: copper_neck,
    },
    Rule {
        id: "zone-islands",
        category: Category::Zone,
        severity: Severity::Info,
        summary: "counts the fill islands that reach nothing of the zone's net and were removed",
        when: "every board",
        applies: every,
        check: recorded,
    },
];

fn with_zones(s: &Setup) -> bool {
    s.zones
}

fn copper_neck(cx: &Ctx, r: &mut Report) {
    for z in cx.zones {
        let w = z.min_width;
        if w <= 0.0 || z.rings.is_empty() {
            continue;
        }
        let spots = neck_spots(&z.rings, w);
        let Some(&(d, m)) = spots.iter().min_by(|a, b| a.0.total_cmp(&b.0)) else { continue };
        r.emit(
            format!("zone {} on {}", cx.nets[z.net].name, z.layer),
            format!(
                "{} copper necks narrower than the zone's min_width {}, narrowest {} at [{:.3}, {:.3}]; the fill left them, widen the gap or raise min_width",
                spots.len(),
                Length::mm(w),
                Length::mm((d * 1000.0).round() / 1000.0),
                m[0],
                m[1]
            ),
        );
    }
}

fn neck_spots(rings: &[Vec<P>], w: f64) -> Vec<(f64, P)> {
    let limit = w * 0.9;
    let mut edges: Vec<(P, P, P)> = Vec::new();
    let mut along: Vec<(usize, f64, f64)> = Vec::new();
    let mut perimeter = Vec::new();
    for (ri, ring) in rings.iter().enumerate() {
        let mut s = 0.0;
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            let l = geom::dist(a, b);
            if l > 1e-9 {
                edges.push((a, b, [-(b[1] - a[1]) / l, (b[0] - a[0]) / l]));
                along.push((ri, s, s + l));
            }
            s += l;
        }
        perimeter.push(s);
    }
    let apart = |i: usize, j: usize| {
        let ((ri, si, ei), (rj, sj, ej)) = (along[i], along[j]);
        ri != rj || (sj - ei).min(perimeter[ri] - ej + si) > 2.0 * w
    };
    let cell = w.max(0.05);
    let key = |p: P| ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64);
    let mut bins: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (k, (a, b, _)) in edges.iter().enumerate() {
        let (lo, hi) =
            (key([a[0].min(b[0]), a[1].min(b[1])]), key([a[0].max(b[0]), a[1].max(b[1])]));
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                bins.entry((x, y)).or_default().push(k);
            }
        }
    }
    let mut necks: Vec<(f64, P)> = Vec::new();
    let mut seen: Vec<usize> = Vec::new();
    for (i, (a, b, n1)) in edges.iter().enumerate() {
        let (lo, hi) = (
            key([a[0].min(b[0]) - limit, a[1].min(b[1]) - limit]),
            key([a[0].max(b[0]) + limit, a[1].max(b[1]) + limit]),
        );
        seen.clear();
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                seen.extend(bins.get(&(x, y)).into_iter().flatten().filter(|&&j| j > i));
            }
        }
        seen.sort_unstable();
        seen.dedup();
        for &j in &seen {
            let (c, e, n2) = edges[j];
            if geom::dist(*a, e) < 1e-9
                || geom::dist(*b, c) < 1e-9
                || n1[0] * n2[0] + n1[1] * n2[1] > -0.9
                || !apart(i, j)
            {
                continue;
            }
            let (p, q) = closest(*a, *b, c, e);
            let d = geom::dist(p, q);
            if d >= limit || d < 1e-6 {
                continue;
            }
            let u = [(q[0] - p[0]) / d, (q[1] - p[1]) / d];
            if n1[0] * u[0] + n1[1] * u[1] < 0.95 || -(n2[0] * u[0] + n2[1] * u[1]) < 0.95 {
                continue;
            }
            let mid = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
            necks.push((d, mid));
        }
    }
    let mut spots: Vec<(f64, P)> = Vec::new();
    for (d, m) in necks {
        match spots.iter_mut().find(|s| geom::dist(s.1, m) < 2.0 * w) {
            Some(s) if d < s.0 => *s = (d, m),
            Some(_) => {}
            None => spots.push((d, m)),
        }
    }
    spots
}

fn closest(a: P, b: P, c: P, e: P) -> (P, P) {
    let on = |p: P, s: P, t: P| {
        let (dx, dy) = (t[0] - s[0], t[1] - s[1]);
        let l2 = dx * dx + dy * dy;
        let k = if l2 == 0.0 {
            0.0
        } else {
            (((p[0] - s[0]) * dx + (p[1] - s[1]) * dy) / l2).clamp(0.0, 1.0)
        };
        [s[0] + k * dx, s[1] + k * dy]
    };
    [(a, on(a, c, e)), (b, on(b, c, e)), (on(c, a, b), c), (on(e, a, b), e)]
        .into_iter()
        .min_by(|x, y| geom::dist(x.0, x.1).total_cmp(&geom::dist(y.0, y.1)))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<P> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    #[test]
    fn a_neck_under_min_width_is_found_once() {
        let dumbbell = vec![
            [0.0, 0.0],
            [2.0, 0.0],
            [2.0, 0.95],
            [3.0, 0.95],
            [3.0, 0.0],
            [5.0, 0.0],
            [5.0, 2.0],
            [3.0, 2.0],
            [3.0, 1.05],
            [2.0, 1.05],
            [2.0, 2.0],
            [0.0, 2.0],
        ];
        let spots = neck_spots(&[dumbbell], 0.25);
        assert!(spots.len() == 1 && (spots[0].0 - 0.1).abs() < 1e-9, "{spots:?}");
        let mut hole = rect(1.0, 0.1, 3.0, 1.9);
        hole.reverse();
        let spots = neck_spots(&[rect(0.0, 0.0, 4.0, 2.0), hole], 0.25);
        assert!(spots.len() == 2 && (spots[0].0 - 0.1).abs() < 1e-9, "{spots:?}");
        assert!(neck_spots(&[rect(0.0, 0.0, 4.0, 0.3)], 0.25).is_empty());
        let round: Vec<P> = (0..64)
            .map(|k| {
                let t = std::f64::consts::TAU * k as f64 / 64.0;
                [0.5 * t.cos(), 0.5 * t.sin()]
            })
            .collect();
        assert!(neck_spots(&[round], 0.25).is_empty());
        let half = 28f64.to_radians();
        let c = [1.0 / half.tan() - 0.125 / half.sin(), 0.0];
        let tip: Vec<P> = std::iter::once([0.0, -1.0])
            .chain((0..=40).map(|k| {
                let t = -(90f64.to_radians() - half) * (1.0 - k as f64 / 20.0);
                [c[0] + 0.125 * t.cos(), c[1] + 0.125 * t.sin()]
            }))
            .chain([[0.0, 1.0]])
            .collect();
        assert!(neck_spots(&[tip], 0.25).is_empty());
        let jag = vec![
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 2.0],
            [2.0005, 2.0],
            [2.0005, 2.001],
            [2.0, 2.001],
            [2.0, 2.0],
            [0.0, 2.0],
        ];
        assert!(neck_spots(&[jag], 0.25).is_empty());
    }
}
