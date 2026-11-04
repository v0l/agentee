use super::defaults::JSTD020_CLASSIFICATION_CYCLES;
use super::inputs::{PartInput, Side};
use super::limits::Resolved;
use super::thermal::{self, SURFACE_TENSION_G_PER_MM2};
use super::{AssemblyFile, Capable, Finding};
use crate::diag::Severity;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Hold {
    pub reference: String,
    pub mass_g: f64,
    pub pad_area_mm2: f64,
    pub g_per_mm2: f64,
    pub holds: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Exposure {
    pub reference: String,
    pub msl: String,
    pub floor_life_hours: f64,
    pub pass: u32,
    pub exposed_hours: Option<f64>,
    pub remaining_hours: Option<f64>,
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PassPlan {
    pub double_sided: bool,
    pub first_side: Option<Side>,
    pub chosen_automatically: bool,
    pub passes: u32,
    pub holds: Vec<Hold>,
    pub exposures: Vec<Exposure>,
    pub after_reflow: Vec<String>,
}

fn passes_seen(side: Side, first: Side, double: bool) -> u32 {
    if double && side == first { 2 } else { 1 }
}

struct Cost {
    errors: u32,
    first: Side,
}

fn hold_of(p: &PartInput, r: &Resolved) -> Hold {
    let mass = thermal::body_mass_g(&thermal::Geometry {
        family: r.family,
        body: p.body,
        footprint: p.footprint_mm,
        pad_area_mm2: p.pad_area_mm2,
        mass_g: r.mass_g,
        local_copper_fill: None,
    });
    let per = if p.pad_area_mm2 > 0.0 { mass / p.pad_area_mm2 } else { f64::INFINITY };
    Hold {
        reference: p.reference.clone(),
        mass_g: mass,
        pad_area_mm2: p.pad_area_mm2,
        g_per_mm2: per,
        holds: per <= SURFACE_TENSION_G_PER_MM2,
    }
}

fn violations(parts: &[(&PartInput, &Resolved)], first: Side, double: bool) -> u32 {
    let mut n = 0;
    for (p, r) in parts {
        if r.limits.capable != Capable::Reflow {
            continue;
        }
        let seen = passes_seen(p.side, first, double);
        if r.limits.max_passes < seen {
            n += 1;
        }
        if double {
            if p.side == first && !hold_of(p, r).holds {
                n += 1;
            }
            if r.limits.last_pass && p.side == first {
                n += 1;
            }
        }
    }
    n
}

pub fn plan(
    parts: &[PartInput],
    resolved: &[Resolved],
    assembly: Option<&AssemblyFile>,
    out: &mut Vec<Finding>,
) -> PassPlan {
    let pairs: Vec<(&PartInput, &Resolved)> = parts
        .iter()
        .filter_map(|p| resolved.iter().find(|r| r.reference == p.reference).map(|r| (p, r)))
        .collect();
    let in_oven: Vec<_> = pairs.iter().filter(|(_, r)| r.limits.capable.in_oven()).collect();
    let top = in_oven.iter().any(|(p, _)| p.side == Side::Top);
    let bottom = in_oven.iter().any(|(p, _)| p.side == Side::Bottom);
    let double = top && bottom;
    let asked = assembly.and_then(|a| a.first_side.as_deref()).and_then(|s| {
        match s.to_lowercase().as_str() {
            "top" | "f" => Some(Side::Top),
            "bottom" | "b" => Some(Side::Bottom),
            _ => None,
        }
    });
    if let Some(a) = assembly.and_then(|a| a.first_side.as_deref())
        && asked.is_none()
    {
        out.push(Finding::new(
            Severity::Error,
            "reflow-first-side",
            "assembly",
            format!("first_side `{a}` is not top or bottom"),
        ));
    }
    let costs =
        [Side::Top, Side::Bottom].map(|s| Cost { errors: violations(&pairs, s, double), first: s });
    let (first, auto) = match asked {
        Some(s) => (s, false),
        None => {
            let best = costs
                .iter()
                .min_by_key(|c| (c.errors, if c.first == Side::Top { 0 } else { 1 }))
                .map(|c| c.first)
                .unwrap_or(Side::Top);
            (best, true)
        }
    };
    let mut holds = Vec::new();
    if double {
        for (p, r) in &in_oven.iter().map(|x| **x).collect::<Vec<_>>() {
            if p.side != first {
                continue;
            }
            let h = hold_of(p, r);
            if !h.holds {
                out.push(Finding::new(
                    Severity::Error,
                    "reflow-hangs",
                    &p.reference,
                    format!(
                        "{} weighs {:.3} g on {:.1} mm2 of pad ({:.4} g/mm2, over {:.4}): it is upside down on the second pass and can fall off, so put it on the second-pass side ({})",
                        p.reference, h.mass_g, h.pad_area_mm2, h.g_per_mm2, SURFACE_TENSION_G_PER_MM2, first.other().label()
                    ),
                ));
            }
            holds.push(h);
        }
    }
    for (p, r) in &pairs {
        if r.limits.capable != Capable::Reflow {
            continue;
        }
        let seen = passes_seen(p.side, first, double);
        if r.limits.max_passes < seen {
            out.push(Finding::new(
                Severity::Error,
                "reflow-passes",
                &p.reference,
                format!(
                    "{} allows {} reflow pass(es) but sits on the {} side, which sees {} with {} first: move it to the {} side or hand solder it",
                    p.reference, r.limits.max_passes, p.side.label(), seen, first.label(), first.other().label()
                ),
            ));
        }
        if double && r.limits.last_pass && p.side == first {
            out.push(Finding::new(
                Severity::Error,
                "reflow-last-pass",
                &p.reference,
                format!(
                    "{} must go through the last pass only but is on the first-pass side ({})",
                    p.reference,
                    first.label()
                ),
            ));
        }
        if r.limits.max_passes < JSTD020_CLASSIFICATION_CYCLES
            && r.limits.max_passes > 1
            && seen == r.limits.max_passes
        {
            out.push(Finding::new(
                Severity::Info,
                "reflow-pass-budget",
                &p.reference,
                format!(
                    "{} uses all {} of its allowed reflow passes: no rework pass is left",
                    p.reference, seen
                ),
            ));
        }
    }
    let mut exposures = Vec::new();
    let h1 = assembly.and_then(|a| a.hours_before_first_pass);
    let h2 = assembly.and_then(|a| a.hours_between_passes);
    for (p, r) in &pairs {
        if r.limits.capable != Capable::Reflow {
            continue;
        }
        let life = r.limits.floor_life_hours;
        if life.is_infinite() {
            continue;
        }
        let seen = passes_seen(p.side, first, double);
        for pass in 1..=seen {
            let exposed = match (pass, seen, double, h1, h2) {
                (1, _, false, Some(a), _) => Some(a),
                (1, _, true, Some(a), _) if p.side == first => Some(a),
                (_, 2, true, Some(a), Some(b)) if pass == 2 => Some(a + b),
                _ => None,
            };
            let exposed = if double && p.side != first && pass == 1 { h2 } else { exposed };
            let ok = exposed.is_none_or(|e| e <= life);
            if !ok {
                out.push(Finding::new(
                    Severity::Error,
                    "reflow-floor-life",
                    &p.reference,
                    format!(
                        "{} is MSL {} (floor life {} h) and is exposed {:.0} h before pass {}: bake it first (J-STD-033)",
                        p.reference, r.limits.msl, life, exposed.unwrap_or(0.0), pass
                    ),
                ));
            }
            exposures.push(Exposure {
                reference: p.reference.clone(),
                msl: r.limits.msl.clone(),
                floor_life_hours: life,
                pass,
                exposed_hours: exposed,
                remaining_hours: exposed.map(|e| (life - e).max(0.0)),
                ok,
            });
        }
        if life == 0.0 {
            out.push(Finding::new(Severity::Warning, "reflow-msl6", &p.reference, format!("{} is MSL 6: bake it before every use, mount and reflow within the time on label", p.reference)));
        }
    }
    let after_reflow = pairs
        .iter()
        .filter(|(_, r)| r.limits.capable != Capable::Reflow)
        .map(|(p, r)| {
            let how = if r.limits.capable == Capable::Selective {
                "selective or wave solder"
            } else {
                "hand solder"
            };
            format!("{}: {how} after the last reflow pass", p.reference)
        })
        .collect();
    PassPlan {
        double_sided: double,
        first_side: if in_oven.is_empty() { None } else { Some(first) },
        chosen_automatically: auto,
        passes: if double { 2 } else { u32::from(!in_oven.is_empty()) },
        holds,
        exposures,
        after_reflow,
    }
}
