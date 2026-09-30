use super::{Category, Ctx, Report, Rule, Setup, edge_distance, rings_bounds};
use crate::diag::Severity;
use crate::geom::{self, P};
use crate::graphic::Bounds;
use crate::layout::Placed;
use crate::place::{self, Role};

pub static RULES: &[Rule] = &[
    Rule {
        id: "placement-decoupling-distance",
        category: Category::Placement,
        severity: Severity::Info,
        summary: "a decoupling capacitor (between a supply and ground) whose supply pad is farther than decoupling_distance from the nearest supply pin of an IC on that net; 2.5 times that for bulk capacitors over 1 uF",
        when: "placed parts",
        applies: with_parts,
        check: decoupling_distance,
    },
    Rule {
        id: "placement-crystal-distance",
        category: Category::Placement,
        severity: Severity::Info,
        summary: "a crystal or oscillator whose signal pad is farther than crystal_distance from the IC pin it drives",
        when: "placed parts",
        applies: with_parts,
        check: crystal_distance,
    },
    Rule {
        id: "placement-large-part-off-centre",
        category: Category::Placement,
        severity: Severity::Info,
        summary: "a large chip (BGA, a 16+ pin package over 25 mm2, else the highest pin count part) whose centre is more than off_centre of the way from the board centre to the edge",
        when: "placed parts",
        applies: with_parts,
        check: large_part_off_centre,
    },
    Rule {
        id: "placement-hot-parts-close",
        category: Category::Placement,
        severity: Severity::Info,
        summary: "two hot parts whose courtyards are closer than hot_distance, so their heat adds up: the [[sources]] of this layout's thermal sims at 0.25 W or more, and large packages (courtyard 49 mm2 or more) those sims do not list",
        when: "placed parts",
        applies: with_parts,
        check: hot_parts_close,
    },
    Rule {
        id: "placement-cluster-spread",
        category: Category::Placement,
        severity: Severity::Info,
        summary: "a passive whose signal nets reach only one IC, farther than cluster_spread from that IC's pin",
        when: "placed parts",
        applies: with_parts,
        check: cluster_spread,
    },
    Rule {
        id: "placement-connector-not-at-edge",
        category: Category::Placement,
        severity: Severity::Info,
        summary: "a connector whose body is farther than connector_edge from the board edge",
        when: "placed parts",
        applies: with_parts,
        check: connector_not_at_edge,
    },
];

fn with_parts(s: &Setup) -> bool {
    s.parts > 0
}

struct Limits {
    decap: f64,
    crystal: f64,
    off_centre: f64,
    hot: f64,
    spread: f64,
    edge: f64,
}

fn limits(cx: &Ctx) -> Limits {
    let f = cx.board.drc.placement.clone().unwrap_or_default();
    let mm = |v: Option<crate::units::Length>, d: f64| v.map(|l| l.to_mm()).unwrap_or(d);
    Limits {
        decap: mm(f.decoupling_distance, place::DECOUPLING_DISTANCE),
        crystal: mm(f.crystal_distance, place::CRYSTAL_DISTANCE),
        off_centre: f.off_centre.unwrap_or(place::OFF_CENTRE),
        hot: mm(f.hot_distance, place::HOT_DISTANCE),
        spread: mm(f.cluster_spread, place::CLUSTER_SPREAD),
        edge: mm(f.connector_edge, place::CONNECTOR_EDGE),
    }
}

fn mm(v: f64) -> String {
    format!("{v:.2} mm")
}

fn role(p: &Placed) -> Role {
    place::role_of(&p.reference, &p.footprint_name, &p.footprint)
}

fn pad_centre(p: &Placed, k: usize) -> P {
    rings_bounds(&p.pads[k].outlines).center()
}

fn power(cx: &Ctx, n: usize) -> bool {
    let net = &cx.nets[n];
    place::is_power_net(cx.board, &net.name, &net.class)
}

fn courtyard(p: &Placed) -> Vec<Vec<P>> {
    let t = p.transform();
    let mut out: Vec<Vec<P>> = ["F.CrtYd", "B.CrtYd"]
        .iter()
        .flat_map(|l| place::courtyard_loops(&p.footprint, l))
        .map(|l| l.into_iter().map(|q| t.apply(q)).collect())
        .collect();
    if out.is_empty() {
        out = p.pads.iter().flat_map(|q| q.outlines.clone()).collect();
    }
    out
}

fn pins_on(cx: &Ctx, net: usize, skip: usize, keep: impl Fn(Role) -> bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (i, p) in cx.parts.iter().enumerate() {
        if i == skip || !keep(role(p)) {
            continue;
        }
        for (k, q) in p.pads.iter().enumerate() {
            if q.net == Some(net) && !q.outlines.is_empty() {
                out.push((i, k));
            }
        }
    }
    out
}

fn nearest(cx: &Ctx, from: P, pins: &[(usize, usize)]) -> Option<(f64, usize, usize)> {
    pins.iter()
        .map(|&(i, k)| (geom::dist(from, pad_centre(&cx.parts[i], k)), i, k))
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

fn decoupling_distance(cx: &Ctx, r: &mut Report) {
    let lim = limits(cx);
    for (i, p) in cx.parts.iter().enumerate() {
        if role(p) != Role::Passive || !place::is_capacitor(&p.reference, &p.footprint_name) {
            continue;
        }
        let nets: Vec<usize> = p.pads.iter().filter_map(|q| q.net).collect();
        if nets.is_empty() || !nets.iter().all(|n| power(cx, *n)) {
            continue;
        }
        if !nets.iter().any(|n| place::is_ground(&cx.nets[*n].name)) {
            continue;
        }
        let Some(k) = p.pads.iter().position(|q| {
            q.net.is_some_and(|n| !place::is_ground(&cx.nets[n].name)) && !q.outlines.is_empty()
        }) else {
            continue;
        };
        let rail = p.pads[k].net.unwrap_or(0);
        let pins = pins_on(cx, rail, i, |r| r == Role::Chip);
        let Some((d, j, pin)) = nearest(cx, pad_centre(p, k), &pins) else { continue };
        let bulk = place::cap_farads(&p.value).is_some_and(|f| f > 1.1e-6);
        let limit = if bulk { lim.decap * 2.5 } else { lim.decap };
        if d > limit + 1e-6 {
            r.emit(
                format!("part {}", p.reference),
                format!(
                    "{} decouples {} from {} away, its nearest supply pin is {}; keep it within {} of the pin, beside it or under it on the back",
                    p.reference,
                    cx.nets[rail].name,
                    mm(d),
                    cx.pad_name(j, pin),
                    mm(limit)
                ),
            );
        }
    }
}

fn crystal_distance(cx: &Ctx, r: &mut Report) {
    let lim = limits(cx);
    for (i, p) in cx.parts.iter().enumerate() {
        if role(p) != Role::Crystal {
            continue;
        }
        let mut worst: Option<(f64, usize, usize, usize)> = None;
        for (k, q) in p.pads.iter().enumerate() {
            let Some(n) = q.net else { continue };
            if power(cx, n) {
                continue;
            }
            let pins = pins_on(cx, n, i, |r| r == Role::Chip);
            if let Some((d, j, pin)) = nearest(cx, pad_centre(p, k), &pins)
                && worst.is_none_or(|w| d > w.0)
            {
                worst = Some((d, j, pin, n));
            }
        }
        if let Some((d, j, pin, n)) = worst
            && d > lim.crystal + 1e-6
        {
            r.emit(
                format!("part {}", p.reference),
                format!(
                    "{} is {} from {} on {}; keep a crystal or oscillator within {} of its clock pin so the trace stays short and quiet",
                    p.reference,
                    mm(d),
                    cx.pad_name(j, pin),
                    cx.nets[n].name,
                    mm(lim.crystal)
                ),
            );
        }
    }
}

fn is_large(p: &Placed) -> bool {
    let b = rings_bounds(&courtyard(p));
    let s = b.size();
    p.footprint_name.to_ascii_lowercase().contains("bga")
        || (place::copper_pad_numbers(&p.footprint) >= 16 && s[0] * s[1] >= 25.0)
}

fn large_parts(cx: &Ctx) -> Vec<usize> {
    let mut v: Vec<usize> = (0..cx.parts.len())
        .filter(|i| role(&cx.parts[*i]) == Role::Chip && is_large(&cx.parts[*i]))
        .collect();
    if v.is_empty() {
        let most = cx
            .parts
            .iter()
            .filter(|p| role(p) == Role::Chip)
            .map(|p| place::copper_pad_numbers(&p.footprint))
            .max()
            .unwrap_or(0);
        if most >= 8 {
            v = (0..cx.parts.len())
                .filter(|i| {
                    let p = &cx.parts[*i];
                    role(p) == Role::Chip && place::copper_pad_numbers(&p.footprint) == most
                })
                .collect();
        }
    }
    v
}

fn board_bounds(cx: &Ctx) -> Bounds {
    let mut b = Bounds::EMPTY;
    cx.outline.iter().for_each(|q| b.add(*q));
    b
}

fn large_part_off_centre(cx: &Ctx, r: &mut Report) {
    if cx.outline.len() < 3 {
        return;
    }
    let lim = limits(cx);
    let bb = board_bounds(cx);
    let (c, s) = (bb.center(), bb.size());
    for i in large_parts(cx) {
        let p = &cx.parts[i];
        let at = rings_bounds(&courtyard(p)).center();
        let off = ((at[0] - c[0]).abs() / (s[0] / 2.0)).max((at[1] - c[1]).abs() / (s[1] / 2.0));
        if off > lim.off_centre + 1e-9 {
            r.emit(
                format!("part {}", p.reference),
                format!(
                    "{} sits {:.0}% of the way from the board centre to the edge; large chips escape their pins, spread heat and stay clear of board flex best near the centre (off_centre {:.0}%)",
                    p.reference,
                    off * 100.0,
                    lim.off_centre * 100.0
                ),
            );
        }
    }
}

fn box_gap(a: &Bounds, b: &Bounds) -> f64 {
    let dx = (a.min[0] - b.max[0]).max(b.min[0] - a.max[0]).max(0.0);
    let dy = (a.min[1] - b.max[1]).max(b.min[1] - a.max[1]).max(0.0);
    dx.hypot(dy)
}

fn hot_parts_close(cx: &Ctx, r: &mut Report) {
    let lim = limits(cx);
    let hot: Vec<(usize, Bounds)> = cx
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| (i, p, rings_bounds(&courtyard(p))))
        .filter(|(_, p, b)| match cx.heat.iter().find(|h| h.0 == p.reference) {
            Some(h) => h.1 >= place::HOT_WATTS,
            None => role(p) == Role::Chip && is_large(p) && b.size()[0] * b.size()[1] >= 49.0,
        })
        .map(|(i, _, b)| (i, b))
        .collect();
    for (x, (i, a)) in hot.iter().enumerate() {
        for (j, b) in &hot[x + 1..] {
            let gap = box_gap(a, b);
            if gap < lim.hot - 1e-6 {
                r.emit(
                    format!("part {}", cx.parts[*i].reference),
                    format!(
                        "{} and {} are {} apart, closer than {}; spread large, hot packages so their heat does not stack",
                        cx.parts[*i].reference,
                        cx.parts[*j].reference,
                        mm(gap),
                        mm(lim.hot)
                    ),
                );
            }
        }
    }
}

fn cluster_spread(cx: &Ctx, r: &mut Report) {
    let lim = limits(cx);
    for (i, p) in cx.parts.iter().enumerate() {
        if role(p) != Role::Passive {
            continue;
        }
        let nets: Vec<usize> =
            p.pads.iter().filter_map(|q| q.net).filter(|n| !power(cx, *n)).collect();
        if nets.is_empty() {
            continue;
        }
        let mut others: Vec<usize> = Vec::new();
        let mut pins: Vec<(usize, usize)> = Vec::new();
        for n in &nets {
            for (j, k) in pins_on(cx, *n, i, |r| matches!(r, Role::Chip | Role::Connector)) {
                if !others.contains(&j) {
                    others.push(j);
                }
                pins.push((j, k));
            }
        }
        if others.len() != 1 || role(&cx.parts[others[0]]) != Role::Chip {
            continue;
        }
        let from = rings_bounds(&courtyard(p)).center();
        let Some((d, j, pin)) = nearest(cx, from, &pins) else { continue };
        if d > lim.spread + 1e-6 {
            r.emit(
                format!("part {}", p.reference),
                format!(
                    "{} connects only to {} but sits {} from {}; keep it within {} of the pin it serves",
                    p.reference,
                    cx.parts[j].reference,
                    mm(d),
                    cx.pad_name(j, pin),
                    mm(lim.spread)
                ),
            );
        }
    }
}

fn connector_not_at_edge(cx: &Ctx, r: &mut Report) {
    if cx.outline.len() < 3 {
        return;
    }
    let lim = limits(cx);
    for p in cx.parts.iter().filter(|p| role(p) == Role::Connector) {
        let rings = courtyard(p);
        let pts: Vec<P> = rings.iter().flatten().copied().collect();
        if pts.iter().any(|q| !geom::point_in_polygon(*q, cx.outline)) {
            continue;
        }
        let gap = pts.iter().map(|q| edge_distance(cx.outline, *q)).fold(f64::MAX, f64::min);
        if gap > lim.edge + 1e-6 {
            r.emit(
                format!("part {}", p.reference),
                format!(
                    "connector {} is {} in from the board edge; put connectors on the edge, within {}, so cables and mating parts clear the board",
                    p.reference,
                    mm(gap),
                    mm(lim.edge)
                ),
            );
        }
    }
}
