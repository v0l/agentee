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
    for _ in 0..64 {
        let Some((k, s, from, to)) = doubled(tracks, width_of) else { break };
        let t = tracks[k].clone();
        let (a, b) = (t.points[s], t.points[s + 1]);
        let len = geom::dist(a, b);
        let at = |d: f64| [a[0] + (b[0] - a[0]) * d / len, a[1] + (b[1] - a[1]) * d / len];
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
    cut
}

fn doubled(
    tracks: &[RoutedTrack],
    width_of: &dyn Fn(&RoutedTrack) -> f64,
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
        assert!(doubled(&tracks, &width).is_none());
    }

    #[test]
    fn partial_overlap_keeps_the_free_ends() {
        let mut tracks = vec![
            track(&[[0.0, 0.0], [10.0, 0.0]], 0.2),
            track(&[[8.0, 1.0], [8.0, 0.1], [12.0, 0.1], [12.0, 2.0]], 0.2),
        ];
        let width = |t: &RoutedTrack| t.width.unwrap();
        assert_eq!(cut_doubled(&mut tracks, &width), 1);
        assert!(doubled(&tracks, &width).is_none());
        let ends: Vec<P> =
            tracks.iter().flat_map(|t| [t.points[0], *t.points.last().unwrap()]).collect();
        assert!(
            ends.contains(&[0.0, 0.0]) && ends.contains(&[12.0, 2.0]) && ends.contains(&[8.0, 1.0])
        );
    }
}
