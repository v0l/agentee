use super::{Category, Ctx, Report, Rule, Setup, every, is_smd, near, pads_where};
use crate::diag::Severity;
use crate::geom::{self, P};
use crate::units::Length;
use std::collections::BTreeMap;

pub static RULES: &[Rule] = &[
    Rule {
        id: "smd-pad-gap",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "SMD pads of different nets closer than min_smd_pad_gap",
        when: "every board",
        applies: every,
        check: smd_pad_gap,
    },
    Rule {
        id: "pad-to-edge",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "pad copper closer than min_copper_to_edge to the board outline, unless the pad is marked `edge = true`",
        when: "every board",
        applies: every,
        check: pad_to_edge,
    },
    Rule {
        id: "edge-pad-reach",
        category: Category::Copper,
        severity: Severity::Warning,
        summary: "a pad marked `edge = true` that does not reach the board edge",
        when: "every board",
        applies: every,
        check: edge_pad_reach,
    },
    Rule {
        id: "starved-thermal",
        category: Category::Zone,
        severity: Severity::Warning,
        summary: "a pad joined to its pour by fewer than two spokes and less copper than its own width",
        when: "zones",
        applies: with_zones,
        check: starved_thermal,
    },
];

fn with_zones(s: &Setup) -> bool {
    s.zones
}

fn mm(v: f64) -> Length {
    Length::mm(v)
}

fn smd_pad_gap(cx: &Ctx, r: &mut Report) {
    let need = cx.board.rules.min_smd_pad_gap.to_mm();
    let pads = pads_where(cx.parts, is_smd);
    let mut pairs: BTreeMap<(usize, usize), (usize, f64, String)> = BTreeMap::new();
    for (i, a) in pads.iter().enumerate() {
        for b in &pads[i + 1..] {
            if (a.q.net.is_some() && a.q.net == b.q.net)
                || !a.q.copper.iter().any(|l| b.q.copper.contains(l))
                || !near(&a.bounds, b.bounds.min, need + b.bounds.size()[0].max(b.bounds.size()[1]))
            {
                continue;
            }
            let gap =
                a.q.outlines
                    .iter()
                    .flat_map(|x| b.q.outlines.iter().map(move |y| geom::polygon_distance(x, y)))
                    .fold(f64::MAX, f64::min);
            if gap <= 1e-6 || gap + 1e-6 >= need {
                continue;
            }
            let key = (a.part.min(b.part), a.part.max(b.part));
            let e = pairs.entry(key).or_insert((0, f64::MAX, String::new()));
            e.0 += 1;
            if gap < e.1 {
                e.1 = gap;
                e.2 = format!("{} and {}", cx.pad_name(a.part, a.pad), cx.pad_name(b.part, b.pad));
            }
        }
    }
    for ((pa, pb), (n, gap, first)) in pairs {
        let at = if pa == pb {
            format!("part {}", cx.parts[pa].reference)
        } else {
            format!("parts {}/{}", cx.parts[pa].reference, cx.parts[pb].reference)
        };
        r.emit(
            at,
            format!(
                "{n} SMD pad pairs of different nets closer than {}, closest {} ({first})",
                mm(need),
                mm(gap)
            ),
        );
    }
}

fn edge_gap(cx: &Ctx, q: &crate::layout::PlacedPad) -> f64 {
    let n = cx.outline.len();
    q.outlines
        .iter()
        .flat_map(|o| {
            (0..o.len()).flat_map(move |i| {
                (0..n).map(move |j| {
                    geom::segment_segment_distance(
                        o[i],
                        o[(i + 1) % o.len()],
                        cx.outline[j],
                        cx.outline[(j + 1) % n],
                    )
                })
            })
        })
        .fold(f64::MAX, f64::min)
}

fn edge_pads(cx: &Ctx, marked: bool) -> Vec<(usize, usize, f64, bool)> {
    let mut out = Vec::new();
    if cx.outline.len() < 3 {
        return out;
    }
    for (pi, p) in cx.parts.iter().enumerate() {
        for (k, (q, f)) in p.pads.iter().zip(&p.footprint.pads).enumerate() {
            if q.copper.is_empty() || f.edge != marked {
                continue;
            }
            let off = q.outlines.iter().flatten().any(|c| !geom::point_in_polygon(*c, cx.outline));
            out.push((pi, k, edge_gap(cx, q), off));
        }
    }
    out
}

fn pad_to_edge(cx: &Ctx, r: &mut Report) {
    let need = cx.board.rules.min_copper_to_edge.to_mm();
    let mut worst: BTreeMap<usize, (f64, usize)> = BTreeMap::new();
    for (pi, k, gap, off) in edge_pads(cx, false) {
        if off || gap + 1e-6 >= need {
            continue;
        }
        let e = worst.entry(pi).or_insert((gap, k));
        if gap < e.0 {
            *e = (gap, k);
        }
    }
    for (pi, (gap, k)) in worst {
        r.emit(
            format!("part {}", cx.parts[pi].reference),
            format!(
                "pad {} is {} from the board edge, the fab keeps copper {} away; mark the footprint pad `edge = true` if it is meant to reach the edge",
                cx.pad_name(pi, k),
                mm(gap),
                mm(need)
            ),
        );
    }
}

fn edge_pad_reach(cx: &Ctx, r: &mut Report) {
    for (pi, k, gap, off) in edge_pads(cx, true) {
        if off || gap <= 0.01 {
            continue;
        }
        r.emit(
            format!("pad {}", cx.pad_name(pi, k)),
            format!(
                "is marked `edge = true` but stops {} short of the board edge; move the part to the edge or drop the mark",
                mm(gap)
            ),
        );
    }
}

fn boundary_samples(outlines: &[Vec<P>], step: f64, off: f64) -> Vec<P> {
    let mut out = Vec::new();
    for (k, o) in outlines.iter().enumerate() {
        for i in 0..o.len() {
            let (a, b) = (o[i], o[(i + 1) % o.len()]);
            let l = geom::dist(a, b);
            if l < 1e-9 {
                continue;
            }
            let u = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
            let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            let flip = geom::point_in_polygon([mid[0] + u[1] * 1e-4, mid[1] - u[0] * 1e-4], o);
            let n = if flip { [-u[1], u[0]] } else { [u[1], -u[0]] };
            let steps = (l / step).ceil().max(1.0) as usize;
            for s in 0..steps {
                let t = (s as f64 + 0.5) / steps as f64 * l;
                let p = [a[0] + u[0] * t + n[0] * off, a[1] + u[1] * t + n[1] * off];
                let other = outlines
                    .iter()
                    .enumerate()
                    .any(|(j, q)| j != k && geom::point_in_polygon(p, q));
                if !other {
                    out.push(p);
                }
            }
        }
    }
    out
}

fn starved_thermal(cx: &Ctx, r: &mut Report) {
    let w = cx.board.rules.min_track_width.to_mm();
    let step = 0.02;
    for (pi, p) in cx.parts.iter().enumerate() {
        for (k, q) in p.pads.iter().enumerate() {
            let Some(net) = q.net else { continue };
            let pb = super::rings_bounds(&q.outlines);
            for (z, f) in cx.zones.iter().zip(cx.fills()) {
                if z.net != net
                    || !q.copper.contains(&z.layer)
                    || !near(&f.bounds, pb.center(), pb.size()[0].max(pb.size()[1]))
                {
                    continue;
                }
                let samples = boundary_samples(&q.outlines, step, 0.05);
                if samples.is_empty() {
                    continue;
                }
                let inside: Vec<bool> = samples.iter().map(|s| f.contains(*s)).collect();
                let hit = inside.iter().filter(|x| **x).count();
                if hit == 0 || hit * 2 >= inside.len() {
                    continue;
                }
                let start = inside.iter().position(|x| !x).unwrap_or(0);
                let mut runs = Vec::new();
                let mut run = 0usize;
                for i in 0..inside.len() {
                    if inside[(start + i) % inside.len()] {
                        run += 1;
                    } else if run > 0 {
                        runs.push(run as f64 * step);
                        run = 0;
                    }
                }
                if run > 0 {
                    runs.push(run as f64 * step);
                }
                let spokes = runs.iter().filter(|x| **x + 1e-9 >= w).count();
                let contact: f64 = runs.iter().sum();
                let size = pb.size()[0].min(pb.size()[1]);
                if spokes >= 2 || contact + 1e-9 >= size {
                    continue;
                }
                r.emit(
                    format!("pad {}", cx.pad_name(pi, k)),
                    format!(
                        "joined to the {} pour on {} by {spokes} spoke{} and {} of copper in all, under the pad's {} width; widen the neck or run a track to it",
                        cx.nets[net].name,
                        z.layer,
                        if spokes == 1 { "" } else { "s" },
                        mm((contact * 1000.0).round() / 1000.0),
                        mm((size * 1000.0).round() / 1000.0)
                    ),
                );
            }
        }
    }
}
