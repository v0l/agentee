use super::solve::Profile;
use super::{Finding, OvenFile, OvenKind};
use crate::diag::Severity;
use serde::Serialize;

pub const SETPOINT_HOLD_MIN_S: f64 = 5.0;

#[derive(Clone, Debug, Serialize)]
pub struct Setpoint {
    pub at_s: f64,
    pub temperature_c: f64,
    pub action: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ZoneSetting {
    pub zone: String,
    pub length_mm: f64,
    pub temperature_c: f64,
    pub dwell_s: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Tolerance {
    pub peak_c: f64,
    pub time_above_liquidus_s: f64,
    pub note: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct OvenPlan {
    pub kind: OvenKind,
    pub setpoints: Vec<Setpoint>,
    pub zones: Vec<ZoneSetting>,
    pub conveyor_mm_min: Option<f64>,
    pub tolerance: Tolerance,
    pub advice: Vec<String>,
}

pub fn plan(p: &Profile, oven: Option<&OvenFile>, out: &mut Vec<Finding>) -> OvenPlan {
    let kind = oven.map(|o| o.kind).unwrap_or_default();
    let mut setpoints = Vec::new();
    for s in &p.stages {
        let action = match s.name.as_str() {
            "preheat ramp" => "heat at full power to the soak start",
            "soak" => "hold the soak, rising slowly",
            "ramp to peak" => "full power to the peak",
            "peak dwell" => "hold the peak",
            _ => "heat off, open the door or start the cooling fan",
        };
        setpoints.push(Setpoint {
            at_s: s.start_s,
            temperature_c: s.end_c,
            action: format!("{}: {action}", s.name),
        });
    }
    let last = p.stages.last().map(|s| s.end_s).unwrap_or(0.0);
    setpoints.push(Setpoint {
        at_s: last,
        temperature_c: super::solve::SOAK_END_COOL_C,
        action: "done: remove the board once below the cooling floor".into(),
    });
    if let Some(m) = oven.and_then(|o| o.max_temp)
        && p.oven_peak_c > m
    {
        out.push(Finding::new(
            Severity::Error,
            "reflow-oven-max",
            "oven",
            format!(
                "the profile needs {:.0} C oven air but the oven tops out at {m:.0} C",
                p.oven_peak_c
            ),
        ));
    }
    let up_cap = oven.and_then(|o| o.max_ramp_up);
    if let Some(cap) = up_cap
        && cap
            < p.stages.iter().find(|s| s.name == "preheat ramp").map(|s| s.rate_c_s).unwrap_or(0.0)
                - 1e-6
    {
        out.push(Finding::new(
            Severity::Warning,
            "reflow-oven-ramp",
            "oven",
            format!(
                "the oven heats at {cap:.1} C/s, under the {:.1} C/s the curve asks for",
                p.ramp_up_c_s
            ),
        ));
    }
    let mut zones = Vec::new();
    let mut conveyor = None;
    if kind == OvenKind::Zones
        && let Some(o) = oven
        && !o.zones.is_empty()
    {
        match o.conveyor_speed {
            Some(speed) if speed > 0.0 => {
                let total: f64 = o.zones.iter().map(|z| z.length).sum();
                let transit = total / speed * 60.0;
                let curve_len = last.max(1.0);
                let scale = curve_len / transit.max(1.0);
                conveyor = Some(speed * scale);
                let mut at = 0.0;
                for z in &o.zones {
                    let dwell = z.length / (speed * scale) * 60.0;
                    let mid = at + dwell / 2.0;
                    let t = sample(p, mid);
                    zones.push(ZoneSetting {
                        zone: z.name.clone(),
                        length_mm: z.length,
                        temperature_c: t.round(),
                        dwell_s: dwell,
                    });
                    at += dwell;
                }
            }
            _ => out.push(Finding::new(
                Severity::Error,
                "reflow-oven-zones",
                "oven",
                "zones need a conveyor_speed above zero, mm per minute",
            )),
        }
    }
    let tolerance = Tolerance {
        peak_c: p.tolerance_c,
        time_above_liquidus_s: p
            .parts
            .iter()
            .map(|x| x.time_above_liquidus_s)
            .fold(f64::MAX, f64::min),
        note: format!(
            "the oven air may be off the curve by {:.1} C at peak before the coldest joint misses {:.0} C or the {} passes {:.0} C; keep the thermocouple reading within that band",
            p.tolerance_c, p.required_joint_peak_c, p.limiter, p.limiter_limit_c
        ),
    };
    let mut advice = Vec::new();
    match kind {
        OvenKind::Toaster => {
            advice.push("toaster oven: run the controller to the setpoints above, with the board on a rack mid-height and the door cracked open for cooling".into());
        }
        OvenKind::Hotplate => {
            advice.push("hot plate: setpoints are plate temperature; the board bottom runs hotter than the top, so expect the top joints to lag and read the thermocouple".into());
        }
        OvenKind::Convection => advice.push("convection oven: hold the board mid-height away from the elements".into()),
        OvenKind::Zones => advice.push("conveyor oven: set zone temperatures and belt speed below, then trim from a thermocouple profile run".into()),
    }
    OvenPlan { kind, setpoints, zones, conveyor_mm_min: conveyor, tolerance, advice }
}

fn sample(p: &Profile, t: f64) -> f64 {
    let i =
        ((t / super::solve::DT_S).round() as usize).min(p.series.oven_c.len().saturating_sub(1));
    p.series.oven_c.get(i).copied().unwrap_or(0.0)
}
