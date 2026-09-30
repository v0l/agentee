use super::courtyard::{BodyFrom, body_outlines};
use super::{Category, Ctx, Report, Rule, Setup, is_smd};
use crate::diag::Severity;
use crate::footprint::{PadKind, PadShape};
use crate::geom::{self, P};
use crate::layout::Placed;
use crate::units::Length;
use std::collections::BTreeSet;

pub static RULES: &[Rule] = &[
    Rule {
        id: "part-to-edge",
        category: Category::Assembly,
        severity: Severity::Warning,
        summary: "SMD part pads closer than min_part_to_edge to the board outline, where depaneling stress cracks parts",
        when: "placed parts",
        applies: with_parts,
        check: part_to_edge,
    },
    Rule {
        id: "part-body-to-edge",
        category: Category::Assembly,
        severity: Severity::Info,
        summary: "a part body (fab outline, else courtyard, else pads) closer than min_body_to_edge to the board outline, or past it; skips `overhang = true` footprints",
        when: "placed parts",
        applies: with_parts,
        check: part_body_to_edge,
    },
    Rule {
        id: "fiducials",
        category: Category::Assembly,
        severity: Severity::Info,
        summary: "no fiducial footprints on the board",
        when: "placed parts",
        applies: with_parts,
        check: fiducials,
    },
    Rule {
        id: "tooling-holes",
        category: Category::Assembly,
        severity: Severity::Info,
        summary: "no non-plated tooling holes of 1.5 mm or more",
        when: "placed parts",
        applies: with_parts,
        check: tooling_holes,
    },
    Rule {
        id: "bga-pad",
        category: Category::Assembly,
        severity: Severity::Error,
        summary: "BGA pads smaller than min_bga_pad",
        when: "a BGA",
        applies: with_bga,
        check: bga_pad,
    },
    Rule {
        id: "bga-pitch",
        category: Category::Assembly,
        severity: Severity::Error,
        summary: "BGA pitch finer than min_bga_pitch",
        when: "a BGA",
        applies: with_bga,
        check: bga_pitch_rule,
    },
    Rule {
        id: "bga-pad-ratio",
        category: Category::Assembly,
        severity: Severity::Warning,
        summary: "BGA pad diameter outside 40% to 65% of the pitch",
        when: "a BGA",
        applies: with_bga,
        check: bga_pad_ratio,
    },
    Rule {
        id: "paste-without-mask",
        category: Category::Mask,
        severity: Severity::Warning,
        summary: "a pad with paste but no mask opening on that side, so the stencil prints onto mask",
        when: "placed parts",
        applies: with_parts,
        check: paste_without_mask,
    },
];

fn with_parts(s: &Setup) -> bool {
    s.parts > 0
}

fn with_bga(s: &Setup) -> bool {
    s.bga
}

fn mm(v: f64) -> Length {
    Length::mm(v)
}

fn is_marker(p: &Placed) -> bool {
    let n = p.footprint_name.to_lowercase();
    n.contains("fiducial") || n.contains("mountinghole")
}

fn part_to_edge(cx: &Ctx, r: &mut Report) {
    let edge = cx.edge();
    if !edge.is_closed() {
        return;
    }
    let need = cx.board.rules.min_part_to_edge.to_mm();
    for p in cx.parts.iter().filter(|p| !is_marker(p)) {
        if p.footprint.pads.iter().any(|f| f.edge) {
            continue;
        }
        let mut worst: Option<(f64, &str)> = None;
        for q in p.pads.iter().filter(|q| is_smd(q)) {
            if q.outlines.iter().flatten().any(|c| !edge.contains(*c)) {
                continue;
            }
            let gap = q.outlines.iter().map(|o| edge.polygon_distance(o)).fold(f64::MAX, f64::min);
            if gap + 1e-6 < need && worst.is_none_or(|w| gap < w.0) {
                worst = Some((gap, &q.number));
            }
        }
        if let Some((gap, pad)) = worst {
            r.emit(
                format!("part {}", p.reference),
                format!(
                    "pad {pad} is {} from the board edge, keep parts {} in so depaneling does not crack them",
                    mm(gap),
                    mm(need)
                ),
            );
        }
    }
}

fn ring_edge_gap(ring: &[P], edge: geom::BoardEdge) -> Option<f64> {
    if ring.iter().any(|c| !edge.contains(*c))
        || edge.cutouts.iter().any(|c| geom::polygon_distance(ring, c) <= 0.0)
    {
        return None;
    }
    Some(edge.polygon_distance(ring))
}

fn part_body_to_edge(cx: &Ctx, r: &mut Report) {
    let edge = cx.edge();
    if !edge.is_closed() {
        return;
    }
    let need = cx.board.rules.min_body_to_edge.to_mm();
    for p in cx.parts.iter().filter(|p| !is_marker(p) && !p.footprint.overhang) {
        if p.footprint.pads.iter().any(|f| f.edge) {
            continue;
        }
        let (from, rings) = body_outlines(p);
        let gaps: Vec<Option<f64>> = rings.iter().map(|o| ring_edge_gap(o, edge)).collect();
        let at = format!("part {}", p.reference);
        if from != BodyFrom::Pads && gaps.iter().any(Option::is_none) {
            r.emit(
                at,
                format!(
                    "its {} reaches past the board edge; keep bodies {} in, or set `overhang = true` on a connector footprint meant to hang over the edge",
                    from.name(),
                    mm(need)
                ),
            );
            continue;
        }
        let gap = gaps.into_iter().flatten().fold(f64::MAX, f64::min);
        if gap + 1e-6 < need {
            r.emit(
                at,
                format!(
                    "its {} is {} from the board edge; assembly DFM guides keep part bodies {} in so depaneling does not crack them",
                    from.name(),
                    mm((gap * 1000.0).round() / 1000.0),
                    mm(need)
                ),
            );
        }
    }
}

fn fiducials(cx: &Ctx, r: &mut Report) {
    if !cx.parts.iter().any(|p| p.footprint_name.to_lowercase().contains("fiducial")) {
        r.emit(
            "parts",
            "no fiducials: JLCPCB puts them on the rails of an assembly panel, place three on the board for a line that assembles it bare",
        );
    }
}

fn tooling_holes(cx: &Ctx, r: &mut Report) {
    let found = cx.parts.iter().flat_map(|p| p.pads.iter()).any(|q| {
        q.kind == PadKind::Npth && q.drill.is_some_and(|(_, s, _)| s[0].min(s[1]) >= 1.5 - 1e-6)
    });
    if !found {
        r.emit(
            "parts",
            "no non-plated tooling holes of 1.5 mm or more: JLCPCB wants three for high precision routing, otherwise the panel rails carry them",
        );
    }
}

fn bgas<'a>(cx: &Ctx<'a>) -> Vec<(&'a Placed, f64, f64)> {
    let mut seen = BTreeSet::new();
    cx.parts
        .iter()
        .filter(|p| seen.insert(p.footprint_name.clone()))
        .filter_map(|p| bga_pitch(p).map(|(pitch, pad)| (p, pitch, pad)))
        .collect()
}

fn bga_pad(cx: &Ctx, r: &mut Report) {
    let min = cx.board.rules.min_bga_pad;
    for (p, _, pad) in bgas(cx) {
        if pad + 1e-6 < min.to_mm() {
            r.emit(
                format!("part {}", p.reference),
                format!(
                    "BGA pads of {} are {}, under the fab minimum {min}{}",
                    p.footprint_name,
                    mm(pad),
                    if cx.board.stackup.finish.eq_ignore_ascii_case("ENIG") {
                        ""
                    } else {
                        " (0.2 mm with ENIG)"
                    }
                ),
            );
        }
    }
}

fn bga_pitch_rule(cx: &Ctx, r: &mut Report) {
    let min = cx.board.rules.min_bga_pitch;
    for (p, pitch, _) in bgas(cx) {
        if pitch + 1e-6 < min.to_mm() {
            r.emit(
                format!("part {}", p.reference),
                format!(
                    "{} has a {} ball pitch, finer than the assembler's {min}",
                    p.footprint_name,
                    mm(pitch)
                ),
            );
        }
    }
}

fn bga_pad_ratio(cx: &Ctx, r: &mut Report) {
    for (p, pitch, pad) in bgas(cx) {
        let ratio = pad / pitch;
        if !(0.4 - 1e-6..=0.65 + 1e-6).contains(&ratio) {
            r.emit(
                format!("part {}", p.reference),
                format!(
                    "BGA pads of {} are {} on a {} pitch ({:.0}%), IPC-7351 lands are 40% to 65% of the pitch",
                    p.footprint_name,
                    mm(pad),
                    mm(pitch),
                    ratio * 100.0
                ),
            );
        }
    }
}

fn paste_without_mask(cx: &Ctx, r: &mut Report) {
    let mut seen = BTreeSet::new();
    for p in cx.parts {
        let bad: Vec<&str> = p
            .pads
            .iter()
            .filter(|q| {
                !q.copper.is_empty()
                    && q.paste.iter().any(|l| {
                        let side = l.trim_end_matches(".Paste");
                        !q.mask.iter().any(|m| m.trim_end_matches(".Mask") == side)
                    })
            })
            .map(|q| q.number.as_str())
            .collect();
        if !bad.is_empty() && seen.insert(p.footprint_name.clone()) {
            let bad: Vec<String> = bad.iter().map(|s| s.to_string()).collect();
            r.emit(
                format!("part {}", p.reference),
                format!(
                    "pads {} of {} have paste but no mask opening; add `F.Mask` (or `B.Mask`) to their layers",
                    super::list(&bad),
                    p.footprint_name
                ),
            );
        }
    }
}

pub fn bga_pitch(p: &Placed) -> Option<(f64, f64)> {
    let balls: Vec<&crate::footprint::Pad> = p
        .footprint
        .pads
        .iter()
        .filter(|q| q.kind == PadKind::Smd && q.shape == PadShape::Circle)
        .collect();
    if balls.len() < 16 {
        return None;
    }
    let mut pitch = f64::MAX;
    for (i, a) in balls.iter().enumerate() {
        for b in &balls[i + 1..] {
            let d = geom::dist(a.at.to_mm(), b.at.to_mm());
            if d > 1e-6 {
                pitch = pitch.min(d);
            }
        }
    }
    let pad = balls.iter().map(|q| q.size.to_mm()[0]).fold(f64::MAX, f64::min);
    (pitch < f64::MAX).then_some((pitch, pad))
}
