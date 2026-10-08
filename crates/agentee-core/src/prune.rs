use crate::footprint::PadKind;
use crate::geom::{self, P};
use crate::graphic::Bounds;
use crate::layout::{Layout, ViaSource};
use crate::tie::{filled_cells, islands, on_pad, track_gap};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pruned {
    pub vias: Vec<usize>,
    pub tracks: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq)]
enum Node {
    Pad(usize, usize),
    Via(usize),
    Track(usize),
    Island(usize, u32),
}

struct Net {
    nodes: Vec<Node>,
    links: Vec<Vec<usize>>,
    pads: Vec<usize>,
}

impl Net {
    fn groups(&self, gone: &[bool]) -> usize {
        let mut seen = gone.to_vec();
        let mut count = 0;
        for &p in &self.pads {
            if seen[p] {
                continue;
            }
            count += 1;
            seen[p] = true;
            let mut open = vec![p];
            while let Some(i) = open.pop() {
                for &j in &self.links[i] {
                    if !seen[j] {
                        seen[j] = true;
                        open.push(j);
                    }
                }
            }
        }
        count
    }
}

fn ends(points: &[P]) -> [P; 2] {
    [points[0], *points.last().unwrap_or(&points[0])]
}

fn build(layout: &Layout, net: usize, labels: &mut [Option<Vec<u32>>]) -> Net {
    let mut nodes = Vec::new();
    for (pi, p) in layout.parts.iter().enumerate() {
        for (qi, q) in p.pads.iter().enumerate() {
            if q.net == Some(net) && q.kind != PadKind::Npth {
                nodes.push(Node::Pad(pi, qi));
            }
        }
    }
    let pads: Vec<usize> = (0..nodes.len()).collect();
    nodes.extend((0..layout.vias.len()).filter(|&k| layout.vias[k].net == net).map(Node::Via));
    nodes
        .extend((0..layout.tracks.len()).filter(|&k| layout.tracks[k].net == net).map(Node::Track));
    let zones: Vec<usize> =
        (0..layout.zones.len()).filter(|&z| layout.zones[z].net == net).collect();
    for &zi in &zones {
        let label = labels[zi].get_or_insert_with(|| islands(&layout.zones[zi]));
        let mut ls: Vec<u32> = label.iter().copied().filter(|&l| l != 0).collect();
        ls.sort_unstable();
        ls.dedup();
        nodes.extend(ls.into_iter().map(|l| Node::Island(zi, l)));
    }
    let n = nodes.len();
    let mut links = vec![Vec::new(); n];
    let link = |a: usize, b: usize, links: &mut Vec<Vec<usize>>| {
        if a != b && !links[a].contains(&b) {
            links[a].push(b);
            links[b].push(a);
        }
    };
    for a in 0..n {
        for b in a + 1..n {
            if joins(layout, nodes[a], nodes[b]) {
                link(a, b, &mut links);
            }
        }
    }
    let island = |zi: usize, l: u32| nodes.iter().position(|&m| m == Node::Island(zi, l));
    for &zi in &zones {
        let z = &layout.zones[zi];
        let label = labels[zi].as_ref().expect("labelled above");
        for (a, &node) in nodes.iter().enumerate() {
            let cells = match node {
                Node::Pad(pi, qi) => {
                    let q = &layout.parts[pi].pads[qi];
                    if !q.copper.contains(&z.layer) {
                        continue;
                    }
                    on_pad(z, &q.outlines)
                }
                Node::Via(k) => {
                    let v = &layout.vias[k];
                    if !v.layers.contains(&z.layer) {
                        continue;
                    }
                    let r = v.diameter / 2.0;
                    let b =
                        Bounds { min: [v.at[0] - r, v.at[1] - r], max: [v.at[0] + r, v.at[1] + r] };
                    filled_cells(z, b, |c| geom::dist(c, v.at) <= r)
                }
                Node::Track(k) => {
                    let t = &layout.tracks[k];
                    if t.layer != z.layer {
                        continue;
                    }
                    let mut b = Bounds::EMPTY;
                    t.points.iter().for_each(|q| b.add(*q));
                    let h = t.width / 2.0;
                    let b = Bounds {
                        min: [b.min[0] - h, b.min[1] - h],
                        max: [b.max[0] + h, b.max[1] + h],
                    };
                    filled_cells(z, b, |c| track_gap(&t.points, c) <= h)
                }
                Node::Island(..) => continue,
            };
            let mut hit: Vec<u32> =
                cells.into_iter().map(|c| label[c]).filter(|&l| l != 0).collect();
            hit.sort_unstable();
            hit.dedup();
            for l in hit {
                if let Some(i) = island(zi, l) {
                    link(a, i, &mut links);
                }
            }
        }
    }
    Net { nodes, links, pads }
}

fn joins(layout: &Layout, a: Node, b: Node) -> bool {
    let pad = |pi: usize, qi: usize| &layout.parts[pi].pads[qi];
    match (a, b) {
        (Node::Pad(pa, qa), Node::Pad(pb, qb)) => {
            let (x, y) = (pad(pa, qa), pad(pb, qb));
            x.copper.iter().any(|l| y.copper.contains(l))
                && x.outlines
                    .iter()
                    .any(|o| y.outlines.iter().any(|w| geom::polygon_distance(o, w) <= 1e-6))
        }
        (Node::Pad(pi, qi), Node::Via(k)) | (Node::Via(k), Node::Pad(pi, qi)) => {
            let (q, v) = (pad(pi, qi), &layout.vias[k]);
            q.copper.iter().any(|l| v.layers.contains(l))
                && crate::drc::rings_point_gap(&q.outlines, v.at) <= v.diameter / 2.0
        }
        (Node::Pad(pi, qi), Node::Track(k)) | (Node::Track(k), Node::Pad(pi, qi)) => {
            let (q, t) = (pad(pi, qi), &layout.tracks[k]);
            q.copper.contains(&t.layer)
                && ends(&t.points)
                    .iter()
                    .any(|&e| crate::drc::rings_point_gap(&q.outlines, e) <= t.width / 2.0)
        }
        (Node::Via(a), Node::Via(b)) => {
            let (v, w) = (&layout.vias[a], &layout.vias[b]);
            v.layers.iter().any(|l| w.layers.contains(l))
                && geom::dist(v.at, w.at) <= (v.diameter + w.diameter) / 2.0
        }
        (Node::Via(k), Node::Track(t)) | (Node::Track(t), Node::Via(k)) => {
            let (v, t) = (&layout.vias[k], &layout.tracks[t]);
            v.layers.contains(&t.layer)
                && track_gap(&t.points, v.at) <= (t.width + v.diameter) / 2.0
        }
        (Node::Track(a), Node::Track(b)) => {
            let (s, t) = (&layout.tracks[a], &layout.tracks[b]);
            let reach = (s.width + t.width) / 2.0;
            s.layer == t.layer
                && (ends(&s.points).iter().any(|&e| track_gap(&t.points, e) <= reach)
                    || ends(&t.points).iter().any(|&e| track_gap(&s.points, e) <= reach))
        }
        _ => false,
    }
}

fn dangling(layout: &Layout, net: &Net, gone: &[bool], i: usize) -> bool {
    let Node::Track(k) = net.nodes[i] else { return false };
    let t = &layout.tracks[k];
    let h = t.width / 2.0;
    ends(&t.points).iter().any(|&e| {
        !net.links[i].iter().any(|&j| {
            !gone[j]
                && match net.nodes[j] {
                    Node::Pad(pi, qi) => {
                        crate::drc::rings_point_gap(&layout.parts[pi].pads[qi].outlines, e) <= h
                    }
                    Node::Via(v) => {
                        let v = &layout.vias[v];
                        geom::dist(v.at, e) <= h + v.diameter / 2.0
                    }
                    Node::Track(o) => {
                        let o = &layout.tracks[o];
                        track_gap(&o.points, e) <= h + o.width / 2.0
                    }
                    Node::Island(zi, _) => {
                        let z = &layout.zones[zi];
                        z.filled(e)
                            || (0..16).any(|s| {
                                let a = s as f64 * std::f64::consts::TAU / 16.0;
                                z.filled([e[0] + h * a.cos(), e[1] + h * a.sin()])
                            })
                    }
                }
        })
    })
}

fn layers_used(layout: &Layout, net: &Net, gone: &[bool], i: usize) -> usize {
    let mut used: Vec<&str> = Vec::new();
    for &j in &net.links[i] {
        if gone[j] {
            continue;
        }
        let layers: Vec<&str> = match net.nodes[j] {
            Node::Track(k) => vec![layout.tracks[k].layer.as_str()],
            Node::Island(zi, _) => vec![layout.zones[zi].layer.as_str()],
            Node::Pad(pi, qi) => {
                let Node::Via(v) = net.nodes[i] else { continue };
                let v = &layout.vias[v];
                layout.parts[pi].pads[qi]
                    .copper
                    .iter()
                    .filter(|l| v.layers.contains(l))
                    .map(String::as_str)
                    .collect()
            }
            Node::Via(_) => continue,
        };
        for l in layers {
            if !used.contains(&l) {
                used.push(l);
            }
        }
    }
    used.len()
}

fn drops_to_pour(layout: &Layout, net: &Net, gone: &[bool], i: usize) -> bool {
    let live = || net.links[i].iter().filter(|&&j| !gone[j]).map(|&j| net.nodes[j]);
    let tracked: Vec<&str> = live()
        .filter_map(|m| match m {
            Node::Track(k) => Some(layout.tracks[k].layer.as_str()),
            _ => None,
        })
        .collect();
    live().any(
        |m| matches!(m, Node::Island(zi, _) if !tracked.contains(&layout.zones[zi].layer.as_str())),
    )
}

fn cascade(layout: &Layout, net: &Net, gone: &mut [bool], track_ok: &dyn Fn(usize) -> bool) -> f64 {
    let mut freed = 0.0;
    loop {
        let next = (0..net.nodes.len()).find(|&i| {
            !gone[i]
                && matches!(net.nodes[i], Node::Track(k) if track_ok(k))
                && dangling(layout, net, gone, i)
        });
        let Some(i) = next else { return freed };
        let Node::Track(k) = net.nodes[i] else { unreachable!() };
        freed += layout.tracks[k].points.windows(2).map(|w| geom::dist(w[0], w[1])).sum::<f64>();
        gone[i] = true;
    }
}

pub fn prune(
    layout: &Layout,
    via_ok: &dyn Fn(usize) -> bool,
    track_ok: &dyn Fn(usize) -> bool,
) -> Pruned {
    let mut out = Pruned::default();
    let mut labels: Vec<Option<Vec<u32>>> = vec![None; layout.zones.len()];
    let mut nets: Vec<usize> = (0..layout.vias.len())
        .filter(|&k| via_ok(k) && !matches!(layout.vias[k].source, ViaSource::Stitch(_)))
        .map(|k| layout.vias[k].net)
        .collect();
    nets.sort_unstable();
    nets.dedup();
    for n in nets {
        let net = build(layout, n, &mut labels);
        let mut gone = vec![false; net.nodes.len()];
        let groups = net.groups(&gone);
        let candidates: Vec<usize> = net
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                matches!(m, Node::Via(k) if via_ok(*k) && !matches!(layout.vias[*k].source, ViaSource::Stitch(_)))
            })
            .map(|(i, _)| i)
            .collect();
        loop {
            let mut best: Option<(f64, usize, Vec<bool>)> = None;
            for &i in candidates.iter().rev() {
                if gone[i] {
                    continue;
                }
                let useless = layers_used(layout, &net, &gone, i) <= 1;
                if !useless && drops_to_pour(layout, &net, &gone, i) {
                    continue;
                }
                let mut trial = gone.clone();
                trial[i] = true;
                let freed = cascade(layout, &net, &mut trial, track_ok);
                if net.groups(&trial) > groups {
                    continue;
                }
                if best.as_ref().is_none_or(|b| freed > b.0 + 1e-9) {
                    best = Some((freed, i, trial));
                }
            }
            let Some((_, _, trial)) = best else { break };
            gone = trial;
        }
        for (i, m) in net.nodes.iter().enumerate() {
            if !gone[i] {
                continue;
            }
            match *m {
                Node::Via(k) => out.vias.push(k),
                Node::Track(k) => out.tracks.push(k),
                _ => {}
            }
        }
    }
    out.vias.sort_unstable();
    out.tracks.sort_unstable();
    out
}
