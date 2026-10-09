use crate::diag::{Diags, Severity};
use crate::schematic::{Net, PinRef, Schematic};
use crate::symbol::PinType;

pub const UNSUPPLIED: &str = "erc-unsupplied";
pub const UNDRIVEN: &str = "erc-undriven";
pub const DRIVE_CONFLICT: &str = "erc-drive-conflict";
pub const TRISTATE: &str = "erc-tristate-contention";
pub const NO_PULL: &str = "erc-no-pull";
pub const NC_PIN: &str = "erc-nc-pin";

struct Pin {
    r: PinRef,
    part: usize,
    kind: PinType,
    capacitor: bool,
}

fn pins(s: &Schematic, n: &Net) -> Vec<Pin> {
    n.pins
        .iter()
        .filter(|r| {
            let p = &s.parts[r.part];
            !p.dnp && !p.symbol.power
        })
        .map(|r| {
            let p = &s.parts[r.part];
            let part = s.parts.iter().position(|q| q.reference == p.reference).unwrap_or(r.part);
            Pin {
                r: *r,
                part,
                kind: p.symbol.pins[r.pin].kind,
                capacitor: p.symbol.reference == "C",
            }
        })
        .collect()
}

fn label(s: &Schematic, r: PinRef) -> String {
    let pin = &s.parts[r.part].symbol.pins[r.pin];
    if pin.name.is_empty() || pin.name == pin.number || pin.name == "~" {
        s.pin_label(r)
    } else {
        format!("{} ({})", s.pin_label(r), pin.name)
    }
}

fn list(s: &Schematic, ps: &[&Pin]) -> String {
    ps.iter().map(|p| label(s, p.r)).collect::<Vec<_>>().join(", ")
}

fn push_pull(k: PinType) -> bool {
    matches!(k, PinType::Output | PinType::PowerOut)
}

fn open(k: PinType) -> bool {
    matches!(k, PinType::OpenCollector | PinType::OpenEmitter)
}

fn listens(k: PinType) -> bool {
    matches!(k, PinType::Input | PinType::PowerIn | PinType::NoConnect)
}

fn parts_of(ps: &[&Pin]) -> usize {
    let mut v: Vec<usize> = ps.iter().map(|p| p.part).collect();
    v.sort_unstable();
    v.dedup();
    v.len()
}

pub fn check(s: &Schematic, d: &mut Diags) {
    for n in &s.nets {
        check_net(s, n, d);
    }
}

fn check_net(s: &Schematic, n: &Net, d: &mut Diags) {
    let at = format!("net {}", n.name);
    let all = pins(s, n);
    let of = |f: &dyn Fn(&Pin) -> bool| -> Vec<&Pin> { all.iter().filter(|p| f(p)).collect() };

    for p in of(&|p| p.kind == PinType::NoConnect) {
        d.push_rule(
            Severity::Warning,
            NC_PIN,
            &at,
            format!(
                "{} is a no-connect pin on its symbol; take it out of the net, or make the pin `passive` if the datasheet says to tie it",
                label(s, p.r)
            ),
        );
    }

    let drivers = of(&|p| push_pull(p.kind));
    if parts_of(&drivers) > 1 {
        d.push_rule(
            Severity::Error,
            DRIVE_CONFLICT,
            &at,
            format!("several outputs drive it: {}", list(s, &drivers)),
        );
    }
    let opens = of(&|p| open(p.kind) && !drivers.iter().any(|q| q.part == p.part));
    if !drivers.is_empty() && !opens.is_empty() {
        d.push_rule(
            Severity::Error,
            DRIVE_CONFLICT,
            &at,
            format!(
                "push-pull {} fights open {} {}: the output holds the line the open drain wants to pull",
                list(s, &drivers),
                if opens.iter().all(|p| p.kind == PinType::OpenEmitter) {
                    "emitter"
                } else {
                    "collector"
                },
                list(s, &opens)
            ),
        );
    }
    let tri = of(&|p| p.kind == PinType::TriState && !drivers.iter().any(|q| q.part == p.part));
    if !drivers.is_empty() && !tri.is_empty() {
        d.push_rule(
            Severity::Warning,
            TRISTATE,
            &at,
            format!(
                "{} always drives it, so tri-state {} contends whenever it drives too",
                list(s, &drivers),
                list(s, &tri)
            ),
        );
    }

    let sources = of(&|p| !p.capacitor && !listens(p.kind));
    let caps = of(&|p| p.capacitor);
    let supplied = of(&|p| p.kind == PinType::PowerIn);
    let inputs = of(&|p| p.kind == PinType::Input);
    let levels_cover = !s.rails.is_empty()
        && !inputs.is_empty()
        && inputs.iter().all(|p| crate::levels::pin_levels(s, p.r).is_some());
    if sources.is_empty() && !supplied.is_empty() {
        let (severity, why) = if caps.is_empty() || parts_of(&supplied) > 1 {
            (Severity::Error, "no regulator, connector or other source is on the net".to_string())
        } else {
            (
                Severity::Warning,
                format!(
                    "only {} sit{} on it; if this is the cap pin of an internal regulator, make the pin `power_out` on the symbol",
                    list(s, &caps),
                    if caps.len() == 1 { "s" } else { "" }
                ),
            )
        };
        d.push_rule(
            severity,
            UNSUPPLIED,
            &at,
            format!(
                "nothing supplies it: {} {} power input{} and {why}",
                list(s, &supplied),
                if supplied.len() == 1 { "is a" } else { "are" },
                if supplied.len() == 1 { "" } else { "s" }
            ),
        );
    } else if sources.is_empty() && caps.is_empty() && !inputs.is_empty() && !levels_cover {
        d.push_rule(
            Severity::Error,
            UNDRIVEN,
            &at,
            format!("nothing drives {}: every pin on the net is an input", list(s, &inputs)),
        );
    }

    let pulls = of(&|p| !p.capacitor && !listens(p.kind) && !open(p.kind));
    let internal = inputs.iter().any(|p| {
        crate::levels::pin_levels(s, p.r)
            .is_some_and(|l| l.pull_up.is_some() || l.pull_down.is_some())
    });
    let all_open = of(&|p| open(p.kind));
    if !all_open.is_empty()
        && pulls.is_empty()
        && !internal
        && !levels_cover
        && !s.rails.contains_key(&n.name)
    {
        let up = all_open.iter().any(|p| p.kind == PinType::OpenCollector);
        d.push_rule(
            Severity::Warning,
            NO_PULL,
            &at,
            format!(
                "nothing pulls it {dir}: {} can only pull it {}, add a pull-{dir} resistor or give the input its internal `pull_{dir}` in `levels`",
                list(s, &all_open),
                if up { "low" } else { "high" },
                dir = if up { "up" } else { "down" },
            ),
        );
    }
}
