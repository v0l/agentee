use super::{Category, Ctx, Report, Rule, Setup};
use crate::diag::Severity;
use crate::footprint::PadKind;
use crate::geom::{self, P};
use crate::graphic::Bounds;
use crate::layout::Placed;

pub static RULES: &[Rule] = &[
    Rule {
        id: "courtyard-overlap",
        category: Category::Assembly,
        severity: Severity::Error,
        summary: "the courtyards of two parts on one side overlap",
        when: "parts",
        applies: with_parts,
        check: courtyard_overlap,
    },
    Rule {
        id: "courtyard-hole",
        category: Category::Assembly,
        severity: Severity::Error,
        summary: "a courtyard that covers a mounting hole or a non-plated hole of another part",
        when: "parts",
        applies: with_parts,
        check: courtyard_hole,
    },
];

fn with_parts(s: &Setup) -> bool {
    s.parts > 0
}

fn courtyard_overlap(cx: &Ctx, r: &mut Report) {
    for (at, message) in courtyard_issues(cx.parts, false).0 {
        r.emit(at, message);
    }
}

fn courtyard_hole(cx: &Ctx, r: &mut Report) {
    for (at, message) in courtyard_issues(cx.parts, true).1 {
        r.emit(at, message);
    }
}

struct Courtyard {
    part: usize,
    side: String,
    rings: Vec<Vec<P>>,
    bounds: Bounds,
}

fn nest_rings(rings: Vec<Vec<P>>) -> Vec<Vec<P>> {
    let areas: Vec<f64> = rings.iter().map(|r| geom::signed_area(r).abs()).collect();
    let depth = |i: usize| {
        (0..rings.len())
            .filter(|&j| {
                j != i
                    && areas[j] > areas[i]
                    && rings[i].iter().all(|q| geom::point_in_polygon(*q, &rings[j]))
            })
            .count()
    };
    let depths: Vec<usize> = (0..rings.len()).map(depth).collect();
    rings
        .into_iter()
        .zip(depths)
        .map(|(mut r, d)| {
            if (geom::signed_area(&r) > 0.0) != (d % 2 == 0) {
                r.reverse();
            }
            r
        })
        .collect()
}

fn chain_loops(mut open: Vec<Vec<P>>) -> Vec<Vec<P>> {
    let near = |a: P, b: P| geom::dist(a, b) < 1e-3;
    let mut out = Vec::new();
    while let Some(mut cur) = open.pop() {
        loop {
            let end = *cur.last().unwrap();
            if cur.len() > 2 && near(cur[0], end) {
                cur.pop();
                break;
            }
            if let Some(j) = open.iter().position(|p| near(p[0], end)) {
                let next = open.swap_remove(j);
                cur.extend_from_slice(&next[1..]);
            } else if let Some(j) = open.iter().position(|p| near(*p.last().unwrap(), end)) {
                let next = open.swap_remove(j);
                cur.extend(next.into_iter().rev().skip(1));
            } else {
                break;
            }
        }
        if cur.len() >= 3 {
            out.push(cur);
        }
    }
    out
}

fn outlines_on(p: &Placed, layer: &str) -> Vec<Vec<P>> {
    let tf = p.transform();
    let mut closed = Vec::new();
    let mut open = Vec::new();
    for g in p.footprint.graphics.iter().filter(|g| g.layer == layer) {
        let mut path: Vec<P> =
            crate::footprint::graphic_path(g).into_iter().map(|q| tf.apply(q)).collect();
        if path.len() < 2 {
            continue;
        }
        let shut = match &g.shape {
            crate::graphic::Shape::Rect { .. } | crate::graphic::Shape::Circle { .. } => true,
            crate::graphic::Shape::Polyline { closed, .. } => *closed,
            _ => false,
        };
        if shut {
            if geom::dist(path[0], *path.last().unwrap()) < 1e-9 {
                path.pop();
            }
            closed.push(path);
        } else {
            open.push(path);
        }
    }
    closed.extend(chain_loops(open));
    closed.retain(|c| c.len() >= 3);
    closed
}

fn courtyards_of(pi: usize, p: &Placed) -> Vec<Courtyard> {
    let mut out = Vec::new();
    for side in ["F", "B"] {
        let layer = format!("{side}.CrtYd");
        let placed_side = p.flip_layer(&layer).trim_end_matches(".CrtYd").to_string();
        let rings = outlines_on(p, &layer);
        if rings.is_empty() {
            continue;
        }
        let mut bounds = Bounds::EMPTY;
        rings.iter().flatten().for_each(|q| bounds.add(*q));
        out.push(Courtyard { part: pi, side: placed_side, rings: nest_rings(rings), bounds });
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BodyFrom {
    Fab,
    Courtyard,
    Pads,
}

impl BodyFrom {
    pub fn name(self) -> &'static str {
        match self {
            BodyFrom::Fab => "fab outline",
            BodyFrom::Courtyard => "courtyard",
            BodyFrom::Pads => "pad copper",
        }
    }
}

pub fn body_outlines(p: &Placed) -> (BodyFrom, Vec<Vec<P>>) {
    for (from, layers) in
        [(BodyFrom::Fab, ["F.Fab", "B.Fab"]), (BodyFrom::Courtyard, ["F.CrtYd", "B.CrtYd"])]
    {
        let rings: Vec<Vec<P>> = layers.iter().flat_map(|l| outlines_on(p, l)).collect();
        if !rings.is_empty() {
            return (from, rings);
        }
    }
    (BodyFrom::Pads, p.pads.iter().flat_map(|q| q.outlines.iter().cloned()).collect())
}

pub fn body_region(p: &Placed) -> Vec<Vec<P>> {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;
    let (from, rings) = body_outlines(p);
    let holes: Vec<Vec<P>> = if from == BodyFrom::Fab {
        let court: Vec<Vec<P>> =
            ["F.CrtYd", "B.CrtYd"].iter().flat_map(|l| outlines_on(p, l)).collect();
        nest_rings(court).into_iter().filter(|r| geom::signed_area(r) < 0.0).collect()
    } else {
        Vec::new()
    };
    if rings.len() < 2 && holes.is_empty() {
        return rings;
    }
    rings
        .overlay(&holes, OverlayRule::Difference, FillRule::EvenOdd)
        .into_iter()
        .flatten()
        .filter(|r| r.len() >= 3)
        .collect()
}

pub fn region_gap(poly: &[P], region: &[Vec<P>]) -> f64 {
    if poly.is_empty() || region.is_empty() {
        return f64::MAX;
    }
    let inside = region.iter().filter(|r| geom::point_in_polygon(poly[0], r)).count() % 2 == 1;
    if inside || region.iter().any(|r| geom::point_in_polygon(r[0], poly)) {
        return 0.0;
    }
    let n = poly.len();
    let mut best = f64::MAX;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        for r in region {
            let m = r.len();
            for j in 0..m {
                best = best.min(geom::segment_segment_distance(a, b, r[j], r[(j + 1) % m]));
            }
        }
    }
    best
}

fn overlap_area(a: &[Vec<P>], b: &[Vec<P>]) -> f64 {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;
    a.to_vec()
        .overlay(&b.to_vec(), OverlayRule::Intersect, FillRule::NonZero)
        .iter()
        .map(|shape| {
            shape
                .iter()
                .enumerate()
                .map(|(k, r)| {
                    let area = crate::contour::area(r).abs();
                    if k == 0 { area } else { -area }
                })
                .sum::<f64>()
        })
        .sum()
}

const MIN_AREA: f64 = 1e-4;

type Issue = (String, String);

type Hole = (usize, String, Option<String>, Vec<Vec<P>>);

fn courtyard_issues(parts: &[Placed], holes_too: bool) -> (Vec<Issue>, Vec<Issue>) {
    let (mut over, mut covers) = (Vec::new(), Vec::new());
    let courts: Vec<Courtyard> =
        parts.iter().enumerate().flat_map(|(i, p)| courtyards_of(i, p)).collect();
    let mut overlapping: Vec<(usize, usize)> = Vec::new();
    for (i, a) in courts.iter().enumerate() {
        for b in &courts[i + 1..] {
            if a.part == b.part
                || a.side != b.side
                || !a.bounds.overlaps(&b.bounds)
                || overlapping.contains(&(a.part.min(b.part), a.part.max(b.part)))
            {
                continue;
            }
            let area = overlap_area(&a.rings, &b.rings);
            if area > MIN_AREA {
                overlapping.push((a.part.min(b.part), a.part.max(b.part)));
                over.push((
                    format!("part {}", parts[a.part].reference),
                    format!(
                        "courtyard overlaps {} on {}.CrtYd by {area:.3} mm2",
                        parts[b.part].reference, a.side
                    ),
                ));
            }
        }
    }
    if !holes_too {
        return (over, covers);
    }
    let mut holes: Vec<Hole> = Vec::new();
    for (pi, p) in parts.iter().enumerate() {
        if p.footprint_name.starts_with("MountingHole") {
            let own: Vec<&Courtyard> = courts.iter().filter(|c| c.part == pi).collect();
            if own.is_empty() {
                for pad in p.pads.iter().filter(|q| q.drill.is_some()) {
                    for o in &pad.outlines {
                        holes.push((
                            pi,
                            format!("mounting hole {}", p.reference),
                            None,
                            vec![o.clone()],
                        ));
                    }
                }
            }
            for c in own {
                let name = format!("mounting hole {}", p.reference);
                holes.push((pi, name, Some(c.side.clone()), c.rings.clone()));
            }
            continue;
        }
        for pad in p.pads.iter().filter(|q| q.kind == PadKind::Npth) {
            for o in &pad.outlines {
                holes.push((
                    pi,
                    format!("hole {}.{}", p.reference, pad.number),
                    None,
                    vec![o.clone()],
                ));
            }
        }
    }
    let mut reported: Vec<(usize, usize)> = Vec::new();
    for (hp, name, side, poly) in &holes {
        let mut hb = Bounds::EMPTY;
        poly.iter().flatten().for_each(|q| hb.add(*q));
        for c in &courts {
            let pair = (c.part.min(*hp), c.part.max(*hp));
            if c.part == *hp
                || side.as_ref() == Some(&c.side)
                || !c.bounds.overlaps(&hb)
                || overlapping.contains(&pair)
                || reported.contains(&(c.part, *hp))
            {
                continue;
            }
            if overlap_area(&c.rings, poly) > MIN_AREA {
                reported.push((c.part, *hp));
                covers.push((
                    format!("part {}", parts[c.part].reference),
                    format!("courtyard on {}.CrtYd covers the {name}", c.side),
                ));
            }
        }
    }
    (over, covers)
}
