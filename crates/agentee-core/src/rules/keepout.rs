use super::Context;
use crate::drc::{CuShape, HoleOf, Owner};
use crate::graphic::Bounds;

#[derive(Clone, Debug)]
pub struct Keepout {
    pub rule: &'static str,
    pub owner: Owner,
    pub net: Option<usize>,
    pub every: bool,
    pub layers: Vec<String>,
    pub shape: CuShape,
    pub gap: f64,
    pub fixed: bool,
}

impl Keepout {
    pub fn need(&self, cx: &impl Context, t: &super::zone::Template) -> Option<f64> {
        if !self.every && t.owns(self.net) {
            return None;
        }
        if self.fixed {
            return Some(self.gap);
        }
        Some(self.gap.max(cx.spacing().class.of(t.net)))
    }
}

fn within(b: &Bounds, window: Option<&Bounds>, reach: f64) -> bool {
    window.is_none_or(|w| {
        !b.is_empty()
            && b.min[0] <= w.max[0] + reach
            && b.max[0] >= w.min[0] - reach
            && b.min[1] <= w.max[1] + reach
            && b.max[1] >= w.min[1] - reach
    })
}

pub fn copper<C: Context>(cx: &C, window: Option<&Bounds>, reach: f64) -> Vec<Keepout> {
    let min = cx.board().rules.min_clearance.to_mm();
    let class = &cx.spacing().class;
    let items: Vec<usize> = match window {
        Some(w) => cx.items_near(w, reach),
        None => cx.item_subjects(reach),
    };
    items
        .into_iter()
        .map(|i| cx.item(i))
        .filter(|c| within(&c.bounds, window, reach))
        .map(|c| {
            let footprint = match c.owner {
                Owner::Pad(p, _) => cx.parts().get(p).and_then(|p| p.footprint.clearance),
                _ => None,
            };
            Keepout {
                rule: "clearance",
                owner: c.owner,
                net: c.net,
                every: false,
                layers: c.layers.clone(),
                shape: c.shape.clone(),
                gap: footprint.map_or(class.of(c.net), |f| f.max(min)),
                fixed: footprint.is_some(),
            }
        })
        .collect()
}

pub fn holes<C: Context>(cx: &C, window: Option<&Bounds>, reach: f64) -> Vec<Keepout> {
    let r = &cx.board().rules;
    let copper = cx.copper();
    let inner: Vec<String> =
        if copper.len() >= 3 { copper[1..copper.len() - 1].to_vec() } else { Vec::new() };
    let all: Vec<usize> = match window {
        Some(w) => cx.holes_near(w, reach),
        None => (0..cx.hole_count()).collect(),
    };
    let mut out = Vec::new();
    for i in all {
        let h = cx.hole(i);
        let shape = super::zone::hole_shape(h);
        let push = |out: &mut Vec<Keepout>, rule, layers: Vec<String>, gap: f64, every| {
            if !layers.is_empty() {
                out.push(Keepout {
                    rule,
                    owner: match h.of {
                        HoleOf::Via(v) => Owner::Via(v),
                        HoleOf::Pad(p, k) => Owner::Pad(p, k),
                    },
                    net: h.net,
                    every,
                    layers,
                    shape: shape.clone(),
                    gap,
                    fixed: true,
                });
            }
        };
        if !h.plated {
            push(&mut out, "npth-to-copper", copper.to_vec(), r.min_npth_to_copper.to_mm(), true);
            continue;
        }
        let need = match h.of {
            HoleOf::Via(_) => r.min_via_hole_to_copper.to_mm(),
            HoleOf::Pad(..) => r.min_pth_hole_to_copper.to_mm(),
        };
        push(&mut out, "hole-to-copper", h.layers.clone(), need, false);
        if matches!(h.of, HoleOf::Pad(..)) {
            let on: Vec<String> = h.layers.iter().filter(|l| inner.contains(l)).cloned().collect();
            push(
                &mut out,
                "inner-hole-to-copper",
                on,
                r.min_inner_pth_hole_to_copper.to_mm(),
                false,
            );
        }
    }
    out
}

pub fn edge<C: Context>(cx: &C) -> Vec<Keepout> {
    let e = cx.edge();
    if !e.is_closed() {
        return Vec::new();
    }
    let gap = cx.board().rules.min_copper_to_edge.to_mm();
    e.segments()
        .map(|(a, b)| Keepout {
            rule: "copper-to-edge",
            owner: Owner::Copper(usize::MAX),
            net: None,
            every: true,
            layers: cx.copper().to_vec(),
            shape: CuShape::Seg(a, b, 0.0),
            gap,
            fixed: true,
        })
        .collect()
}

pub fn all<C: Context>(cx: &C, window: Option<&Bounds>, reach: f64) -> Vec<Keepout> {
    let mut out = copper(cx, window, reach);
    out.extend(holes(cx, window, reach));
    out.extend(edge(cx));
    out
}
