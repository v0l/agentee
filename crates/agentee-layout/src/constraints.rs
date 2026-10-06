use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::EngineFile;
use agentee_core::geom::P;
use agentee_core::graphic::Bounds;
use agentee_core::layout::Layout;
use agentee_core::place::{self, Role};
use serde::Serialize;

pub struct Constraints;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Groups {
    pub chains: Vec<Chain>,
    pub decaps: Vec<Decap>,
    pub clocks: Vec<Clock>,
}

const RF_DEPTH: usize = 16;
const RF_BRANCHES: usize = 3;

#[derive(Clone, Debug, Serialize)]
pub struct Chain {
    pub from: String,
    pub parts: Vec<String>,
    pub to: Option<String>,
    pub rf: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Decap {
    pub cap: String,
    pub ic: String,
    pub pin: String,
    pub net: String,
    pub farads: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Clock {
    pub part: String,
    pub ic: String,
    pub nets: Vec<String>,
}

fn role(p: &agentee_core::layout::Placed) -> Role {
    place::role_of(&p.reference, &p.footprint_name, &p.footprint)
}

pub fn pad_centre(pad: &agentee_core::layout::PlacedPad) -> P {
    let mut b = Bounds::EMPTY;
    pad.outlines.iter().flatten().for_each(|q| b.add(*q));
    b.center()
}

struct View<'a> {
    layout: &'a Layout,
    signal: Vec<bool>,
    on_net: Vec<Vec<usize>>,
}

impl View<'_> {
    fn signal_nets(&self, part: usize) -> Vec<usize> {
        let mut v: Vec<usize> = self.layout.parts[part]
            .pads
            .iter()
            .filter_map(|q| q.net)
            .filter(|&n| self.signal[n])
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

pub fn find(model: &Model) -> Groups {
    let l = &model.layout;
    let b = model.board;
    let signal: Vec<bool> =
        l.nets.iter().map(|n| !place::is_power_net(b, &n.name, &n.class)).collect();
    let mut on_net: Vec<Vec<usize>> = vec![Vec::new(); l.nets.len()];
    for (pi, p) in l.parts.iter().enumerate() {
        for q in &p.pads {
            if let Some(n) = q.net
                && !on_net[n].contains(&pi)
            {
                on_net[n].push(pi);
            }
        }
    }
    let v = View { layout: l, signal, on_net };
    let mut groups = Groups { chains: chains(&v, model), ..Default::default() };
    groups.decaps = decaps(model);
    groups.clocks = clocks(&v);
    groups
}

fn chains(v: &View, model: &Model) -> Vec<Chain> {
    let l = v.layout;
    let mut used: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    for (ci, c) in l.parts.iter().enumerate() {
        if role(c) != Role::Connector {
            continue;
        }
        for start in v.signal_nets(ci) {
            let rf = place::is_rf_class(model.board, &l.nets[start].class);
            if rf {
                let (parts, to) = rf_path(v, model, ci, start, &used);
                if !parts.is_empty() {
                    used.extend(&parts);
                    out.push(Chain {
                        from: c.reference.clone(),
                        parts: parts.iter().map(|&k| l.parts[k].reference.clone()).collect(),
                        to: to.map(|k| l.parts[k].reference.clone()),
                        rf,
                    });
                }
                continue;
            }
            let mut parts = Vec::new();
            let mut net = start;
            let mut to = None;
            for _ in 0..16 {
                let next: Vec<usize> = v.on_net[net]
                    .iter()
                    .copied()
                    .filter(|&k| k != ci && !parts.contains(&k))
                    .filter(|&k| v.signal_nets(k).len() >= 2)
                    .collect();
                if next.len() != 1 {
                    break;
                }
                let k = next[0];
                let nets = v.signal_nets(k);
                if nets.len() > 2 || role(&l.parts[k]) == Role::Connector {
                    to = Some(l.parts[k].reference.clone());
                    break;
                }
                if used.contains(&k) {
                    break;
                }
                parts.push(k);
                let Some(&onward) = nets.iter().find(|&&n| n != net) else { break };
                net = onward;
            }
            if parts.len() >= 2 || (rf && !parts.is_empty()) {
                used.extend(&parts);
                out.push(Chain {
                    from: c.reference.clone(),
                    parts: parts.iter().map(|&k| l.parts[k].reference.clone()).collect(),
                    to,
                    rf,
                });
            }
        }
    }
    out
}

fn rf_nets(v: &View, model: &Model, part: usize) -> Vec<usize> {
    let nets = &v.layout.nets;
    v.signal_nets(part)
        .into_iter()
        .filter(|&n| place::is_rf_class(model.board, &nets[n].class))
        .collect()
}

fn rf_path(
    v: &View,
    model: &Model,
    from: usize,
    net: usize,
    seen: &[usize],
) -> (Vec<usize>, Option<usize>) {
    if seen.len() > RF_DEPTH {
        return (Vec::new(), None);
    }
    let next: Vec<usize> = v.on_net[net]
        .iter()
        .copied()
        .filter(|&k| k != from && !seen.contains(&k))
        .filter(|&k| rf_nets(v, model, k).len() >= 2)
        .collect();
    let [k] = next[..] else { return (Vec::new(), None) };
    let nets = rf_nets(v, model, k);
    if role(&v.layout.parts[k]) == Role::Connector || nets.len() > RF_BRANCHES {
        return (Vec::new(), Some(k));
    }
    let mut inside = seen.to_vec();
    inside.extend([from, k]);
    let mut ways: Vec<(Vec<usize>, Option<usize>)> =
        nets.iter().filter(|&&n| n != net).map(|&n| rf_path(v, model, k, n, &inside)).collect();
    ways.sort_by_key(|w| std::cmp::Reverse(w.0.len()));
    let mut out = vec![k];
    match ways.as_slice() {
        [best, second, ..] if best.0.len() == second.0.len() => (out, None),
        [best, ..] => {
            out.extend(&best.0);
            (out, best.1)
        }
        [] => (out, None),
    }
}

fn decaps(model: &Model) -> Vec<Decap> {
    let l = &model.layout;
    let b = model.board;
    let mut out = Vec::new();
    let mut taken: Vec<(usize, usize)> = Vec::new();
    let mut caps: Vec<(usize, usize, f64)> = Vec::new();
    for (ci, c) in l.parts.iter().enumerate() {
        if !place::is_capacitor(&c.reference, &c.footprint_name)
            || c.pads.iter().filter(|q| !q.copper.is_empty()).count() != 2
        {
            continue;
        }
        let Some(rail) = c.pads.iter().filter_map(|q| q.net).find(|&n| {
            let net = &l.nets[n];
            !place::is_ground(&net.name) && place::is_power_net(b, &net.name, &net.class)
        }) else {
            continue;
        };
        let grounded =
            c.pads.iter().filter_map(|q| q.net).any(|n| place::is_ground(&l.nets[n].name));
        if !grounded {
            continue;
        }
        let farads = agentee_core::sim::parse_value(&c.value).unwrap_or(1e-6);
        caps.push((ci, rail, farads));
    }
    caps.sort_by(|a, b| a.2.total_cmp(&b.2));
    for (ci, rail, farads) in caps {
        let here = l.parts[ci].at.to_mm();
        let mut best: Option<(f64, usize, usize)> = None;
        for (pi, p) in l.parts.iter().enumerate() {
            if pi == ci || role(p) != Role::Chip {
                continue;
            }
            let pins: Vec<usize> =
                (0..p.pads.len()).filter(|&k| p.pads[k].net == Some(rail)).collect();
            for k in pins {
                let used = taken.iter().filter(|t| t.0 == pi && t.1 == k).count();
                let d = agentee_core::geom::dist(here, pad_centre(&p.pads[k])) + used as f64 * 50.0;
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, pi, k));
                }
            }
        }
        let Some((_, pi, k)) = best else { continue };
        taken.push((pi, k));
        let p = &l.parts[pi];
        out.push(Decap {
            cap: l.parts[ci].reference.clone(),
            ic: p.reference.clone(),
            pin: p.pads[k].number.clone(),
            net: l.nets[rail].name.clone(),
            farads,
        });
    }
    out
}

fn clocks(v: &View) -> Vec<Clock> {
    let l = v.layout;
    let mut out = Vec::new();
    for (xi, x) in l.parts.iter().enumerate() {
        if role(x) != Role::Crystal {
            continue;
        }
        let nets = v.signal_nets(xi);
        let mut best: Option<(usize, usize)> = None;
        for &n in &nets {
            for &k in &v.on_net[n] {
                if k != xi && role(&l.parts[k]) == Role::Chip {
                    let hits = nets.iter().filter(|m| v.on_net[**m].contains(&k)).count();
                    if best.is_none_or(|b| hits > b.1) {
                        best = Some((k, hits));
                    }
                }
            }
        }
        if let Some((k, _)) = best {
            out.push(Clock {
                part: x.reference.clone(),
                ic: l.parts[k].reference.clone(),
                nets: nets.iter().map(|&n| l.nets[n].name.clone()).collect(),
            });
        }
    }
    out
}

impl Phase for Constraints {
    fn name(&self) -> &'static str {
        "constraints"
    }

    fn run(&self, model: &mut Model, _cfg: &EngineFile) -> PhaseReport {
        let mut report = PhaseReport { phase: "constraints".into(), ..Default::default() };
        let g = find(model);
        for c in &g.chains {
            report.notes.push(format!(
                "chain{}: {} > {}{}",
                if c.rf { " (RF)" } else { "" },
                c.from,
                c.parts.join(" > "),
                c.to.as_ref().map(|t| format!(" > {t}")).unwrap_or_default()
            ));
        }
        report.notes.push(format!(
            "{} chains, {} decaps bound to supply pins, {} clock parts",
            g.chains.len(),
            g.decaps.len(),
            g.clocks.len()
        ));
        report.changed = true;
        model.constraints = Some(g);
        report
    }
}
