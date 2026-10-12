use super::{Context, Rule, Violation};
use crate::drc::Owner;

pub struct KeepoutArea;

impl Rule for KeepoutArea {
    fn id(&self) -> &'static str {
        "keepout"
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        if cx.keepouts().is_empty() {
            return;
        }
        for i in cx.item_subjects(0.0) {
            let planned = cx.planned_item(i);
            let c = cx.item(i);
            let routed = matches!(c.owner, Owner::Track(_) | Owner::Via(_));
            if !(routed || planned && matches!(c.owner, Owner::Pad(..))) {
                continue;
            }
            if !cx.counts(planned, false) {
                continue;
            }
            for (k, area) in cx.keepouts().iter().enumerate() {
                if !area.bars(c.net, &c.layers) || !area.reaches(&c.shape) {
                    continue;
                }
                let net = c.net.map(|n| cx.nets()[n].name.as_str()).unwrap_or("no net");
                let layers: Vec<String> =
                    c.layers.iter().filter(|l| area.layers.contains(l)).cloned().collect();
                out.push(Violation {
                    rule: self.id(),
                    group: format!("keepouts[{k}]"),
                    subject: cx.describe(i),
                    other: format!("keepouts[{k}]"),
                    detail: format!(
                        "enters keepouts[{k}] on {}; route it around, or list {net} in the keepout's `nets`",
                        layers.join(", ")
                    ),
                    at: c.bounds.center(),
                    layers,
                    nets: c.net.map(|n| (n, n)),
                    ..Default::default()
                });
            }
        }
    }
}
