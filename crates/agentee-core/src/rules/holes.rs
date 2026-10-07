use super::{Context, Rule, Violation};
use crate::drc::{HoleOf, Owner};
use crate::geom;
use crate::units::Length;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Which {
    Plated,
    Inner,
    Npth,
}

pub struct HoleToCopper(pub Which);

impl HoleToCopper {
    fn need<C: Context>(&self, cx: &C, of: HoleOf) -> f64 {
        let r = &cx.board().rules;
        match (self.0, of) {
            (Which::Plated, HoleOf::Via(_)) => r.min_via_hole_to_copper.to_mm(),
            (Which::Plated, HoleOf::Pad(..)) => r.min_pth_hole_to_copper.to_mm(),
            (Which::Inner, _) => r.min_inner_pth_hole_to_copper.to_mm(),
            (Which::Npth, _) => r.min_npth_to_copper.to_mm(),
        }
    }

    fn takes(&self, plated: bool, of: HoleOf) -> bool {
        match self.0 {
            Which::Plated => plated,
            Which::Inner => plated && matches!(of, HoleOf::Pad(..)),
            Which::Npth => !plated,
        }
    }
}

pub fn group<C: Context>(cx: &C, i: usize) -> String {
    let h = cx.hole(i);
    match cx.part_of_hole(i) {
        Some(p) => format!("part {}", cx.parts()[p].reference),
        None => format!("vias {}", Length::mm(h.size[0])),
    }
}

impl Rule for HoleToCopper {
    fn id(&self) -> &'static str {
        match self.0 {
            Which::Plated => "hole-to-copper",
            Which::Inner => "inner-hole-to-copper",
            Which::Npth => "npth-to-copper",
        }
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        let copper = cx.copper();
        let inner =
            |l: &str| copper.len() >= 3 && copper[1..copper.len() - 1].iter().any(|c| c == l);
        let layer_ok = |l: &str| self.0 != Which::Inner || inner(l);
        let any_net = self.0 == Which::Npth;
        let r = &cx.board().rules;
        let reach = [
            r.min_via_hole_to_copper,
            r.min_pth_hole_to_copper,
            r.min_inner_pth_hole_to_copper,
            r.min_npth_to_copper,
        ]
        .iter()
        .map(|l| l.to_mm())
        .fold(0.0, f64::max);
        for i in cx.hole_subjects(reach) {
            let h = cx.hole(i);
            if !self.takes(h.plated, h.of) {
                continue;
            }
            let need = self.need(cx, h.of);
            let hb = h.bounds();
            let shares =
                |layers: &[String]| layers.iter().any(|l| h.layers.contains(l) && layer_ok(l));
            let other_net = |n: Option<usize>| any_net || n.is_none() || n != h.net;
            let mut worst: Option<(f64, String)> = None;
            for j in cx.items_near(&hb, need) {
                if !cx.counts(cx.planned_hole(i), cx.planned_item(j))
                    || cx.rigid(owner_of(h.of), cx.item(j).owner)
                {
                    continue;
                }
                let c = cx.item(j);
                let own = match (h.of, c.owner) {
                    (HoleOf::Via(a), Owner::Via(b)) => a == b,
                    (HoleOf::Pad(p, k), Owner::Pad(q, m)) => (p, k) == (q, m),
                    _ => false,
                };
                if own || !other_net(c.net) || !shares(&c.layers) {
                    continue;
                }
                let gap = h.gap_to(&c.shape);
                if gap + 1e-6 < need && worst.as_ref().is_none_or(|w| gap < w.0) {
                    worst = Some((gap, cx.describe(j)));
                }
            }
            if cx.counts(cx.planned_hole(i), false) {
                for (z, f) in cx.zones().iter().zip(cx.fills()) {
                    if !other_net(Some(z.net))
                        || !h.layers.contains(&z.layer)
                        || !layer_ok(&z.layer)
                        || !crate::drc::near(
                            &f.bounds,
                            hb.center(),
                            need + hb.size()[0].max(hb.size()[1]),
                        )
                    {
                        continue;
                    }
                    let gap = h.gap_to_fill(f, need);
                    if gap + 1e-6 < need && worst.as_ref().is_none_or(|w| gap < w.0) {
                        worst = Some((
                            gap,
                            format!("the {} pour on {}", cx.nets()[z.net].name, z.layer),
                        ));
                    }
                }
            }
            if let Some((gap, other)) = worst {
                out.push(Violation {
                    rule: self.id(),
                    group: group(cx, i),
                    subject: cx.hole_name(i),
                    other,
                    gap,
                    need,
                    at: h.a,
                    ..Default::default()
                });
            }
        }
    }
}

pub struct HoleToHole;

fn span<C: Context>(cx: &C, layers: &[String]) -> (usize, usize) {
    let copper = cx.copper();
    let at = |l: Option<&String>| l.and_then(|l| copper.iter().position(|c| c == l));
    match (at(layers.first()), at(layers.last())) {
        (Some(a), Some(b)) => (a.min(b), a.max(b)),
        _ => (0, copper.len().saturating_sub(1)),
    }
}

impl Rule for HoleToHole {
    fn id(&self) -> &'static str {
        "hole-to-hole"
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        let need = cx.board().rules.min_hole_to_hole.to_mm();
        let subjects = cx.hole_subjects(need);
        let chosen: std::collections::HashSet<usize> = subjects.iter().copied().collect();
        let name = |i: usize| {
            let h = cx.hole(i);
            match h.of {
                HoleOf::Via(_) => format!("via at [{:.3}, {:.3}]", h.a[0], h.a[1]),
                HoleOf::Pad(..) => cx.hole_name(i),
            }
        };
        for &i in &subjects {
            let a = cx.hole(i);
            let sa = span(cx, &a.layers);
            for j in cx.holes_near(&a.bounds(), need) {
                if j == i || (j < i && chosen.contains(&j)) {
                    continue;
                }
                if !cx.counts(cx.planned_hole(i), cx.planned_hole(j))
                    || cx.rigid(owner_of(a.of), owner_of(cx.hole(j).of))
                {
                    continue;
                }
                let b = cx.hole(j);
                let same_part =
                    cx.part_of_hole(i).is_some() && cx.part_of_hole(i) == cx.part_of_hole(j);
                let sb = span(cx, &b.layers);
                if same_part || sa.0.max(sb.0) >= sa.1.min(sb.1) {
                    continue;
                }
                let centres = geom::dist(a.a, b.a);
                let gap = centres - a.r - b.r;
                if gap + 1e-6 < need && centres > 1e-6 {
                    out.push(Violation {
                        rule: self.id(),
                        group: "drills".into(),
                        subject: name(i),
                        other: name(j),
                        gap,
                        need,
                        at: a.a,
                        ..Default::default()
                    });
                }
            }
        }
    }
}

impl super::zone::Constrains for HoleToCopper {
    fn constrain<C: Context>(
        &self,
        cx: &C,
        t: &super::zone::Template,
        zone: &mut super::zone::Zone,
    ) {
        let r = &cx.board().rules;
        let most = [
            r.min_via_hole_to_copper,
            r.min_pth_hole_to_copper,
            r.min_inner_pth_hole_to_copper,
            r.min_npth_to_copper,
        ]
        .iter()
        .map(|l| l.to_mm())
        .fold(0.0, f64::max);
        let window = zone.window();
        for k in super::keepout::holes(cx, Some(&window), most + t.half()) {
            if k.rule != self.id() {
                continue;
            }
            let shared: Vec<String> =
                k.layers.iter().filter(|l| t.layers.contains(l)).cloned().collect();
            if shared.is_empty() {
                continue;
            }
            let Some(need) = k.need(cx, t) else { continue };
            zone.forbid(Some(&shared), &k.shape, need + t.half());
        }
        let super::zone::Kind::Via { drill, ref hole, .. } = t.kind else { return };
        if self.0 != Which::Plated {
            return;
        }
        let need = r.min_via_hole_to_copper.to_mm();
        for j in cx.items_near(&window, need + drill / 2.0) {
            let c = cx.item(j);
            if t.owns(c.net) || !c.layers.iter().any(|l| hole.contains(l)) {
                continue;
            }
            zone.forbid(None, &c.shape, need + drill / 2.0);
        }
        for (z, f) in cx.zones().iter().zip(cx.fills()) {
            if t.owns(Some(z.net)) || !hole.contains(&z.layer) || z.rings.is_empty() {
                continue;
            }
            let _ = f;
            zone.forbid(None, &crate::drc::CuShape::Poly(z.rings.clone()), need + drill / 2.0);
        }
    }
}

impl super::zone::Constrains for HoleToHole {
    fn constrain<C: Context>(
        &self,
        cx: &C,
        t: &super::zone::Template,
        zone: &mut super::zone::Zone,
    ) {
        let super::zone::Kind::Via { drill, ref hole, .. } = t.kind else { return };
        let need = cx.board().rules.min_hole_to_hole.to_mm();
        let mine = span(cx, hole);
        let window = zone.window();
        for i in cx.holes_near(&window, need + drill) {
            let h = cx.hole(i);
            let theirs = span(cx, &h.layers);
            if mine.0.max(theirs.0) >= mine.1.min(theirs.1) {
                continue;
            }
            zone.forbid(None, &super::zone::hole_shape(h), need + drill / 2.0);
        }
    }
}

fn owner_of(h: HoleOf) -> Owner {
    match h {
        HoleOf::Via(v) => Owner::Via(v),
        HoleOf::Pad(p, k) => Owner::Pad(p, k),
    }
}
