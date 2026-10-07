use crate::board::Board;
use crate::footprint::PadKind;
use crate::geom::{self, P};
use crate::interface::{Interface, Limit};
use crate::layout::{Layout, LayoutNet, glob, serpentine};
use crate::rules::Spacing;
use serde::Serialize;

#[derive(Clone, Debug)]
pub struct TuneOptions {
    pub nets: Vec<String>,
    pub amplitude: Option<f64>,
    pub pitch: Option<f64>,
}

impl Default for TuneOptions {
    fn default() -> Self {
        TuneOptions { nets: vec!["*".into()], amplitude: None, pitch: None }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Tuned {
    pub net: String,
    pub why: String,
    pub wanted_mm: f64,
    pub added_mm: f64,
    pub meanders: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct TrackEdit {
    pub track: usize,
    pub points: Vec<P>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TuneResult {
    pub tuned: Vec<Tuned>,
    pub failed: Vec<Tuned>,
    pub over: Vec<String>,
    #[serde(skip)]
    pub edits: Vec<TrackEdit>,
}

enum Shape {
    Poly(Vec<P>),
    Seg(P, P, f64),
    Circle(P, f64),
}

pub(crate) struct Obstacle {
    pub(crate) net: Option<usize>,
    pub(crate) layers: Vec<String>,
    shape: Shape,
    pub(crate) lo: P,
    pub(crate) hi: P,
}

impl Obstacle {
    fn new(net: Option<usize>, layers: Vec<String>, shape: Shape) -> Self {
        let (lo, hi) = match &shape {
            Shape::Poly(p) => p.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), q| {
                ([lo[0].min(q[0]), lo[1].min(q[1])], [hi[0].max(q[0]), hi[1].max(q[1])])
            }),
            Shape::Seg(a, b, r) => {
                ([a[0].min(b[0]) - r, a[1].min(b[1]) - r], [a[0].max(b[0]) + r, a[1].max(b[1]) + r])
            }
            Shape::Circle(c, r) => ([c[0] - r, c[1] - r], [c[0] + r, c[1] + r]),
        };
        Obstacle { net, layers, shape, lo, hi }
    }

    pub(crate) fn distance(&self, line: &[P]) -> f64 {
        match &self.shape {
            Shape::Poly(p) => geom::polyline_polygon_distance(line, p),
            Shape::Seg(a, b, r) => {
                line.windows(2)
                    .map(|w| geom::segment_segment_distance(w[0], w[1], *a, *b))
                    .fold(f64::MAX, f64::min)
                    - r
            }
            Shape::Circle(c, r) => {
                line.windows(2)
                    .map(|w| geom::point_segment_distance(*c, w[0], w[1]))
                    .fold(f64::MAX, f64::min)
                    - r
            }
        }
    }
}

struct Demand {
    nets: Vec<usize>,
    add: f64,
    why: String,
    tie: Vec<usize>,
}

pub fn tune(layout: &Layout, board: &Board, opts: &TuneOptions) -> Result<TuneResult, String> {
    let wanted = |n: usize| opts.nets.iter().any(|g| glob(g, &layout.nets[n].name));
    let mut out = TuneResult::default();
    let mut demands: Vec<Demand> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for pair in &layout.pairs {
        let Some(limit) = pair.limit_mm else { continue };
        if pair.skew_mm.abs() <= limit + 1e-9 || !seen.insert(pair.chain.clone()) {
            continue;
        }
        let pick = |m: &(usize, usize)| if pair.skew_mm > 0.0 { m.1 } else { m.0 };
        let short: Vec<usize> = pair.chain.iter().map(pick).collect();
        if short.iter().any(|&n| wanted(n)) {
            let other = |m: &(usize, usize)| if pair.skew_mm > 0.0 { m.0 } else { m.1 };
            let partner: Vec<&str> =
                pair.chain.iter().map(|m| layout.nets[other(m)].name.as_str()).collect();
            demands.push(Demand {
                nets: short,
                add: pair.skew_mm.abs(),
                why: format!("skew to {}", partner.join("+")),
                tie: Vec::new(),
            });
        }
    }
    for g in &layout.match_groups {
        for &n in &g.nets {
            let off = layout.nets[n].length_mm - g.target_mm;
            if off < -g.tolerance_mm - 1e-9 && wanted(n) {
                if let Some(d) = demands.iter_mut().find(|d| d.nets == [n]) {
                    d.add = d.add.max(-off);
                } else {
                    demands.push(Demand {
                        nets: vec![n],
                        add: -off,
                        why: format!("match group {}", g.name),
                        tie: Vec::new(),
                    });
                }
            } else if off > g.tolerance_mm + 1e-9 && wanted(n) {
                out.over.push(format!(
                    "{} is {:.3} mm over the {} target, shorten it by hand",
                    layout.nets[n].name, off, g.name
                ));
            }
        }
    }
    for (d, sum) in interface_demands(&layout.interfaces, &layout.nets) {
        if !d.nets.iter().any(|&n| wanted(n)) {
            continue;
        }
        match demands.iter_mut().find(|x| x.nets == d.nets) {
            Some(x) if sum => {
                x.add += d.add;
                x.why = format!("{}, {}", x.why, d.why);
            }
            Some(x) => x.add = x.add.max(d.add),
            None => demands.push(d),
        }
    }

    let mut obstacles = obstacles_of(layout);
    let spacing = crate::rules::Spacings::new(board, &layout.nets, layout.copper.len());
    let world = crate::drc::Ctx::new(
        board,
        &layout.copper,
        &layout.outline,
        &layout.board_cutouts,
        &layout.parts,
        &layout.tracks,
        &layout.vias,
        &[],
        &layout.nets,
    );
    let base = crate::rules::Placed::new(&world);
    let mut kept = crate::rules::Plan::default();
    let mut points: Vec<Vec<P>> = layout.tracks.iter().map(|t| t.points.clone()).collect();
    let edge = board.rules.min_copper_to_edge.to_mm();
    let floor = board.rules.min_clearance.to_mm();

    let partner_of = |n: usize| {
        layout.pairs.iter().find_map(|p| {
            if p.p == n {
                Some(p.n)
            } else if p.n == n {
                Some(p.p)
            } else {
                None
            }
        })
    };
    let gap_of = |n: usize| {
        board
            .netclasses
            .iter()
            .find(|c| c.name == layout.nets[n].class)
            .and_then(|c| c.diff_gap)
            .map(|g| g.to_mm())
    };

    let mut shortfall: std::collections::HashMap<Vec<usize>, f64> =
        std::collections::HashMap::new();
    for mut d in demands {
        if let Some(&cut) = shortfall.get(&d.tie).filter(|&&c| c > 1e-4) {
            d.add = (d.add - cut).max(0.0);
            let names: Vec<&str> = d.tie.iter().map(|&n| layout.nets[n].name.as_str()).collect();
            d.why = format!("{}, {cut:.3} mm less to stay matched to {}", d.why, names.join("+"));
        }
        let legs: Vec<(usize, usize, f64)> = d
            .nets
            .iter()
            .filter_map(|&n| {
                let other = partner_of(n).filter(|o| !d.nets.contains(o))?;
                Some((n, other, gap_of(n)?))
            })
            .collect();
        let mut left = d.add;
        let mut meanders = 0;
        'grow: while left > 1e-4 {
            let segs: Vec<(usize, usize, f64)> = layout
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| d.nets.contains(&t.net))
                .flat_map(|(ti, _)| {
                    points[ti]
                        .windows(2)
                        .enumerate()
                        .map(|(k, w)| (ti, k, geom::dist(w[0], w[1])))
                        .collect::<Vec<_>>()
                })
                .collect();
            let beside = |ti: usize, line: &[P]| {
                let t = &layout.tracks[ti];
                legs.iter()
                    .filter(|l| l.0 == t.net)
                    .map(|&(_, other, gap)| {
                        pair_runs(line, t.width, gap, &t.layer, other, layout, &points)
                    })
                    .fold((false, false), |a, b| (a.0 || b.0, a.1 || b.1))
            };
            let coupled: Vec<bool> = segs
                .iter()
                .map(|&(ti, k, _)| {
                    let (at_gap, off_gap) = beside(ti, &points[ti][k..k + 2]);
                    at_gap || off_gap
                })
                .collect();
            let mut order: Vec<usize> = (0..segs.len()).collect();
            order.sort_by(|&i, &j| {
                coupled[i].cmp(&coupled[j]).then(segs[j].2.total_cmp(&segs[i].2))
            });
            let segs: Vec<(usize, usize, f64)> = order.into_iter().map(|i| segs[i]).collect();
            for (ti, k, _) in segs {
                let t = &layout.tracks[ti];
                let net = &layout.nets[t.net];
                let (a, b) = (points[ti][k], points[ti][k + 1]);
                let on =
                    spacing.layer(layout.copper.iter().position(|c| c == &t.layer).unwrap_or(0));
                let floor_pitch = t.width + net.clearance.max(floor);
                let meander = |line: &[P]| crate::layout::Track {
                    source: usize::MAX,
                    net: t.net,
                    layer: t.layer.clone(),
                    width: t.width,
                    points: line.to_vec(),
                };
                let pitches: Vec<f64> = match opts.pitch {
                    Some(p) => vec![p],
                    None => {
                        [3.0, 2.5, 2.0].iter().map(|f| (f * t.width).max(floor_pitch)).collect()
                    }
                };
                let Some((pts, added)) = pitches.iter().find_map(|&pitch| {
                    fit(a, b, left, pitch, opts.amplitude, |line| {
                        !beside(ti, line).1
                            && legal(
                                line,
                                t.width,
                                &t.layer,
                                t.net,
                                0.0,
                                floor,
                                edge,
                                &obstacles,
                                layout.edge(),
                                &|o| {
                                    crate::rules::Spacing::gap(&spacing.class, Some(t.net), o, on)
                                        .max(spacing.isolation.gap(Some(t.net), o, on))
                                },
                            )
                            && crate::rules::legal(&crate::rules::Planned::after(
                                &base,
                                &kept,
                                crate::rules::Plan {
                                    tracks: vec![meander(line)],
                                    ..Default::default()
                                },
                            ))
                            .is_ok()
                    })
                }) else {
                    continue;
                };
                kept.tracks.push(meander(&pts));
                for w in pts.windows(2) {
                    obstacles.push(Obstacle::new(
                        Some(t.net),
                        vec![t.layer.clone()],
                        Shape::Seg(w[0], w[1], t.width / 2.0),
                    ));
                }
                let mut np = points[ti][..k].to_vec();
                np.extend(pts);
                np.extend_from_slice(&points[ti][k + 2..]);
                points[ti] = np;
                left -= added;
                meanders += 1;
                if !out.edits.iter().any(|e| e.track == ti) {
                    out.edits.push(TrackEdit { track: ti, points: Vec::new() });
                }
                continue 'grow;
            }
            break;
        }
        shortfall.insert(d.nets.clone(), left.max(0.0));
        let t = Tuned {
            net: d.nets.iter().map(|&n| layout.nets[n].name.as_str()).collect::<Vec<_>>().join("+"),
            why: d.why,
            wanted_mm: d.add,
            added_mm: d.add - left.max(0.0),
            meanders,
        };
        if left < 1e-4 { out.tuned.push(t) } else { out.failed.push(t) }
    }
    for e in &mut out.edits {
        e.points = points[e.track].clone();
    }
    for e in &mut out.edits {
        e.track = layout.tracks[e.track].source;
    }
    Ok(out)
}

fn pair_runs(
    line: &[P],
    width: f64,
    gap: f64,
    layer: &str,
    other: usize,
    layout: &Layout,
    points: &[Vec<P>],
) -> (bool, bool) {
    let (mut at_gap, mut off_gap) = (false, false);
    for (ti, t) in layout.tracks.iter().enumerate() {
        if t.net != other || t.layer != layer {
            continue;
        }
        let want = gap + (width + t.width) / 2.0;
        for s in line.windows(2) {
            for o in points[ti].windows(2) {
                let Some((overlap, sep)) = parallel_overlap(s[0], s[1], o[0], o[1]) else {
                    continue;
                };
                if (sep - want).abs() <= 0.1 * gap + 0.005 {
                    at_gap = true;
                } else if sep < want + 2.0 * gap && overlap > 0.05 {
                    off_gap = true;
                }
            }
        }
    }
    (at_gap, off_gap)
}

fn parallel_overlap(a0: P, a1: P, b0: P, b1: P) -> Option<(f64, f64)> {
    let la = geom::dist(a0, a1);
    let lb = geom::dist(b0, b1);
    if la < 1e-9 || lb < 1e-9 {
        return None;
    }
    let u = [(a1[0] - a0[0]) / la, (a1[1] - a0[1]) / la];
    let cross = (u[0] * (b1[1] - b0[1]) - u[1] * (b1[0] - b0[0])).abs() / lb;
    if cross > 0.02 {
        return None;
    }
    let proj = |p: P| (p[0] - a0[0]) * u[0] + (p[1] - a0[1]) * u[1];
    let (s0, s1) = (proj(b0).min(proj(b1)), proj(b0).max(proj(b1)));
    let overlap = s1.min(la) - s0.max(0.0);
    if overlap <= 0.0 {
        return None;
    }
    let mid = [b0[0] - a0[0], b0[1] - a0[1]];
    Some((overlap, (u[0] * mid[1] - u[1] * mid[0]).abs()))
}

fn interface_demands(ifaces: &[Interface], nets: &[LayoutNet]) -> Vec<(Demand, bool)> {
    let mut out = Vec::new();
    for f in ifaces {
        let members: Vec<Vec<usize>> = f
            .lanes
            .iter()
            .map(|l| l.nets.iter().filter_map(|n| nets.iter().position(|x| &x.name == n)).collect())
            .collect();
        let rate = |lanes: &[usize]| {
            let (ps, mm) = lanes
                .iter()
                .flat_map(|&l| &members[l])
                .fold((0.0, 0.0), |(p, m), &n| (p + nets[n].delay_ps, m + nets[n].length_mm));
            (mm > 1e-9 && ps > 1e-9).then(|| ps / mm)
        };
        let label = |lane: usize| f.lanes[lane].nets.join("+");
        let mut lengthen = |lanes: &[usize], ps: f64, why: String, sum: bool| {
            if let Some(r) = rate(lanes) {
                for &lane in lanes {
                    let tie = lanes.iter().find(|&&o| o != lane).map(|&o| members[o].clone());
                    out.push((
                        Demand {
                            nets: members[lane].clone(),
                            add: ps / r,
                            why: why.clone(),
                            tie: tie.unwrap_or_default(),
                        },
                        sum,
                    ));
                }
            }
        };
        for &(a, b, skew_mm, skew_ps) in &f.pairs {
            let (short, long) = if skew_mm > 0.0 { (b, a) } else { (a, b) };
            match f.spec.max_skew {
                Some(Limit::Ps(limit)) if skew_ps.abs() > limit + 1e-9 => {
                    let (short, long) = if skew_ps > 0.0 { (b, a) } else { (a, b) };
                    lengthen(
                        &[short],
                        skew_ps.abs(),
                        format!(
                            "skew {:.2} ps to {} (interface {})",
                            skew_ps.abs(),
                            label(long),
                            f.name
                        ),
                        false,
                    );
                }
                Some(Limit::Mm(limit)) if skew_mm.abs() > limit + 1e-9 => {
                    if let Some(r) = rate(&[short]) {
                        lengthen(
                            &[short],
                            skew_mm.abs() * r,
                            format!("skew to {} (interface {})", label(long), f.name),
                            false,
                        );
                    }
                }
                _ => {}
            }
        }
        let t = &f.timing;
        let clock = t.iter().position(|x| x.clock);
        let data: Vec<usize> = (0..t.len()).filter(|&k| Some(k) != clock).collect();
        if data.is_empty() || (f.max_bus_skew_ps.is_none() && f.clock_window_ps.is_none()) {
            continue;
        }
        let mut add = vec![0.0; t.len()];
        let latest = data.iter().map(|&k| t[k].delay_ps).fold(f64::MIN, f64::max);
        let window = clock.zip(f.clock_window_ps);
        if let Some((k, [lo, hi])) = window {
            let need = latest - hi - t[k].delay_ps;
            if need > 1e-9 {
                add[k] = need + (0.1 * (hi - lo)).min(2.0);
            }
        }
        let edges = window.map(|(k, [lo, hi])| {
            let c = t[k].delay_ps + add[k];
            (c + lo, c + hi)
        });
        let top = edges.map_or(latest, |(lo, _)| latest.max(lo));
        for &k in &data {
            let d = t[k].delay_ps;
            let low = f64::max(
                f.max_bus_skew_ps.map_or(f64::MIN, |limit| top - limit),
                edges.map_or(f64::MIN, |(lo, _)| lo),
            );
            if d >= low - 1e-9 {
                continue;
            }
            let aim = match (f.max_bus_skew_ps, edges) {
                (Some(_), Some((lo, hi))) => top.clamp(lo, hi),
                (Some(_), None) => top,
                (None, Some((lo, hi))) => (lo + 0.25 * (hi - lo)).min(hi),
                (None, None) => continue,
            };
            add[k] = aim - d;
        }
        for (k, ps) in add.into_iter().enumerate() {
            if ps > 1e-6 {
                lengthen(
                    &t[k].lanes,
                    ps,
                    format!("interface {} timing, {ps:.1} ps later", f.name),
                    true,
                );
            }
        }
    }
    out
}

fn fit(
    a: P,
    b: P,
    want: f64,
    pitch: f64,
    amplitude: Option<f64>,
    ok: impl Fn(&[P]) -> bool,
) -> Option<(Vec<P>, f64)> {
    let l = geom::dist(a, b);
    let margin = (pitch / 2.0).max(0.15);
    let room = l - 2.0 * margin;
    if room < 2.0 * pitch {
        return None;
    }
    let u = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
    let at = |s: f64| [a[0] + u[0] * s, a[1] + u[1] * s];
    let amps: Vec<f64> = match amplitude {
        Some(x) => vec![x],
        None => [1.2, 1.0, 0.8, 0.6, 0.5, 0.4, 0.3, 0.25, 0.2]
            .into_iter()
            .filter(|x| *x >= pitch * 0.5)
            .collect(),
    };
    let mut best: Option<(Vec<P>, f64)> = None;
    for amp in amps {
        let max_bumps = (room / (2.0 * pitch)).floor();
        let bumps = (want / (2.0 * amp)).ceil().min(max_bumps).max(1.0);
        let add = (2.0 * amp * bumps).min(want);
        let run = 2.0 * pitch * bumps;
        let steps = (((room - run) / pitch).floor() as usize).min(24);
        let mut starts: Vec<f64> =
            (0..=steps).map(|i| margin + (room - run) / 2.0 + i as f64 * pitch / 2.0).collect();
        starts.extend((1..=steps).map(|i| margin + (room - run) / 2.0 - i as f64 * pitch / 2.0));
        for s0 in starts {
            if s0 < margin - 1e-9 || s0 + run > l - margin + 1e-9 {
                continue;
            }
            let (p, q) = (at(s0), at(s0 + run));
            for side in [1.0, -1.0] {
                let Ok(mut pts) = (if side > 0.0 {
                    serpentine(p, q, add, amp, pitch)
                } else {
                    serpentine(q, p, add, amp, pitch)
                }) else {
                    continue;
                };
                if side < 0.0 {
                    pts.reverse();
                }
                if ok(&pts) {
                    let mut full = vec![a];
                    full.extend(pts);
                    full.push(b);
                    full.dedup_by(|x, y| geom::dist(*x, *y) < 1e-9);
                    if best.as_ref().is_none_or(|(_, got)| add > *got + 1e-9) {
                        best = Some((full, add));
                    }
                    break;
                }
            }
            if best.as_ref().is_some_and(|(_, got)| *got >= want - 1e-9) {
                return best;
            }
        }
    }
    best
}

#[allow(clippy::too_many_arguments)]
fn legal(
    line: &[P],
    width: f64,
    layer: &str,
    net: usize,
    clearance: f64,
    floor: f64,
    edge: f64,
    obstacles: &[Obstacle],
    board: geom::BoardEdge,
    apart: &dyn Fn(Option<usize>) -> f64,
) -> bool {
    let half = width / 2.0;
    let (lo, hi) = line.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), q| {
        ([lo[0].min(q[0]), lo[1].min(q[1])], [hi[0].max(q[0]), hi[1].max(q[1])])
    });
    if !line.iter().all(|p| board.contains(*p)) {
        return false;
    }
    if line.windows(2).any(|w| board.segment_distance(w[0], w[1]) < edge + half - 1e-9) {
        return false;
    }
    let reach = half + clearance.max(floor) + 1.0;
    for o in obstacles {
        if o.net == Some(net) || !o.layers.iter().any(|l| l == layer) {
            continue;
        }
        if o.hi[0] < lo[0] - reach
            || o.lo[0] > hi[0] + reach
            || o.hi[1] < lo[1] - reach
            || o.lo[1] > hi[1] + reach
        {
            continue;
        }
        let need = clearance.max(floor).max(apart(o.net)) + half;
        if o.distance(line) < need - 1e-6 {
            return false;
        }
    }
    true
}

pub(crate) fn obstacles_of(layout: &Layout) -> Vec<Obstacle> {
    let mut out = Vec::new();
    let nl = layout.copper.len();
    for part in &layout.parts {
        for pad in &part.pads {
            for o in &pad.outlines {
                out.push(Obstacle::new(pad.net, pad.copper.clone(), Shape::Poly(o.clone())));
            }
            if let Some((c, s, _)) = pad.drill {
                let r = s[0].max(s[1]) / 2.0;
                let hole = Shape::Circle(c, r);
                if pad.kind == PadKind::Npth || pad.copper.is_empty() {
                    out.push(Obstacle::new(None, layout.copper.clone(), hole));
                } else {
                    let (outer, inner): (Vec<_>, Vec<_>) = layout
                        .copper
                        .iter()
                        .enumerate()
                        .partition(|(i, _)| *i == 0 || *i + 1 == nl);
                    let names =
                        |v: Vec<(usize, &String)>| v.into_iter().map(|(_, l)| l.clone()).collect();
                    out.push(Obstacle::new(pad.net, names(outer), Shape::Circle(c, r)));
                    out.push(Obstacle::new(pad.net, names(inner), hole));
                }
            }
        }
    }
    for t in &layout.tracks {
        for w in t.points.windows(2) {
            out.push(Obstacle::new(
                Some(t.net),
                vec![t.layer.clone()],
                Shape::Seg(w[0], w[1], t.width / 2.0),
            ));
        }
    }
    for v in &layout.vias {
        out.push(Obstacle::new(
            Some(v.net),
            v.layers.clone(),
            Shape::Circle(v.at, v.diameter / 2.0),
        ));
        out.push(Obstacle::new(
            Some(v.net),
            layout.copper.clone(),
            Shape::Circle(v.at, v.drill / 2.0),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(name: &str, length_mm: f64) -> LayoutNet {
        LayoutNet {
            name: name.into(),
            class: "x".into(),
            width: 0.1,
            clearance: 0.1,
            unrouted: 0,
            length_mm,
            delay_ps: length_mm * 6.0,
        }
    }

    fn lane(n: &LayoutNet) -> crate::interface::Lane {
        crate::interface::Lane {
            nets: vec![n.name.clone()],
            length_mm: n.length_mm,
            delay_ps: n.delay_ps,
            vias: 0,
            stub_mm: 0.0,
            unreferenced_mm: 0.0,
        }
    }

    fn timing(lanes: Vec<usize>, clock: bool, delay_ps: f64) -> crate::interface::Timing {
        crate::interface::Timing {
            signal: String::new(),
            lanes,
            clock,
            delay_ps,
            to_clock_ps: None,
        }
    }

    #[test]
    fn interface_budgets_in_ps_become_lengths() {
        let nets = vec![net("D0", 10.0), net("D1", 20.0), net("CLK", 10.0)];
        let bus = Interface {
            name: "bus".into(),
            preset: None,
            spec: Default::default(),
            lanes: nets.iter().map(lane).collect(),
            pairs: Vec::new(),
            timing: vec![
                timing(vec![0], false, 60.0),
                timing(vec![1], false, 120.0),
                timing(vec![2], true, 60.0),
            ],
            max_bus_skew_ps: Some(20.0),
            clock_window_ps: Some([-10.0, 10.0]),
            measure: Vec::new(),
        };
        let got = interface_demands(&[bus], &nets);
        let add = |n: usize| got.iter().find(|d| d.0.nets == [n]).map(|d| d.0.add);
        assert!((add(2).unwrap() - 52.0 / 6.0).abs() < 1e-9, "the clock moves into the window");
        assert!((add(0).unwrap() - 10.0).abs() < 1e-9, "D0 matches the latest line");
        assert_eq!(add(1), None);

        let nets = vec![net("P", 10.0), net("N", 9.0)];
        let pair = Interface {
            name: "pair".into(),
            preset: None,
            spec: crate::interface::Spec { max_skew: Some(Limit::Ps(1.0)), ..Default::default() },
            lanes: nets.iter().map(lane).collect(),
            pairs: vec![(0, 1, 1.0, 6.0)],
            timing: Vec::new(),
            max_bus_skew_ps: None,
            clock_window_ps: None,
            measure: Vec::new(),
        };
        let got = interface_demands(&[pair], &nets);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0.nets, [1]);
        assert!((got[0].0.add - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_meander_fits_between_walls_and_adds_what_was_asked() {
        let wall = |y: f64| {
            Obstacle::new(Some(1), vec!["F.Cu".into()], Shape::Seg([0.0, y], [10.0, y], 0.05))
        };
        let obstacles = vec![wall(0.8), wall(-0.8)];
        let outline = vec![[-1.0, -5.0], [11.0, -5.0], [11.0, 5.0], [-1.0, 5.0]];
        let board = geom::BoardEdge::new(&outline, &[]);
        let ok = |l: &[P]| legal(l, 0.1, "F.Cu", 0, 0.1, 0.1, 0.2, &obstacles, board, &|_| 0.0);
        let (pts, added) = fit([0.0, 0.0], [10.0, 0.0], 1.5, 0.3, None, ok).unwrap();
        let len: f64 = pts.windows(2).map(|w| geom::dist(w[0], w[1])).sum();
        assert!((added - 1.5).abs() < 1e-9 && (len - 11.5).abs() < 1e-9, "{added} {len}");
        assert!(pts.iter().all(|p| p[1].abs() <= 0.55 + 1e-9));
        assert!(fit([0.0, 0.0], [0.8, 0.0], 1.5, 0.3, None, ok).is_none());
        let tight =
            |l: &[P]| legal(l, 0.1, "F.Cu", 0, 0.1, 0.1, 0.2, &obstacles[..1], board, &|_| 0.0);
        let (pts, _) = fit([0.0, 0.0], [10.0, 0.0], 1.5, 0.3, None, tight).unwrap();
        assert!(pts.iter().all(|p| p[1] <= 1e-9), "meanders away from the wall");
        let slot = vec![vec![[0.0, 0.5], [10.0, 0.5], [10.0, 1.2], [0.0, 1.2]]];
        let open =
            |l: &[P]| legal(l, 0.1, "F.Cu", 0, 0.1, 0.1, 0.2, &obstacles[1..], board, &|_| 0.0);
        let (pts, _) = fit([0.0, 0.0], [10.0, 0.0], 1.5, 0.3, None, open).unwrap();
        assert!(pts.iter().any(|p| p[1] > 0.1));
        let cut = geom::BoardEdge::new(&outline, &slot);
        let beside_slot =
            |l: &[P]| legal(l, 0.1, "F.Cu", 0, 0.1, 0.1, 0.2, &obstacles[1..], cut, &|_| 0.0);
        let (pts, _) = fit([0.0, 0.0], [10.0, 0.0], 1.5, 0.3, None, beside_slot).unwrap();
        assert!(pts.iter().all(|p| p[1] <= 1e-9), "meanders away from the board cutout");
    }
}
