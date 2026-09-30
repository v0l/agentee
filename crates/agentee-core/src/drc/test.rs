use super::{Category, Ctx, Report, Rule, Setup};
use crate::diag::Severity;
use crate::geom;
use crate::testpoint::{self as tp, TestSpec};
use crate::units::Length;

pub static RULES: &[Rule] = &[
    Rule {
        id: "test-access",
        category: Category::Test,
        severity: Severity::Info,
        summary: "nets the [test] section asks for (power, ground, reset, enable, clock and bus nets by default) with no test point pad, exposed through-hole pad or, when allowed, via to probe; impedance and pair nets are exempt",
        when: "placed parts",
        applies: with_parts,
        check: test_access,
    },
    Rule {
        id: "test-pad-geometry",
        category: Category::Test,
        severity: Severity::Info,
        summary: "test points smaller than min_test_pad, closer than min_test_pad_pitch to another, closer than min_test_pad_to_body to a part body, closer than min_test_pad_to_edge to the edge or a tooling hole, or off the probe side",
        when: "test points",
        applies: with_test_points,
        check: test_pad_geometry,
    },
];

fn with_parts(s: &Setup) -> bool {
    s.parts > 0
}

fn with_test_points(s: &Setup) -> bool {
    s.test_points
}

fn spec(cx: &Ctx) -> TestSpec {
    cx.test.cloned().unwrap_or_default()
}

fn test_access(cx: &Ctx, r: &mut Report) {
    let spec = spec(cx);
    let mut missing = Vec::new();
    let mut exempt = Vec::new();
    for (ni, net) in cx.nets.iter().enumerate() {
        if !tp::wanted(&spec, cx.board, net) {
            continue;
        }
        if !cx.parts.iter().any(|p| p.pads.iter().any(|q| q.net == Some(ni))) {
            continue;
        }
        if tp::has_access(&spec, cx.parts, cx.vias, ni) {
            continue;
        }
        match tp::exempt(cx.board, cx.pairs, ni, net) {
            Some(why) => exempt.push(format!("{} ({why})", net.name)),
            None => missing.push(net.name.clone()),
        }
    }
    if missing.is_empty() {
        return;
    }
    let skipped = if exempt.is_empty() {
        String::new()
    } else {
        format!("; exempt, stubs hurt them: {}", exempt.join(", "))
    };
    r.emit(
        "test",
        format!(
            "{} nets have no probe access from {}: {}; `agentee testpoints NAME --nets ...` adds test pads{skipped}",
            missing.len(),
            spec.side,
            missing.join(", ")
        ),
    );
}

fn test_pad_geometry(cx: &Ctx, r: &mut Report) {
    let spec = spec(cx);
    let cu = spec.copper();
    let mm = |v: f64| Length::mm((v * 1000.0).round() / 1000.0);
    let tooling = tp::tooling_holes(cx.parts);
    let mut pads = Vec::new();
    for (pi, p) in cx.parts.iter().enumerate().filter(|(_, p)| tp::is_test_point(p)) {
        for (k, q) in p.pads.iter().enumerate().filter(|(_, q)| !q.copper.is_empty()) {
            let c = tp::pad_center(q);
            let dia = tp::pad_diameter(q);
            let mut issues = Vec::new();
            if dia + 1e-6 < spec.min_test_pad {
                issues.push(format!(
                    "pad is {} across, under min_test_pad {}",
                    mm(dia),
                    mm(spec.min_test_pad)
                ));
            }
            if q.drill.is_none() && !q.copper.contains(&cu) {
                issues.push(format!("is not on the probe side {}", spec.side));
            }
            if cx.edge().is_closed() {
                let gap = if cx.edge().contains(c) {
                    cx.edge().distance(c) - dia / 2.0
                } else {
                    -dia / 2.0
                };
                if gap + 1e-6 < spec.min_test_pad_to_edge {
                    issues.push(format!(
                        "is {} from the board edge, under min_test_pad_to_edge {}",
                        mm(gap.max(0.0)),
                        mm(spec.min_test_pad_to_edge)
                    ));
                }
            }
            if let Some(g) = tooling
                .iter()
                .map(|(h, hr)| geom::dist(c, *h) - hr - dia / 2.0)
                .filter(|g| *g + 1e-6 < spec.min_test_pad_to_edge)
                .reduce(f64::min)
            {
                issues.push(format!(
                    "is {} from a tooling hole, under min_test_pad_to_edge {}",
                    mm(g.max(0.0)),
                    mm(spec.min_test_pad_to_edge)
                ));
            }
            let near: Vec<String> = cx
                .parts
                .iter()
                .enumerate()
                .filter(|(k, o)| {
                    *k != pi
                        && !tp::is_test_point(o)
                        && o.bottom == spec.bottom()
                        && tp::body_rect(o, &spec.side).is_some_and(|b| {
                            tp::gap_to_rect(c, dia / 2.0, &b) + 1e-6 < spec.min_test_pad_to_body
                        })
                })
                .map(|(_, o)| o.reference.clone())
                .collect();
            if !near.is_empty() {
                issues.push(format!(
                    "is closer than min_test_pad_to_body {} to {}",
                    mm(spec.min_test_pad_to_body),
                    super::list(&near)
                ));
            }
            if !issues.is_empty() {
                r.emit(
                    p.reference.clone(),
                    format!("{} {}", cx.pad_name(pi, k), issues.join(", ")),
                );
            }
            pads.push((pi, c, dia));
        }
    }
    for (i, a) in pads.iter().enumerate() {
        for b in &pads[i + 1..] {
            let d = geom::dist(a.1, b.1);
            if a.0 != b.0 && d + 1e-6 < spec.min_test_pad_pitch {
                let (ra, rb) = (&cx.parts[a.0].reference, &cx.parts[b.0].reference);
                r.emit(
                    format!("{ra}, {rb}"),
                    format!(
                        "{ra} and {rb} are {} apart centre to centre, under min_test_pad_pitch {}",
                        mm(d),
                        mm(spec.min_test_pad_pitch)
                    ),
                );
            }
        }
    }
}
