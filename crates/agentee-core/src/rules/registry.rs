use super::{Context, Rule, Violation};

pub struct Registered(pub &'static crate::drc::Rule);

impl Rule for Registered {
    fn id(&self) -> &'static str {
        self.0.id
    }

    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>) {
        let mut runs: Vec<Vec<crate::diag::Diagnostic>> = Vec::new();
        cx.layouts(&mut |ctx, _| runs.push(crate::drc::messages(self.0, ctx)));
        let (before, now) = match runs.as_slice() {
            [now] => (&[][..], now),
            [before, now, ..] => (&before[..], now),
            [] => return,
        };
        for d in now {
            if before.iter().any(|b| b.at == d.at && b.message == d.message) {
                continue;
            }
            out.push(Violation {
                rule: self.0.id,
                group: d.at.clone(),
                detail: d.message.clone(),
                ..Default::default()
            });
        }
    }
}

pub fn everything<C: Context>(cx: &C, out: &mut Vec<Violation>) {
    for rule in crate::drc::registry() {
        Registered(rule).eval(cx, out);
    }
}
