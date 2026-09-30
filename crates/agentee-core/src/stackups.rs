use crate::board::{LayerFile, LayerKind};
use crate::units::Length;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const FABS: &[(&str, &str)] = &[
    ("jlcpcb", include_str!("../stackups/jlcpcb.toml")),
    ("pcbway", include_str!("../stackups/pcbway.toml")),
    ("generic", include_str!("../stackups/generic.toml")),
];

const LOSS_TANGENT: f64 = 0.02;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FabFile {
    fab: String,
    source: String,
    fetched: String,
    #[serde(default)]
    mask_thickness: Option<f64>,
    #[serde(default)]
    mask_er: Option<f64>,
    stackups: Vec<PresetFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetFile {
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    description: String,
    layers: Vec<PresetLayer>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetLayer {
    copper: Option<f64>,
    prepreg: Option<f64>,
    core: Option<f64>,
    material: Option<String>,
    er: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StackupPreset {
    pub name: String,
    pub aliases: Vec<String>,
    pub fab: String,
    pub description: String,
    pub source: String,
    pub fetched: String,
    pub copper_layers: usize,
    pub thickness_mm: f64,
    #[serde(skip)]
    pub layers: Vec<LayerFile>,
}

impl StackupPreset {
    pub fn matches(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
            || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
    }
}

fn layer(
    kind: LayerKind,
    t: Option<f64>,
    material: Option<String>,
    er: Option<f64>,
    tan: Option<f64>,
) -> LayerFile {
    LayerFile { kind, name: None, thickness: t.map(Length::mm), material, er, loss_tangent: tan }
}

fn build(fab: &FabFile, p: &PresetFile) -> StackupPreset {
    let mask = || {
        layer(
            LayerKind::Mask,
            Some(fab.mask_thickness.unwrap_or(0.0152)),
            Some("LPI".into()),
            Some(fab.mask_er.unwrap_or(3.8)),
            None,
        )
    };
    let bare = |kind| layer(kind, None, None, None, None);
    let mut layers = vec![bare(LayerKind::Silk), bare(LayerKind::Paste), mask()];
    for l in &p.layers {
        let (kind, t, material) = match (l.copper, l.prepreg, l.core) {
            (Some(t), None, None) => (LayerKind::Copper, t, None),
            (None, Some(t), None) => (LayerKind::Prepreg, t, l.material.clone()),
            (None, None, Some(t)) => {
                (LayerKind::Core, t, Some(l.material.clone().unwrap_or("FR4".into())))
            }
            _ => panic!("{}: {}: a layer is one of copper, prepreg or core", fab.fab, p.name),
        };
        let tan = (kind != LayerKind::Copper).then_some(LOSS_TANGENT);
        layers.push(layer(kind, Some(t), material, l.er, tan));
    }
    layers.extend([mask(), bare(LayerKind::Paste), bare(LayerKind::Silk)]);
    let copper_layers = p.layers.iter().filter(|l| l.copper.is_some()).count();
    let thickness_mm =
        p.layers.iter().filter_map(|l| l.copper.or(l.prepreg).or(l.core)).sum::<f64>()
            + 2.0 * fab.mask_thickness.unwrap_or(0.0152);
    StackupPreset {
        name: p.name.clone(),
        aliases: p.aliases.clone(),
        fab: fab.fab.clone(),
        description: p.description.clone(),
        source: fab.source.clone(),
        fetched: fab.fetched.clone(),
        copper_layers,
        thickness_mm,
        layers,
    }
}

pub fn stackup_presets() -> &'static [StackupPreset] {
    static ALL: OnceLock<Vec<StackupPreset>> = OnceLock::new();
    ALL.get_or_init(|| {
        FABS.iter()
            .flat_map(|(fab, text)| {
                let file: FabFile =
                    toml::from_str(text).unwrap_or_else(|e| panic!("stackups/{fab}.toml: {e}"));
                file.stackups.iter().map(|p| build(&file, p)).collect::<Vec<_>>()
            })
            .collect()
    })
}

pub fn find_stackup_preset(name: &str) -> Option<&'static StackupPreset> {
    stackup_presets().iter().find(|p| p.matches(name))
}

pub fn suggest_stackup_presets(name: &str, n: usize) -> Vec<&'static str> {
    let wanted = name.to_ascii_lowercase();
    let common = |a: &str| {
        a.to_ascii_lowercase().chars().zip(wanted.chars()).take_while(|(x, y)| x == y).count()
    };
    let mut scored: Vec<_> = stackup_presets()
        .iter()
        .map(|p| {
            (
                p.aliases.iter().map(|a| common(a)).chain([common(&p.name)]).max().unwrap_or(0),
                p.name.as_str(),
            )
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    scored.into_iter().take(n).map(|s| s.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_loads_and_alternates() {
        let all = stackup_presets();
        assert!(all.len() > 500);
        for p in all {
            let body: Vec<_> = p
                .layers
                .iter()
                .filter(|l| l.kind != LayerKind::Silk && l.kind != LayerKind::Paste)
                .collect();
            assert_eq!(body.first().map(|l| l.kind), Some(LayerKind::Mask), "{}", p.name);
            let cu: Vec<_> = body.iter().filter(|l| l.kind == LayerKind::Copper).collect();
            assert_eq!(cu.len(), p.copper_layers, "{}", p.name);
            assert!(p.copper_layers >= 2 && p.copper_layers % 2 == 0, "{}", p.name);
            for l in &body {
                if l.kind.is_dielectric() {
                    let er = l.er.unwrap_or_else(|| panic!("{}: dielectric without er", p.name));
                    assert!((3.5..5.0).contains(&er), "{}: er {er}", p.name);
                }
            }
        }
    }

    #[test]
    fn names_are_unique_across_fabs() {
        let mut names: Vec<String> = stackup_presets()
            .iter()
            .flat_map(|p| p.aliases.iter().cloned().chain([p.name.clone()]))
            .map(|n| n.to_ascii_lowercase())
            .collect();
        let before = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), before);
    }

    #[test]
    fn jlc_six_layer_1080_matches_the_published_build() {
        let p = find_stackup_preset("jlc06161h-1080").unwrap();
        assert_eq!(p.copper_layers, 6);
        let t: Vec<f64> = p.layers.iter().filter_map(|l| l.thickness).map(|t| t.to_mm()).collect();
        let body = &t[1..t.len() - 1];
        let want =
            [0.035, 0.0764, 0.0152, 0.55, 0.0152, 0.2104, 0.0152, 0.55, 0.0152, 0.0764, 0.035];
        assert_eq!(body.len(), want.len());
        for (a, b) in body.iter().zip(want) {
            assert!((a - b).abs() < 1e-9, "{body:?}");
        }
        assert!((p.thickness_mm - 1.6).abs() < 0.05, "{}", p.thickness_mm);
    }

    #[test]
    fn old_names_still_resolve() {
        for n in ["jlcpcb-2l-1.6mm", "jlcpcb-4l-1.6mm-7628", "jlcpcb-4l-1.6mm-3313"] {
            assert!(find_stackup_preset(n).is_some(), "{n}");
        }
        assert_eq!(find_stackup_preset("jlcpcb-4l-1.6mm-7628").unwrap().name, "JLC04161H-7628");
    }

    #[test]
    fn suggestions_start_from_the_typed_prefix() {
        let s = suggest_stackup_presets("JLC06161H-10", 3);
        assert!(s.iter().all(|n| n.starts_with("JLC06161H-10")), "{s:?}");
    }
}
