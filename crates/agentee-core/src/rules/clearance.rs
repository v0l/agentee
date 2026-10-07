use super::{Layer, Spacing};
use crate::board::Board;
use crate::layout::LayoutNet;

#[derive(Clone, Default)]
pub struct ClassClearance {
    pub of: Vec<f64>,
    pub unconnected: f64,
    widest: f64,
}

impl ClassClearance {
    pub fn new(board: &Board, nets: &[LayoutNet]) -> ClassClearance {
        let of: Vec<f64> = nets.iter().map(|n| n.clearance).collect();
        let unconnected = board
            .netclass("Default")
            .map(|c| c.clearance.to_mm())
            .unwrap_or(board.rules.min_clearance.to_mm());
        let widest = of.iter().copied().fold(unconnected, f64::max);
        ClassClearance { of, unconnected, widest }
    }

    pub fn of(&self, net: Option<usize>) -> f64 {
        match net {
            Some(n) => self.of.get(n).copied().unwrap_or(self.unconnected),
            None => self.unconnected,
        }
    }
}

impl Spacing for ClassClearance {
    fn rule(&self) -> &'static str {
        "clearance"
    }

    fn gap(&self, a: Option<usize>, b: Option<usize>, _: Layer) -> f64 {
        if a.is_some() && a == b {
            return 0.0;
        }
        self.of(a).max(self.of(b))
    }

    fn reach(&self, net: Option<usize>) -> f64 {
        self.of(net).max(self.widest)
    }
}

pub struct NetClearance;

pub fn reach<C: super::Context>(cx: &C) -> f64 {
    let min_clearance = cx.board().rules.min_clearance.to_mm();
    cx.spacing()
        .class
        .reach(None)
        .max(cx.parts().iter().filter_map(|p| p.footprint.clearance).fold(min_clearance, f64::max))
}

pub fn need<C: super::Context>(
    cx: &C,
    a: (crate::drc::Owner, Option<usize>),
    b: &crate::drc::Cu,
) -> Option<f64> {
    use crate::drc::Owner;
    let (owner, net) = a;
    if net.is_some() && net == b.net {
        return None;
    }
    let board = cx.board();
    let parts = cx.parts();
    let spacing = cx.spacing();
    let min_clearance = board.rules.min_clearance.to_mm();
    let footprint = |o: Owner| match o {
        Owner::Pad(p, _) => parts.get(p).and_then(|p| p.footprint.clearance),
        _ => None,
    };
    let tied = |o: Owner, other: Option<usize>| {
        let Owner::Pad(pi, k) = o else { return false };
        let Some(p) = parts.get(pi) else { return false };
        let Some(group) = p.footprint.net_tie_group(&p.pads[k].number) else { return false };
        other.is_some() && p.pads.iter().any(|q| q.net == other && group.contains(&q.number))
    };
    if tied(owner, b.net) || tied(b.owner, net) {
        return None;
    }
    let same_part = matches!((owner, b.owner), (Owner::Pad(p, _), Owner::Pad(q, _)) if p == q);
    if let (true, Owner::Pad(p, k1), Owner::Pad(_, k2)) = (same_part, owner, b.owner) {
        let n = |k: usize| parts[p].pads[k].number.as_str();
        if parts[p].footprint.spark_gap(n(k1), n(k2)).is_some() {
            return None;
        }
    }
    let explicit = |n: usize| {
        let net = &cx.nets()[n];
        board.domain_of(&net.name, &net.class).first().is_some_and(|&d| !board.domains[d].implicit)
    };
    Some(match (footprint(owner), footprint(b.owner)) {
        (None, None) if same_part => [net, b.net]
            .into_iter()
            .flatten()
            .filter(|&n| explicit(n))
            .map(|n| cx.nets()[n].clearance)
            .fold(min_clearance, f64::max),
        (None, None) => spacing.class.gap(net, b.net, spacing.layer(0)),
        (x, y) => x.unwrap_or(0.0).max(y.unwrap_or(0.0)).max(min_clearance),
    })
}

impl super::Rule for NetClearance {
    fn id(&self) -> &'static str {
        "clearance"
    }

    fn eval<C: super::Context>(&self, cx: &C, out: &mut Vec<super::Violation>) {
        let reach = reach(cx);
        let subjects = cx.item_subjects(reach);
        let chosen: std::collections::HashSet<usize> = subjects.iter().copied().collect();
        for &i in &subjects {
            let a = cx.item(i);
            for j in cx.items_near(&a.bounds, reach) {
                if j == i || (j < i && chosen.contains(&j)) {
                    continue;
                }
                if !cx.counts(cx.planned_item(i), cx.planned_item(j)) {
                    continue;
                }
                let b = cx.item(j);
                let (a, b, i, j) = if j < i { (b, a, j, i) } else { (a, b, i, j) };
                if !a.layers.iter().any(|l| b.layers.contains(l)) {
                    continue;
                }
                let Some(need) = need(cx, (a.owner, a.net), b) else { continue };
                let dist = a.shape.distance(&b.shape);
                if dist > 1e-6 && dist + crate::layout::DRC_EPSILON >= need {
                    continue;
                }
                let at = match a.shape {
                    crate::drc::CuShape::Seg(p, _, _) | crate::drc::CuShape::Circle(p, _) => p,
                    crate::drc::CuShape::Poly(ref v) => {
                        v.first().and_then(|r| r.first()).copied().unwrap_or([0.0, 0.0])
                    }
                };
                out.push(super::Violation {
                    rule: if dist <= 1e-6 { "short" } else { "clearance" },
                    group: if dist <= 1e-6 { "short" } else { "clearance" }.into(),
                    subject: cx.describe(i),
                    other: cx.describe(j),
                    gap: dist.max(0.0),
                    need,
                    at,
                    ..Default::default()
                });
            }
        }
    }
}

impl super::zone::Constrains for NetClearance {
    fn constrain<C: super::Context>(
        &self,
        cx: &C,
        t: &super::zone::Template,
        zone: &mut super::zone::Zone,
    ) {
        let window = zone.window();
        for k in super::keepout::copper(cx, Some(&window), reach(cx) + t.half()) {
            let shared: Vec<String> =
                k.layers.iter().filter(|l| t.layers.contains(l)).cloned().collect();
            if shared.is_empty() {
                continue;
            }
            let Some(need) = k.need(cx, t) else { continue };
            zone.forbid(Some(&shared), &k.shape, need + t.half());
        }
    }
}
