use crate::tangle::{self, BUS_CROSS};
use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::EngineFile;
use agentee_core::geom::{self, P};
use agentee_core::graphic::Bounds;
use agentee_core::place;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

pub struct Layers;

#[derive(Clone, Debug, Default, Serialize)]
pub struct LayerGroup {
    pub name: String,
    pub nets: Vec<String>,
    pub layer: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct LayerPlan {
    pub groups: Vec<LayerGroup>,
    pub before: f64,
    pub after: f64,
}

impl LayerPlan {
    pub fn layer_of(&self, net: &str) -> Option<&str> {
        self.groups.iter().find(|g| g.nets.iter().any(|n| n == net)).map(|g| g.layer.as_str())
    }
}

struct Group {
    name: String,
    nets: Vec<usize>,
    bundle: bool,
    weight: f64,
    length: f64,
    cost: Vec<f64>,
}

const VIA: f64 = 0.5;

impl Phase for Layers {
    fn name(&self) -> &'static str {
        "layers"
    }

    fn run(
        &self,
        model: &mut Model,
        _cfg: &EngineFile,
        _field: &mut crate::field::CostField,
    ) -> PhaseReport {
        let mut report = PhaseReport { phase: "layers".into(), ..Default::default() };
        let l = &model.layout;
        let b = model.board;
        let planes = crate::plane_nets(&model.file);
        let user = l.engine.tangle.clone().map(|t| t.weights).unwrap_or_default();
        let tnets = tangle::nets_of(l, b, &planes, &user);
        let full: Vec<(&str, &String)> = model
            .file
            .zones
            .iter()
            .filter(|z| z.outline.as_ref().is_none_or(|o| o.is_empty()))
            .flat_map(|z| z.layers.iter().map(move |ly| (z.net.as_str(), ly)))
            .collect();
        let outer = |ly: &str| {
            l.copper.first().is_some_and(|c| c == ly) || l.copper.last().is_some_and(|c| c == ly)
        };
        let ground_layer =
            |ly: &str| !outer(ly) && full.iter().any(|(n, l2)| *l2 == ly && place::is_ground(n));
        let plane_layer = |ly: &str| !outer(ly) && full.iter().any(|(_, l2)| *l2 == ly);
        let layers: Vec<String> =
            l.copper.iter().filter(|c| !ground_layer(c) && !plane_layer(c)).cloned().collect();

        let mut key_of: BTreeMap<String, usize> = BTreeMap::new();
        let mut groups: Vec<Group> = Vec::new();
        let pins = tangle::pins_of(l);
        for (n, t) in tnets.iter().enumerate() {
            let Some(t) = t else { continue };
            if t.power || pins[n].len() < 2 {
                continue;
            }
            let key = match t.bundle {
                Some(bi) => format!("bus {}", l.interfaces[bi].name),
                None if t.unit != n => format!("pair {}", l.nets[t.unit].name),
                None => l.nets[n].name.clone(),
            };
            let gi = *key_of.entry(key.clone()).or_insert_with(|| {
                groups.push(Group {
                    name: key,
                    nets: Vec::new(),
                    bundle: t.bundle.is_some(),
                    weight: 0.0,
                    length: 0.0,
                    cost: vec![0.0; layers.len()],
                });
                groups.len() - 1
            });
            groups[gi].nets.push(n);
            groups[gi].weight = groups[gi].weight.max(t.weight);
        }

        let mut net_group: HashMap<usize, usize> = HashMap::new();
        for (gi, g) in groups.iter().enumerate() {
            for &n in &g.nets {
                net_group.insert(n, gi);
            }
        }
        let class_layers = |n: usize| -> Vec<String> {
            b.netclasses
                .iter()
                .find(|c| c.name == l.nets[n].class)
                .map(|c| c.layers.clone())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| l.copper.clone())
        };
        for g in groups.iter_mut() {
            for &n in &g.nets {
                let allowed = class_layers(n);
                let ends = ends_of(model, n);
                let span: f64 = tangle::mst(&pins[n]).iter().map(|(a, c)| geom::dist(*a, *c)).sum();
                for (k, ly) in layers.iter().enumerate() {
                    if !allowed.contains(ly) {
                        g.cost[k] = f64::INFINITY;
                        continue;
                    }
                    g.cost[k] += ends.iter().filter(|e| !e.contains(ly)).count() as f64 * VIA;
                }
                g.length += span;
            }
        }

        let mut links: Vec<(usize, P, P)> = Vec::new();
        for (gi, g) in groups.iter().enumerate() {
            for &n in &g.nets {
                for (a, c) in tangle::mst(&pins[n]) {
                    links.push((gi, a, c));
                }
            }
        }
        let mut pair_w: HashMap<(usize, usize), f64> = HashMap::new();
        for i in 0..links.len() {
            for j in i + 1..links.len() {
                let (gi, a, c) = links[i];
                let (gj, d, e) = links[j];
                if gi == gj {
                    continue;
                }
                let shared =
                    [a, c].iter().any(|p| geom::dist(*p, d) < 1e-6 || geom::dist(*p, e) < 1e-6);
                if shared || !geom::segments_intersect(a, c, d, e) {
                    continue;
                }
                let key = (gi.min(gj), gi.max(gj));
                let (x, y) = (&groups[key.0], &groups[key.1]);
                let w = pair_w.entry(key).or_insert(0.0);
                if x.bundle && y.bundle {
                    *w = BUS_CROSS;
                } else if x.bundle || y.bundle {
                    let lone = if x.bundle { y } else { x };
                    *w = (*w + lone.weight).min(lone.weight * lone.nets.len() as f64 * 3.0);
                } else {
                    *w += x.weight.min(y.weight);
                }
            }
        }
        let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); groups.len()];
        for (&(a, c), &w) in &pair_w {
            adj[a].push((c, w));
            adj[c].push((a, w));
        }

        let total: f64 = groups.iter().map(|g| g.length).sum::<f64>().max(1.0);
        let balance = 4.0 / total;
        let mut assign: Vec<usize> = groups
            .iter()
            .map(|g| {
                (0..layers.len()).min_by(|&a, &c| g.cost[a].total_cmp(&g.cost[c])).unwrap_or(0)
            })
            .collect();
        let same_layer = |assign: &[usize]| -> f64 {
            pair_w.iter().filter(|((a, c), _)| assign[*a] == assign[*c]).map(|(_, w)| w).sum()
        };
        let before = same_layer(&assign);
        let mut load = vec![0.0; layers.len()];
        for (gi, g) in groups.iter().enumerate() {
            load[assign[gi]] += g.length;
        }
        let mut order: Vec<usize> = (0..groups.len()).collect();
        order.sort_by(|&a, &c| adj[c].len().cmp(&adj[a].len()));
        for _ in 0..20 {
            let mut moved = false;
            for &gi in &order {
                let g = &groups[gi];
                let here = assign[gi];
                let cost_at = |k: usize, load: &[f64]| -> f64 {
                    let cross: f64 =
                        adj[gi].iter().filter(|(o, _)| assign[*o] == k).map(|(_, w)| w).sum();
                    let l_k = if k == here { load[k] } else { load[k] + g.length };
                    g.cost[k] + cross + balance * l_k * g.length
                };
                let now = cost_at(here, &load);
                let best = (0..layers.len())
                    .filter(|&k| g.cost[k].is_finite())
                    .min_by(|&a, &c| cost_at(a, &load).total_cmp(&cost_at(c, &load)))
                    .unwrap_or(here);
                if best != here && cost_at(best, &load) < now - 1e-9 {
                    load[here] -= g.length;
                    load[best] += g.length;
                    assign[gi] = best;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        let after = same_layer(&assign);
        let mut per: BTreeMap<&str, (usize, f64)> = BTreeMap::new();
        for (gi, g) in groups.iter().enumerate() {
            let e = per.entry(layers[assign[gi]].as_str()).or_default();
            e.0 += 1;
            e.1 += g.length;
        }
        report.notes.push(format!(
            "{} groups, same-layer crossings {before:.0} -> {after:.0}; {}",
            groups.len(),
            per.iter()
                .map(|(ly, (n, len))| format!("{ly} {n} groups {len:.0} mm"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for (gi, g) in groups.iter().enumerate().filter(|(_, g)| g.bundle) {
            report.notes.push(format!("{} on {}", g.name, layers[assign[gi]]));
        }
        let plan = LayerPlan {
            groups: groups
                .iter()
                .enumerate()
                .map(|(gi, g)| LayerGroup {
                    name: g.name.clone(),
                    nets: g.nets.iter().map(|&n| l.nets[n].name.clone()).collect(),
                    layer: layers[assign[gi]].clone(),
                })
                .collect(),
            before,
            after,
        };
        report.changed = true;
        model.layers = Some(plan);
        report
    }
}

fn ends_of(model: &Model, n: usize) -> Vec<Vec<String>> {
    let l = &model.layout;
    let mut out = Vec::new();
    for p in &l.parts {
        for pad in p.pads.iter().filter(|q| q.net == Some(n)) {
            let mut bb = Bounds::EMPTY;
            pad.outlines.iter().flatten().for_each(|q| bb.add(*q));
            let c = bb.center();
            let length = |t: &&agentee_core::layout::Track| -> f64 {
                t.points.windows(2).map(|w| geom::dist(w[0], w[1])).sum()
            };
            let escape = l
                .tracks
                .iter()
                .filter(|t| t.net == n && t.points.first().is_some_and(|f| geom::dist(*f, c) < 1.0))
                .max_by(|a, b| length(a).total_cmp(&length(b)));
            match escape {
                Some(t) => out.push(vec![t.layer.clone()]),
                None if crate::escape::is_bga(p) => out.push(l.copper.clone()),
                None => out.push(pad.copper.clone()),
            }
        }
    }
    out
}
