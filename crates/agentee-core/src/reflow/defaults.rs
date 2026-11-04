use super::{Capable, Confidence, LimitsFile, Termination};
use crate::footprint::Mount;

pub const JSTD020_PBFREE_TP_S: f64 = 30.0;
pub const JSTD020_SNPB_TP_S: f64 = 30.0;
pub const JSTD020_TAL_MAX_S: f64 = 150.0;
pub const JSTD020_RAMP_UP_C_S: f64 = 3.0;
pub const JSTD020_RAMP_DOWN_C_S: f64 = 6.0;
pub const JSTD020_CLASSIFICATION_CYCLES: u32 = 3;
pub const JSTD020_TP_WINDOW_C: f64 = 5.0;
pub const JSTD020_MAX_TIME_25_TO_PEAK_S: f64 = 480.0;

pub const CONSERVATIVE_PEAK_C: f64 = 245.0;
pub const CONSERVATIVE_TP_S: f64 = JSTD020_PBFREE_TP_S;
pub const CONSERVATIVE_PASSES: u32 = 2;
pub const ASSUMED_MSL: &str = "3";

pub const JSTD020_SOURCE: &str = "J-STD-020E Tables 4-1, 4-2 and 5-2 (classification temperatures by package thickness and volume)";
pub const ASSUMED_SOURCE: &str =
    "conservative default for the package family, not from a datasheet";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Chip,
    Led,
    Ic,
    Bga,
    Connector,
    Electrolytic,
    Crystal,
    Shield,
    ThroughHole,
    Other,
}

impl Family {
    pub fn label(self) -> &'static str {
        match self {
            Family::Chip => "chip passive",
            Family::Led => "LED",
            Family::Ic => "leaded or leadless IC",
            Family::Bga => "BGA",
            Family::Connector => "SMD connector",
            Family::Electrolytic => "SMD electrolytic capacitor",
            Family::Crystal => "crystal or oscillator",
            Family::Shield => "shield can",
            Family::ThroughHole => "through-hole part",
            Family::Other => "other part",
        }
    }
}

pub fn classify(footprint: &str, reference: &str, mount: Mount) -> Family {
    let upper = footprint.to_uppercase();
    let prefix: String = reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let prefix = prefix.to_uppercase();
    let has = |s: &str| upper.contains(s);
    if mount == Mount::Tht || has("_THT") || has("DIP-") || has("_VERTICAL") && has("PIN") {
        return Family::ThroughHole;
    }
    if has("CP_ELEC") || has("C_ELEC") {
        return Family::Electrolytic;
    }
    if has("CRYSTAL") || has("OSCILLATOR") || prefix == "Y" {
        return Family::Crystal;
    }
    if has("SHIELD") {
        return Family::Shield;
    }
    if has("BGA") || has("WLCSP") || has("WLL") || has("CSP") {
        return Family::Bga;
    }
    if matches!(prefix.as_str(), "J" | "P" | "CN" | "CON" | "X") || has("CONN") || has("SMA_") {
        return Family::Connector;
    }
    if has("LED") || prefix == "LED" {
        return Family::Led;
    }
    if has("METRIC") && matches!(prefix.as_str(), "R" | "C" | "L" | "FB" | "F" | "D" | "RN") {
        return Family::Chip;
    }
    if matches!(prefix.as_str(), "R" | "C" | "FB") && has("_0") {
        return Family::Chip;
    }
    if matches!(prefix.as_str(), "U" | "Q" | "D" | "IC") || has("SOT") || has("SO") || has("QFN") {
        return Family::Ic;
    }
    Family::Other
}

pub fn pbfree_classification_c(thickness_mm: f64, volume_mm3: f64) -> f64 {
    let column = if volume_mm3 < 350.0 {
        0
    } else if volume_mm3 <= 2000.0 {
        1
    } else {
        2
    };
    let row: [f64; 3] = if thickness_mm < 1.6 {
        [260.0, 260.0, 260.0]
    } else if thickness_mm <= 2.5 {
        [260.0, 250.0, 245.0]
    } else {
        [250.0, 245.0, 245.0]
    };
    row[column]
}

pub fn snpb_classification_c(thickness_mm: f64, volume_mm3: f64) -> f64 {
    if thickness_mm < 2.5 { if volume_mm3 < 350.0 { 235.0 } else { 220.0 } } else { 220.0 }
}

pub fn floor_life_hours(msl: &str) -> Option<f64> {
    match msl.trim().to_lowercase().as_str() {
        "1" => Some(f64::INFINITY),
        "2" => Some(365.0 * 24.0),
        "2a" => Some(4.0 * 7.0 * 24.0),
        "3" => Some(168.0),
        "4" => Some(72.0),
        "5" => Some(48.0),
        "5a" => Some(24.0),
        "6" => Some(0.0),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct Basis {
    pub family: Family,
    pub body: Option<[f64; 3]>,
    pub lead_free: bool,
}

pub fn package_default(b: &Basis, mount: Mount) -> LimitsFile {
    let (thickness, volume) = match b.body {
        Some([l, w, h]) => (Some(h), Some(l * w * h)),
        None => (None, None),
    };
    let classified = |t: f64, v: f64| {
        if b.lead_free { pbfree_classification_c(t, v) } else { snpb_classification_c(t, v) }
    };
    let tp = if b.lead_free { JSTD020_PBFREE_TP_S } else { JSTD020_SNPB_TP_S };
    let body_peak = match (thickness, volume) {
        (Some(t), Some(v)) => classified(t, v),
        _ => CONSERVATIVE_PEAK_C,
    };
    let mut l = LimitsFile {
        max_peak: Some(CONSERVATIVE_PEAK_C),
        max_time_at_peak: Some(CONSERVATIVE_TP_S),
        max_time_above_liquidus: Some(JSTD020_TAL_MAX_S),
        max_ramp_up: Some(JSTD020_RAMP_UP_C_S),
        max_ramp_down: Some(JSTD020_RAMP_DOWN_C_S),
        msl: Some("1".into()),
        max_passes: Some(CONSERVATIVE_PASSES),
        capable: Some(Capable::Reflow),
        source: Some(ASSUMED_SOURCE.into()),
        confidence: Some(Confidence::Assumed),
        ..Default::default()
    };
    match b.family {
        Family::Chip => {
            l.max_peak = Some(body_peak);
            l.max_time_at_peak = Some(tp);
            l.max_passes = Some(JSTD020_CLASSIFICATION_CYCLES);
            l.source = Some(JSTD020_SOURCE.into());
        }
        Family::Ic | Family::Bga => {
            l.max_peak = Some(body_peak);
            l.max_time_at_peak = Some(tp);
            l.max_passes = Some(JSTD020_CLASSIFICATION_CYCLES);
            l.msl = Some(ASSUMED_MSL.into());
            l.source = Some(JSTD020_SOURCE.into());
        }
        Family::Led => {
            l.msl = Some(ASSUMED_MSL.into());
        }
        Family::Connector | Family::Shield | Family::Other => {}
        Family::Electrolytic => {
            l.max_passes = Some(1);
        }
        Family::Crystal => {
            l.max_peak = Some(body_peak.max(CONSERVATIVE_PEAK_C));
        }
        Family::ThroughHole => {
            l.capable = Some(Capable::Selective);
        }
    }
    if mount != Mount::Smd && b.family != Family::ThroughHole && b.family != Family::Chip {
        l.capable = Some(Capable::Selective);
    }
    if b.family == Family::Bga {
        l.termination =
            Some(if b.lead_free { Termination::LeadFree } else { Termination::TinLead });
    }
    l
}
