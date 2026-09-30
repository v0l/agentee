use super::{
    Category, Ctx, CuShape, Owner, Report, Rule, Setup, every, is_smd, near, pads_where, recorded,
};
use crate::diag::Severity;
use crate::geom::{self, P};
use crate::graphic::Bounds;
use crate::units::Length;
use std::collections::BTreeMap;

pub static RULES: &[Rule] = &[
    Rule {
        id: "short",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "copper of two different nets touches",
        when: "every board",
        applies: every,
        check: recorded,
    },
    Rule {
        id: "clearance",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "copper of two nets closer than their net class clearance, or a pad footprint clearance, or copper run into a non-plated hole",
        when: "every board",
        applies: every,
        check: recorded,
    },
    Rule {
        id: "unrouted",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "a net whose pads are not all joined by tracks, vias and pours",
        when: "every board",
        applies: every,
        check: recorded,
    },
    Rule {
        id: "dangling-track",
        category: Category::Copper,
        severity: Severity::Warning,
        summary: "a track end that touches no copper of its net",
        when: "every board",
        applies: every,
        check: dangling_track,
    },
    Rule {
        id: "track-grazes-pad",
        category: Category::Copper,
        severity: Severity::Warning,
        summary: "tracks that reach a pad only by their edge, not their centre line",
        when: "every board",
        applies: every,
        check: track_grazes_pad,
    },
    Rule {
        id: "copper-to-edge",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "a track or via closer than min_copper_to_edge to the board outline, or off the board",
        when: "every board",
        applies: every,
        check: copper_to_edge,
    },
    Rule {
        id: "pad-off-board",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "pads outside the board outline, unless the pad is marked `edge = true`",
        when: "every board",
        applies: every,
        check: pad_off_board,
    },
    Rule {
        id: "stitching",
        category: Category::Copper,
        severity: Severity::Info,
        summary: "counts the vias each [[stitching]] entry placed",
        when: "every board",
        applies: every,
        check: recorded,
    },
    Rule {
        id: "stitching-empty",
        category: Category::Copper,
        severity: Severity::Warning,
        summary: "a [[stitching]] entry that placed no via: every spot is blocked, or outside a zone of the net",
        when: "every board",
        applies: every,
        check: recorded,
    },
    Rule {
        id: "fanout-empty",
        category: Category::Copper,
        severity: Severity::Warning,
        summary: "a [[fanouts]] entry that placed no via, no pad with a net is left after its skips",
        when: "every board",
        applies: every,
        check: recorded,
    },
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
    let mut reported: Vec<(usize, &str, &str)> = Vec::new();
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
                let key = (pi, q.number.as_str(), z.layer.as_str());
                if spokes >= 2 || contact + 1e-9 >= size || reported.contains(&key) {
                    continue;
                }
                reported.push(key);
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

fn is_interior_join(points: &[P], end: P) -> bool {
    points[1..points.len() - 1].iter().any(|p| geom::dist(*p, end) < 1e-9)
}

fn dangling_track(cx: &Ctx, r: &mut Report) {
    let items = cx.copper_items();
    for (ti, t) in cx.tracks.iter().enumerate() {
        for end in [t.points[0], *t.points.last().unwrap()] {
            let mut b = Bounds::EMPTY;
            b.add_circle(end, t.width / 2.0);
            let touches = cx.items_near(&b, 0.01).into_iter().any(|i| {
                let c = &items[i];
                c.net == Some(t.net)
                    && c.layers.contains(&t.layer)
                    && c.owner != Owner::Track(ti)
                    && c.shape.circle_gap(end, t.width / 2.0) <= 1e-6
            }) || t.points.len() > 2 && is_interior_join(&t.points, end)
                || cx.zones.iter().any(|z| z.net == t.net && z.layer == t.layer && z.filled(end));
            if !touches {
                r.emit(
                    format!("tracks[{ti}] {}", cx.nets[t.net].name),
                    format!("end at [{:.3}, {:.3}] connects to nothing", end[0], end[1]),
                );
            }
        }
    }
}

fn seg_rings_gap(a: P, b: P, rings: &[Vec<P>]) -> f64 {
    rings
        .iter()
        .map(|poly| {
            if geom::point_in_polygon(a, poly) || geom::point_in_polygon(b, poly) {
                0.0
            } else {
                geom::polyline_polygon_distance(&[a, b], poly)
            }
        })
        .fold(f64::MAX, f64::min)
}

fn track_grazes_pad(cx: &Ctx, r: &mut Report) {
    let items = cx.copper_items();
    for pad in items.iter().filter(|c| matches!(c.owner, Owner::Pad(..)) && c.net.is_some()) {
        let (Owner::Pad(pi, k), CuShape::Poly(rings)) = (pad.owner, &pad.shape) else { continue };
        let in_zone = cx.zones.iter().any(|z| {
            Some(z.net) == pad.net && pad.layers.contains(&z.layer) && z.filled(pad.bounds.center())
        });
        if in_zone {
            continue;
        }
        let mut touching = Vec::new();
        for i in cx.items_near(&pad.bounds, 0.01) {
            let seg = &items[i];
            let CuShape::Seg(a, b, hw) = seg.shape else { continue };
            if seg.net != pad.net || !seg.layers.iter().any(|l| pad.layers.contains(l)) {
                continue;
            }
            let centre = seg_rings_gap(a, b, rings);
            if centre - hw <= 0.0 {
                touching.push((hw - centre.max(0.0), hw));
            }
        }
        if !touching.is_empty() && touching.iter().all(|(depth, hw)| *depth < *hw) {
            let worst = touching.iter().map(|t| t.0).fold(f64::MAX, f64::min);
            r.emit(
                format!("pad {}", cx.pad_name(pi, k)),
                format!(
                    "the track only grazes the pad ({worst:.3} mm of overlap), run it into the pad"
                ),
            );
        }
    }
}

fn copper_to_edge(cx: &Ctx, r: &mut Report) {
    let outline = cx.outline;
    if outline.len() < 3 {
        return;
    }
    let need = cx.board.rules.min_copper_to_edge.to_mm();
    let edges = || (0..outline.len()).map(|i| (outline[i], outline[(i + 1) % outline.len()]));
    for c in cx.copper_items() {
        let (inside, to_edge) = match c.shape {
            CuShape::Seg(a, b, hw) => (
                geom::point_in_polygon(a, outline) && geom::point_in_polygon(b, outline),
                edges()
                    .map(|(p, q)| geom::segment_segment_distance(a, b, p, q) - hw)
                    .fold(f64::MAX, f64::min),
            ),
            CuShape::Circle(o, ro) => (
                geom::point_in_polygon(o, outline),
                edges()
                    .map(|(p, q)| geom::point_segment_distance(o, p, q) - ro)
                    .fold(f64::MAX, f64::min),
            ),
            CuShape::Poly(_) => continue,
        };
        if !inside {
            r.emit("edge", format!("{} leaves the board", cx.describe(c)));
        } else if to_edge + crate::layout::DRC_EPSILON < need {
            r.emit(
                "edge",
                format!(
                    "{} is {} from the board edge, needs {}",
                    cx.describe(c),
                    mm(to_edge),
                    mm(need)
                ),
            );
        }
    }
}

fn pad_off_board(cx: &Ctx, r: &mut Report) {
    let outline = cx.outline;
    if outline.len() < 3 {
        return;
    }
    for p in cx.parts {
        let off: Vec<&str> = p
            .pads
            .iter()
            .zip(&p.footprint.pads)
            .filter(|(_, f)| !f.edge)
            .map(|(q, _)| q)
            .filter(|q| {
                q.outlines.iter().flatten().any(|c| {
                    !geom::point_in_polygon(*c, outline)
                        && (0..outline.len()).all(|i| {
                            geom::point_segment_distance(
                                *c,
                                outline[i],
                                outline[(i + 1) % outline.len()],
                            ) > 1e-3
                        })
                })
            })
            .map(|q| q.number.as_str())
            .collect();
        if !off.is_empty() {
            r.emit(
                format!("part {}", p.reference),
                format!("pads {} hang off the board", off.join(", ")),
            );
        }
    }
}
