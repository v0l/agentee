use super::{Category, Ctx, Report, Rule, Setup, edge_distance, every};
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
        summary: "a via in a pad drilled larger than the fab fills and caps (max_filled_via_drill), or whose via type sets a `fill` other than filled_capped",
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

fn run(cx: &Ctx, rule: impl crate::rules::Rule) -> Vec<crate::rules::Violation> {
    let placed = crate::rules::Placed::new(cx);
    let mut out = Vec::new();
    rule.eval(&placed, &mut out);
    out
}

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

fn via_cuts_pad(cx: &Ctx, r: &mut Report) {
    for v in run(cx, crate::rules::ViaCutsPad) {
        let net = v.nets.map(|n| cx.nets[n.0].name.clone()).unwrap_or_default();
        r.emit(
            v.group,
            format!(
                "{net} via {}: solder wicks down the barrel and the pad is damaged; centre it in the pad (filled and capped) or move it clear",
                v.detail
            ),
        );
    }
}

fn via_annulus_past_pad(cx: &Ctx, r: &mut Report) {
    for v in run(cx, crate::rules::ViaAnnulusPastPad) {
        let net = v.nets.map(|n| cx.nets[n.0].name.clone()).unwrap_or_default();
        r.emit(
            v.group,
            format!(
                "{net} via sits in {} {:.3} mm from its edge, its {:.3} mm annulus reaches past the pad under the mask; centre it or use a smaller via",
                v.other,
                v.gap,
                v.need
            ),
        );
    }
}

fn via_in_pad(cx: &Ctx, r: &mut Report) {
    let found = run(cx, crate::rules::ViaInPad);
    if found.is_empty() {
        return;
    }
    let mut pads: Vec<String> = found.iter().map(|v| v.other.clone()).collect();
    pads.dedup();
    let mut kinds: Vec<String> = found.iter().map(|v| v.detail.clone()).collect();
    kinds.sort();
    kinds.dedup();
    r.emit(
        "vias",
        format!(
            "{} vias sit in SMD pads ({}): the fab notes ask to fill and cap them (IPC-4761 type VII); {}",
            found.len(),
            super::list(&pads),
            kinds.join(", ")
        ),
    );
}

fn via_in_pad_fill(cx: &Ctx, r: &mut Report) {
    let max = cx.board.rules.max_filled_via_drill.to_mm();
    let found = run(cx, crate::rules::ViaInPadFill);
    let mut big: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut open: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for v in &found {
        if v.group == "drill" {
            big.entry(format!("{}", Length::mm(v.gap))).or_insert((0, v.other.clone())).0 += 1;
        } else {
            open.entry(v.detail.clone()).or_insert((0, v.other.clone())).0 += 1;
        }
    }
    for (what, (n, first)) in open {
        r.emit(
            "vias",
            format!(
                "{n} vias sit in pads (first {first}) but via {what}: solder wicks into the hole or the pad is not flat; use fill = \"filled_capped\" (type VII) for via-in-pad"
            ),
        );
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
    let mut per_part: BTreeMap<String, (usize, f64, String)> = BTreeMap::new();
    for v in run(cx, crate::rules::HoleToSmdPad) {
        let e = per_part.entry(v.group).or_insert((0, f64::MAX, String::new()));
        e.0 += 1;
        if v.gap < e.1 {
            e.1 = v.gap;
            e.2 = v.detail;
        }
    }
    for (part, (n, gap, first)) in per_part {
        r.emit(
            part,
            format!(
                "{n} via holes closer than {} to its pads, closest {} from {first}; paste and solder can flow into the hole",
                Length::mm(need),
                Length::mm(gap)
            ),
        );
    }
}
