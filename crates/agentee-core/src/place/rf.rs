use super::{Edge, PlaceInput, Placement, Role, courtyard_loops, is_rf_class, role_of};
use crate::geom::{self, P};
use crate::graphic::Bounds;
use crate::schematic::PinRef;

const GAP: f64 = 0.4;
const SPREAD: f64 = 1.5;

#[derive(Clone, Debug)]
pub struct Chain {
    pub start: String,
    pub end: Option<String>,
    pub series: Vec<String>,
    pub nodes: Vec<usize>,
    pub shunts: Vec<(usize, String)>,
}

pub struct Geo {
    pub reference: String,
    pub pads: Vec<(P, Option<usize>)>,
    pub boxes: Vec<Bounds>,
    pub court: Bounds,
    pub role: Role,
}

pub fn geometry(input: &PlaceInput) -> Vec<Geo> {
    let sch = input.schematic;
    let mut refs: Vec<&str> = Vec::new();
    for p in &sch.parts {
        if p.footprint.is_some() && !refs.contains(&p.reference.as_str()) {
            refs.push(&p.reference);
        }
    }
    let mut out = Vec::new();
    for r in refs {
        let units: Vec<(usize, &crate::schematic::Part)> =
            sch.parts.iter().enumerate().filter(|(_, p)| p.reference == r).collect();
        let fp_name = units[0].1.footprint.clone().unwrap_or_default();
        let Some(fp) = input.footprints.get(fp_name.as_str()).copied() else { continue };
        let boxes: Vec<Bounds> = fp
            .pads
            .iter()
            .filter(|q| q.is_copper())
            .map(|pad| {
                let mut b = Bounds::EMPTY;
                pad.outlines().iter().flatten().for_each(|q| b.add(*q));
                b
            })
            .collect();
        let pads: Vec<(P, Option<usize>)> = fp
            .pads
            .iter()
            .filter(|q| q.is_copper())
            .map(|pad| {
                let net = units.iter().find_map(|(pi, p)| {
                    p.symbol
                        .pins
                        .iter()
                        .position(|n| n.number == pad.number)
                        .and_then(|ni| sch.net_of(PinRef { part: *pi, pin: ni }))
                });
                let mut b = Bounds::EMPTY;
                pad.outlines().iter().flatten().for_each(|q| b.add(*q));
                (if b.is_empty() { pad.at.to_mm() } else { b.center() }, net)
            })
            .collect();
        let mut court = Bounds::EMPTY;
        courtyard_loops(fp, "F.CrtYd").iter().flatten().for_each(|q| court.add(*q));
        if court.is_empty() {
            fp.pads.iter().flat_map(|q| q.outlines()).flatten().for_each(|q| court.add(q));
            if !court.is_empty() {
                court.min = [court.min[0] - 0.25, court.min[1] - 0.25];
                court.max = [court.max[0] + 0.25, court.max[1] + 0.25];
            }
        }
        out.push(Geo {
            reference: r.to_string(),
            pads,
            boxes,
            court,
            role: role_of(r, &fp_name, fp),
        });
    }
    out
}

fn rf_nets_of(g: &Geo, rf: &[bool]) -> Vec<usize> {
    let mut v: Vec<usize> = g.pads.iter().filter_map(|(_, n)| *n).filter(|n| rf[*n]).collect();
    v.sort_unstable();
    v.dedup();
    v
}

struct Search<'a> {
    geo: &'a [Geo],
    nets_of: &'a [Vec<usize>],
    passable: &'a [bool],
    terminal: &'a [bool],
    used: &'a [bool],
}

#[derive(Clone, Default)]
struct Path {
    parts: Vec<usize>,
    nodes: Vec<usize>,
    end: Option<usize>,
}

impl Search<'_> {
    fn better(a: &Path, b: &Path) -> bool {
        (a.end.is_some(), a.parts.len()) > (b.end.is_some(), b.parts.len())
    }

    fn walk(&self, start: usize, net: usize, path: &mut Path, best: &mut Path) {
        if path.nodes.len() > 24 {
            return;
        }
        let here: Vec<usize> = (0..self.geo.len())
            .filter(|i| {
                *i != start
                    && !self.used[*i]
                    && !path.parts.contains(i)
                    && self.nets_of[*i].contains(&net)
            })
            .collect();
        if let Some(e) = here.iter().copied().find(|i| self.terminal[*i]) {
            let done = Path { end: Some(e), ..path.clone() };
            if Self::better(&done, best) {
                *best = done;
            }
        } else if Self::better(path, best) {
            *best = path.clone();
        }
        for k in here.into_iter().filter(|i| self.passable[*i]) {
            let onward: Vec<usize> = self.nets_of[k]
                .iter()
                .copied()
                .filter(|n| *n != net && !path.nodes.contains(n))
                .collect();
            for next in onward {
                path.parts.push(k);
                path.nodes.push(next);
                self.walk(start, next, path, best);
                path.parts.pop();
                path.nodes.pop();
            }
        }
    }
}

pub fn chains(input: &PlaceInput, geo: &[Geo]) -> Vec<Chain> {
    let board = input.board;
    let sch = input.schematic;
    let rf: Vec<bool> = sch.nets.iter().map(|n| is_rf_class(board, &n.class)).collect();
    if !rf.iter().any(|x| *x) {
        return Vec::new();
    }
    let nets_of: Vec<Vec<usize>> = geo.iter().map(|g| rf_nets_of(g, &rf)).collect();
    let netted = |i: usize| geo[i].pads.iter().filter(|(_, n)| n.is_some()).count();
    let terminal: Vec<bool> = (0..geo.len())
        .map(|i| {
            !nets_of[i].is_empty()
                && (geo[i].role == Role::Connector || (geo[i].role == Role::Chip && netted(i) > 16))
        })
        .collect();
    let passable: Vec<bool> =
        (0..geo.len()).map(|i| !terminal[i] && nets_of[i].len() >= 2 && netted(i) <= 16).collect();
    let shunt = |i: usize| {
        nets_of[i].len() == 1
            && !terminal[i]
            && netted(i) <= 3
            && geo[i].pads.iter().any(|(_, n)| n.is_none_or(|n| !rf[n]))
    };
    let mut used = vec![false; geo.len()];
    let mut out = Vec::new();
    let mut starts: Vec<usize> =
        (0..geo.len()).filter(|i| geo[*i].role == Role::Connector && terminal[*i]).collect();
    starts.sort_by(|a, b| {
        let input_like = |i: usize| {
            nets_of[i].iter().any(|n| sch.nets[*n].name.to_ascii_uppercase().contains("IN"))
        };
        input_like(*b).cmp(&input_like(*a)).then(geo[*a].reference.cmp(&geo[*b].reference))
    });
    for s in starts {
        if used[s] {
            continue;
        }
        let mut best = Path::default();
        for &net in &nets_of[s] {
            let search = Search {
                geo,
                nets_of: &nets_of,
                passable: &passable,
                terminal: &terminal,
                used: &used,
            };
            let mut path = Path { parts: Vec::new(), nodes: vec![net], end: None };
            search.walk(s, net, &mut path, &mut best);
        }
        if best.parts.is_empty() {
            continue;
        }
        let mut chain = Chain {
            start: geo[s].reference.clone(),
            end: best.end.map(|e| geo[e].reference.clone()),
            series: best.parts.iter().map(|k| geo[*k].reference.clone()).collect(),
            nodes: best.nodes.clone(),
            shunts: Vec::new(),
        };
        used[s] = true;
        best.parts.iter().for_each(|k| used[*k] = true);
        if let Some(e) = best.end.filter(|e| geo[*e].role == Role::Connector) {
            used[e] = true;
        }
        for (ni, n) in chain.nodes.iter().enumerate() {
            for i in 0..geo.len() {
                if !used[i] && shunt(i) && nets_of[i].contains(n) {
                    chain.shunts.push((ni, geo[i].reference.clone()));
                    used[i] = true;
                }
            }
        }
        out.push(chain);
    }
    out
}

fn world(at: P, rot: f64, local: P) -> P {
    let r = geom::rotate(local, rot);
    [at[0] + r[0], at[1] + r[1]]
}

fn court_world(g: &Geo, at: P, rot: f64) -> Bounds {
    let mut b = Bounds::EMPTY;
    let c = g.court;
    for q in [c.min, c.max, [c.min[0], c.max[1]], [c.max[0], c.min[1]]] {
        b.add(world(at, rot, q));
    }
    b
}

fn court_world_of(b: &Bounds, rot: f64) -> Bounds {
    let mut out = Bounds::EMPTY;
    for q in [b.min, b.max, [b.min[0], b.max[1]], [b.max[0], b.min[1]]] {
        out.add(geom::rotate(q, rot));
    }
    out
}

fn pad_on(g: &Geo, net: usize) -> Option<P> {
    g.pads.iter().find(|(_, n)| *n == Some(net)).map(|(c, _)| *c)
}

fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn turn_toward(local: P, want: P) -> f64 {
    [0.0, 90.0, 180.0, 270.0]
        .into_iter()
        .max_by(|a, b| {
            dot(geom::rotate(local, *a), want).total_cmp(&dot(geom::rotate(local, *b), want))
        })
        .unwrap_or(0.0)
}

fn span(b: &Bounds, d: P) -> (f64, f64) {
    let c = [b.min, b.max, [b.min[0], b.max[1]], [b.max[0], b.min[1]]];
    let v: Vec<f64> = c.iter().map(|q| dot(*q, d)).collect();
    (v.iter().copied().fold(f64::MAX, f64::min), v.iter().copied().fold(f64::MIN, f64::max))
}

pub fn edges_for(chain: &Chain, input: &PlaceInput, geo: &[Geo]) -> Vec<(String, Edge)> {
    let Some(end) = &chain.end else { return Vec::new() };
    let is_conn = |r: &str| geo.iter().any(|g| g.reference == r && g.role == Role::Connector);
    if !is_conn(end) {
        return Vec::new();
    }
    let mut ob = Bounds::EMPTY;
    input.outline.iter().for_each(|q| ob.add(*q));
    let [w, h] = ob.size();
    let (a, b) = if w >= h { (Edge::Left, Edge::Right) } else { (Edge::Top, Edge::Bottom) };
    let pinned = |r: &str| input.spec.edges.get(r).copied();
    match (pinned(&chain.start), pinned(end)) {
        (Some(_), Some(_)) => Vec::new(),
        (Some(s), None) => vec![(end.clone(), opposite(s))],
        (None, Some(e)) => vec![(chain.start.clone(), opposite(e))],
        (None, None) => vec![(chain.start.clone(), a), (end.clone(), b)],
    }
}

fn opposite(e: Edge) -> Edge {
    match e {
        Edge::Left => Edge::Right,
        Edge::Right => Edge::Left,
        Edge::Top => Edge::Bottom,
        Edge::Bottom => Edge::Top,
    }
}

pub struct Pose {
    pub at: P,
    pub rot: f64,
    pub movable: bool,
}

pub fn lay_out(
    chain: &Chain,
    input: &PlaceInput,
    geo: &[Geo],
    pose: &dyn Fn(&str) -> Option<Pose>,
    outline: &[P],
) -> Vec<Placement> {
    let g = |r: &str| geo.iter().find(|g| g.reference == r);
    let Some(sg) = g(&chain.start) else { return Vec::new() };
    let Some(sp) = pose(&chain.start) else { return Vec::new() };
    if chain
        .series
        .iter()
        .chain(chain.shunts.iter().map(|s| &s.1))
        .any(|r| pose(r).is_some_and(|p| !p.movable))
    {
        return Vec::new();
    }
    let Some(a_local) = pad_on(sg, chain.nodes[0]) else { return Vec::new() };
    let mut start_at = sp.at;
    let mut a = world(start_at, sp.rot, a_local);
    let end = chain.end.as_deref().and_then(|e| Some((g(e)?, pose(e)?)));
    let last = *chain.nodes.last().unwrap();
    let mut ob = Bounds::EMPTY;
    outline.iter().for_each(|q| ob.add(*q));
    let inward_of = |b: &Bounds| {
        let c = b.center();
        [
            (c[0] - ob.min[0], [1.0, 0.0]),
            (ob.max[0] - c[0], [-1.0, 0.0]),
            (c[1] - ob.min[1], [0.0, 1.0]),
            (ob.max[1] - c[1], [0.0, -1.0]),
        ]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|x| x.1)
        .unwrap()
    };
    let d: P = inward_of(&court_world(sg, start_at, sp.rot));
    let end = end.filter(|(eg, ep)| {
        eg.role != Role::Connector || {
            let e = inward_of(&court_world(eg, ep.at, ep.rot));
            dot(e, d) < -0.5
        }
    });
    let n = [-d[1], d[0]];
    let mut out = Vec::new();
    let mut end_pose = end.as_ref().map(|(_, p)| (p.at, p.rot, p.movable));
    if let (Some((eg, ep)), true) = (&end, sp.movable)
        && eg.role == Role::Connector
        && let Some(bl) = pad_on(eg, last)
    {
        let b = world(ep.at, ep.rot, bl);
        let centre = (dot(ob.center(), n) / super::GRID).round() * super::GRID;
        let shift_a = centre - dot(a, n);
        let shift_b = centre - dot(b, n);
        let fits = |gg: &Geo, at: P, rot: f64| {
            let cb = court_world(gg, at, rot);
            ob.min[0] <= cb.min[0] + 3.0
                && cb.max[0] <= ob.max[0] + 3.0
                && ob.min[1] <= cb.min[1] + 3.0
                && cb.max[1] <= ob.max[1] + 3.0
        };
        let na = [sp.at[0] + n[0] * shift_a, sp.at[1] + n[1] * shift_a];
        let nb = [ep.at[0] + n[0] * shift_b, ep.at[1] + n[1] * shift_b];
        if ep.movable && fits(sg, na, sp.rot) && fits(eg, nb, ep.rot) {
            start_at = na;
            end_pose = Some((nb, ep.rot, true));
        } else if ep.movable {
            let shift = dot(a, n) - dot(b, n);
            end_pose = Some(([ep.at[0] + n[0] * shift, ep.at[1] + n[1] * shift], ep.rot, true));
        }
        a = world(start_at, sp.rot, a_local);
    }
    if sp.movable {
        out.push(Placement {
            reference: chain.start.clone(),
            at: start_at,
            rotation: sp.rot,
            bottom: false,
            label: None,
        });
    }
    if let (Some((eg, _)), Some((at, rot, true))) = (&end, end_pose)
        && eg.role == Role::Connector
    {
        out.push(Placement {
            reference: eg.reference.clone(),
            at,
            rotation: rot,
            bottom: false,
            label: None,
        });
    }
    let line = dot(a, n);
    let s0 = span(&court_world(sg, start_at, sp.rot), d).1;
    let s1 = match (&end, end_pose) {
        (Some((eg, _)), Some((at, rot, _))) => Some(span(&court_world(eg, at, rot), d).0),
        _ => None,
    };

    struct Laid<'a> {
        g: &'a Geo,
        rot: f64,
        lo: f64,
        hi: f64,
        cross: f64,
    }
    let mut laid: Vec<Laid> = Vec::new();
    for (k, r) in chain.series.iter().enumerate() {
        let Some(sg) = g(r) else { return Vec::new() };
        let (Some(pin), Some(pout)) = (pad_on(sg, chain.nodes[k]), pad_on(sg, chain.nodes[k + 1]))
        else {
            return Vec::new();
        };
        let rot = turn_toward([pout[0] - pin[0], pout[1] - pin[1]], d);
        let (lo, hi) = span(&court_world(sg, [0.0, 0.0], rot), d);
        let pin_w = geom::rotate(pin, rot);
        laid.push(Laid { g: sg, rot, lo, hi, cross: dot(pin_w, n) });
    }
    let shunt_room = |node: usize| {
        chain
            .shunts
            .iter()
            .filter(|(k, _)| *k == node)
            .filter_map(|(_, r)| g(r))
            .map(|sg| {
                let a = sg.court;
                (a.max[0] - a.min[0]).min(a.max[1] - a.min[1])
            })
            .fold(0.0, f64::max)
    };
    let gaps: Vec<f64> = (0..=laid.len()).map(|k| GAP.max(shunt_room(k) + 2.0 * GAP)).collect();
    let need: f64 =
        laid.iter().map(|l| l.hi - l.lo).sum::<f64>() + gaps[..laid.len()].iter().sum::<f64>();
    let spare = s1.map(|s1| s1 - s0 - need - gaps[laid.len()]).unwrap_or(0.0).max(0.0);
    let extra = (spare / (laid.len() + 1) as f64).min(SPREAD);
    let mut s = s0;
    let mut node_at = Vec::new();
    for (k, l) in laid.iter().enumerate() {
        node_at.push(s);
        s += gaps[k] + extra;
        let origin_s = s - l.lo;
        let at =
            [d[0] * origin_s + n[0] * (line - l.cross), d[1] * origin_s + n[1] * (line - l.cross)];
        out.push(Placement {
            reference: l.g.reference.clone(),
            at,
            rotation: l.rot,
            bottom: false,
            label: None,
        });
        s = origin_s + l.hi;
    }
    node_at.push(s);
    let mut sides: Vec<usize> = vec![0; chain.nodes.len()];
    let toward_centre = if dot(ob.center(), n) >= line { 1.0 } else { -1.0 };
    for (k, r) in &chain.shunts {
        let Some(sg) = g(r) else { continue };
        let Some(pin) = pad_on(sg, chain.nodes[*k]) else { continue };
        let Some(other) = sg
            .pads
            .iter()
            .map(|(c, _)| *c)
            .max_by(|a, b| geom::dist(*a, pin).total_cmp(&geom::dist(*b, pin)))
        else {
            continue;
        };
        let side = if sides[*k].is_multiple_of(2) { toward_centre } else { -toward_centre };
        sides[*k] += 1;
        let want = [n[0] * side, n[1] * side];
        let rot = turn_toward([other[0] - pin[0], other[1] - pin[1]], want);
        let (lo, _) = span(&court_world(sg, [0.0, 0.0], rot), d);
        let pin_w = geom::rotate(pin, rot);
        let net = chain.nodes[*k];
        let keep = input
            .board
            .netclasses
            .iter()
            .find(|c| c.name == input.schematic.nets[net].class)
            .map(|c| c.width_on("F.Cu").to_mm() / 2.0 + c.clearance.to_mm())
            .unwrap_or(0.35);
        let near = sg
            .pads
            .iter()
            .zip(&sg.boxes)
            .filter(|((_, pn), _)| *pn != Some(net))
            .map(|(_, b)| span(&court_world_of(b, rot), want).0)
            .fold(f64::MAX, f64::min);
        let line_s = line * side;
        let pin_s = dot(pin_w, want);
        let o = if near == f64::MAX {
            line_s - pin_s
        } else {
            (line_s - pin_s).max(line_s + keep - near)
        };
        let along = node_at[*k] + GAP - lo;
        let at = [d[0] * along + want[0] * o, d[1] * along + want[1] * o];
        out.push(Placement {
            reference: sg.reference.clone(),
            at,
            rotation: rot,
            bottom: false,
            label: None,
        });
    }
    for p in out.iter_mut() {
        if geo.iter().any(|x| x.reference == p.reference && x.role != Role::Connector) {
            p.at = p.at.map(|v| (v / super::GRID).round() * super::GRID);
        }
    }
    out
}

pub fn prune(
    laid: Vec<Placement>,
    geo: &[Geo],
    taken: &mut Vec<Bounds>,
    input: &PlaceInput,
) -> Vec<Placement> {
    let body = input.board.rules.min_body_to_edge.to_mm();
    let keepouts: Vec<Vec<P>> = input
        .spec
        .keepouts
        .iter()
        .map(|k| k.iter().map(|q| q.to_mm()).collect())
        .filter(|k: &Vec<P>| k.len() >= 3)
        .collect();
    let edge = geom::BoardEdge::new(input.outline, input.cutouts);
    let mut kept = Vec::new();
    for p in laid {
        let Some(g) = geo.iter().find(|g| g.reference == p.reference) else { continue };
        let b = court_world(g, p.at, p.rotation);
        let rect = vec![b.min, [b.max[0], b.min[1]], b.max, [b.min[0], b.max[1]]];
        let connector = g.role == Role::Connector;
        let inside = rect.iter().all(|q| edge.contains(*q))
            && (0..4).all(|k| edge.segment_distance(rect[k], rect[(k + 1) % 4]) >= body - 1e-6);
        let clear = keepouts.iter().all(|k| geom::polygon_distance(&rect, k) > 1e-6)
            && input
                .silk
                .iter()
                .filter(|a| !a.bottom && a.poly.len() >= 3)
                .all(|a| geom::polygon_distance(&rect, &a.poly) > 1e-6)
            && !taken.iter().any(|t| {
                t.min[0] < b.max[0] - 1e-3
                    && b.min[0] < t.max[0] - 1e-3
                    && t.min[1] < b.max[1] - 1e-3
                    && b.min[1] < t.max[1] - 1e-3
            });
        if (inside || connector) && clear {
            taken.push(b);
            kept.push(p);
        }
    }
    kept
}

pub fn footprint_box(geo: &[Geo], reference: &str, at: P, rot: f64) -> Option<Bounds> {
    geo.iter().find(|g| g.reference == reference).map(|g| court_world(g, at, rot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_aligns_a_pad_pair_with_the_line() {
        assert_eq!(turn_toward([1.0, 0.0], [1.0, 0.0]), 0.0);
        assert_eq!(geom::rotate([1.0, 0.0], turn_toward([1.0, 0.0], [0.0, 1.0])), [0.0, 1.0]);
        assert_eq!(geom::rotate([0.0, -2.0], turn_toward([0.0, -2.0], [-1.0, 0.0]))[0], -2.0);
    }
}
