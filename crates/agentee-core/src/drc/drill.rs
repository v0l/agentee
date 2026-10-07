use super::{Category, Ctx, Hole, HoleOf, Report, Rule, Setup, every};
use crate::board::{LayerKind, ViaKind};
use crate::diag::Severity;
use crate::geom::P;
use crate::units::Length;
use std::collections::BTreeMap;

pub static RULES: &[Rule] = &[
    Rule {
        id: "drill-size",
        category: Category::Drill,
        severity: Severity::Error,
        summary: "pad holes under min_drill (plated) or min_npth_drill (non-plated), or over max_drill",
        when: "every board",
        applies: every,
        check: drill_size,
    },
    Rule {
        id: "slot-size",
        category: Category::Drill,
        severity: Severity::Error,
        summary: "slots narrower than min_plated_slot_width or min_npth_slot_width, or shorter than twice their width",
        when: "slotted holes",
        applies: with_slots,
        check: slot_size,
    },
    Rule {
        id: "aspect-ratio",
        category: Category::Drill,
        severity: Severity::Error,
        summary: "plated holes whose depth over drill exceeds max_aspect_ratio",
        when: "every board",
        applies: every,
        check: aspect_ratio,
    },
    Rule {
        id: "hole-to-copper",
        category: Category::Drill,
        severity: Severity::Error,
        summary: "a via or plated pad hole wall closer than min_via_hole_to_copper or min_pth_hole_to_copper to another net's copper",
        when: "every board",
        applies: every,
        check: hole_to_copper,
    },
    Rule {
        id: "inner-hole-to-copper",
        category: Category::Drill,
        severity: Severity::Error,
        summary: "a plated pad hole wall closer than min_inner_pth_hole_to_copper to another net's copper on an inner layer",
        when: "4 or more copper layers",
        applies: with_inner_layers,
        check: inner_hole_to_copper,
    },
    Rule {
        id: "npth-to-copper",
        category: Category::Drill,
        severity: Severity::Error,
        summary: "a non-plated hole wall closer than min_npth_to_copper to any copper",
        when: "non-plated holes",
        applies: with_npth,
        check: npth_to_copper,
    },
    Rule {
        id: "hole-to-edge",
        category: Category::Drill,
        severity: Severity::Error,
        summary: "a non-plated hole wall closer than min_copper_to_edge to the board outline or a board cutout",
        when: "non-plated holes",
        applies: with_npth,
        check: hole_to_edge,
    },
    Rule {
        id: "hole-to-hole",
        category: Category::Drill,
        severity: Severity::Error,
        summary: "drill holes of different parts or vias closer than min_hole_to_hole, wall to wall, where their spans share a dielectric",
        when: "every board",
        applies: every,
        check: hole_to_hole,
    },
    Rule {
        id: "stacked-via",
        category: Category::Drill,
        severity: Severity::Error,
        summary: "a via on the same spot as another via whose span shares a dielectric, a stacked via the fab does not build or that is out of the lamination build order, or any via stacked with a controlled depth via",
        when: "every board",
        applies: every,
        check: stacked_via,
    },
    Rule {
        id: "via-lamination",
        category: Category::Drill,
        severity: Severity::Error,
        summary: "a via whose span and drill kind match no drill step of the lamination, the build-up sequence derived from the stackup or set in [stackup] lamination; the message lists the spans it drills",
        when: "every board",
        applies: every,
        check: via_lamination,
    },
];

fn with_slots(s: &Setup) -> bool {
    s.slots
}

fn with_npth(s: &Setup) -> bool {
    s.npth
}

pub fn with_inner_layers(s: &Setup) -> bool {
    s.copper_layers >= 4
}

fn mm(v: f64) -> Length {
    Length::mm(v)
}

fn pad_holes(cx: &Ctx) -> Vec<(usize, usize, Hole)> {
    cx.holes()
        .into_iter()
        .filter_map(|h| match h.of {
            HoleOf::Pad(p, k) => Some((p, k, h)),
            HoleOf::Via(_) => None,
        })
        .collect()
}

fn once_per_footprint(cx: &Ctx, found: Vec<(usize, usize, String)>, r: &mut Report) {
    let mut seen: BTreeMap<(String, String), (String, usize)> = BTreeMap::new();
    let mut order = Vec::new();
    for (p, k, msg) in found {
        let key = (cx.parts[p].footprint_name.clone(), msg);
        let e = seen.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            (cx.pad_name(p, k), 0)
        });
        e.1 += 1;
    }
    for key in order {
        let (first, n) = &seen[&key];
        let more = if *n > 1 { format!(" ({n} placements)") } else { String::new() };
        r.emit(format!("pad {first}"), format!("{}{more}, footprint {}", key.1, key.0));
    }
}

fn drill_size(cx: &Ctx, r: &mut Report) {
    let rules = &cx.board.rules;
    let mut found = Vec::new();
    for (p, k, h) in pad_holes(cx) {
        let (small, big) = (h.size[0].min(h.size[1]), h.size[0].max(h.size[1]));
        let floor = if h.plated { rules.min_drill } else { rules.min_npth_drill };
        let kind = if h.plated { "plated" } else { "non-plated" };
        if small + 1e-6 < floor.to_mm() {
            found.push((
                p,
                k,
                format!("{kind} hole {} is under the fab minimum {floor}", mm(small)),
            ));
        }
        if big > rules.max_drill.to_mm() + 1e-6 {
            found.push((
                p,
                k,
                format!(
                    "hole {} is over the largest drill {}, draw it as a routed cutout",
                    mm(big),
                    rules.max_drill
                ),
            ));
        }
    }
    once_per_footprint(cx, found, r);
}

fn slot_size(cx: &Ctx, r: &mut Report) {
    let rules = &cx.board.rules;
    let mut found = Vec::new();
    for (p, k, h) in pad_holes(cx).into_iter().filter(|x| x.2.slot()) {
        let (w, l) = (h.size[0].min(h.size[1]), h.size[0].max(h.size[1]));
        let floor = if h.plated { rules.min_plated_slot_width } else { rules.min_npth_slot_width };
        let kind = if h.plated { "plated" } else { "non-plated" };
        if w + 1e-6 < floor.to_mm() {
            found.push((
                p,
                k,
                format!("{kind} slot {} wide is under the fab minimum {floor}", mm(w)),
            ));
        }
        if l + 1e-6 < 2.0 * w {
            found.push((
                p,
                k,
                format!(
                    "{kind} slot {} x {} is shorter than twice its width, the fab drills it as a hole",
                    mm(w),
                    mm(l)
                ),
            ));
        }
    }
    once_per_footprint(cx, found, r);
}

fn span(cx: &Ctx, layers: &[String]) -> f64 {
    let st = &cx.board.stackup;
    let (Some(a), Some(b)) =
        (layers.first().and_then(|l| st.index_of(l)), layers.last().and_then(|l| st.index_of(l)))
    else {
        return 0.0;
    };
    st.layers[a.min(b)..=a.max(b)]
        .iter()
        .filter(|l| l.kind == LayerKind::Copper || l.kind.is_dielectric())
        .map(|l| l.thickness.to_mm())
        .sum()
}

fn aspect_ratio(cx: &Ctx, r: &mut Report) {
    let max = cx.board.rules.max_aspect_ratio;
    let mut vias: BTreeMap<(String, String, String), (usize, String)> = BTreeMap::new();
    let mut found = Vec::new();
    for h in cx.holes().into_iter().filter(|h| h.plated) {
        if let HoleOf::Via(v) = h.of
            && cx.vias[v].kind == ViaKind::Microvia
        {
            continue;
        }
        let drill = h.size[0].min(h.size[1]);
        let depth = span(cx, &h.layers);
        if drill <= 0.0 || depth / drill <= max + 1e-9 {
            continue;
        }
        let what = format!(
            "{} deep over a {} drill is {:.1}:1, over the fab's {max}:1",
            mm(depth),
            mm(drill),
            depth / drill
        );
        match h.of {
            HoleOf::Via(_) => {
                let key = (
                    format!("{}", mm(drill)),
                    h.layers.first().cloned().unwrap_or_default(),
                    h.layers.last().cloned().unwrap_or_default(),
                );
                let e = vias.entry(key).or_insert((0, what));
                e.0 += 1;
            }
            HoleOf::Pad(p, k) => found.push((p, k, format!("plated hole {what}"))),
        }
    }
    for ((drill, from, to), (n, what)) in vias {
        r.emit(format!("vias {drill} {from}-{to}"), format!("{n} vias: {what}"));
    }
    once_per_footprint(cx, found, r);
}

struct Hit {
    gap: f64,
    hole: String,
    other: String,
}

fn report_groups(
    groups: BTreeMap<String, Vec<Hit>>,
    need_of: &dyn Fn(&str) -> f64,
    what: &str,
    r: &mut Report,
) {
    for (group, mut hits) in groups {
        hits.sort_by(|a, b| a.gap.total_cmp(&b.gap));
        let first: Vec<String> = hits
            .iter()
            .take(3)
            .map(|h| format!("{} is {} from {}", h.hole, mm(h.gap), h.other))
            .collect();
        r.emit(
            group.clone(),
            format!(
                "{} holes closer than {} to {what}: {}",
                hits.len(),
                mm(need_of(&group)),
                first.join("; ")
            ),
        );
    }
}

fn from_rule(cx: &Ctx, rule: impl crate::rules::Rule, what: &str, r: &mut Report) {
    let placed = crate::rules::Placed::new(cx);
    let mut found = Vec::new();
    rule.eval(&placed, &mut found);
    let mut groups: BTreeMap<String, Vec<Hit>> = BTreeMap::new();
    let mut need: BTreeMap<String, f64> = BTreeMap::new();
    for v in found {
        need.insert(v.group.clone(), v.need);
        groups.entry(v.group).or_default().push(Hit {
            gap: v.gap,
            hole: v.subject,
            other: v.other,
        });
    }
    report_groups(groups, &|g| need.get(g).copied().unwrap_or(0.0), what, r);
}

fn hole_to_copper(cx: &Ctx, r: &mut Report) {
    let rule = crate::rules::HoleToCopper(crate::rules::Which::Plated);
    from_rule(cx, rule, "another net's copper", r);
}

fn inner_hole_to_copper(cx: &Ctx, r: &mut Report) {
    let rule = crate::rules::HoleToCopper(crate::rules::Which::Inner);
    from_rule(cx, rule, "another net's copper on an inner layer", r);
}

fn npth_to_copper(cx: &Ctx, r: &mut Report) {
    from_rule(cx, crate::rules::HoleToCopper(crate::rules::Which::Npth), "copper", r);
}

fn hole_to_edge(cx: &Ctx, r: &mut Report) {
    let edge = cx.edge();
    if !edge.is_closed() {
        return;
    }
    let need = cx.board.rules.min_copper_to_edge.to_mm();
    for h in cx.holes().into_iter().filter(|h| !h.plated) {
        let inside = edge.contains(h.a) && edge.contains(h.b);
        let gap = edge.segment_distance(h.a, h.b) - h.r;
        if !inside || gap + 1e-6 < need {
            let how = if !inside || gap < 0.0 {
                "breaks through the board edge".to_string()
            } else {
                format!("is {} from the board edge", mm(gap))
            };
            r.emit(
                format!("pad {}", cx.hole_name(&h)),
                format!("non-plated hole {how}, needs {}", mm(need)),
            );
        }
    }
}

fn hole_to_hole(cx: &Ctx, r: &mut Report) {
    use crate::rules::Rule;
    let placed = crate::rules::Placed::new(cx);
    let mut found = Vec::new();
    crate::rules::HoleToHole.eval(&placed, &mut found);
    if let Some(f) = found.first() {
        r.emit(
            "drills",
            format!(
                "{} drill pairs closer than {}, first: {} is {} from {}",
                found.len(),
                mm(f.need),
                f.subject,
                mm(f.gap),
                f.other
            ),
        );
    }
}

fn stacked_via(cx: &Ctx, r: &mut Report) {
    use crate::rules::Rule;
    let placed = crate::rules::Placed::new(cx);
    let mut found = Vec::new();
    crate::rules::StackedVia.eval(&placed, &mut found);
    let count = |g: &str| found.iter().filter(|v| v.group == g).count();
    let first = |g: &str| found.iter().find(|v| v.group == g).map(|v| v.detail.clone());
    if let Some(f) = first("depth") {
        r.emit(
            "vias",
            format!("{} vias are stacked with a controlled depth via, whose drill stops by depth on a plain pad and cannot land on or under another via: stagger them, first at {f}", count("depth")),
        );
    }
    if let Some(f) = first("order") {
        r.emit(
            "vias",
            format!("{} vias are stacked out of the lamination's build order: the via under a stack must be drilled at the same or an earlier step than the one on top, stagger them or reorder [stackup] lamination, first at {f}", count("order")),
        );
    }
    if let Some(f) = first("doubled") {
        r.emit(
            "vias",
            format!("{} vias sit on another of their net at the same spot through the same layers, first at {f}", count("doubled")),
        );
    }
    if let Some(f) = first("stacked") {
        r.emit(
            "vias",
            format!("{} vias are stacked on another via, which the fab does not build (set [rules] stacked_microvias = true for an HDI fab, or stagger them), first at {f}", count("stacked")),
        );
    }
}

fn via_lamination(cx: &Ctx, r: &mut Report) {
    let st = &cx.board.stackup;
    let mut bad: BTreeMap<(String, String, String), (usize, P, String)> = BTreeMap::new();
    for v in cx.vias {
        let (Some(from), Some(to)) = (v.hole.first(), v.hole.last()) else { continue };
        if let Err(e) = st.drillable(from, to, v.drill_kind, v.stacked) {
            bad.entry((v.name.clone(), from.clone(), to.clone())).or_insert((0, v.at, e)).0 += 1;
        }
    }
    for ((name, from, to), (n, at, e)) in bad {
        let def = match cx.board.vias.iter().position(|b| b.name == name) {
            Some(i) => format!(" (board vias[{i}])"),
            None => String::new(),
        };
        r.emit(
            format!("vias {name}"),
            format!(
                "{n} vias `{name}`{def} from {from} to {to} cannot be drilled, first at [{:.3}, {:.3}]: {e}",
                at[0], at[1]
            ),
        );
    }
}
