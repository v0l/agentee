use super::{Context, Rule, Violation};
use crate::drc::CuShape;

pub struct CopperToEdge;

impl Rule for CopperToEdge {
    fn id(&self) -> &'static str {
        "copper-to-edge"
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        let edge = cx.edge();
        if !edge.is_closed() {
            return;
        }
        let need = cx.board().rules.min_copper_to_edge.to_mm();
        for i in cx.item_subjects(0.0) {
            if !cx.counts(cx.planned_item(i), false) {
                continue;
            }
            let c = cx.item(i);
            let (inside, to_edge, at) = match c.shape {
                CuShape::Seg(a, b, hw) => {
                    let centre = edge.segment_distance(a, b);
                    (edge.contains(a) && edge.contains(b) && centre > 0.0, centre - hw, a)
                }
                CuShape::Circle(o, ro) => (edge.contains(o), edge.distance(o) - ro, o),
                CuShape::Poly(_) => continue,
            };
            if !inside || to_edge + crate::layout::DRC_EPSILON < need {
                out.push(Violation {
                    rule: self.id(),
                    group: "edge".into(),
                    subject: cx.describe(i),
                    other: if inside { "the board edge".into() } else { "off the board".into() },
                    gap: if inside { to_edge } else { f64::NEG_INFINITY },
                    need,
                    at,
                });
            }
        }
    }
}
