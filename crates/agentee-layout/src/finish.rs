use crate::{Model, Phase, PhaseReport};
use agentee_core::engine::EngineFile;
use agentee_core::geom::{self, P};
use agentee_core::route::RoutedTrack;

pub struct Finish;

impl Phase for Finish {
    fn name(&self) -> &'static str {
        "finish"
    }

    fn run(&self, model: &mut Model, cfg: &EngineFile) -> PhaseReport {
        let mut report = PhaseReport { phase: "finish".into(), ..Default::default() };
        if model.detail.is_none() {
            report.notes.push("nothing routed to finish".into());
            return report;
        }
        if let Err(e) = model.ensure_base(&crate::detail::options(cfg)) {
            report.failed.push(e);
            return report;
        }
        let base = model.base.as_ref().expect("base is built");
        let plan = model.detail.as_mut().expect("detail plan");
        let undo = crate::negotiate::spread(&model.layout, base, &mut plan.tracks);
        report
            .notes
            .push(format!("spread: {} tracks moved to the middle of their free space", undo.len()));
        report.changed = !undo.is_empty();
        plan.undo = undo;
        report
    }
}

pub fn check_spread(model: &mut Model, cfg: &EngineFile, report: &mut PhaseReport) -> bool {
    if model.ensure_base(&crate::detail::options(cfg)).is_err() {
        return false;
    }
    let base = model.base.as_ref().expect("base is built");
    let Some(plan) = model.detail.as_mut() else { return false };
    let which: Vec<usize> = plan.undo.iter().map(|u| u.0).collect();
    let bad = crate::negotiate::illegal(&model.layout, base, &plan.tracks, &which);
    for (ti, old) in std::mem::take(&mut plan.undo) {
        if bad.contains(&ti) {
            plan.tracks[ti].points = old;
        }
    }
    if !bad.is_empty() {
        report
            .notes
            .push(format!("spread: {} moves put back, they crowded a neighbour", bad.len()));
    }
    !bad.is_empty()
}

pub fn tune(model: &mut Model, report: &mut PhaseReport) {
    let Some(plan) = model.detail.as_mut() else { return };
    let opts = agentee_core::tune::TuneOptions::default();
    let r = match agentee_core::tune::tune(&model.layout, model.board, &opts) {
        Ok(r) => r,
        Err(e) => {
            report.failed.push(format!("tune: {e}"));
            return;
        }
    };
    let first = model.file.tracks.len().saturating_sub(plan.tracks.len());
    let width_of = |t: &agentee_core::route::RoutedTrack| {
        t.width.unwrap_or_else(|| {
            model.layout.nets.iter().find(|n| n.name == t.net).map(|n| n.width).unwrap_or(0.1)
        })
    };
    let mut edited = 0;
    for e in &r.edits {
        let Some(k) = e.track.checked_sub(first).filter(|&k| k < plan.tracks.len()) else {
            continue;
        };
        let w = width_of(&plan.tracks[k]);
        let crowds = plan.tracks.iter().enumerate().any(|(j, o)| {
            j != k
                && o.net == plan.tracks[k].net
                && o.layer == plan.tracks[k].layer
                && e.points[1..e.points.len().saturating_sub(1)].iter().any(|p| {
                    o.points.windows(2).any(|s| {
                        agentee_core::geom::point_segment_distance(*p, s[0], s[1])
                            < (w + width_of(o)) / 2.0
                    })
                })
        });
        if !crowds && !folds(&e.points, w) {
            plan.tracks[k].points = e.points.clone();
            edited += 1;
        }
    }
    report.notes.push(format!(
        "tune: {} nets tuned, {} short of their length, {edited} tracks meandered",
        r.tuned.len(),
        r.failed.len()
    ));
    for f in &r.failed {
        report.failed.push(format!("{}: tune {}", f.net, f.why));
    }
    if edited > 0 {
        report.changed = true;
    }
}

pub fn neck(model: &mut Model, report: &mut PhaseReport) {
    let Some(plan) = model.detail.as_mut() else { return };
    let opts = agentee_core::neck::NeckOptions::default();
    let r = match agentee_core::neck::neck(&model.layout, model.board, &opts) {
        Ok(r) => r,
        Err(e) => {
            report.failed.push(format!("neck: {e}"));
            return;
        }
    };
    let first = model.file.tracks.len().saturating_sub(plan.tracks.len());
    let mut edited = 0;
    for e in &r.edits {
        let Some(k) = e.track.checked_sub(first).filter(|&k| k < plan.tracks.len()) else {
            continue;
        };
        plan.tracks[k].points = e.points.clone();
        let (net, layer) = (plan.tracks[k].net.clone(), plan.tracks[k].layer.clone());
        for n in &e.necks {
            plan.tracks.push(agentee_core::route::RoutedTrack {
                net: net.clone(),
                layer: layer.clone(),
                width: Some(n.width),
                points: n.points.clone(),
            });
        }
        edited += 1;
    }
    if edited > 0 {
        report.notes.push(format!("neck: {edited} track ends narrowed at their pads"));
        report.changed = true;
    }
}

fn folds(points: &[agentee_core::geom::P], width: f64) -> bool {
    let n = points.len();
    (0..n.saturating_sub(1)).any(|i| {
        (i + 2..n.saturating_sub(1)).any(|j| {
            agentee_core::geom::segment_segment_distance(
                points[i],
                points[i + 1],
                points[j],
                points[j + 1],
            ) < width
        })
    })
}

pub fn drop_vias(model: &mut Model, report: &mut PhaseReport) -> bool {
    let l = &model.layout;
    let Some(plan) = model.detail.as_ref() else { return false };
    let net_of = |name: &str| l.nets.iter().position(|n| n.name == name);
    let same = |a: &[P], b: &[P]| {
        a.len() == b.len() && a.iter().zip(b).all(|(p, q)| geom::dist(*p, *q) < 1e-3)
    };
    let planned_via = |k: usize| {
        let v = &l.vias[k];
        plan.vias.iter().any(|p| net_of(&p.net) == Some(v.net) && geom::dist(p.at, v.at) < 1e-3)
    };
    let planned_track = |k: usize| {
        let t = &l.tracks[k];
        plan.tracks.iter().any(|p| {
            net_of(&p.net) == Some(t.net) && p.layer == t.layer && same(&p.points, &t.points)
        })
    };
    let pruned = agentee_core::prune::prune(l, &planned_via, &planned_track);
    if pruned.vias.is_empty() {
        return false;
    }
    let vias: Vec<(usize, P)> =
        pruned.vias.iter().map(|&k| (l.vias[k].net, l.vias[k].at)).collect();
    let tracks: Vec<(usize, String, Vec<P>)> = pruned
        .tracks
        .iter()
        .map(|&k| (l.tracks[k].net, l.tracks[k].layer.clone(), l.tracks[k].points.clone()))
        .collect();
    let plan = model.detail.as_mut().expect("checked above");
    plan.vias.retain(|p| {
        !vias.iter().any(|(n, at)| net_of(&p.net) == Some(*n) && geom::dist(p.at, *at) < 1e-3)
    });
    plan.tracks.retain(|p| {
        !tracks.iter().any(|(n, layer, pts)| {
            net_of(&p.net) == Some(*n) && &p.layer == layer && same(&p.points, pts)
        })
    });
    report.notes.push(format!(
        "{} vias and {} tracks dropped, the net was joined without them",
        vias.len(),
        tracks.len()
    ));
    true
}

pub fn dedouble(model: &mut Model, report: &mut PhaseReport) {
    let Some(plan) = model.detail.as_mut() else { return };
    let nets = &model.layout.nets;
    let width_of = |t: &RoutedTrack| {
        t.width.unwrap_or_else(|| {
            nets.iter().find(|n| n.name == t.net).map(|n| n.width).unwrap_or(0.1)
        })
    };
    let cut = cut_doubled(&mut plan.tracks, &width_of);
    if cut > 0 {
        report.notes.push(format!("dedouble: {cut} doubled runs cut back"));
        report.changed = true;
    }
}

fn cut_doubled(tracks: &mut Vec<RoutedTrack>, width_of: &dyn Fn(&RoutedTrack) -> f64) -> usize {
    let mut cut = 0;
    let mut skip: Vec<(usize, usize)> = Vec::new();
    for _ in 0..64 {
        let Some((k, s, from, to)) = doubled_except(tracks, width_of, &skip) else { break };
        let t = tracks[k].clone();
        let (a, b) = (t.points[s], t.points[s + 1]);
        let len = geom::dist(a, b);
        let at = |d: f64| [a[0] + (b[0] - a[0]) * d / len, a[1] + (b[1] - a[1]) * d / len];
        let on_centre = |p: P| {
            tracks.iter().enumerate().any(|(j, o)| {
                j != k
                    && o.net == t.net
                    && o.layer == t.layer
                    && o.points
                        .windows(2)
                        .any(|w| geom::point_segment_distance(p, w[0], w[1]) < 1e-3)
            })
        };
        let whole = t.points.len() == 2 && from <= 1e-6 && to >= len - 1e-6;
        let moves_end = !whole
            && ((s == 0 && from <= 1e-6 && !on_centre(at(to)))
                || (s + 2 == t.points.len() && to >= len - 1e-6 && !on_centre(at(from))));
        if moves_end {
            skip.push((k, s));
            continue;
        }
        let mut head: Vec<P> = t.points[..=s].to_vec();
        if from > 1e-6 {
            head.push(at(from));
        }
        let mut tail: Vec<P> = if to < len - 1e-6 { vec![at(to)] } else { Vec::new() };
        tail.extend_from_slice(&t.points[s + 1..]);
        let keep = |p: &Vec<P>| p.len() >= 2 && p.windows(2).any(|w| geom::dist(w[0], w[1]) > 1e-6);
        let mut parts = [head, tail].into_iter().filter(keep);
        match parts.next() {
            Some(p) => tracks[k].points = p,
            None => {
                tracks.remove(k);
            }
        }
        for p in parts {
            tracks.push(RoutedTrack { points: p, ..t.clone() });
        }
        cut += 1;
    }
    tracks.retain(|t| t.points.windows(2).any(|w| geom::dist(w[0], w[1]) > 1e-6));
    cut
}

fn doubled_except(
    tracks: &[RoutedTrack],
    width_of: &dyn Fn(&RoutedTrack) -> f64,
    skip: &[(usize, usize)],
) -> Option<(usize, usize, f64, f64)> {
    let mut groups: std::collections::BTreeMap<(&str, &str), Vec<(usize, usize)>> =
        Default::default();
    for (ti, t) in tracks.iter().enumerate() {
        for k in 0..t.points.len().saturating_sub(1) {
            groups.entry((t.net.as_str(), t.layer.as_str())).or_default().push((ti, k));
        }
    }
    for segs in groups.values() {
        for x in 0..segs.len() {
            for y in x + 1..segs.len() {
                let ((ta, ka), (tb, kb)) = (segs[x], segs[y]);
                if ta == tb && ka.abs_diff(kb) <= 1 {
                    continue;
                }
                let (wa, wb) = (width_of(&tracks[ta]), width_of(&tracks[tb]));
                let seg = |t: usize, k: usize| (tracks[t].points[k], tracks[t].points[k + 1]);
                let ((a0, a1), (b0, b1)) = (seg(ta, ka), seg(tb, kb));
                let (la, lb) = (geom::dist(a0, a1), geom::dist(b0, b1));
                let first = wa < wb - 1e-9 || ((wa - wb).abs() <= 1e-9 && la <= lb);
                let ((t, k, p0, p1), (q0, q1)) =
                    if first { ((ta, ka, a0, a1), (b0, b1)) } else { ((tb, kb, b0, b1), (a0, a1)) };
                if skip.contains(&(t, k)) {
                    continue;
                }
                let Some((overlap, sep)) = parallel(p0, p1, q0, q1) else { continue };
                if sep >= (wa + wb) / 2.0 - 1e-6 || overlap <= wa.max(wb) {
                    continue;
                }
                let len = geom::dist(p0, p1);
                let u = [(p1[0] - p0[0]) / len, (p1[1] - p0[1]) / len];
                let proj = |p: P| (p[0] - p0[0]) * u[0] + (p[1] - p0[1]) * u[1];
                let (s0, s1) = (proj(q0).min(proj(q1)), proj(q0).max(proj(q1)));
                return Some((t, k, s0.max(0.0), s1.min(len)));
            }
        }
    }
    None
}

fn parallel(a0: P, a1: P, b0: P, b1: P) -> Option<(f64, f64)> {
    let la = geom::dist(a0, a1);
    let lb = geom::dist(b0, b1);
    if la < 1e-9 || lb < 1e-9 {
        return None;
    }
    let u = [(a1[0] - a0[0]) / la, (a1[1] - a0[1]) / la];
    if (u[0] * (b1[1] - b0[1]) - u[1] * (b1[0] - b0[0])).abs() / lb > 0.02 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn track(points: &[P], width: f64) -> RoutedTrack {
        RoutedTrack {
            net: "N".into(),
            layer: "F.Cu".into(),
            width: Some(width),
            points: points.to_vec(),
        }
    }

    #[test]
    fn doubled_run_is_cut_from_the_narrower_track() {
        let mut tracks =
            vec![track(&[[0.0, 0.0], [10.0, 0.0]], 0.3), track(&[[2.0, 0.05], [5.0, 0.05]], 0.2)];
        let width = |t: &RoutedTrack| t.width.unwrap();
        assert_eq!(cut_doubled(&mut tracks, &width), 1);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].points, vec![[0.0, 0.0], [10.0, 0.0]]);
        assert!(doubled_except(&tracks, &width, &[]).is_none());
    }

    #[test]
    fn partial_overlap_keeps_the_free_ends() {
        let mut tracks = vec![
            track(&[[0.0, 0.0], [10.0, 0.0]], 0.2),
            track(&[[8.0, 1.0], [8.0, 0.1], [12.0, 0.1], [12.0, 2.0]], 0.2),
        ];
        let width = |t: &RoutedTrack| t.width.unwrap();
        assert_eq!(cut_doubled(&mut tracks, &width), 1);
        assert!(doubled_except(&tracks, &width, &[]).is_none());
        let ends: Vec<P> =
            tracks.iter().flat_map(|t| [t.points[0], *t.points.last().unwrap()]).collect();
        assert!(
            ends.contains(&[0.0, 0.0]) && ends.contains(&[12.0, 2.0]) && ends.contains(&[8.0, 1.0])
        );
    }
}

pub fn trim_pours(model: &mut Model, report: &mut PhaseReport) {
    let pours: Vec<(String, String, P, P)> = model
        .file
        .zones
        .iter()
        .filter(|z| z.priority == Some(crate::planes::PIN_POUR_PRIORITY))
        .filter_map(|z| {
            let o = z.outline.as_ref()?;
            let mut lo = [f64::MAX; 2];
            let mut hi = [f64::MIN; 2];
            for q in o {
                let q = q.to_mm();
                lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                hi = [hi[0].max(q[0]), hi[1].max(q[1])];
            }
            Some(z.layers.iter().map(move |l| (z.net.clone(), l.clone(), lo, hi)))
        })
        .flatten()
        .collect();
    if pours.is_empty() {
        return;
    }
    let Some(plan) = model.detail.as_mut() else { return };
    let nets = &model.layout.nets;
    let mut out: Vec<RoutedTrack> = Vec::new();
    let mut trimmed = 0;
    for t in plan.tracks.drain(..) {
        let h = t.width.unwrap_or_else(|| {
            nets.iter().find(|n| n.name == t.net).map(|n| n.width).unwrap_or(0.1)
        }) / 2.0;
        let mine: Vec<(P, P)> = pours
            .iter()
            .filter(|p| p.0 == t.net && p.1 == t.layer)
            .map(|p| ([p.2[0] + h, p.2[1] + h], [p.3[0] - h, p.3[1] - h]))
            .filter(|(lo, hi)| lo[0] <= hi[0] && lo[1] <= hi[1])
            .collect();
        if mine.is_empty() {
            out.push(t);
            continue;
        }
        let pieces = outside(&t.points, &mine);
        if pieces.len() == 1 && pieces[0] == t.points {
            out.push(t);
            continue;
        }
        trimmed += 1;
        out.extend(pieces.into_iter().map(|points| RoutedTrack { points, ..t.clone() }));
    }
    plan.tracks = out;
    if trimmed > 0 {
        report.notes.push(format!("{trimmed} tracks cut back to the pin pour they end on"));
        report.changed = true;
    }
}

fn clip(a: P, b: P, lo: P, hi: P) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    let d = [b[0] - a[0], b[1] - a[1]];
    for k in 0..2 {
        if d[k].abs() < 1e-12 {
            if a[k] < lo[k] || a[k] > hi[k] {
                return None;
            }
            continue;
        }
        let (u, v) = ((lo[k] - a[k]) / d[k], (hi[k] - a[k]) / d[k]);
        t0 = t0.max(u.min(v));
        t1 = t1.min(u.max(v));
    }
    (t0 <= t1).then_some((t0, t1))
}

fn outside(points: &[P], rects: &[(P, P)]) -> Vec<Vec<P>> {
    let mut pieces: Vec<Vec<P>> = Vec::new();
    let mut cur: Vec<P> = Vec::new();
    let at = |a: P, b: P, t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let mut cover: Vec<(f64, f64)> =
            rects.iter().filter_map(|r| clip(a, b, r.0, r.1)).collect();
        cover.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for c in cover {
            match merged.last_mut() {
                Some(m) if c.0 <= m.1 + 1e-9 => m.1 = m.1.max(c.1),
                _ => merged.push(c),
            }
        }
        let mut t = 0.0;
        for (s, e) in merged {
            if s > t + 1e-9 {
                if cur.is_empty() {
                    cur.push(at(a, b, t));
                }
                cur.push(at(a, b, s));
                pieces.push(std::mem::take(&mut cur));
            } else if !cur.is_empty() {
                cur.push(at(a, b, s.max(t)));
                pieces.push(std::mem::take(&mut cur));
            }
            t = t.max(e);
        }
        if t < 1.0 - 1e-9 {
            if cur.is_empty() {
                cur.push(at(a, b, t));
            }
            cur.push(b);
        }
    }
    if cur.len() >= 2 {
        pieces.push(cur);
    }
    pieces
        .into_iter()
        .map(|p| {
            let mut q: Vec<P> = Vec::new();
            for x in p {
                if q.last().is_none_or(|l| geom::dist(*l, x) > 1e-9) {
                    q.push(x);
                }
            }
            q
        })
        .filter(|p| {
            p.len() >= 2 && p.windows(2).map(|w| geom::dist(w[0], w[1])).sum::<f64>() > 1e-6
        })
        .collect()
}

#[cfg(test)]
mod pour_tests {
    use super::outside;

    #[test]
    fn track_ending_in_a_pour_stops_at_its_edge() {
        let pieces = outside(&[[0.0, 0.0], [0.0, 5.0]], &[([-1.0, 3.0], [1.0, 6.0])]);
        assert_eq!(pieces, vec![vec![[0.0, 0.0], [0.0, 3.0]]]);
    }

    #[test]
    fn track_inside_a_pour_goes() {
        assert!(outside(&[[0.0, 3.5], [0.0, 5.0]], &[([-1.0, 3.0], [1.0, 6.0])]).is_empty());
    }

    #[test]
    fn track_across_a_pour_splits_in_two() {
        let pieces = outside(&[[-3.0, 4.0], [3.0, 4.0]], &[([-1.0, 3.0], [1.0, 6.0])]);
        assert_eq!(pieces, vec![vec![[-3.0, 4.0], [-1.0, 4.0]], vec![[1.0, 4.0], [3.0, 4.0]]]);
    }

    #[test]
    fn track_clear_of_pours_is_kept() {
        let pts = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]];
        assert_eq!(outside(&pts, &[([5.0, 5.0], [6.0, 6.0])]), vec![pts]);
    }
}

const LOCAL: f64 = 3.0;

enum Item {
    Seg(usize, P, P, f64),
    Via(Vec<usize>, P, f64),
    Pad(Vec<usize>, Vec<Vec<P>>),
}

fn touches(a: &Item, b: &Item) -> bool {
    touches_how(a, b, false)
}

fn touches_how(a: &Item, b: &Item, loose: bool) -> bool {
    let poly_gap = |pts: &[P], outlines: &[Vec<P>]| {
        outlines
            .iter()
            .map(|o| {
                if pts.iter().any(|p| geom::point_in_polygon(*p, o)) {
                    0.0
                } else {
                    geom::polyline_polygon_distance(pts, o)
                }
            })
            .fold(f64::MAX, f64::min)
    };
    match (a, b) {
        (Item::Seg(la, a0, a1, ha), Item::Seg(lb, b0, b1, hb)) => {
            la == lb
                && if loose {
                    geom::segment_segment_distance(*a0, *a1, *b0, *b1) <= ha + hb + 1e-6
                } else {
                    [*a0, *a1].iter().any(|p| geom::point_segment_distance(*p, *b0, *b1) < 1e-3)
                        || [*b0, *b1]
                            .iter()
                            .any(|p| geom::point_segment_distance(*p, *a0, *a1) < 1e-3)
                }
        }
        (Item::Seg(l, s0, s1, h), Item::Via(ls, c, r))
        | (Item::Via(ls, c, r), Item::Seg(l, s0, s1, h)) => {
            ls.contains(l)
                && if loose {
                    geom::point_segment_distance(*c, *s0, *s1) <= h + r + 1e-6
                } else {
                    geom::point_segment_distance(*c, *s0, *s1) < 1e-3
                }
        }
        (Item::Seg(l, s0, s1, h), Item::Pad(ls, o))
        | (Item::Pad(ls, o), Item::Seg(l, s0, s1, h)) => {
            ls.contains(l) && poly_gap(&[*s0, *s1], o) <= h + 1e-6
        }
        (Item::Via(la, a, ra), Item::Via(lb, b, rb)) => {
            la.iter().any(|l| lb.contains(l)) && geom::dist(*a, *b) <= ra + rb + 1e-6
        }
        (Item::Via(lv, c, r), Item::Pad(lp, o)) | (Item::Pad(lp, o), Item::Via(lv, c, r)) => {
            lv.iter().any(|l| lp.contains(l)) && poly_gap(&[*c, *c], o) <= r + 1e-6
        }
        (Item::Pad(..), Item::Pad(..)) => false,
    }
}

pub fn drop_fragments(model: &mut Model, report: &mut PhaseReport) {
    let l = &model.layout;
    let Some(plan) = model.detail.as_mut() else { return };
    let layer_of = |n: &str| l.copper.iter().position(|c| c == n);
    let half_of = |t: &RoutedTrack| {
        t.width.unwrap_or_else(|| l.nets.iter().find(|n| n.name == t.net).map_or(0.1, |n| n.width))
            / 2.0
    };
    let mut dropped = 0;
    let mut k = 0;
    while k < plan.tracks.len() {
        let t = &plan.tracks[k];
        let h = half_of(t);
        let len: f64 = t.points.windows(2).map(|w| geom::dist(w[0], w[1])).sum();
        let (Some(layer), Some(net)) =
            (layer_of(&t.layer), l.nets.iter().position(|n| n.name == t.net))
        else {
            k += 1;
            continue;
        };
        let poured = l
            .zones
            .iter()
            .any(|z| z.net == net && z.layer == t.layer && t.points.iter().any(|p| z.filled(*p)));
        if len > 2.0 * h || poured {
            k += 1;
            continue;
        }
        let mut lo = [f64::MAX; 2];
        let mut hi = [f64::MIN; 2];
        for p in &t.points {
            lo = [lo[0].min(p[0] - LOCAL), lo[1].min(p[1] - LOCAL)];
            hi = [hi[0].max(p[0] + LOCAL), hi[1].max(p[1] + LOCAL)];
        }
        let near = |p: P| p[0] >= lo[0] && p[0] <= hi[0] && p[1] >= lo[1] && p[1] <= hi[1];
        let mut items: Vec<Item> = Vec::new();
        for (j, o) in plan.tracks.iter().enumerate() {
            if j == k || o.net != t.net || !o.points.iter().any(|p| near(*p)) {
                continue;
            }
            let Some(ol) = layer_of(&o.layer) else { continue };
            let oh = half_of(o);
            for w in o.points.windows(2) {
                items.push(Item::Seg(ol, w[0], w[1], oh));
            }
            if o.points.len() == 1 {
                items.push(Item::Seg(ol, o.points[0], o.points[0], oh));
            }
        }
        for v in l.vias.iter().filter(|v| v.net == net && near(v.at)) {
            let ls = v.layers.iter().filter_map(|x| layer_of(x)).collect();
            items.push(Item::Via(ls, v.at, v.diameter / 2.0));
        }
        for q in l.parts.iter().flat_map(|p| &p.pads).filter(|q| q.net == Some(net)) {
            if !q.outlines.iter().flatten().any(|p| near(*p)) {
                continue;
            }
            let ls = q.copper.iter().filter_map(|x| layer_of(x)).collect();
            items.push(Item::Pad(ls, q.outlines.clone()));
        }
        let mine: Vec<Item> =
            t.points.windows(2).map(|w| Item::Seg(layer, w[0], w[1], h)).collect();
        let neighbours: Vec<usize> = (0..items.len())
            .filter(|&i| mine.iter().any(|m| touches_how(m, &items[i], true)))
            .collect();
        let mut root: Vec<usize> = (0..items.len()).collect();
        fn find(r: &mut [usize], i: usize) -> usize {
            let mut x = i;
            while r[x] != x {
                r[x] = r[r[x]];
                x = r[x];
            }
            x
        }
        if neighbours.len() > 1 {
            for i in 0..items.len() {
                for j in i + 1..items.len() {
                    if find(&mut root, i) != find(&mut root, j) && touches(&items[i], &items[j]) {
                        let (a, b) = (find(&mut root, i), find(&mut root, j));
                        root[a] = b;
                    }
                }
            }
        }
        let first = neighbours.first().map(|&i| find(&mut root, i));
        let joined = neighbours.iter().all(|&i| Some(find(&mut root, i)) == first);
        if joined {
            plan.tracks.remove(k);
            dropped += 1;
        } else {
            k += 1;
        }
    }
    if dropped > 0 {
        report.notes.push(format!("{dropped} short track fragments dropped, nothing they joined"));
        report.changed = true;
    }
}

fn closest_on(p: P, a: P, b: P) -> P {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    if l2 < 1e-12 {
        return a;
    }
    let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0);
    [a[0] + d[0] * t, a[1] + d[1] * t]
}

pub fn close_joints(model: &mut Model, cfg: &EngineFile, report: &mut PhaseReport) -> bool {
    if model.ensure_base(&crate::detail::options(cfg)).is_err() {
        return false;
    }
    let base = model.base.as_ref().expect("base is built");
    let l = &model.layout;
    let Some(plan) = model.detail.as_mut() else { return false };
    let half_of = |t: &RoutedTrack| {
        t.width.unwrap_or_else(|| l.nets.iter().find(|n| n.name == t.net).map_or(0.1, |n| n.width))
            / 2.0
    };
    let end_gap = |e: P, b: &RoutedTrack| {
        b.points
            .windows(2)
            .map(|w| geom::point_segment_distance(e, w[0], w[1]))
            .fold(f64::MAX, f64::min)
    };
    let debug = std::env::var("AGENTEE_JOINT_DEBUG").is_ok();
    let mut closed = 0;
    for _ in 0..4 {
        let before = closed;
        for a in 0..plan.tracks.len() {
            let ta = plan.tracks[a].clone();
            let ha = half_of(&ta);
            if ta.points.len() < 2 {
                continue;
            }
            let Some(net) = l.nets.iter().position(|n| n.name == ta.net) else { continue };
            let same: Vec<usize> = (0..plan.tracks.len())
                .filter(|&j| {
                    j != a && plan.tracks[j].net == ta.net && plan.tracks[j].layer == ta.layer
                })
                .collect();
            for (front, e) in [(true, ta.points[0]), (false, *ta.points.last().unwrap())] {
                let mut targets: Vec<P> = l
                    .vias
                    .iter()
                    .filter(|v| {
                        v.net == net
                            && v.layers.iter().any(|c| *c == ta.layer)
                            && geom::dist(v.at, e) <= ha + v.diameter / 2.0 + 1e-6
                    })
                    .map(|v| v.at)
                    .collect();
                targets.extend(
                    l.parts
                        .iter()
                        .flat_map(|p| &p.pads)
                        .filter(|q| {
                            q.net == Some(net)
                                && q.copper.iter().any(|c| *c == ta.layer)
                                && q.outlines.iter().any(|o| {
                                    geom::point_in_polygon(e, o)
                                        || geom::polyline_polygon_distance(&[e, e], o) <= ha + 1e-6
                                })
                        })
                        .filter_map(|q| {
                            let mut b = agentee_core::graphic::Bounds::EMPTY;
                            q.outlines.iter().flatten().for_each(|r| b.add(*r));
                            if b.is_empty() {
                                return None;
                            }
                            let c = b.center();
                            let reach = |k: usize| ((b.max[k] - b.min[k]) / 2.0 - ha).max(0.0);
                            Some([
                                e[0].clamp(c[0] - reach(0), c[0] + reach(0)),
                                e[1].clamp(c[1] - reach(1), c[1] + reach(1)),
                            ])
                        }),
                );
                let on_fixed = targets.iter().any(|c| geom::dist(*c, e) < 1e-3);
                let in_pad = l.parts.iter().flat_map(|p| &p.pads).any(|q| {
                    q.net == Some(net)
                        && q.copper.iter().any(|c| *c == ta.layer)
                        && q.outlines.iter().any(|o| {
                            geom::point_in_polygon(e, o)
                                && (0..o.len())
                                    .map(|k| {
                                        geom::point_segment_distance(e, o[k], o[(k + 1) % o.len()])
                                    })
                                    .fold(f64::MAX, f64::min)
                                    >= ha - 1e-6
                        })
                });
                if on_fixed || in_pad {
                    continue;
                }
                let mut best: Option<(f64, P, f64)> = targets
                    .iter()
                    .map(|c| (geom::dist(*c, e), *c, ha))
                    .min_by(|x, y| x.0.total_cmp(&y.0));
                if best.is_none() {
                    let mut on_centre = false;
                    for &b in &same {
                        let tb = &plan.tracks[b];
                        let hb = half_of(tb);
                        let gap = end_gap(e, tb);
                        if gap < 1e-3 {
                            on_centre = true;
                            break;
                        }
                        if gap > ha + hb + 1e-6 {
                            continue;
                        }
                        for w in tb.points.windows(2) {
                            let q = closest_on(e, w[0], w[1]);
                            let d = geom::dist(q, e);
                            if best.is_none_or(|b| d < b.0) {
                                best = Some((d, q, hb));
                            }
                        }
                    }
                    if on_centre {
                        continue;
                    }
                }
                let Some((_, q, hq)) = best else { continue };
                let own = ta
                    .points
                    .windows(2)
                    .any(|w| geom::point_segment_distance(q, w[0], w[1]) < 1e-3);
                if own {
                    continue;
                }
                let probe = |w: f64| {
                    let mut trial = plan.tracks.clone();
                    trial.push(RoutedTrack { points: vec![e, q], width: Some(w), ..ta.clone() });
                    let k = trial.len() - 1;
                    crate::negotiate::illegal(l, base, &trial, &[k]).is_empty()
                };
                if probe(2.0 * ha) {
                    if front {
                        plan.tracks[a].points.insert(0, q);
                    } else {
                        plan.tracks[a].points.push(q);
                    }
                    closed += 1;
                    continue;
                }
                if hq < ha - 1e-9 && probe(2.0 * hq) {
                    plan.tracks.push(RoutedTrack {
                        points: vec![e, q],
                        width: Some(2.0 * hq),
                        ..ta.clone()
                    });
                    closed += 1;
                    continue;
                }
                if debug {
                    eprintln!(
                        "joint refused {} {} {:?} -> {:?} w {}",
                        ta.net,
                        ta.layer,
                        e,
                        q,
                        2.0 * ha
                    );
                }
            }
        }
        if closed == before {
            break;
        }
    }
    if closed > 0 {
        report.notes.push(format!("{closed} track ends extended onto the centre they met"));
    }
    closed > 0
}

pub fn widen(model: &mut Model, cfg: &EngineFile, report: &mut PhaseReport) -> bool {
    if model.ensure_base(&crate::detail::options(cfg)).is_err() {
        return false;
    }
    let base = model.base.as_ref().expect("base is built");
    let grid = &base.grid;
    let l = &model.layout;
    let Some(plan) = model.detail.as_mut() else { return false };
    let step = grid.g * 0.5;
    let mut widened = 0;
    let mut extra: Vec<RoutedTrack> = Vec::new();
    for t in plan.tracks.iter_mut() {
        let Some(w) = t.width else { continue };
        let (Some(net), Some(layer)) = (
            l.nets.iter().position(|n| n.name == t.net),
            l.copper.iter().position(|c| *c == t.layer),
        ) else {
            continue;
        };
        let Some(rule) = base.rules.nets.get(net).and_then(|r| r.as_ref()) else { continue };
        let full = rule.width[layer];
        if w >= full - 1e-9 || t.points.len() < 2 {
            continue;
        }
        let room = |p: P| {
            let (d, q) = grid.exact(layer, p, net as u16);
            (d - rule.clearance).min(q) - 0.002
        };
        let fits = |p: P| room(p) >= full / 2.0;
        let pts = t.points.clone();
        let samples = |a: P, b: P| {
            let n = (geom::dist(a, b) / step).ceil().max(1.0) as usize;
            (0..=n).map(move |k| {
                let f = k as f64 / n as f64;
                [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
            })
        };
        let last = pts.len() - 1;
        if !fits(pts[last]) {
            continue;
        }
        let mut cut: Option<(usize, P)> = None;
        'scan: for s in 0..last {
            for p in samples(pts[s], pts[s + 1]) {
                if fits(p) && samples(p, pts[last]).all(fits) {
                    let tail: Vec<P> =
                        std::iter::once(p).chain(pts[s + 1..].iter().copied()).collect();
                    let ok = tail.windows(2).all(|v| samples(v[0], v[1]).all(fits));
                    if ok {
                        cut = Some((s, p));
                        break 'scan;
                    }
                }
            }
        }
        let Some((s, p)) = cut else { continue };
        if geom::dist(p, pts[last]) < grid.g {
            continue;
        }
        let mut wide: Vec<P> = vec![p];
        wide.extend(pts[s + 1..].iter().copied());
        let mut narrow: Vec<P> = pts[..=s].to_vec();
        narrow.push(p);
        let narrow_len: f64 = narrow.windows(2).map(|v| geom::dist(v[0], v[1])).sum();
        extra.push(RoutedTrack { points: wide, width: None, ..t.clone() });
        if narrow_len < 1e-6 {
            t.points.clear();
        } else {
            t.points = narrow;
        }
        widened += 1;
    }
    plan.tracks.retain(|t| t.points.len() >= 2);
    plan.tracks.extend(extra);
    if widened > 0 {
        report.notes.push(format!("{widened} necks shortened to where the class width fits"));
    }
    widened > 0
}

const SINK_REACH: f64 = 1.0;
const SINK_STEP: f64 = 0.025;
const SINK_DEPTH: f64 = 0.1;

pub fn sink_into_pours(model: &mut Model, cfg: &EngineFile, report: &mut PhaseReport) -> bool {
    if model.ensure_base(&crate::detail::options(cfg)).is_err() {
        return false;
    }
    let base = model.base.as_ref().expect("base is built");
    let l = &model.layout;
    let Some(plan) = model.detail.as_mut() else { return false };
    let mut sunk = 0;
    for a in 0..plan.tracks.len() {
        let ta = plan.tracks[a].clone();
        if ta.points.len() < 2 {
            continue;
        }
        let Some(net) = l.nets.iter().position(|n| n.name == ta.net) else { continue };
        let fills: Vec<&agentee_core::layout::ZoneFill> = l
            .zones
            .iter()
            .filter(|z| z.net == net && z.layer == ta.layer && z.mask.len() >= z.width * z.height)
            .collect();
        if fills.is_empty() {
            continue;
        }
        let h = ta.width.unwrap_or(l.nets[net].width) / 2.0;
        let filled = |p: P| fills.iter().any(|z| z.filled(p));
        let deep = |p: P| {
            let r = h + SINK_DEPTH.max(h);
            filled(p)
                && (0..16).all(|k| {
                    let t = k as f64 * std::f64::consts::TAU / 16.0;
                    filled([p[0] + r * t.cos(), p[1] + r * t.sin()])
                })
        };
        let n = ta.points.len();
        for front in [true, false] {
            let (e, prev) = if front {
                (ta.points[0], ta.points[1])
            } else {
                (ta.points[n - 1], ta.points[n - 2])
            };
            let touches = (0..16).any(|k| {
                let t = k as f64 * std::f64::consts::TAU / 16.0;
                filled([e[0] + h * t.cos(), e[1] + h * t.sin()])
            }) || filled(e);
            if !touches || deep(e) {
                continue;
            }
            let joined =
                l.vias.iter().any(|v| v.net == net && geom::dist(v.at, e) <= h + v.diameter / 2.0)
                    || l.parts.iter().flat_map(|p| &p.pads).any(|q| {
                        q.net == Some(net)
                            && q.copper.iter().any(|c| *c == ta.layer)
                            && q.outlines.iter().any(|o| {
                                geom::point_in_polygon(e, o)
                                    || geom::polyline_polygon_distance(&[e, e], o) <= h
                            })
                    })
                    || plan.tracks.iter().enumerate().any(|(j, o)| {
                        j != a
                            && o.net == ta.net
                            && o.layer == ta.layer
                            && o.points
                                .windows(2)
                                .any(|w| geom::point_segment_distance(e, w[0], w[1]) < 1e-3)
                    });
            if joined {
                continue;
            }
            let len = geom::dist(e, prev);
            if len < 1e-9 {
                continue;
            }
            let u = [(e[0] - prev[0]) / len, (e[1] - prev[1]) / len];
            let mut dirs: Vec<P> = vec![u];
            for k in 0..16 {
                let t = k as f64 * std::f64::consts::TAU / 16.0;
                dirs.push([t.cos(), t.sin()]);
            }
            let mut best: Option<P> = None;
            'search: for s in 1..=(SINK_REACH / SINK_STEP) as usize {
                for d in &dirs {
                    let q =
                        [e[0] + d[0] * s as f64 * SINK_STEP, e[1] + d[1] * s as f64 * SINK_STEP];
                    if !deep(q) {
                        continue;
                    }
                    let mut trial = plan.tracks.clone();
                    trial.push(RoutedTrack { points: vec![e, q], ..ta.clone() });
                    let k = trial.len() - 1;
                    if crate::negotiate::illegal(l, base, &trial, &[k]).is_empty() {
                        best = Some(q);
                        break 'search;
                    }
                }
            }
            let Some(q) = best else { continue };
            if front {
                plan.tracks[a].points.insert(0, q);
            } else {
                plan.tracks[a].points.push(q);
            }
            sunk += 1;
        }
    }
    if sunk > 0 {
        report.notes.push(format!("{sunk} track ends run into the pour they join"));
    }
    sunk > 0
}

fn tidy(points: &[P]) -> Vec<P> {
    let mut out: Vec<P> = Vec::new();
    for &q in points {
        if out.last().is_some_and(|l| geom::dist(*l, q) < 1e-6) {
            continue;
        }
        while out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let cross = (b[0] - a[0]) * (q[1] - b[1]) - (b[1] - a[1]) * (q[0] - b[0]);
            let dot = (b[0] - a[0]) * (q[0] - b[0]) + (b[1] - a[1]) * (q[1] - b[1]);
            if cross.abs() > 1e-9 || dot < 0.0 {
                break;
            }
            out.pop();
        }
        out.push(q);
    }
    let on = |p: P, run: &[P]| {
        run.windows(2).any(|w| geom::point_segment_distance(p, w[0], w[1]) < 1e-6)
    };
    while out.len() > 2 && on(out[out.len() - 1], &out[..out.len() - 1]) {
        out.pop();
    }
    while out.len() > 2 && on(out[0], &out[1..]) {
        out.remove(0);
    }
    out
}

pub fn tidy_tracks(model: &mut Model, report: &mut PhaseReport) -> bool {
    let Some(plan) = model.detail.as_mut() else { return false };
    let mut changed = 0;
    for t in plan.tracks.iter_mut() {
        let new = tidy(&t.points);
        if new != t.points {
            t.points = new;
            changed += 1;
        }
    }
    plan.tracks.retain(|t| t.points.len() >= 2);
    if changed > 0 {
        report.notes.push(format!("{changed} tracks tidied: repeated points and fold-backs"));
    }
    changed > 0
}

#[cfg(test)]
mod tidy_tests {
    use super::tidy;

    #[test]
    fn fold_back_end_is_dropped() {
        let t = tidy(&[[0.0, 35.85], [0.0, 35.75], [0.0, 35.7], [0.0, 35.85]]);
        assert_eq!(t, vec![[0.0, 35.85], [0.0, 35.7]]);
    }

    #[test]
    fn corner_is_kept() {
        let pts = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        assert_eq!(tidy(&pts), pts);
    }

    #[test]
    fn collinear_points_merge() {
        assert_eq!(tidy(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]), vec![[0.0, 0.0], [2.0, 0.0]]);
    }
}
