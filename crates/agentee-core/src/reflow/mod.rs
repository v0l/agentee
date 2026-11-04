pub mod defaults;
pub mod export;
pub mod inputs;
pub mod limits;
pub mod oven;
pub mod passes;
pub mod paste;
pub mod solve;
pub mod thermal;

use crate::diag::Severity;
use serde::{Deserialize, Serialize};

pub const EXT: &str = ".reflow.toml";
pub const AMBIENT_C: f64 = 25.0;

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub rule: String,
    pub at: String,
    pub message: String,
}

impl Finding {
    pub fn new(
        severity: Severity,
        rule: &str,
        at: impl Into<String>,
        message: impl Into<String>,
    ) -> Finding {
        Finding { severity, rule: rule.to_string(), at: at.into(), message: message.into() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Datasheet,
    Inferred,
    Assumed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capable {
    Reflow,
    Selective,
    Hand,
}

impl Capable {
    pub fn in_oven(self) -> bool {
        self == Capable::Reflow
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Termination {
    LeadFree,
    TinLead,
}

fn msl_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Num(i64),
        Text(String),
    }
    Ok(Option::<Raw>::deserialize(d)?.map(|r| match r {
        Raw::Num(n) => n.to_string(),
        Raw::Text(s) => s.trim().to_lowercase(),
    }))
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsFile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_peak: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_time_at_peak: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_time_above_liquidus: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ramp_up: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ramp_down: Option<f64>,
    #[serde(default, deserialize_with = "msl_text", skip_serializing_if = "Option::is_none")]
    pub msl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor_life_hours: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_passes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capable: Option<Capable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination: Option<Termination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ball_alloy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mass_g: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub last_pass: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_copper_fill: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasteFile {
    #[serde(default)]
    pub name: Option<String>,
    pub alloy: String,
    #[serde(default)]
    pub lead_free: Option<bool>,
    #[serde(default)]
    pub solidus: Option<f64>,
    #[serde(default)]
    pub liquidus: Option<f64>,
    #[serde(default)]
    pub peak: Option<[f64; 2]>,
    #[serde(default)]
    pub preheat: Option<[f64; 2]>,
    #[serde(default)]
    pub soak_time: Option<[f64; 2]>,
    #[serde(default)]
    pub max_ramp_up: Option<f64>,
    #[serde(default)]
    pub time_above_liquidus: Option<[f64; 2]>,
    #[serde(default)]
    pub cooling: Option<[f64; 2]>,
    #[serde(default)]
    pub flux_activation: Option<[f64; 2]>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub page: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OvenKind {
    #[default]
    Toaster,
    Hotplate,
    Convection,
    Zones,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneFile {
    pub name: String,
    pub length: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OvenFile {
    #[serde(default)]
    pub kind: OvenKind,
    #[serde(default)]
    pub max_ramp_up: Option<f64>,
    #[serde(default)]
    pub max_ramp_down: Option<f64>,
    #[serde(default)]
    pub max_temp: Option<f64>,
    #[serde(default)]
    pub h_eff: Option<f64>,
    #[serde(default)]
    pub zones: Vec<ZoneFile>,
    #[serde(default)]
    pub conveyor_speed: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyFile {
    #[serde(default)]
    pub copper_fill: Option<f64>,
    #[serde(default)]
    pub convection: Option<f64>,
    #[serde(default)]
    pub first_side: Option<String>,
    #[serde(default)]
    pub hours_before_first_pass: Option<f64>,
    #[serde(default)]
    pub hours_between_passes: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReflowFile {
    pub name: String,
    pub layout: String,
    pub paste: PasteFile,
    #[serde(default)]
    pub oven: Option<OvenFile>,
    #[serde(default)]
    pub assembly: Option<AssemblyFile>,
    #[serde(default)]
    pub parts: Vec<LimitsFile>,
}

impl ReflowFile {
    pub fn parse(src: &str) -> Result<ReflowFile, String> {
        toml::from_str(src).map_err(|e| e.to_string())
    }
}
