use super::{Context, Rule, Violation};
use crate::drc::{CuShape, Owner};
use crate::layout::parallel_overlap;

pub struct TrackOverlap;

fn segment<C: Context>(cx: &C, i: usize) -> Option<(usize, crate::geom::P, crate::geom::P, f64)> {
    match (cx.item(i).owner, &cx.item(i).shape) {
        (Owner::Track(t), CuShape::Seg(a, b, h)) => Some((t, *a, *b, h * 2.0)),
        _ => None,
    }
}

fn index_in_track<C: Context>(cx: &C, i: usize) -> usize {
    let owner = cx.item(i).owner;
    (1..=i).take_while(|k| cx.item(i - k).owner == owner).count()
}

fn name<C: Context>(cx: &C, i: usize, t: usize) -> String {
    if cx.planned_item(i) { cx.describe(i) } else { format!("tracks[{t}]") }
}

impl Rule for TrackOverlap {
    fn id(&self) -> &'static str {
        "track-overlap"
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        let subjects = cx.item_subjects(0.0);
        let chosen: std::collections::HashSet<usize> = subjects.iter().copied().collect();
        let mut found = Vec::new();
        for &i in &subjects {
            let Some((ti, a0, a1, wa)) = segment(cx, i) else { continue };
            let a = cx.item(i);
            for j in cx.items_near(&a.bounds, 0.0) {
                if j == i || (j < i && chosen.contains(&j)) {
                    continue;
                }
                if !cx.counts(cx.planned_item(i), cx.planned_item(j)) {
                    continue;
                }
                let Some((tj, b0, b1, wb)) = segment(cx, j) else { continue };
                let b = cx.item(j);
                if b.net != a.net || b.layers != a.layers {
                    continue;
                }
                let (lo, hi) = if i < j { (i, j) } else { (j, i) };
                if ti == tj && index_in_track(cx, lo).abs_diff(index_in_track(cx, hi)) <= 1 {
                    continue;
                }
                let (p0, p1, q0, q1) = if i < j { (a0, a1, b0, b1) } else { (b0, b1, a0, a1) };
                let Some((overlap, sep)) = parallel_overlap(p0, p1, q0, q1) else { continue };
                if sep < (wa + wb) / 2.0 - 1e-6 && overlap > wa.max(wb) {
                    found.push((a.net, a.layers[0].clone(), lo, hi, overlap, sep));
                }
            }
        }
        found.sort_by(|x, y| (x.0, &x.1, x.2, x.3).cmp(&(y.0, &y.1, y.2, y.3)));
        let mut seen = std::collections::HashSet::new();
        for (net, layer, lo, hi, overlap, sep) in found {
            let (ta, tb) = (segment(cx, lo).unwrap().0, segment(cx, hi).unwrap().0);
            if !seen.insert((ta.min(tb), ta.max(tb))) {
                continue;
            }
            let net_name = net.map(|n| cx.nets()[n].name.as_str()).unwrap_or("");
            let other = name(cx, hi, tb);
            out.push(Violation {
                rule: self.id(),
                group: format!("{} {net_name}", name(cx, lo, ta)),
                detail: format!(
                    "runs on top of {other} on {layer} for {overlap:.2} mm, {sep:.3} mm apart; the copper is doubled, merge or remove one"
                ),
                subject: name(cx, lo, ta),
                other,
                gap: sep,
                at: cx.item(lo).bounds.center(),
                nets: net.map(|n| (n, n)),
                layers: vec![layer],
                ..Default::default()
            });
        }
    }
}
