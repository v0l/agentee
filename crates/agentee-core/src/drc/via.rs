use super::{Category, Ctx, Report, Rule, Setup, edge_distance, every, is_smd, near, pads_where};
use crate::diag::Severity;
use crate::geom::{self, P};
use crate::layout::{PlacedPad, Via};
use crate::units::Length;
use std::collections::BTreeMap;

pub static RULES: &[Rule] = &[
    Rule {
        id: "via-cuts-pad",
        category: Category::Copper,
        severity: Severity::Error,
        summary: "a via whose copper overlaps or touches an SMD pad of its net while its drill is not fully inside the pad; a via of another net touching a pad is a `short`",
        when: "every board",
        applies: every,
        check: via_cuts_pad,
    },
    Rule {
        id: "via-annulus-past-pad",
        category: Category::Copper,
        severity: Severity::Warning,
        summary: "a via whose drill sits in an SMD pad but whose annulus crosses the pad edge",
        when: "vias in SMD pads",
        applies: with_via_in_pad,
        check: via_annulus_past_pad,
    },
    Rule {
        id: "via-in-pad",
        category: Category::Drill,
        severity: Severity::Info,
        summary: "vias fully inside SMD pads, which the fab notes ask to fill and cap",
        when: "vias in SMD pads",
        applies: with_via_in_pad,
        check: via_in_pad,
    },
    Rule {
        id: "via-in-pad-fill",
        category: Category::Drill,
        severity: Severity::Error,
        summary: "a via in a pad drilled larger than the fab fills and caps (max_filled_via_drill)",
        when: "vias in SMD pads",
        applies: with_via_in_pad,
        check: via_in_pad_fill,
    },
    Rule {
        id: "hole-to-smd-pad",
        category: Category::Drill,
        severity: Severity::Warning,
        summary: "a via hole closer than min_hole_to_smd_pad to an SMD pad of its net it does not sit in",
        when: "every board",
        applies: every,
        check: hole_to_smd_pad,
    },
];

fn with_via_in_pad(s: &Setup) -> bool {
    s.via_in_pad
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViaOnPad {
    Inside,
    AnnulusPast { edge: f64 },
    Cuts { centre_inside: bool, edge: f64 },
}

impl ViaOnPad {
    pub fn in_pad(self) -> bool {
        matches!(self, ViaOnPad::Inside | ViaOnPad::AnnulusPast { .. })
    }
}

fn circle_inside(q: &PlacedPad, c: P, r: f64) -> bool {
    geom::circle(c, (r - 0.002).max(0.0), 32)
        .into_iter()
        .all(|p| q.outlines.iter().any(|o| geom::point_in_polygon(p, o)))
}

pub fn via_on_pad(v: &Via, q: &PlacedPad) -> Option<ViaOnPad> {
    let r = v.diameter / 2.0;
    let centre_inside = q.outlines.iter().any(|o| geom::point_in_polygon(v.at, o));
    let edge = q.outlines.iter().map(|o| edge_distance(o, v.at)).fold(f64::MAX, f64::min);
    if !centre_inside && edge > r + 1e-6 {
        return None;
    }
    Some(if centre_inside && circle_inside(q, v.at, r) {
        ViaOnPad::Inside
    } else if centre_inside && circle_inside(q, v.at, v.drill / 2.0) {
        ViaOnPad::AnnulusPast { edge }
    } else {
        ViaOnPad::Cuts { centre_inside, edge }
    })
}

fn at(v: &Via) -> String {
    format!("[{:.3}, {:.3}]", v.at[0], v.at[1])
}

fn via_cuts_pad(cx: &Ctx, r: &mut Report) {
    let pads = pads_where(cx.parts, is_smd);
    for v in cx.vias {
        let rad = v.diameter / 2.0;
        for p in &pads {
            if p.q.net != Some(v.net)
                || !near(&p.bounds, v.at, rad)
                || !p.q.copper.iter().any(|l| v.layers.contains(l))
            {
                continue;
            }
            let Some(ViaOnPad::Cuts { centre_inside, edge }) = via_on_pad(v, p.q) else {
                continue;
            };
            let pad = cx.pad_name(p.part, p.pad);
            let how = if centre_inside {
                format!(
                    "sits in {pad} {edge:.3} mm from its edge, so the {:.3} mm drill crosses it",
                    v.drill / 2.0
                )
            } else if edge < rad - 1e-6 {
                format!("cuts {:.3} mm into {pad}", rad - edge)
            } else {
                format!("touches the edge of {pad}")
            };
            r.emit(
                format!("via {}", at(v)),
                format!(
                    "{} via {how}: solder wicks down the barrel and the pad is damaged; centre it in the pad (filled and capped) or move it clear",
                    cx.nets[v.net].name
                ),
            );
        }
    }
}

fn via_annulus_past_pad(cx: &Ctx, r: &mut Report) {
    for (vi, p, k) in super::vias_in_pads(cx.parts, cx.vias) {
        let v = &cx.vias[vi];
        if let Some(ViaOnPad::AnnulusPast { edge }) = via_on_pad(v, &cx.parts[p].pads[k]) {
            r.emit(
                format!("via {}", at(v)),
                format!(
                    "{} via sits in {} {edge:.3} mm from its edge, its {:.3} mm annulus reaches past the pad under the mask; centre it or use a smaller via",
                    cx.nets[v.net].name,
                    cx.pad_name(p, k),
                    v.diameter / 2.0
                ),
            );
        }
    }
}

fn via_in_pad(cx: &Ctx, r: &mut Report) {
    let found = super::vias_in_pads(cx.parts, cx.vias);
    if found.is_empty() {
        return;
    }
    let mut pads: Vec<String> = found.iter().map(|&(_, p, k)| cx.pad_name(p, k)).collect();
    pads.dedup();
    r.emit(
        "vias",
        format!(
            "{} vias sit in SMD pads ({}): the fab notes ask to fill and cap them (IPC-4761 type VII)",
            found.len(),
            super::list(&pads)
        ),
    );
}

fn via_in_pad_fill(cx: &Ctx, r: &mut Report) {
    let max = cx.board.rules.max_filled_via_drill.to_mm();
    let mut big: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for (vi, p, k) in super::vias_in_pads(cx.parts, cx.vias) {
        let v = &cx.vias[vi];
        if v.drill > max + 1e-6 {
            let e = big.entry(format!("{}", Length::mm(v.drill))).or_insert((0, cx.pad_name(p, k)));
            e.0 += 1;
        }
    }
    for (drill, (n, first)) in big {
        r.emit(
            "vias",
            format!(
                "{n} vias of {drill} drill sit in pads (first {first}), the fab fills and caps holes up to {}",
                Length::mm(max)
            ),
        );
    }
}

fn hole_to_smd_pad(cx: &Ctx, r: &mut Report) {
    let need = cx.board.rules.min_hole_to_smd_pad.to_mm();
    let pads = pads_where(cx.parts, is_smd);
    let mut per_part: BTreeMap<usize, (usize, f64, String)> = BTreeMap::new();
    for v in cx.vias {
        let hole = v.drill / 2.0;
        for p in pads.iter().filter(|p| p.q.net == Some(v.net) || p.q.net.is_none()) {
            if !near(&p.bounds, v.at, hole + need)
                || !p.q.copper.iter().any(|l| v.layers.contains(l))
                || via_on_pad(v, p.q).is_some()
            {
                continue;
            }
            let gap =
                p.q.outlines.iter().map(|o| edge_distance(o, v.at)).fold(f64::MAX, f64::min) - hole;
            if gap + 1e-6 < need {
                let e = per_part.entry(p.part).or_insert((0, f64::MAX, String::new()));
                e.0 += 1;
                if gap < e.1 {
                    e.1 = gap;
                    e.2 = format!("{} at {}", cx.pad_name(p.part, p.pad), at(v));
                }
            }
        }
    }
    for (part, (n, gap, first)) in per_part {
        r.emit(
            format!("part {}", cx.parts[part].reference),
            format!(
                "{n} via holes closer than {} to its pads, closest {} from {first}; paste and solder can flow into the hole",
                Length::mm(need),
                Length::mm(gap)
            ),
        );
    }
}
