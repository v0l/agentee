use crate::field::CostField;
use agentee_core::board::Board;
use agentee_core::engine::{SCORE_TERMS, default_weight};
use agentee_core::geom::{self, P};
use agentee_core::graphic::Bounds;
use agentee_core::layout::Layout;
use agentee_core::place::{self, Role};
use agentee_core::schematic::Schematic;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Score {
    pub total: f64,
    pub terms: BTreeMap<String, Term>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Term {
    pub raw: f64,
    pub weight: f64,
    pub weighted: f64,
    pub measured: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub worst: Vec<(String, f64)>,
}

pub struct Weights(pub BTreeMap<String, f64>);

impl Weights {
    pub fn from_file(file: &BTreeMap<String, f64>) -> Weights {
        let mut w = BTreeMap::new();
        for t in SCORE_TERMS {
            w.insert(t.to_string(), file.get(*t).copied().unwrap_or_else(|| default_weight(t)));
        }
        Weights(w)
    }

    fn of(&self, term: &str) -> f64 {
        self.0.get(term).copied().unwrap_or(0.0)
    }
}

pub struct Context<'a> {
    pub board: &'a Board,
    pub schematic: &'a Schematic,
    pub layout: &'a Layout,
    pub field: Option<&'a CostField>,
    pub keepouts: &'a [Vec<P>],
    pub heat: &'a [(String, f64)],
    pub planes: &'a [String],
    pub tangle: &'a BTreeMap<String, f64>,
}

impl Score {
    pub fn measure(cx: &Context, weights: &Weights) -> Score {
        let mut s = Score::default();
        let mut put = |name: &str, raw: f64, measured: bool, worst: Vec<(String, f64)>| {
            let weight = weights.of(name);
            let mut worst = worst;
            worst.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            worst.truncate(5);
            s.terms.insert(
                name.to_string(),
                Term { raw, weight, weighted: raw * weight, measured, worst },
            );
        };
        let l = cx.layout;
        let b = cx.board;

        let class_weight = |class: &str| -> f64 {
            let c = b.netclasses.iter().find(|c| c.name == class);
            match c {
                Some(c) if c.current.is_some() => 0.0,
                Some(c) if c.diff_gap.is_some() || c.impedance.is_some() => 3.0,
                _ => 1.0,
            }
        };
        let mut wirelength = 0.0;
        let mut pins_of: Vec<Vec<P>> = vec![Vec::new(); l.nets.len()];
        for part in &l.parts {
            for pad in &part.pads {
                if let Some(n) = pad.net {
                    let mut bb = Bounds::EMPTY;
                    pad.outlines.iter().flatten().for_each(|q| bb.add(*q));
                    if !bb.is_empty() {
                        pins_of[n].push(bb.center());
                    }
                }
            }
        }
        for (n, pins) in pins_of.iter().enumerate() {
            if pins.len() < 2 || place::is_ground(&l.nets[n].name) {
                continue;
            }
            let mut bb = Bounds::EMPTY;
            pins.iter().for_each(|p| bb.add(*p));
            let [w, h] = bb.size();
            wirelength += (w + h) * class_weight(&l.nets[n].class);
        }
        put("wirelength", wirelength, true, Vec::new());

        let tangle = crate::tangle::Tangle::new(
            crate::tangle::nets_of(l, b, cx.planes, cx.tangle),
            &pins_of,
            &l.ratsnest,
        );
        put("crossings", tangle.cost(), true, tangle.worst());

        put(
            "unrouted",
            l.nets.iter().map(|n| n.unrouted as f64).sum(),
            true,
            l.nets
                .iter()
                .filter(|n| n.unrouted > 0)
                .map(|n| (n.name.clone(), n.unrouted as f64))
                .collect(),
        );

        let mut via_excess = 0.0;
        let mut vias_per_net = vec![0usize; l.nets.len()];
        for v in &l.vias {
            vias_per_net[v.net] += 1;
        }
        let mut via_worst = Vec::new();
        for (n, count) in vias_per_net.iter().enumerate() {
            if place::is_power_net(b, &l.nets[n].name, &l.nets[n].class) {
                continue;
            }
            let allowance = pins_of[n].len().max(2) - 1;
            let excess = count.saturating_sub(allowance) as f64;
            if excess > 0.0 {
                via_worst.push((l.nets[n].name.clone(), excess));
            }
            via_excess += excess;
        }
        put("vias", via_excess, true, via_worst);

        let courtyards: Vec<(usize, Vec<Vec<P>>)> = l
            .parts
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let layer = if p.bottom { "B.CrtYd" } else { "F.CrtYd" };
                let t = p.transform();
                let loops = place::courtyard_loops(&p.footprint, layer)
                    .into_iter()
                    .map(|l| l.into_iter().map(|q| t.apply(q)).collect())
                    .collect();
                (i, loops)
            })
            .collect();
        let bbox = |loops: &[Vec<P>]| {
            let mut bb = Bounds::EMPTY;
            loops.iter().flatten().for_each(|q| bb.add(*q));
            bb
        };
        let mut overlap = 0.0;
        let mut overlap_worst = Vec::new();
        for i in 0..courtyards.len() {
            for j in i + 1..courtyards.len() {
                if l.parts[i].bottom != l.parts[j].bottom {
                    continue;
                }
                let (a, bb) = (bbox(&courtyards[i].1), bbox(&courtyards[j].1));
                if a.is_empty() || bb.is_empty() || !a.overlaps(&bb) {
                    continue;
                }
                let mut here = 0.0;
                for la in &courtyards[i].1 {
                    for lb in &courtyards[j].1 {
                        here += geom::intersection_area(la, lb);
                    }
                }
                if here > 0.0 {
                    overlap_worst
                        .push((format!("{} {}", l.parts[i].reference, l.parts[j].reference), here));
                    overlap += here;
                }
            }
        }
        put("overlap", overlap, true, overlap_worst);

        let edge = geom::BoardEdge::new(&l.outline, &l.board_cutouts);
        let body_edge = b.rules.min_body_to_edge.to_mm();
        let mut edge_term = 0.0;
        let mut keepout = 0.0;
        let mut edge_worst = Vec::new();
        let mut keepout_worst = Vec::new();
        for (i, loops) in &courtyards {
            let p = &l.parts[*i];
            let role = place::role_of(&p.reference, &p.footprint_name, &p.footprint);
            if matches!(role, Role::Connector | Role::Hole) {
                continue;
            }
            let (mut e, mut k_here) = (0.0, 0.0);
            for lp in loops {
                let d = edge.polygon_distance(lp);
                if d < body_edge {
                    e += body_edge - d;
                }
                for k in cx.keepouts {
                    k_here += geom::intersection_area(lp, k);
                }
            }
            if e > 0.0 {
                edge_worst.push((p.reference.clone(), e));
            }
            if k_here > 0.0 {
                keepout_worst.push((p.reference.clone(), k_here));
            }
            edge_term += e;
            keepout += k_here;
        }
        put("edge", edge_term, true, edge_worst);
        put("keepout", keepout, true, keepout_worst);

        let pd = b.drc.placement.clone().unwrap_or_default();
        let decap_limit =
            pd.decoupling_distance.map(|l| l.to_mm()).unwrap_or(place::DECOUPLING_DISTANCE);
        let hot_limit = pd.hot_distance.map(|l| l.to_mm()).unwrap_or(place::HOT_DISTANCE);
        let chips: Vec<usize> = (0..l.parts.len())
            .filter(|&i| {
                place::role_of(
                    &l.parts[i].reference,
                    &l.parts[i].footprint_name,
                    &l.parts[i].footprint,
                ) == Role::Chip
            })
            .collect();
        let mut decap = 0.0;
        let mut decap_worst = Vec::new();
        for (i, p) in l.parts.iter().enumerate() {
            if !place::is_capacitor(&p.reference, &p.footprint_name) {
                continue;
            }
            let nets: Vec<usize> = p.pads.iter().filter_map(|q| q.net).collect();
            let supply = nets.iter().find(|&&n| {
                place::is_power_net(b, &l.nets[n].name, &l.nets[n].class)
                    && !place::is_ground(&l.nets[n].name)
            });
            let Some(&supply) = supply else { continue };
            let mine = p.at.to_mm();
            let nearest = chips
                .iter()
                .filter(|&&c| c != i)
                .flat_map(|&c| {
                    l.parts[c].pads.iter().filter(move |q| q.net == Some(supply)).map(|q| {
                        let mut bb = Bounds::EMPTY;
                        q.outlines.iter().flatten().for_each(|r| bb.add(*r));
                        geom::dist(mine, bb.center())
                    })
                })
                .fold(f64::MAX, f64::min);
            if nearest < f64::MAX && nearest > decap_limit {
                decap += nearest - decap_limit;
                decap_worst.push((p.reference.clone(), nearest - decap_limit));
            }
        }
        put("decap", decap, true, decap_worst);

        let hot: Vec<P> = l
            .parts
            .iter()
            .filter(|p| cx.heat.iter().any(|(r, w)| *r == p.reference && *w >= 0.25))
            .map(|p| p.at.to_mm())
            .collect();
        let mut hot_term = 0.0;
        for i in 0..hot.len() {
            for j in i + 1..hot.len() {
                let d = geom::dist(hot[i], hot[j]);
                if d < hot_limit {
                    hot_term += hot_limit - d;
                }
            }
        }
        put("hot", hot_term, true, Vec::new());

        let flex = b.rules.flex_zone.to_mm();
        let mut flex_term = 0.0;
        let mut flex_worst = Vec::new();
        for p in &l.parts {
            if !place::is_capacitor(&p.reference, &p.footprint_name) || p.mlcc == Some(false) {
                continue;
            }
            let big =
                place::chip_length(&p.footprint_name, &p.footprint).is_some_and(|len| len >= 2.0);
            if big && edge.distance(p.at.to_mm()) < flex {
                flex_term += 1.0;
                flex_worst.push((p.reference.clone(), 1.0));
            }
        }
        put("flex", flex_term, true, flex_worst);

        let mut area = 0.0;
        if l.outline.len() >= 3 {
            area = geom::signed_area(&l.outline).abs();
        }
        put("area", area, true, Vec::new());
        put("layers", l.copper.len() as f64, true, Vec::new());

        match cx.field {
            Some(f) => put("overflow", f.overflow(), true, Vec::new()),
            None => put("overflow", 0.0, false, Vec::new()),
        }
        for t in [
            "chain_order",
            "chain_layer",
            "return_path",
            "pair_coupling",
            "pin_access",
            "via_site",
            "switcher",
            "plane_reach",
        ] {
            put(t, 0.0, false, Vec::new());
        }

        s.total = s.terms.values().map(|t| t.weighted).sum();
        s
    }

    pub fn table(&self) -> String {
        let mut out = String::new();
        let mut rows: Vec<(&String, &Term)> = self.terms.iter().collect();
        rows.sort_by(|a, b| {
            b.1.weighted.partial_cmp(&a.1.weighted).unwrap_or(std::cmp::Ordering::Equal)
        });
        for (name, t) in rows {
            let flag = if t.measured { "" } else { "  (not measured yet)" };
            out.push_str(&format!(
                "{:14} {:>10.2} x {:>6.2} = {:>10.2}{flag}\n",
                name, t.raw, t.weight, t.weighted
            ));
            if !t.worst.is_empty() {
                let list: Vec<String> =
                    t.worst.iter().map(|(n, v)| format!("{n} {v:.2}")).collect();
                out.push_str(&format!("{:14} {}\n", "", list.join(", ")));
            }
        }
        out.push_str(&format!("{:14} {:>32.2}\n", "total", self.total));
        out
    }
}
