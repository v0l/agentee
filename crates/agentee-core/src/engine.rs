use crate::units::Length;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PHASES: &[&str] = &["constraints", "place", "access", "global", "detail", "finish"];

pub const RETIRED: &[(&str, &str)] = &[
    ("floorplan", "place"),
    ("legalise", "place"),
    ("layers", "access"),
    ("escape", "access"),
    ("tie", "access"),
    ("planes", "access"),
    ("assign", "global"),
];

pub fn stage_of(name: &str) -> Option<&'static str> {
    PHASES
        .iter()
        .copied()
        .find(|p| *p == name)
        .or_else(|| RETIRED.iter().find(|(old, _)| *old == name).map(|(_, new)| *new))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineFile {
    #[serde(default, alias = "stages", skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rounds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place_rounds: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub score: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<PlacePhaseFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<AccessFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planes: Option<PlanesFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global: Option<GlobalFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<DetailFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tangle: Option<TangleFile>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TangleFile {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub weights: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rounds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_cost: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bend_cost: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fences: Option<bool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub criticality: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_in_pad: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub escape_layers: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanesFile {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub layers: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neck: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Length>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacePhaseFile {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub weights: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standoff: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spread: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bga_decaps: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rounds: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub direction: BTreeMap<String, Direction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_cost: Option<Length>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    H,
    V,
}

impl EngineFile {
    pub fn phases(&self) -> Vec<String> {
        if self.phases.is_empty() {
            return PHASES.iter().map(|s| s.to_string()).collect();
        }
        let mut out: Vec<String> = Vec::new();
        for p in &self.phases {
            if let Some(s) = stage_of(p)
                && !out.iter().any(|o| o == s)
            {
                out.push(s.to_string());
            }
        }
        out.sort_by_key(|s| PHASES.iter().position(|p| p == s));
        out
    }

    pub fn check(&self, d: &mut crate::diag::Diags) {
        for p in &self.phases {
            match stage_of(p) {
                None => d.error(
                    "engine.phases",
                    format!("no stage `{p}`, the stages are {}", PHASES.join(", ")),
                ),
                Some(s) if s != p => {
                    d.warn("engine.phases", format!("`{p}` is now part of the `{s}` stage"))
                }
                Some(_) => {}
            }
        }
        for k in self.score.keys() {
            if !SCORE_TERMS.contains(&k.as_str()) {
                d.error(
                    "engine.score",
                    format!("no score term `{k}`, the terms are {}", SCORE_TERMS.join(", ")),
                );
            }
        }
    }
}

pub const SCORE_TERMS: &[&str] = &[
    "wirelength",
    "overflow",
    "unrouted",
    "vias",
    "crossings",
    "chain_order",
    "chain_layer",
    "return_path",
    "pair_coupling",
    "pin_access",
    "decap",
    "via_site",
    "switcher",
    "hot",
    "flex",
    "edge",
    "keepout",
    "overlap",
    "plane_reach",
    "area",
    "layers",
    "copper_overlap",
    "access",
];

pub fn default_weight(term: &str) -> f64 {
    match term {
        "wirelength" => 1.0,
        "overflow" => 50.0,
        "unrouted" => 200.0,
        "vias" => 2.0,
        "crossings" => 1.0,
        "chain_order" => 20.0,
        "chain_layer" => 100.0,
        "return_path" => 20.0,
        "pair_coupling" => 10.0,
        "pin_access" => 5.0,
        "decap" => 5.0,
        "via_site" => 100.0,
        "switcher" => 20.0,
        "hot" => 5.0,
        "flex" => 10.0,
        "edge" => 50.0,
        "keepout" => 100.0,
        "overlap" => 100.0,
        "plane_reach" => 20.0,
        "area" => 0.01,
        "layers" => 100.0,
        "copper_overlap" => 200.0,
        "access" => 200.0,
        _ => 0.0,
    }
}
