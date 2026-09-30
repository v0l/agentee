use super::{Category, Ctx, Hole, HoleOf, Owner, Report, Rule, Setup, every, near};
use crate::board::{DrillKind, LayerKind, ViaKind};
use crate::diag::Severity;
use crate::geom::{self, P};
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

fn crowding(
    cx: &Ctx,
    h: &Hole,
    need: f64,
    any_net: bool,
    layer_ok: &dyn Fn(&str) -> bool,
) -> Option<Hit> {
    let mut worst: Option<(f64, String)> = None;
    let items = cx.copper_items();
    let hb = h.bounds();
    let shares = |layers: &[String]| layers.iter().any(|l| h.layers.contains(l) && layer_ok(l));
    let other_net = |n: Option<usize>| any_net || n.is_none() || n != h.net;
    for i in cx.items_near(&hb, need) {
        let c = &items[i];
        let own = match (h.of, c.owner) {
            (HoleOf::Via(a), Owner::Via(b)) => a == b,
            (HoleOf::Pad(p, k), Owner::Pad(q, j)) => (p, k) == (q, j),
            _ => false,
        };
        if own || !other_net(c.net) || !shares(&c.layers) {
            continue;
        }
        let gap = h.gap_to(&c.shape);
        if gap + 1e-6 < need && worst.as_ref().is_none_or(|w| gap < w.0) {
            worst = Some((gap, cx.describe(c)));
        }
    }
    for (z, f) in cx.zones.iter().zip(cx.fills()) {
        if !other_net(Some(z.net))
            || !h.layers.contains(&z.layer)
            || !layer_ok(&z.layer)
            || !near(&f.bounds, hb.center(), need + hb.size()[0].max(hb.size()[1]))
        {
            continue;
        }
        let gap = h.gap_to_fill(f, need);
        if gap + 1e-6 < need && worst.as_ref().is_none_or(|w| gap < w.0) {
            worst = Some((gap, format!("the {} pour on {}", cx.nets[z.net].name, z.layer)));
        }
    }
    worst.map(|(gap, other)| Hit { gap, hole: cx.hole_name(h), other })
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

fn group_of(cx: &Ctx, h: &Hole) -> String {
    match h.of {
        HoleOf::Via(_) => format!("vias {}", mm(h.size[0])),
        HoleOf::Pad(p, _) => format!("part {}", cx.parts[p].reference),
    }
}

fn hole_to_copper(cx: &Ctx, r: &mut Report) {
    let rules = &cx.board.rules;
    let (via, pth) = (rules.min_via_hole_to_copper.to_mm(), rules.min_pth_hole_to_copper.to_mm());
    let mut groups: BTreeMap<String, Vec<Hit>> = BTreeMap::new();
    for h in cx.holes().into_iter().filter(|h| h.plated) {
        let need = if matches!(h.of, HoleOf::Via(_)) { via } else { pth };
        if let Some(hit) = crowding(cx, &h, need, false, &|_| true) {
            groups.entry(group_of(cx, &h)).or_default().push(hit);
        }
    }
    let need_of = |g: &str| if g.starts_with("vias") { via } else { pth };
    report_groups(groups, &need_of, "another net's copper", r);
}

fn inner_hole_to_copper(cx: &Ctx, r: &mut Report) {
    let need = cx.board.rules.min_inner_pth_hole_to_copper.to_mm();
    let inner: Vec<&String> =
        cx.copper.iter().skip(1).take(cx.copper.len().saturating_sub(2)).collect();
    let is_inner = |l: &str| inner.iter().any(|i| i.as_str() == l);
    let mut groups: BTreeMap<String, Vec<Hit>> = BTreeMap::new();
    for h in cx.holes().into_iter().filter(|h| h.plated && matches!(h.of, HoleOf::Pad(..))) {
        if let Some(hit) = crowding(cx, &h, need, false, &is_inner) {
            groups.entry(group_of(cx, &h)).or_default().push(hit);
        }
    }
    report_groups(groups, &|_| need, "another net's copper on an inner layer", r);
}

fn npth_to_copper(cx: &Ctx, r: &mut Report) {
    let need = cx.board.rules.min_npth_to_copper.to_mm();
    let mut groups: BTreeMap<String, Vec<Hit>> = BTreeMap::new();
    for h in cx.holes().into_iter().filter(|h| !h.plated) {
        if let Some(hit) = crowding(cx, &h, need, true, &|_| true) {
            groups.entry(group_of(cx, &h)).or_default().push(hit);
        }
    }
    report_groups(groups, &|_| need, "copper", r);
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

type Drill = (P, f64, String, Option<usize>, (usize, usize));

fn hole_to_hole(cx: &Ctx, r: &mut Report) {
    let all = (0, cx.copper.len().saturating_sub(1));
    let mut drills: Vec<Drill> = cx
        .vias
        .iter()
        .map(|v| {
            let name = format!("via at [{:.3}, {:.3}]", v.at[0], v.at[1]);
            (v.at, v.drill / 2.0, name, None, v.span_of(cx.copper).unwrap_or(all))
        })
        .collect();
    for (pi, p) in cx.parts.iter().enumerate() {
        for pad in &p.pads {
            if let Some((c, s, _)) = pad.drill {
                let name = format!("{}.{}", p.reference, pad.number);
                drills.push((c, s[0].min(s[1]) / 2.0, name, Some(pi), all));
            }
        }
    }
    let hole_gap = cx.board.rules.min_hole_to_hole.to_mm();
    let mut close = 0;
    let mut first = None;
    for i in 0..drills.len() {
        for j in i + 1..drills.len() {
            let (a, b) = (&drills[i], &drills[j]);
            if (a.3.is_some() && a.3 == b.3) || a.4.0.max(b.4.0) >= a.4.1.min(b.4.1) {
                continue;
            }
            let gap = geom::dist(a.0, b.0) - a.1 - b.1;
            if gap + 1e-6 < hole_gap && geom::dist(a.0, b.0) > 1e-6 {
                close += 1;
                first.get_or_insert(format!("{} is {} from {}", a.2, mm(gap), b.2));
            }
        }
    }
    if let Some(f) = first {
        r.emit("drills", format!("{close} drill pairs closer than {}, first: {f}", mm(hole_gap)));
    }
}

fn out_of_build_order(cx: &Ctx, a: &crate::layout::Via, b: &crate::layout::Via) -> Option<String> {
    let st = &cx.board.stackup;
    let last = cx.copper.len().checked_sub(1)?;
    let (sa, sb) = (a.span_of(cx.copper)?, b.span_of(cx.copper)?);
    let (upper, lower) = match (sa.1 == sb.0, sb.1 == sa.0) {
        (true, _) => ((a, sa), (b, sb)),
        (_, true) => ((b, sb), (a, sa)),
        _ => return None,
    };
    let (top, under) = match upper.1.0.cmp(&(last - lower.1.1)) {
        std::cmp::Ordering::Less => (upper.0, lower.0),
        std::cmp::Ordering::Greater => (lower.0, upper.0),
        std::cmp::Ordering::Equal => return None,
    };
    let order = |v: &crate::layout::Via| {
        st.drill_order(v.hole.first()?, v.hole.last()?, v.drill_kind, v.stacked)
    };
    let (top_steps, under_steps) = (order(top)?, order(under)?);
    (under_steps.1 > top_steps.0).then(|| {
        format!(
            "`{}` under `{}` is drilled at lamination step {}, after step {} of the via on top",
            under.name,
            top.name,
            under_steps.1 + 1,
            top_steps.0 + 1
        )
    })
}

fn stacked_via(cx: &Ctx, r: &mut Report) {
    let allowed = cx.board.rules.stacked_microvias;
    let (mut doubled, mut stacked, mut on_depth, mut unordered) = (0, 0, 0, 0);
    let (mut first_doubled, mut first_stacked, mut first_on_depth, mut first_unordered) =
        (None, None, None, None);
    for (i, a) in cx.vias.iter().enumerate() {
        let here = |b: &&crate::layout::Via| b.net == a.net && geom::dist(a.at, b.at) <= 1e-6;
        let spot = || format!("[{:.3}, {:.3}] ({})", a.at[0], a.at[1], cx.nets[a.net].name);
        let below: Vec<&crate::layout::Via> = cx.vias[..i].iter().filter(here).collect();
        let order = below.iter().find_map(|b| out_of_build_order(cx, a, b));
        if below.iter().any(|b| a.shares_dielectric(b, cx.copper)) {
            doubled += 1;
            first_doubled.get_or_insert_with(spot);
        } else if !below.is_empty()
            && std::iter::once(a)
                .chain(below.iter().copied())
                .any(|v| v.drill_kind == DrillKind::ControlledDepth)
        {
            on_depth += 1;
            first_on_depth.get_or_insert_with(spot);
        } else if let Some(o) = order {
            unordered += 1;
            first_unordered.get_or_insert_with(|| format!("{} ({o})", spot()));
        } else if !below.is_empty() && !allowed {
            stacked += 1;
            first_stacked.get_or_insert_with(spot);
        }
    }
    if let Some(f) = first_on_depth {
        r.emit(
            "vias",
            format!("{on_depth} vias are stacked with a controlled depth via, whose drill stops by depth on a plain pad and cannot land on or under another via: stagger them, first at {f}"),
        );
    }
    if let Some(f) = first_unordered {
        r.emit(
            "vias",
            format!("{unordered} vias are stacked out of the lamination's build order: the via under a stack must be drilled at the same or an earlier step than the one on top, stagger them or reorder [stackup] lamination, first at {f}"),
        );
    }
    if let Some(f) = first_doubled {
        r.emit(
            "vias",
            format!("{doubled} vias sit on another of their net at the same spot through the same layers, first at {f}"),
        );
    }
    if let Some(f) = first_stacked {
        r.emit(
            "vias",
            format!("{stacked} vias are stacked on another via, which the fab does not build (set [rules] stacked_microvias = true for an HDI fab, or stagger them), first at {f}"),
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
