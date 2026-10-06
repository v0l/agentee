use super::{Category, Ctx, Report, Rule, every};
use crate::diag::Severity;
use crate::geom;
use crate::graphic::Bounds;
use crate::units::Length;
use std::collections::BTreeMap;

pub static RULES: &[Rule] = &[Rule {
    id: "mask-web",
    category: Category::Mask,
    severity: Severity::Error,
    summary: "pads of different nets whose mask openings leave less than min_mask_web between them; a footprint with `mask_web = false` is skipped within itself",
    when: "every board",
    applies: every,
    check: mask_web,
}];

fn mask_web(cx: &Ctx, r: &mut Report) {
    let parts = cx.parts;
    let web = cx.board.rules.min_mask_web.to_mm();
    for layer in ["F.Mask", "B.Mask"] {
        let mut openings: Vec<(usize, usize, Bounds)> = Vec::new();
        for (pi, p) in parts.iter().enumerate() {
            for (k, pad) in p.pads.iter().enumerate() {
                if pad.mask.iter().any(|m| m == layer) {
                    let mut b = Bounds::EMPTY;
                    pad.outlines.iter().flatten().for_each(|q| b.add(*q));
                    if !b.is_empty() {
                        openings.push((pi, k, b));
                    }
                }
            }
        }
        let mut found: BTreeMap<(usize, usize), (usize, String)> = BTreeMap::new();
        for (i, (pa, ka, ba)) in openings.iter().enumerate() {
            let a = &parts[*pa].pads[*ka];
            let mut grown = *ba;
            grown.add([ba.min[0] - web, ba.min[1] - web]);
            grown.add([ba.max[0] + web, ba.max[1] + web]);
            for (pb, kb, bb) in &openings[i + 1..] {
                let b = &parts[*pb].pads[*kb];
                if (a.net.is_some() && a.net == b.net)
                    || (pa == pb && !parts[*pa].footprint.mask_web)
                    || (pa == pb && parts[*pa].footprint.spark_gap(&a.number, &b.number).is_some())
                    || !(grown.overlaps(bb) || grown.contains(bb) || bb.contains(&grown))
                {
                    continue;
                }
                let gap = a
                    .outlines
                    .iter()
                    .flat_map(|o| b.outlines.iter().map(move |q| geom::polygon_distance(o, q)))
                    .fold(f64::MAX, f64::min);
                if gap + 1e-6 >= web {
                    continue;
                }
                let (ca, cb) = (ba.center(), bb.center());
                let what = if gap <= 0.0 {
                    "overlaps".to_string()
                } else {
                    format!("leaves {} to", Length::mm(gap))
                };
                let entry = found.entry((*pa, *pb)).or_insert_with(|| {
                    (
                        0,
                        format!(
                            "{}.{} {what} {}.{} at [{:.3}, {:.3}]",
                            parts[*pa].reference,
                            a.number,
                            parts[*pb].reference,
                            b.number,
                            (ca[0] + cb[0]) / 2.0,
                            (ca[1] + cb[1]) / 2.0
                        ),
                    )
                });
                entry.0 += 1;
            }
        }
        for ((pa, pb), (count, first)) in found {
            let whom = if pa == pb {
                format!("part {}", parts[pa].reference)
            } else {
                format!("parts {} and {}", parts[pa].reference, parts[pb].reference)
            };
            r.emit(
                format!("{whom} {layer}"),
                format!(
                    "{count} pad pairs of different nets leave less than the {} mask web, first: \
                     {first}; openings follow the pad outlines, narrow the pads or move them apart",
                    Length::mm(web)
                ),
            );
        }
    }
}
