use super::{Confidence, PasteFile};

pub const SAC305_SOURCE: &str = "Amtech SynTECH-LF product data sheet (Chip Quik SMD291SNL), Sn96.5/Ag3.0/Cu0.5 recommended profile, https://media.digikey.com/pdf/Data%20Sheets/Chip%20Quik%20PDFs/SMD291SNL(10).pdf page 2";
pub const SNPB_SOURCE: &str =
    "J-STD-020E Table 4-1 SnPb classification profile, https://pcbsync.com/j-std-020/";
pub const ALLOY_SOURCE: &str =
    "alloy melting range from J-STD-006 alloy designations; profile windows are assumed";

#[derive(Clone, Debug, serde::Serialize)]
pub struct Paste {
    pub name: String,
    pub alloy: String,
    pub lead_free: bool,
    pub solidus: f64,
    pub liquidus: f64,
    pub peak: [f64; 2],
    pub preheat: [f64; 2],
    pub soak_time: [f64; 2],
    pub max_ramp_up: f64,
    pub time_above_liquidus: [f64; 2],
    pub cooling: [f64; 2],
    pub flux_activation: [f64; 2],
    pub source: Option<String>,
    pub confidence: Confidence,
    pub assumed: Vec<String>,
}

struct Alloy {
    names: &'static [&'static str],
    lead_free: bool,
    solidus: f64,
    liquidus: f64,
}

const ALLOYS: &[Alloy] = &[
    Alloy {
        names: &["sac305", "sn96.5ag3cu0.5", "sn96.5/ag3.0/cu0.5", "sn96.5/ag3.0/cu.5"],
        lead_free: true,
        solidus: 217.0,
        liquidus: 220.0,
    },
    Alloy {
        names: &["sac405", "sn95.5ag4cu0.5"],
        lead_free: true,
        solidus: 217.0,
        liquidus: 220.0,
    },
    Alloy {
        names: &["sn63pb37", "sn63/pb37", "snpb"],
        lead_free: false,
        solidus: 183.0,
        liquidus: 183.0,
    },
    Alloy {
        names: &["sn62pb36ag2", "sn62/pb36/ag2"],
        lead_free: false,
        solidus: 179.0,
        liquidus: 179.0,
    },
    Alloy {
        names: &["sn42bi58", "sn42/bi58", "sn42bi57.6ag0.4", "sn42/bi57.6/ag0.4", "snbi", "snbiag"],
        lead_free: true,
        solidus: 138.0,
        liquidus: 140.0,
    },
    Alloy {
        names: &["sn99.3cu0.7", "sn99.3/cu0.7"],
        lead_free: true,
        solidus: 227.0,
        liquidus: 227.0,
    },
];

fn norm(s: &str) -> String {
    s.to_lowercase().replace([' ', '-', '_'], "")
}

fn alloy(name: &str) -> Option<&'static Alloy> {
    let n = norm(name);
    ALLOYS.iter().find(|a| a.names.iter().any(|x| norm(x) == n))
}

fn pick<T>(file: Option<T>, assumed: &mut Vec<String>, key: &str, default: T) -> T {
    match file {
        Some(v) => v,
        None => {
            assumed.push(key.to_string());
            default
        }
    }
}

pub fn resolve(f: &PasteFile) -> Result<Paste, String> {
    let known = alloy(&f.alloy);
    let mut assumed = Vec::new();
    let lead_free = match (f.lead_free, known) {
        (Some(v), _) => v,
        (None, Some(a)) => a.lead_free,
        (None, None) => {
            return Err(format!(
                "unknown alloy `{}`: give lead_free, solidus and liquidus",
                f.alloy
            ));
        }
    };
    let solidus = match (f.solidus, known) {
        (Some(v), _) => v,
        (None, Some(a)) => a.solidus,
        (None, None) => return Err(format!("unknown alloy `{}`: give solidus", f.alloy)),
    };
    let liquidus = match (f.liquidus, known) {
        (Some(v), _) => v,
        (None, Some(a)) => a.liquidus,
        (None, None) => return Err(format!("unknown alloy `{}`: give liquidus", f.alloy)),
    };
    if liquidus < solidus {
        return Err(format!("liquidus {liquidus} C is below solidus {solidus} C"));
    }
    let sac = lead_free && liquidus >= 215.0;
    let hot_lead_free = sac;
    let (d_peak, d_preheat, d_soak, d_ramp, d_tal, d_cool, d_flux, d_src) = if hot_lead_free {
        (
            [230.0, 249.0],
            [140.0, 200.0],
            [60.0, 90.0],
            2.0,
            [30.0, 60.0],
            [2.0, 4.0],
            [140.0, 200.0],
            SAC305_SOURCE,
        )
    } else if !lead_free {
        (
            [liquidus + 20.0, 235.0_f64.max(liquidus + 37.0)],
            [100.0, 150.0],
            [60.0, 120.0],
            3.0,
            [60.0, 150.0],
            [2.0, 6.0],
            [100.0, 150.0],
            SNPB_SOURCE,
        )
    } else {
        (
            [liquidus + 10.0, liquidus + 50.0],
            [90.0, 120.0],
            [60.0, 120.0],
            2.0,
            [30.0, 90.0],
            [2.0, 4.0],
            [90.0, 120.0],
            ALLOY_SOURCE,
        )
    };
    let from_defaults = f.source.is_none();
    let peak = pick(f.peak, &mut assumed, "peak", d_peak);
    let preheat = pick(f.preheat, &mut assumed, "preheat", d_preheat);
    let soak_time = pick(f.soak_time, &mut assumed, "soak_time", d_soak);
    let max_ramp_up = pick(f.max_ramp_up, &mut assumed, "max_ramp_up", d_ramp);
    let time_above_liquidus =
        pick(f.time_above_liquidus, &mut assumed, "time_above_liquidus", d_tal);
    let cooling = pick(f.cooling, &mut assumed, "cooling", d_cool);
    let flux_activation = pick(f.flux_activation, &mut assumed, "flux_activation", d_flux);
    if peak[0] > peak[1]
        || preheat[0] > preheat[1]
        || soak_time[0] > soak_time[1]
        || time_above_liquidus[0] > time_above_liquidus[1]
        || cooling[0] > cooling[1]
    {
        return Err("a paste range has its minimum above its maximum".into());
    }
    let source = f.source.clone().or_else(|| Some(d_src.to_string()).filter(|_| from_defaults));
    let confidence = if f.source.is_some() && assumed.is_empty() {
        Confidence::Datasheet
    } else if f.source.is_some() {
        Confidence::Inferred
    } else {
        Confidence::Assumed
    };
    Ok(Paste {
        name: f.name.clone().unwrap_or_else(|| f.alloy.clone()),
        alloy: f.alloy.clone(),
        lead_free,
        solidus,
        liquidus,
        peak,
        preheat,
        soak_time,
        max_ramp_up,
        time_above_liquidus,
        cooling,
        flux_activation,
        source,
        confidence,
        assumed,
    })
}

pub fn low_temperature() -> Paste {
    resolve(&PasteFile { alloy: "Sn42Bi58".into(), ..Default::default() }).expect("builtin alloy")
}
