use crate::calc::{self, Line, TraceGeometry};
use crate::diag::Diags;
use crate::insulation::Voltage;
use crate::units::{Amps, Kelvin, Length, Ohms, Percent, Point};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fab: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<OutlineFile>,
    pub stackup: StackupFile,
    #[serde(default)]
    pub rules: RulesFile,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vias: Vec<ViaFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub netclasses: Vec<NetclassFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<DomainFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub barriers: Vec<BarrierFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pollution_degree: Option<u8>,
    #[serde(default)]
    pub drc: DrcFile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nets: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BarrierFile {
    pub between: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creepage: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pollution_degree: Option<u8>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Domain {
    pub name: String,
    pub description: String,
    pub classes: Vec<String>,
    pub nets: Vec<String>,
    pub implicit: bool,
}

impl Domain {
    pub fn holds(&self, net: &str, class: &str) -> bool {
        self.classes.iter().any(|c| c == class)
            || self.nets.iter().any(|g| crate::layout::glob(g, net))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Barrier {
    pub between: [usize; 2],
    pub description: String,
    pub clearance: Option<Length>,
    pub creepage: Option<Length>,
    pub pollution_degree: u8,
}

impl Barrier {
    pub fn groove(&self) -> Length {
        Length::mm(match self.pollution_degree {
            1 => 0.25,
            2 => 1.0,
            _ => 1.5,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutlineFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<Point>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cutouts: Vec<BoardCutoutFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardCutoutFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<Point>>,
}

fn outline_shape(
    points: &Option<Vec<Point>>,
    size: Option<Point>,
    origin: Option<Point>,
    corner_radius: Option<Length>,
    at: &str,
    d: &mut Diags,
) -> Option<Outline> {
    match (points, size) {
        (Some(p), None) => Some(Outline::Polygon { points: p.clone() }),
        (None, Some(size)) => Some(Outline::Rect {
            origin: origin.unwrap_or(Point::ZERO),
            size,
            corner_radius: corner_radius.unwrap_or(Length::ZERO),
        }),
        _ => {
            d.error(at, "give either `size` (a rectangle) or `points` (a polygon)");
            None
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StackupFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silk_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub huray: Option<HurayFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickel: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold: Option<Length>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LayerFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lamination: Vec<DrillStep>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrillKind {
    Mechanical,
    Laser,
    ControlledDepth,
}

impl DrillKind {
    pub fn name(self) -> &'static str {
        match self {
            DrillKind::Mechanical => "mechanical",
            DrillKind::Laser => "laser",
            DrillKind::ControlledDepth => "controlled depth",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrillStep {
    pub from: String,
    pub to: String,
    pub drill_kind: DrillKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HurayFile {
    pub radius: Length,
    pub ratio: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    Silk,
    Paste,
    Mask,
    Copper,
    Core,
    Prepreg,
}

impl LayerKind {
    pub fn is_dielectric(self) -> bool {
        matches!(self, LayerKind::Core | LayerKind::Prepreg)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerFile {
    pub kind: LayerKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub er: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss_tangent: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViaKind {
    #[default]
    Through,
    Blind,
    Buried,
    Microvia,
}

impl ViaKind {
    pub fn name(self) -> &'static str {
        match self {
            ViaKind::Through => "through",
            ViaKind::Blind => "blind",
            ViaKind::Buried => "buried",
            ViaKind::Microvia => "microvia",
        }
    }

    pub fn of_span(a: usize, b: usize, layers: usize) -> ViaKind {
        let last = layers.saturating_sub(1);
        match (a.min(b) == 0, a.max(b) == last) {
            (true, true) => ViaKind::Through,
            (false, false) => ViaKind::Buried,
            _ => ViaKind::Blind,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViaFill {
    Tented,
    TentedCovered,
    Plugged,
    PluggedCovered,
    Filled,
    FilledCovered,
    FilledCapped,
}

impl ViaFill {
    pub fn ipc4761(self) -> &'static str {
        match self {
            ViaFill::Tented => "I",
            ViaFill::TentedCovered => "II",
            ViaFill::Plugged => "III",
            ViaFill::PluggedCovered => "IV",
            ViaFill::Filled => "V",
            ViaFill::FilledCovered => "VI",
            ViaFill::FilledCapped => "VII",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            ViaFill::Tented => "tented",
            ViaFill::TentedCovered => "tented and covered",
            ViaFill::Plugged => "plugged",
            ViaFill::PluggedCovered => "plugged and covered",
            ViaFill::Filled => "filled",
            ViaFill::FilledCovered => "filled and covered",
            ViaFill::FilledCapped => "filled and capped",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackdrillFile {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_stub: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diameter: Option<Length>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViaFile {
    pub name: String,
    pub drill: Length,
    pub diameter: Length,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<ViaKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub stacked: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub skip: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drill_kind: Option<DrillKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<ViaFill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backdrill: Option<BackdrillFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ViaNames {
    One(String),
    Many(Vec<String>),
}

impl ViaNames {
    pub fn list(&self) -> Vec<String> {
        match self {
            ViaNames::One(s) => vec![s.clone()],
            ViaNames::Many(v) => v.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Solver {
    #[default]
    Formula,
    Field,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetclassFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voltage: Option<Voltage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_width: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<ViaNames>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<Amps>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_temp_rise: Option<Kelvin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impedance: Option<Ohms>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impedance_tolerance: Option<Percent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solver: Option<Solver>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_gap: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coplanar_gap: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_skew: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_uncoupled: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neckdown: Option<Length>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub widths: std::collections::BTreeMap<String, Length>,
}

macro_rules! rules {
    ($($field:ident: $doc:literal,)*) => {
        #[derive(Clone, Debug, Default, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct RulesFile {
            $(
                #[serde(default, skip_serializing_if = "Option::is_none")]
                pub $field: Option<Length>,
            )*
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub max_aspect_ratio: Option<f64>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub max_microvia_aspect_ratio: Option<f64>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub max_controlled_depth_aspect_ratio: Option<f64>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub hdi: Option<bool>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub stacked_microvias: Option<bool>,
        }

        #[derive(Clone, Debug, PartialEq, Serialize)]
        pub struct Rules {
            $(pub $field: Length,)*
            pub max_aspect_ratio: f64,
            pub max_microvia_aspect_ratio: f64,
            pub max_controlled_depth_aspect_ratio: f64,
            pub hdi: bool,
            pub stacked_microvias: bool,
        }

        impl Rules {
            pub fn overlay(&mut self, f: &RulesFile) {
                $(if let Some(v) = f.$field { self.$field = v; })*
                if let Some(v) = f.max_aspect_ratio {
                    self.max_aspect_ratio = v;
                }
                if let Some(v) = f.max_microvia_aspect_ratio {
                    self.max_microvia_aspect_ratio = v;
                }
                if let Some(v) = f.max_controlled_depth_aspect_ratio {
                    self.max_controlled_depth_aspect_ratio = v;
                }
                if let Some(v) = f.hdi {
                    self.hdi = v;
                }
                if let Some(v) = f.stacked_microvias {
                    self.stacked_microvias = v;
                }
            }

            pub fn table(&self) -> Vec<(&'static str, Length, &'static str)> {
                vec![$((stringify!($field), self.$field, $doc),)*]
            }
        }
    };
}

rules! {
    min_track_width: "narrowest copper track the fab will etch",
    min_clearance: "copper to copper spacing",
    min_drill: "smallest plated hole",
    min_via_drill: "smallest via hole",
    min_via_diameter: "smallest via pad",
    min_annular_ring: "copper left around a via hole",
    min_blind_via_drill: "smallest mechanically drilled blind or buried via hole",
    min_controlled_depth_drill: "smallest blind via hole drilled to a controlled depth",
    min_microvia_drill: "smallest laser drilled microvia hole",
    max_microvia_drill: "largest laser drilled microvia hole",
    min_microvia_diameter: "smallest microvia capture pad",
    min_hole_to_hole: "drill edge to drill edge",
    min_copper_to_edge: "copper to board outline",
    min_silk_width: "silkscreen line width",
    min_silk_text_height: "silkscreen text height",
    min_mask_web: "solder mask left between openings of different nets",
    max_drill: "largest drilled hole",
    min_npth_drill: "smallest non-plated hole",
    min_plated_slot_width: "narrowest plated slot",
    min_npth_slot_width: "narrowest non-plated slot",
    min_pth_annular_ring: "copper left around a plated pad hole",
    min_via_hole_to_copper: "via hole wall to copper of another net",
    min_pth_hole_to_copper: "plated pad hole wall to copper of another net",
    min_inner_pth_hole_to_copper: "plated pad hole wall to other-net copper on inner layers",
    min_npth_to_copper: "non-plated hole wall to any copper",
    min_smd_pad_gap: "SMD pad to SMD pad of another net",
    min_hole_to_smd_pad: "via hole wall to the edge of an SMD pad it does not sit in",
    max_filled_via_drill: "largest via hole the fab fills and caps in a pad",
    min_bga_pad: "smallest BGA pad",
    min_bga_pitch: "finest BGA pitch the assembler places",
    min_part_to_edge: "SMD part pads to board outline, for assembly",
    min_body_to_edge: "part body (courtyard) to board outline, for depaneling",
    flex_zone: "distance from the edge, corners and mounting holes where bending cracks MLCCs",
}

pub const FAB_PRESETS: &[&str] = &["generic", "jlcpcb", "hdi"];

const BLIND_VIA_DRILL: f64 = 0.2;

#[derive(Clone, Debug, PartialEq)]
pub struct FabSetup {
    pub layers: usize,
    pub outer_oz: f64,
    pub finish: String,
}

impl Default for FabSetup {
    fn default() -> Self {
        FabSetup { layers: 2, outer_oz: 1.0, finish: "HASL".into() }
    }
}

pub fn fab_rules(name: &str) -> Option<Rules> {
    fab_rules_for(name, &FabSetup::default())
}

pub fn fab_rules_for(name: &str, s: &FabSetup) -> Option<Rules> {
    let mm = Length::mm;
    let multi = s.layers > 2;
    let enig = s.finish.eq_ignore_ascii_case("ENIG");
    match name {
        "hdi" => Some(Rules {
            min_track_width: mm(0.065),
            min_clearance: mm(0.065),
            min_drill: mm(0.15),
            min_via_drill: mm(0.15),
            min_via_diameter: mm(0.35),
            min_annular_ring: mm(0.1),
            min_blind_via_drill: mm(0.15),
            min_controlled_depth_drill: mm(0.15),
            min_microvia_drill: mm(0.1),
            max_microvia_drill: mm(0.2),
            min_microvia_diameter: mm(0.25),
            min_hole_to_hole: mm(0.25),
            min_via_hole_to_copper: mm(0.15),
            min_copper_to_edge: mm(0.3),
            min_bga_pad: mm(0.2),
            min_bga_pitch: mm(0.4),
            max_aspect_ratio: 14.0,
            max_microvia_aspect_ratio: 0.8,
            hdi: true,
            stacked_microvias: true,
            ..fab_rules_for("generic", s)?
        }),
        "generic" => Some(Rules {
            min_track_width: mm(0.15),
            min_clearance: mm(0.15),
            min_drill: mm(0.3),
            min_via_drill: mm(0.3),
            min_via_diameter: mm(0.6),
            min_annular_ring: mm(0.15),
            min_blind_via_drill: mm(BLIND_VIA_DRILL),
            min_controlled_depth_drill: mm(BLIND_VIA_DRILL),
            min_microvia_drill: mm(0.1),
            max_microvia_drill: mm(0.15),
            min_microvia_diameter: mm(0.3),
            min_hole_to_hole: mm(0.5),
            min_copper_to_edge: mm(0.5),
            min_silk_width: mm(0.12),
            min_silk_text_height: mm(1.0),
            min_mask_web: mm(0.1),
            max_drill: mm(6.3),
            min_npth_drill: mm(0.5),
            min_plated_slot_width: mm(0.5),
            min_npth_slot_width: mm(1.0),
            min_pth_annular_ring: mm(0.2),
            min_via_hole_to_copper: mm(0.25),
            min_pth_hole_to_copper: mm(0.3),
            min_inner_pth_hole_to_copper: mm(0.3),
            min_npth_to_copper: mm(0.25),
            min_smd_pad_gap: mm(0.15),
            min_hole_to_smd_pad: mm(0.2),
            max_filled_via_drill: mm(0.5),
            min_bga_pad: mm(0.25),
            min_bga_pitch: mm(0.5),
            min_part_to_edge: mm(0.5),
            min_body_to_edge: mm(1.0),
            flex_zone: mm(5.0),
            max_aspect_ratio: 8.0,
            max_microvia_aspect_ratio: 0.8,
            max_controlled_depth_aspect_ratio: 1.0,
            hdi: false,
            stacked_microvias: false,
        }),
        "jlcpcb" => {
            let track = match (multi, s.outer_oz) {
                (_, oz) if oz <= 1.25 => {
                    if multi {
                        0.09
                    } else {
                        0.10
                    }
                }
                (true, _) => 0.15,
                (false, oz) if oz <= 2.25 => 0.16,
                (false, oz) if oz <= 2.75 => 0.2,
                (false, oz) if oz <= 3.75 => 0.25,
                _ => 0.3,
            };
            let single = s.layers <= 1;
            Some(Rules {
                min_track_width: mm(track),
                min_clearance: mm(track),
                min_drill: mm(if single { 0.3 } else { 0.15 }),
                min_via_drill: mm(if single { 0.3 } else { 0.15 }),
                min_via_diameter: mm(if single { 0.5 } else { 0.25 }),
                min_annular_ring: mm(0.05),
                min_blind_via_drill: mm(BLIND_VIA_DRILL),
                min_controlled_depth_drill: mm(BLIND_VIA_DRILL),
                min_microvia_drill: mm(0.1),
                max_microvia_drill: mm(0.15),
                min_microvia_diameter: mm(0.3),
                min_hole_to_hole: mm(0.5),
                min_copper_to_edge: mm(0.3),
                min_silk_width: mm(0.15),
                min_silk_text_height: mm(1.0),
                min_mask_web: mm(0.1),
                max_drill: mm(6.3),
                min_npth_drill: mm(0.5),
                min_plated_slot_width: mm(if multi { 0.35 } else { 0.5 }),
                min_npth_slot_width: mm(1.0),
                min_pth_annular_ring: mm(match (multi, s.outer_oz > 1.25) {
                    (_, true) => 0.254,
                    (true, false) => 0.15,
                    (false, false) => 0.18,
                }),
                min_via_hole_to_copper: mm(0.2),
                min_pth_hole_to_copper: mm(0.28),
                min_inner_pth_hole_to_copper: mm(0.3),
                min_npth_to_copper: mm(0.2),
                min_smd_pad_gap: mm(0.15),
                min_hole_to_smd_pad: mm(0.2),
                max_filled_via_drill: mm(0.55),
                min_bga_pad: mm(if enig { 0.2 } else { 0.25 }),
                min_bga_pitch: mm(0.3),
                min_part_to_edge: mm(0.5),
                min_body_to_edge: mm(1.0),
                flex_zone: mm(5.0),
                max_aspect_ratio: 10.7,
                max_microvia_aspect_ratio: 0.8,
                max_controlled_depth_aspect_ratio: 1.0,
                hdi: false,
                stacked_microvias: false,
            })
        }
        _ => None,
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrcFile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disable: Vec<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub severity: std::collections::BTreeMap<String, crate::diag::Severity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tombstone_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<crate::place::PlacementLimits>,
}

pub fn stackup_preset(name: &str) -> Option<Vec<LayerFile>> {
    crate::stackups::find_stackup_preset(name).map(|p| p.layers.clone())
}

#[derive(Clone, Debug, Serialize)]
pub struct Layer {
    pub name: String,
    pub kind: LayerKind,
    pub thickness: Length,
    pub material: String,
    pub er: f64,
    pub loss_tangent: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Stackup {
    pub preset: Option<String>,
    pub finish: String,
    pub mask_color: String,
    pub silk_color: String,
    pub roughness_um: Option<f64>,
    pub huray: Option<(f64, f64)>,
    pub nickel_um: Option<f64>,
    pub gold_um: Option<f64>,
    pub layers: Vec<Layer>,
    pub lamination: Vec<DrillStep>,
}

impl Stackup {
    pub fn copper(&self) -> impl Iterator<Item = (usize, &Layer)> {
        self.layers.iter().enumerate().filter(|(_, l)| l.kind == LayerKind::Copper)
    }

    pub fn outer_oz(&self) -> f64 {
        self.copper().next().map(|(_, l)| l.thickness.to_mm() / 0.035).unwrap_or(1.0)
    }

    pub fn fab_setup(&self) -> FabSetup {
        FabSetup {
            layers: self.copper().count(),
            outer_oz: self.outer_oz(),
            finish: self.finish.clone(),
        }
    }

    pub fn copper_names(&self) -> Vec<String> {
        self.copper().map(|(_, l)| l.name.clone()).collect()
    }

    pub fn thickness(&self) -> Length {
        self.layers
            .iter()
            .filter(|l| {
                l.kind == LayerKind::Copper || l.kind.is_dielectric() || l.kind == LayerKind::Mask
            })
            .map(|l| l.thickness)
            .sum()
    }

    pub fn copper_z(&self, name: &str) -> Option<f64> {
        let mut z = 0.0;
        for l in &self.layers {
            let t = l.thickness.to_mm();
            if l.name == name && l.kind == LayerKind::Copper {
                return Some(z + t / 2.0);
            }
            if l.kind == LayerKind::Copper || l.kind.is_dielectric() {
                z += t;
            }
        }
        None
    }

    pub fn er_between(&self, a: &str, b: &str) -> f64 {
        let (Some(i), Some(j)) = (self.index_of(a), self.index_of(b)) else { return 4.2 };
        let (lo, hi) = (i.min(j), i.max(j));
        let (mut h, mut w) = (0.0, 0.0);
        for l in &self.layers[lo..=hi] {
            if l.kind.is_dielectric() {
                h += l.thickness.to_mm();
                w += l.thickness.to_mm() * l.er;
            }
        }
        if h > 0.0 { w / h } else { 4.2 }
    }

    pub fn depth(&self, from: &str, to: &str) -> Length {
        let (Some(i), Some(j)) = (self.index_of(from), self.index_of(to)) else {
            return Length::ZERO;
        };
        self.layers[i.min(j) + 1..i.max(j)]
            .iter()
            .filter(|l| l.kind == LayerKind::Copper || l.kind.is_dielectric())
            .map(|l| l.thickness)
            .sum()
    }

    pub fn span_depth(&self, from: &str, to: &str) -> Length {
        let (Some(i), Some(j)) = (self.index_of(from), self.index_of(to)) else {
            return Length::ZERO;
        };
        self.layers[i.min(j)..=i.max(j)]
            .iter()
            .filter(|l| l.kind == LayerKind::Copper || l.kind.is_dielectric())
            .map(|l| l.thickness)
            .sum()
    }

    fn core_gaps(&self) -> Vec<bool> {
        let cu: Vec<usize> = self.copper().map(|(i, _)| i).collect();
        cu.windows(2)
            .map(|w| self.layers[w[0] + 1..w[1]].iter().any(|l| l.kind == LayerKind::Core))
            .collect()
    }

    pub fn build_up_layers(&self) -> (usize, usize) {
        let cores = self.core_gaps();
        if !cores.contains(&true) {
            return (0, 0);
        }
        let top = cores.iter().take_while(|c| !**c).count();
        let bottom = cores.iter().rev().take_while(|c| !**c).count();
        (top, bottom)
    }

    pub fn build_name(&self) -> String {
        let n = self.copper().count();
        match self.build_up_layers() {
            (0, 0) => format!("{n} layer single lamination"),
            (t, b) => format!("{t}+{}+{b}", n - t - b),
        }
    }

    pub fn derived_lamination(&self) -> Vec<DrillStep> {
        let cu = self.copper_names();
        if cu.len() < 2 {
            return Vec::new();
        }
        let last = cu.len() - 1;
        let (top, bottom) = self.build_up_layers();
        let presses = top.max(bottom);
        let mut out: Vec<DrillStep> = Vec::new();
        let mut push = |a: usize, b: usize, drill_kind: DrillKind| {
            let s = DrillStep { from: cu[a].clone(), to: cu[b].clone(), drill_kind };
            if !out.contains(&s) {
                out.push(s);
            }
        };
        for (g, core) in self.core_gaps().into_iter().enumerate().take(last - bottom).skip(top) {
            if core {
                push(g, g + 1, DrillKind::Mechanical);
            }
        }
        push(top, last - bottom, DrillKind::Mechanical);
        for k in 1..=presses {
            let (on_top, on_bottom) =
                (k.saturating_sub(presses - top), k.saturating_sub(presses - bottom));
            let (t, b) = (top - on_top, last - bottom + on_bottom);
            if on_top > 0 {
                push(t, t + 1, DrillKind::Laser);
            }
            if on_bottom > 0 {
                push(b - 1, b, DrillKind::Laser);
            }
            if on_top >= 2 {
                push(t, t + 2, DrillKind::Laser);
            }
            if on_bottom >= 2 {
                push(b - 2, b, DrillKind::Laser);
            }
            push(t, b, DrillKind::Mechanical);
        }
        for inner in 1..last {
            push(0, inner, DrillKind::ControlledDepth);
        }
        for inner in 1..last {
            push(inner, last, DrillKind::ControlledDepth);
        }
        out
    }

    pub fn drill_steps(&self) -> Vec<DrillStep> {
        if self.lamination.is_empty() { self.derived_lamination() } else { self.lamination.clone() }
    }

    pub fn drills_via(&self, v: &Via) -> Result<(), String> {
        self.drillable(&v.from, &v.to, v.drill_kind, v.stacked)
    }

    fn copper_span(&self, from: &str, to: &str) -> Option<(usize, usize)> {
        let cu = self.copper_names();
        let a = cu.iter().position(|c| c == from)?;
        let b = cu.iter().position(|c| c == to)?;
        Some((a.min(b), a.max(b)))
    }

    pub fn lamination_summary(&self) -> String {
        let steps = self.drill_steps();
        let mut kinds = Vec::new();
        for kind in [DrillKind::Mechanical, DrillKind::Laser, DrillKind::ControlledDepth] {
            let spans: Vec<String> = steps
                .iter()
                .filter(|s| s.drill_kind == kind)
                .map(|s| format!("{}-{}", s.from, s.to))
                .collect();
            if !spans.is_empty() {
                kinds.push(format!("{} {}", kind.name(), spans.join(", ")));
            }
        }
        let source = if self.lamination.is_empty() {
            format!(
                "the {} build of the stackup (cores, core sub-stack, build-up layers)",
                self.build_name()
            )
        } else {
            "[stackup] lamination".to_string()
        };
        format!("{source} drills {}", kinds.join("; "))
    }

    pub fn drillable(
        &self,
        from: &str,
        to: &str,
        drill_kind: DrillKind,
        stacked: bool,
    ) -> Result<(), String> {
        let Some((a, b)) = self.copper_span(from, to) else {
            return Err(format!("{from} or {to} is not a copper layer"));
        };
        let steps = self.drill_steps();
        let has = |x: usize, y: usize| {
            steps.iter().any(|s| {
                s.drill_kind == drill_kind && self.copper_span(&s.from, &s.to) == Some((x, y))
            })
        };
        let ok = if stacked { (a..b).all(|x| has(x, x + 1)) } else { has(a, b) };
        if ok {
            return Ok(());
        }
        let what = if stacked { "every hop of the stack" } else { "the span" };
        Err(format!(
            "no {} drill step of the lamination matches {what}; {}",
            drill_kind.name(),
            self.lamination_summary()
        ))
    }

    fn step_of(&self, steps: &[DrillStep], a: usize, b: usize, kind: DrillKind) -> Option<usize> {
        steps
            .iter()
            .position(|s| s.drill_kind == kind && self.copper_span(&s.from, &s.to) == Some((a, b)))
    }

    pub fn drill_order(
        &self,
        from: &str,
        to: &str,
        drill_kind: DrillKind,
        stacked: bool,
    ) -> Option<(usize, usize)> {
        let (a, b) = self.copper_span(from, to)?;
        let steps = self.drill_steps();
        if !stacked {
            let i = self.step_of(&steps, a, b, drill_kind)?;
            return Some((i, i));
        }
        let hops = (a..b)
            .map(|x| self.step_of(&steps, x, x + 1, drill_kind))
            .collect::<Option<Vec<usize>>>()?;
        Some((*hops.iter().min()?, *hops.iter().max()?))
    }

    pub fn stack_order_error(&self, from: &str, to: &str, drill_kind: DrillKind) -> Option<String> {
        let (a, b) = self.copper_span(from, to)?;
        let cu = self.copper_names();
        let last = cu.len() - 1;
        let steps = self.drill_steps();
        let hops: Vec<usize> = match a.cmp(&(last - b)) {
            std::cmp::Ordering::Less => (a..b).rev().collect(),
            std::cmp::Ordering::Greater => (a..b).collect(),
            std::cmp::Ordering::Equal => return None,
        };
        let order = hops
            .iter()
            .map(|&x| self.step_of(&steps, x, x + 1, drill_kind))
            .collect::<Option<Vec<usize>>>()?;
        (1..hops.len()).find(|&k| order[k] < order[k - 1]).map(|k| {
            let (outer, inner) = (hops[k], hops[k - 1]);
            format!(
                "the hop {}-{} is drilled at lamination step {}, before the hop {}-{} under it at step {}: a stack is built from the inside out",
                cu[outer],
                cu[outer + 1],
                order[k] + 1,
                cu[inner],
                cu[inner + 1],
                order[k - 1] + 1
            )
        })
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.name == name)
    }

    fn dielectric_run(&self, range: impl Iterator<Item = usize>) -> Option<(f64, f64)> {
        let mut h = 0.0;
        let mut weighted = 0.0;
        for i in range {
            let l = &self.layers[i];
            match l.kind {
                LayerKind::Copper => {
                    return (h > 0.0).then(|| (h, weighted / h));
                }
                k if k.is_dielectric() => {
                    h += l.thickness.to_mm();
                    weighted += l.thickness.to_mm() * l.er;
                }
                _ => {}
            }
        }
        None
    }

    pub fn geometry(&self, layer: &str) -> Option<TraceGeometry> {
        let i = self.index_of(layer)?;
        let l = &self.layers[i];
        if l.kind != LayerKind::Copper {
            return None;
        }
        let t = l.thickness.to_mm();
        let above = self.dielectric_run((0..i).rev());
        let below = self.dielectric_run(i + 1..self.layers.len());
        match (above, below) {
            (Some((h1, e1)), Some((h2, e2))) => Some(TraceGeometry::Stripline {
                h1_mm: h1,
                h2_mm: h2,
                er: (h1 * e1 + h2 * e2) / (h1 + h2),
                t_mm: t,
            }),
            (Some((h, er)), None) | (None, Some((h, er))) => {
                Some(TraceGeometry::Microstrip { h_mm: h, er, t_mm: t })
            }
            (None, None) => None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outline {
    Rect { origin: Point, size: Point, corner_radius: Length },
    Polygon { points: Vec<Point> },
}

impl Outline {
    pub fn points(&self) -> Vec<crate::geom::P> {
        match self {
            Outline::Rect { origin, size, corner_radius } => {
                let [w, h] = size.to_mm();
                let [x0, y0] = origin.to_mm();
                crate::geom::rounded_rect(w, h, corner_radius.to_mm(), 8)
                    .into_iter()
                    .map(|q| [q[0] + x0 + w / 2.0, q[1] + y0 + h / 2.0])
                    .collect()
            }
            Outline::Polygon { points } => points.iter().map(|p| p.to_mm()).collect(),
        }
    }

    fn check(&self, at: &str, what: &str, d: &mut Diags) {
        match self {
            Outline::Polygon { points } if points.len() < 3 => {
                d.error(
                    format!("{at}.points"),
                    format!("a polygon {what} needs at least three points"),
                );
            }
            Outline::Rect { size, corner_radius, .. } => {
                if !size.0.is_positive() || !size.1.is_positive() {
                    d.error(format!("{at}.size"), format!("{what} size must be positive"));
                }
                if *corner_radius * 2.0 > size.0.min(size.1) {
                    d.error(
                        format!("{at}.corner_radius"),
                        format!("corner radius is more than half the {what}"),
                    );
                }
            }
            Outline::Polygon { .. } => {}
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Backdrill {
    pub from: String,
    pub to: String,
    pub max_stub: Length,
    pub diameter: Length,
}

#[derive(Clone, Debug, Serialize)]
pub struct Via {
    pub name: String,
    pub drill: Length,
    pub diameter: Length,
    pub from: String,
    pub to: String,
    pub kind: ViaKind,
    pub stacked: bool,
    pub skip: bool,
    pub drill_kind: DrillKind,
    pub fill: Option<ViaFill>,
    pub backdrill: Option<Backdrill>,
    pub cost: f64,
}

impl Via {
    pub fn annular_ring(&self) -> Length {
        (self.diameter - self.drill) / 2.0
    }

    pub fn span(&self, copper: &[String]) -> (usize, usize) {
        let a = copper.iter().position(|c| *c == self.from).unwrap_or(0);
        let b = copper.iter().position(|c| *c == self.to).unwrap_or(copper.len().saturating_sub(1));
        (a.min(b), a.max(b))
    }

    pub fn hole_layers(&self, copper: &[String]) -> Vec<String> {
        let (a, b) = self.span(copper);
        copper.get(a..=b).map(<[String]>::to_vec).unwrap_or_default()
    }

    pub fn copper_layers(&self, copper: &[String]) -> Vec<String> {
        let (mut a, mut b) = self.span(copper);
        if let Some(bd) = &self.backdrill
            && let (Some(side), Some(keep)) =
                (copper.iter().position(|c| *c == bd.from), copper.iter().position(|c| *c == bd.to))
        {
            if side <= a && keep > a && keep <= b {
                a = keep;
            } else if side >= b && keep < b && keep >= a {
                b = keep;
            }
        }
        copper.get(a..=b).map(<[String]>::to_vec).unwrap_or_default()
    }

    pub fn covers(&self, copper: &[String], x: usize, y: usize) -> bool {
        let (a, b) = self.span(copper);
        let reach = self.copper_layers(copper);
        let on = |l: usize| copper.get(l).is_some_and(|n| reach.contains(n));
        a <= x.min(y) && x.max(y) <= b && on(x) && on(y)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Netclass {
    pub name: String,
    pub description: String,
    pub voltage: Option<Voltage>,
    pub track_width: Length,
    pub clearance: Length,
    pub via: Vec<String>,
    pub current: Option<Amps>,
    pub max_temp_rise: Kelvin,
    pub impedance: Option<Ohms>,
    pub impedance_tolerance: Percent,
    pub solver: Solver,
    pub diff_gap: Option<Length>,
    pub coplanar_gap: Option<Length>,
    pub max_skew: Option<Length>,
    pub max_uncoupled: Option<Length>,
    pub neckdown: Option<Length>,
    pub layers: Vec<String>,
    pub widths: std::collections::BTreeMap<String, Length>,
}

impl Netclass {
    pub fn width_on(&self, layer: &str) -> Length {
        self.widths.get(layer).copied().unwrap_or(self.track_width)
    }

    pub fn line(&self) -> Line {
        Line {
            diff_gap_mm: self.diff_gap.map(Length::to_mm),
            coplanar_gap_mm: self.coplanar_gap.map(Length::to_mm),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Board {
    pub pollution_degree: u8,
    pub name: String,
    pub description: String,
    pub fab: String,
    pub outline: Option<Outline>,
    pub cutouts: Vec<Outline>,
    pub stackup: Stackup,
    pub rules: Rules,
    pub vias: Vec<Via>,
    pub netclasses: Vec<Netclass>,
    pub domains: Vec<Domain>,
    pub barriers: Vec<Barrier>,
    pub drc: DrcFile,
}

impl Board {
    pub fn domain_of(&self, net: &str, class: &str) -> Vec<usize> {
        let held = |implicit: bool| -> Vec<usize> {
            (0..self.domains.len())
                .filter(|&i| {
                    self.domains[i].implicit == implicit && self.domains[i].holds(net, class)
                })
                .collect()
        };
        let explicit = held(false);
        if explicit.is_empty() { held(true) } else { explicit }
    }

    pub fn netclass(&self, name: &str) -> Option<&Netclass> {
        self.netclasses.iter().find(|n| n.name == name)
    }

    pub fn voltage_of(&self, class: &str) -> Option<Voltage> {
        self.netclass(class).and_then(|n| n.voltage)
    }

    pub fn barrier(&self, a: Option<usize>, b: Option<usize>) -> Option<&Barrier> {
        let (a, b) = (a?, b?);
        if a == b {
            return None;
        }
        self.barriers.iter().find(|x| x.between == [a, b] || x.between == [b, a])
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LayerAnalysis {
    pub netclass: String,
    pub layer: String,
    pub geometry: TraceGeometry,
    pub impedance: f64,
    pub impedance_ok: Option<bool>,
    pub width_for_impedance: Option<Length>,
    pub current_capacity: Amps,
    pub width_for_current: Option<Length>,
}

fn copper_name(i: usize, n: usize) -> String {
    match i {
        0 => "F.Cu".into(),
        i if i + 1 == n => "B.Cu".into(),
        i => format!("In{i}.Cu"),
    }
}

impl BoardFile {
    pub fn resolve(&self, d: &mut Diags) -> Board {
        let fab = self.fab.clone().unwrap_or_else(|| "generic".into());
        let stackup = self.resolve_stackup(d);
        let setup = stackup.fab_setup();
        let mut rules = fab_rules_for(&fab, &setup).unwrap_or_else(|| {
            d.error("fab", format!("unknown fab `{fab}`, use one of {}", FAB_PRESETS.join(", ")));
            fab_rules_for("generic", &setup).unwrap()
        });
        rules.overlay(&self.rules);

        let copper = stackup.copper_names();
        let first = copper.first().cloned().unwrap_or_default();
        let last = copper.last().cloned().unwrap_or_default();

        let vias = self
            .vias
            .iter()
            .map(|v| {
                let from = v.from.clone().unwrap_or_else(|| first.clone());
                let to = v.to.clone().unwrap_or_else(|| last.clone());
                let kind = v.kind.unwrap_or_else(|| {
                    match (
                        copper.iter().position(|c| *c == from),
                        copper.iter().position(|c| *c == to),
                    ) {
                        (Some(a), Some(b)) => ViaKind::of_span(a, b, copper.len()),
                        _ => ViaKind::Through,
                    }
                });
                Via {
                    name: v.name.clone(),
                    drill: v.drill,
                    diameter: v.diameter,
                    from,
                    to,
                    kind,
                    stacked: v.stacked,
                    skip: v.skip,
                    drill_kind: v.drill_kind.unwrap_or(if kind == ViaKind::Microvia {
                        DrillKind::Laser
                    } else {
                        DrillKind::Mechanical
                    }),
                    fill: v.fill,
                    backdrill: v.backdrill.as_ref().map(|b| Backdrill {
                        from: b.from.clone(),
                        to: b.to.clone(),
                        max_stub: b.max_stub.unwrap_or(Length::mm(0.25)),
                        diameter: b.diameter.unwrap_or(v.drill + Length::mm(0.2)),
                    }),
                    cost: v.cost.unwrap_or(1.0),
                }
            })
            .collect();

        let netclasses: Vec<Netclass> = self
            .netclasses
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let at = format!("netclasses[{i}]");
                if n.track_width.is_none() && n.impedance.is_none() {
                    d.error(&at, format!("netclass `{}` needs `track_width`", n.name));
                }
                if n.voltage.is_none() {
                    d.warn(
                        &at,
                        format!(
                            "netclass `{}` has no `voltage`: give it the highest voltage its nets reach (`3.3VDC` for 3.3 V logic, `0VDC` for ground); it sets the spacing to other classes, the rails of power nets and the capacitor ratings",
                            n.name
                        ),
                    );
                }
                let layers = if n.layers.is_empty() { copper.clone() } else { n.layers.clone() };
                let track_width = n.track_width.unwrap_or_else(|| {
                    n.impedance
                        .and_then(|z| {
                            let g = stackup.geometry(layers.first()?)?;
                            let line = Line {
                                diff_gap_mm: n.diff_gap.map(Length::to_mm),
                                coplanar_gap_mm: n.coplanar_gap.map(Length::to_mm),
                            };
                            g.width_for(z.0, line).map(Length::mm)
                        })
                        .unwrap_or(rules.min_track_width)
                });
                let rated = n.voltage.map(|v| Length::mm(crate::insulation::own_clearance(v)));
                if let (Some(need), Some(set), Some(v)) = (rated, n.clearance, n.voltage)
                    && set < need
                {
                    d.error(
                        &at,
                        format!(
                            "netclass `{}` carries {v}, which needs {need} between its own nets on an outer layer (IPC-2221B table 6-1, uncoated); `clearance` is {set}, raise it or leave it out",
                            n.name
                        ),
                    );
                }
                let clearance = n
                    .clearance
                    .unwrap_or_else(|| rated.map_or(rules.min_clearance, |r| r.max(rules.min_clearance)));
                Netclass {
                    name: n.name.clone(),
                    description: n.description.clone(),
                    voltage: n.voltage,
                    track_width,
                    clearance,
                    via: n.via.as_ref().map(ViaNames::list).unwrap_or_default(),
                    current: n.current,
                    max_temp_rise: n.max_temp_rise.unwrap_or(Kelvin(10.0)),
                    impedance: n.impedance,
                    impedance_tolerance: n.impedance_tolerance.unwrap_or(Percent(10.0)),
                    solver: n.solver.unwrap_or_default(),
                    diff_gap: n.diff_gap,
                    coplanar_gap: n.coplanar_gap,
                    max_skew: n.max_skew,
                    max_uncoupled: n.max_uncoupled,
                    neckdown: n.neckdown,
                    layers,
                    widths: n.widths.clone(),
                }
            })
            .collect();

        let outline = self.outline.as_ref().and_then(|o| {
            outline_shape(&o.points, o.size, o.origin, o.corner_radius, "outline", d)
        });
        let cutouts = self
            .outline
            .iter()
            .flat_map(|o| o.cutouts.iter().enumerate())
            .filter_map(|(i, c)| {
                let at = format!("outline.cutouts[{i}]");
                outline_shape(&c.points, c.size, c.origin, c.corner_radius, &at, d)
            })
            .collect();

        let pollution_degree = self.pollution_degree.unwrap_or(2);
        if !(1..=3).contains(&pollution_degree) {
            d.error("pollution_degree", "`pollution_degree` is 1, 2 or 3");
        }
        let (mut domains, mut barriers) = self.resolve_isolation(d);
        implicit_isolation(&netclasses, pollution_degree, &mut domains, &mut barriers);
        Board {
            pollution_degree,
            name: self.name.clone(),
            description: self.description.clone(),
            fab,
            outline,
            cutouts,
            stackup,
            rules,
            vias,
            netclasses,
            domains,
            barriers,
            drc: self.drc.clone(),
        }
    }

    fn resolve_isolation(&self, d: &mut Diags) -> (Vec<Domain>, Vec<Barrier>) {
        let mut domains: Vec<Domain> = Vec::new();
        for (i, f) in self.domains.iter().enumerate() {
            let at = format!("domains[{i}]");
            if domains.iter().any(|x| x.name == f.name) {
                d.error(&at, format!("domain `{}` is named twice", f.name));
                continue;
            }
            if f.classes.is_empty() && f.nets.is_empty() {
                d.error(&at, format!("domain `{}` needs `classes` or `nets`", f.name));
            }
            for c in &f.classes {
                if !self.netclasses.iter().any(|n| &n.name == c) {
                    d.error(
                        &at,
                        format!("domain `{}` names netclass `{c}`, which is not defined", f.name),
                    );
                }
            }
            domains.push(Domain {
                name: f.name.clone(),
                description: f.description.clone(),
                classes: f.classes.clone(),
                nets: f.nets.clone(),
                implicit: false,
            });
        }
        let mut barriers: Vec<Barrier> = Vec::new();
        for (i, f) in self.barriers.iter().enumerate() {
            let at = format!("barriers[{i}]");
            let ends: Vec<Option<usize>> =
                f.between.iter().map(|n| domains.iter().position(|x| &x.name == n)).collect();
            if f.between.len() != 2 {
                d.error(&at, "`between` names two domains");
                continue;
            }
            let (Some(a), Some(b)) = (ends[0], ends[1]) else {
                for (n, e) in f.between.iter().zip(&ends) {
                    if e.is_none() {
                        d.error(&at, format!("`{n}` is not a domain"));
                    }
                }
                continue;
            };
            if a == b {
                d.error(
                    &at,
                    format!("a barrier joins two different domains, not `{}` twice", f.between[0]),
                );
                continue;
            }
            if barriers.iter().any(|x| x.between == [a, b] || x.between == [b, a]) {
                d.error(
                    &at,
                    format!("`{}` and `{}` already have a barrier", f.between[0], f.between[1]),
                );
                continue;
            }
            if f.clearance.is_none() && f.creepage.is_none() {
                d.error(&at, "a barrier needs `clearance`, `creepage` or both");
                continue;
            }
            let pollution_degree = f.pollution_degree.unwrap_or(2);
            if !(1..=3).contains(&pollution_degree) {
                d.error(&at, "`pollution_degree` is 1, 2 or 3");
            }
            if let (Some(c), Some(clear)) = (f.creepage, f.clearance)
                && c < clear
            {
                d.warn(
                    &at,
                    "`creepage` is under `clearance`, so the surface path is never the limit",
                );
            }
            barriers.push(Barrier {
                between: [a, b],
                description: f.description.clone(),
                clearance: f.clearance,
                creepage: f.creepage,
                pollution_degree,
            });
        }
        (domains, barriers)
    }

    fn resolve_stackup(&self, d: &mut Diags) -> Stackup {
        let s = &self.stackup;
        let files = match (&s.preset, s.layers.is_empty()) {
            (Some(p), true) => stackup_preset(p).unwrap_or_else(|| {
                let near = crate::stackups::suggest_stackup_presets(p, 5);
                d.error(
                    "stackup.preset",
                    format!(
                        "unknown preset `{p}`; close: {}; `agentee stackups` lists all {}",
                        near.join(", "),
                        crate::stackups::stackup_presets().len()
                    ),
                );
                Vec::new()
            }),
            (Some(_), false) => {
                d.error("stackup", "give either `preset` or `layers`, not both");
                s.layers.clone()
            }
            (None, _) => s.layers.clone(),
        };
        let n_cu = files.iter().filter(|l| l.kind == LayerKind::Copper).count();
        let mut seen_cu = 0;
        let mut dielectrics_here = 0;
        let layers = files
            .iter()
            .map(|f| {
                let stacked = match f.kind {
                    k if k.is_dielectric() => {
                        dielectrics_here += 1;
                        if dielectrics_here > 1 {
                            format!("{}", (b'a' + dielectrics_here as u8 - 2) as char)
                        } else {
                            String::new()
                        }
                    }
                    LayerKind::Copper => {
                        dielectrics_here = 0;
                        String::new()
                    }
                    _ => String::new(),
                };
                let side = if seen_cu == 0 { "F" } else { "B" };
                let name = f.name.clone().unwrap_or_else(|| match f.kind {
                    LayerKind::Copper => copper_name(seen_cu, n_cu),
                    LayerKind::Silk => format!("{side}.SilkS"),
                    LayerKind::Paste => format!("{side}.Paste"),
                    LayerKind::Mask => format!("{side}.Mask"),
                    LayerKind::Core => format!("core{seen_cu}{stacked}"),
                    LayerKind::Prepreg => format!("prepreg{seen_cu}{stacked}"),
                });
                if f.kind == LayerKind::Copper {
                    seen_cu += 1;
                }
                let default_er = match f.kind {
                    LayerKind::Mask => 3.8,
                    k if k.is_dielectric() => 4.5,
                    _ => 1.0,
                };
                Layer {
                    name,
                    kind: f.kind,
                    thickness: f.thickness.unwrap_or(Length::ZERO),
                    material: f.material.clone().unwrap_or_else(|| match f.kind {
                        LayerKind::Copper => "copper".into(),
                        k if k.is_dielectric() => "FR4".into(),
                        _ => String::new(),
                    }),
                    er: f.er.unwrap_or(default_er),
                    loss_tangent: f.loss_tangent.unwrap_or(0.0),
                }
            })
            .collect();
        Stackup {
            preset: s.preset.clone(),
            finish: s.finish.clone().unwrap_or_else(|| "HASL".into()),
            mask_color: s.mask_color.clone().unwrap_or_else(|| "green".into()),
            silk_color: s.silk_color.clone().unwrap_or_else(|| "white".into()),
            roughness_um: s.roughness.map(|r| r.to_mm() * 1e3),
            huray: s.huray.as_ref().map(|h| (h.radius.to_mm() * 1e3, h.ratio)),
            nickel_um: s.nickel.map(|t| t.to_mm() * 1e3),
            gold_um: s.gold.map(|t| t.to_mm() * 1e3),
            layers,
            lamination: s.lamination.clone(),
        }
    }
}

fn n_of<'a>(v: &'a [Netclass], name: &str) -> &'a Netclass {
    v.iter().find(|n| n.name == name).unwrap()
}

impl Board {
    pub fn via_for(
        &self,
        name: Option<&str>,
        class: Option<&Netclass>,
        reach: &[&str],
    ) -> Option<&Via> {
        let names: Vec<String> = name.map(|n| vec![n.to_string()]).unwrap_or_default();
        self.via_among(&names, class, reach)
    }

    pub fn via_among(
        &self,
        names: &[String],
        class: Option<&Netclass>,
        reach: &[&str],
    ) -> Option<&Via> {
        let copper = self.stackup.copper_names();
        let named = |n: &str| self.vias.iter().find(|v| v.name == n);
        let listed: Vec<&Via> = if names.is_empty() {
            class.map(|c| c.via.iter().filter_map(|n| named(n)).collect()).unwrap_or_default()
        } else {
            names.iter().filter_map(|n| named(n)).collect()
        };
        let reaches = |v: &&Via| {
            let on = v.copper_layers(&copper);
            reach.iter().all(|r| on.iter().any(|x| x == r))
        };
        listed.iter().copied().find(reaches).or(listed.first().copied()).or(self.vias.first())
    }

    pub const NECK_SHARE: f64 = 0.5;

    pub fn impedance_widths(
        &self,
        class: &Netclass,
        layer: &str,
        share: f64,
    ) -> Option<(f64, f64)> {
        class.impedance?;
        let g = self.stackup.geometry(layer)?;
        let allow = class.impedance_tolerance.0 / 100.0 * share;
        Some(g.width_range(class.width_on(layer).to_mm(), class.line(), allow))
    }

    pub fn impedance_gap(&self, class: &Netclass, layer: &str, share: f64) -> Option<f64> {
        class.impedance?;
        let g = self.stackup.geometry(layer)?;
        let allow = class.impedance_tolerance.0 / 100.0 * share;
        g.gap_floor(class.width_on(layer).to_mm(), class.line(), allow)
    }

    pub fn needs_pour(&self, class: &Netclass, layer: &str, share: f64) -> bool {
        let (Some(_), Some(s)) = (class.impedance, class.coplanar_gap) else { return false };
        let Some(g) = self.stackup.geometry(layer) else { return false };
        let w = class.width_on(layer).to_mm();
        let bare = g.impedance(w, Line::SINGLE) / g.impedance(w, Line::coplanar(s.to_mm()));
        bare - 1.0 > class.impedance_tolerance.0 / 100.0 * share
    }

    pub fn analyze(&self) -> Vec<LayerAnalysis> {
        let mut out = Vec::new();
        for n in &self.netclasses {
            for layer in &n.layers {
                let Some(g) = self.stackup.geometry(layer) else { continue };
                let gap = n.line();
                let width = n.width_on(layer);
                let z = g.impedance(width.to_mm(), gap);
                let (impedance_ok, width_for_impedance) = match n.impedance {
                    Some(target) => {
                        let tol = target.0 * n.impedance_tolerance.0 / 100.0;
                        (
                            Some((z - target.0).abs() <= tol),
                            g.width_for(target.0, gap).map(Length::mm),
                        )
                    }
                    None => (None, None),
                };
                let rise = n.max_temp_rise.0;
                let cu = g.copper_mm();
                let ext = g.is_external();
                out.push(LayerAnalysis {
                    netclass: n.name.clone(),
                    layer: layer.clone(),
                    geometry: g,
                    impedance: z,
                    impedance_ok,
                    width_for_impedance,
                    current_capacity: Amps(calc::ipc2221_current(width.to_mm(), rise, cu, ext)),
                    width_for_current: n
                        .current
                        .map(|a| Length::mm(calc::ipc2221_width(a.0, rise, cu, ext))),
                });
            }
        }
        out
    }

    pub fn check(&self, d: &mut Diags) {
        self.check_stackup(d);
        let r = &self.rules;
        if self.outline.is_none() {
            d.warn("outline", "no board outline yet");
        }
        if let Some(o) = &self.outline {
            o.check("outline", "board", d);
        }
        for (i, c) in self.cutouts.iter().enumerate() {
            c.check(&format!("outline.cutouts[{i}]"), "cutout", d);
        }

        let copper = self.stackup.copper_names();
        for (i, v) in self.vias.iter().enumerate() {
            self.check_via(i, v, &copper, d);
        }

        for id in self.drc.disable.iter().chain(self.drc.severity.keys()) {
            if crate::drc::find(id).is_none() {
                d.warn("drc", format!("no DRC rule `{id}`, `agentee drc NAME --list` lists them"));
            }
        }

        if !self.netclasses.iter().any(|n| n.name == "Default") {
            d.warn("netclasses", "no `Default` netclass, nets without a class have no rules");
        }
        for (i, n) in self.netclasses.iter().enumerate() {
            let at = format!("netclasses[{i}]");
            if self.netclasses.iter().filter(|o| o.name == n.name).count() > 1 {
                d.error(&at, format!("netclass name `{}` is used twice", n.name));
            }
            for (layer, w) in &n.widths {
                if *w < r.min_track_width {
                    d.error(
                        format!("netclass {}", n.name),
                        format!(
                            "width {w} on {layer} is under the fab minimum {}",
                            r.min_track_width
                        ),
                    );
                }
                if !n.layers.contains(layer) {
                    d.warn(
                        format!("netclass {}", n.name),
                        format!("`widths` names {layer}, which is not in the class `layers`"),
                    );
                }
            }
            if n.track_width < r.min_track_width {
                d.error(
                    &at,
                    format!(
                        "`{}` track {} is under the fab minimum {}",
                        n.name, n.track_width, r.min_track_width
                    ),
                );
            }
            if n.clearance < r.min_clearance {
                d.error(
                    &at,
                    format!(
                        "`{}` clearance {} is under the fab minimum {}",
                        n.name, n.clearance, r.min_clearance
                    ),
                );
            }
            if let Some(g) = n.diff_gap
                && g < r.min_clearance
            {
                d.error(
                    &at,
                    format!(
                        "`{}` pair gap {} is under the fab minimum clearance {}",
                        n.name, g, r.min_clearance
                    ),
                );
            }
            if let Some(g) = n.coplanar_gap {
                if g < r.min_clearance {
                    d.error(
                        &at,
                        format!(
                            "`{}` coplanar gap {} is under the fab minimum clearance {}",
                            n.name, g, r.min_clearance
                        ),
                    );
                }
                if n.diff_gap.is_some() {
                    d.error(&at, format!("`{}` sets both `diff_gap` and `coplanar_gap`, coplanar pairs are not modelled", n.name));
                }
                for l in &n.layers {
                    if matches!(self.stackup.geometry(l), Some(TraceGeometry::Stripline { .. })) {
                        d.error(&at, format!("`{}` coplanar gap on inner layer `{l}`, only outer layers are modelled as grounded coplanar", n.name));
                    }
                }
            }
            if n.diff_gap.is_some() && n.impedance.is_none() {
                d.warn(&at, format!("`{}` has a pair gap but no `impedance` target", n.name));
            }
            for v in &n.via {
                if !self.vias.iter().any(|x| &x.name == v) {
                    d.error(&at, format!("via `{v}` is not defined in [[vias]]"));
                }
            }
            for l in &n.layers {
                if !copper.contains(l) {
                    d.error(
                        &at,
                        format!("layer `{l}` is not a copper layer ({})", copper.join(", ")),
                    );
                }
            }
        }

        for a in self.analyze() {
            let at = format!("netclass {} on {}", a.netclass, a.layer);
            let n = n_of(&self.netclasses, &a.netclass);
            if a.impedance_ok == Some(false) && n.solver != Solver::Field {
                let target = n.impedance.unwrap();
                let key = if n.widths.contains_key(&a.layer) {
                    format!("widths.\"{}\"", a.layer)
                } else {
                    "track_width".to_string()
                };
                let hint = match a.width_for_impedance {
                    Some(w) => format!(", use {key} = \"{w}\""),
                    None => ", no width on this layer reaches it".into(),
                };
                d.error(
                    at.clone(),
                    format!(
                        "{:.1} ohm is outside {} +/- {}{hint}, or restrict `layers`",
                        a.impedance, target, n.impedance_tolerance
                    ),
                );
            }
            if let (Some(need), Some(i)) = (a.width_for_current, n.current)
                && need > n.width_on(&a.layer)
            {
                d.error(
                    at,
                    format!(
                        "{} carries only {} for a {} rise, {} needs {need} (IPC-2221)",
                        n.width_on(&a.layer),
                        a.current_capacity,
                        n.max_temp_rise,
                        i
                    ),
                );
            }
        }
    }

    pub fn via_drill_error(&self, v: &Via) -> Option<String> {
        let e = self.stackup.drills_via(v).err()?;
        Some(format!("{} to {} cannot be drilled: {e}", v.from, v.to))
    }

    fn check_via(&self, i: usize, v: &Via, copper: &[String], d: &mut Diags) {
        let r = &self.rules;
        let at = format!("vias[{i}]");
        if self.vias.iter().filter(|o| o.name == v.name).count() > 1 {
            d.error(&at, format!("via name `{}` is used twice", v.name));
        }
        let (a, b) = match (
            copper.iter().position(|c| *c == v.from),
            copper.iter().position(|c| *c == v.to),
        ) {
            (Some(a), Some(b)) if a < b => (a, b),
            (Some(_), Some(_)) => {
                d.error(&at, "`from` must be above `to` in the stackup");
                return;
            }
            _ => {
                d.error(&at, format!("`from`/`to` must name copper layers: {}", copper.join(", ")));
                return;
            }
        };
        let last = copper.len() - 1;
        let span = format!("{} to {}", v.from, v.to);
        let micro = v.kind == ViaKind::Microvia;
        let depth_drilled = v.drill_kind == DrillKind::ControlledDepth;
        if v.kind != ViaKind::Through && !r.hdi {
            let how = if depth_drilled {
                "drilling to a controlled depth"
            } else {
                "sequential lamination"
            };
            d.error(
                &at,
                format!(
                    "a {} via needs {how}, which fab `{}` does not build: use fab = \"hdi\" or set [rules] hdi = true",
                    v.kind.name(),
                    self.fab
                ),
            );
        }
        match v.drill_kind {
            DrillKind::Laser if !micro => d.error(
                &at,
                "a laser drilled via is a microvia, set type = \"microvia\" or drill_kind = \"mechanical\"",
            ),
            DrillKind::Mechanical | DrillKind::ControlledDepth if micro => {
                d.error(&at, "a microvia is laser drilled, drop `drill_kind` or set it to \"laser\"")
            }
            DrillKind::ControlledDepth if v.kind != ViaKind::Blind => d.error(
                &at,
                format!(
                    "a controlled depth via is drilled from one outer layer into the board, a blind via; {span} is {}",
                    v.kind.name()
                ),
            ),
            _ => {}
        }
        match v.kind {
            ViaKind::Through if a != 0 || b != last => d.error(
                &at,
                format!(
                    "a through via spans {} to {}, not {span}: use type = \"blind\" or \"buried\"",
                    copper[0], copper[last]
                ),
            ),
            ViaKind::Blind if (a == 0) == (b == last) => d.error(
                &at,
                format!("a blind via must touch exactly one outer layer, {span} does not"),
            ),
            ViaKind::Buried if a == 0 || b == last => d.error(
                &at,
                format!("a buried via stays between inner layers, {span} reaches an outer layer"),
            ),
            _ => {}
        }
        if (v.stacked || v.skip) && !micro {
            d.error(&at, "`stacked` and `skip` apply to microvias only");
        }
        let floor = match v.kind {
            _ if depth_drilled => r.min_via_drill.max(r.min_controlled_depth_drill),
            ViaKind::Through => r.min_via_drill,
            ViaKind::Blind | ViaKind::Buried => r.min_via_drill.max(r.min_blind_via_drill),
            ViaKind::Microvia => r.min_microvia_drill,
        };
        if v.drill < floor {
            d.error(&at, format!("drill {} is under the fab minimum {floor}", v.drill));
        }
        let pad_floor = if micro { r.min_microvia_diameter } else { r.min_via_diameter };
        if v.diameter < pad_floor {
            d.error(&at, format!("diameter {} is under the fab minimum {pad_floor}", v.diameter));
        }
        if !micro && v.annular_ring() < r.min_annular_ring {
            d.error(
                &at,
                format!(
                    "annular ring {} is under the fab minimum {}, use a diameter of at least {}",
                    v.annular_ring(),
                    r.min_annular_ring,
                    v.drill + r.min_annular_ring * 2.0
                ),
            );
        }
        if micro {
            self.check_microvia(&at, v, a, b, copper, d);
        }
        if depth_drilled {
            let depth = self.stackup.span_depth(&v.from, &v.to);
            let ratio = depth.to_mm() / v.drill.to_mm().max(1e-9);
            if ratio > r.max_controlled_depth_aspect_ratio + 1e-9 {
                d.error(
                    &at,
                    format!(
                        "controlled depth hole {depth} deep over a {} drill is {ratio:.2}:1, over the fab's {}:1: plating reaches the bottom of a blind hole only this shallow",
                        v.drill, r.max_controlled_depth_aspect_ratio
                    ),
                );
            }
        }
        if let Some(e) = self.via_drill_error(v) {
            d.error(&at, e);
        }
        if let Some(bd) = &v.backdrill {
            self.check_backdrill(&at, v, bd, a, b, copper, d);
        }
        if v.cost <= 0.0 {
            d.error(&at, "`cost` must be positive");
        }
    }

    fn check_microvia(
        &self,
        at: &str,
        v: &Via,
        a: usize,
        b: usize,
        copper: &[String],
        d: &mut Diags,
    ) {
        let r = &self.rules;
        let steps = b - a;
        match (v.stacked, v.skip) {
            (true, true) => d.error(at, "a microvia is `stacked` or `skip`, not both"),
            (false, false) if steps != 1 => d.error(
                at,
                format!(
                    "a microvia spans one dielectric, {} to {} spans {steps}: set `stacked = true` for a stack of microvias or `skip = true` for a skip via over two",
                    v.from, v.to
                ),
            ),
            (false, true) if steps != 2 => {
                d.error(at, format!("a skip microvia spans two dielectrics, not {steps}"))
            }
            _ => {}
        }
        if v.stacked
            && let Some(e) = self.stackup.stack_order_error(&v.from, &v.to, v.drill_kind)
        {
            d.error(at, format!("the stacked microvia cannot be built: {e}"));
        }
        if v.stacked && !r.stacked_microvias {
            d.error(
                at,
                "the fab does not stack microvias: stagger them as separate one-layer microvias, or set [rules] stacked_microvias = true",
            );
        }
        if v.stacked
            && !matches!(
                v.fill,
                Some(ViaFill::Filled | ViaFill::FilledCovered | ViaFill::FilledCapped)
            )
        {
            d.warn(
                at,
                "stacked microvias sit on copper filled microvias, set `fill = \"filled_capped\"`",
            );
        }
        if v.drill > r.max_microvia_drill {
            d.error(
                at,
                format!("a laser drill of {} is over the fab's {}", v.drill, r.max_microvia_drill),
            );
        }
        let depths: Vec<Length> = if v.stacked {
            (a..b).map(|k| self.stackup.depth(&copper[k], &copper[k + 1])).collect()
        } else {
            vec![self.stackup.depth(&v.from, &v.to)]
        };
        for depth in depths {
            let ratio = depth.to_mm() / v.drill.to_mm().max(1e-9);
            if ratio > r.max_microvia_aspect_ratio + 1e-9 {
                d.error(
                    at,
                    format!(
                        "microvia {depth} deep over a {} drill is {ratio:.2}:1, over the fab's {}:1",
                        v.drill, r.max_microvia_aspect_ratio
                    ),
                );
            }
            if depth > Length::mm(0.25) + Length::mm(1e-6) && !v.skip {
                d.warn(at, format!("microvia {depth} deep, IPC-T-50 limits a microvia to 0.25 mm"));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_backdrill(
        &self,
        at: &str,
        v: &Via,
        bd: &Backdrill,
        a: usize,
        b: usize,
        copper: &[String],
        d: &mut Diags,
    ) {
        if matches!(v.kind, ViaKind::Microvia | ViaKind::Buried) {
            d.error(at, format!("a {} via cannot be backdrilled", v.kind.name()));
            return;
        }
        if !self.rules.hdi {
            d.error(
                at,
                format!(
                    "fab `{}` does not backdrill: use fab = \"hdi\" or set [rules] hdi = true",
                    self.fab
                ),
            );
        }
        let last = copper.len() - 1;
        let side = copper.iter().position(|c| *c == bd.from);
        let keep = copper.iter().position(|c| *c == bd.to);
        match (side, keep) {
            (Some(s), Some(k)) => {
                let reaches = (s == 0 && a == 0) || (s == last && b == last);
                if !reaches {
                    d.error(
                        at,
                        format!(
                            "backdrill `from` must be an outer layer the via reaches, not {}",
                            bd.from
                        ),
                    );
                } else if k <= a || k >= b {
                    d.error(
                        at,
                        format!(
                            "backdrill `to` must be a layer strictly inside {} to {}",
                            v.from, v.to
                        ),
                    );
                }
            }
            _ => d.error(
                at,
                format!("backdrill `from`/`to` must name copper layers: {}", copper.join(", ")),
            ),
        }
        if bd.diameter <= v.drill {
            d.error(
                at,
                format!(
                    "backdrill diameter {} must be wider than the drill {}",
                    bd.diameter, v.drill
                ),
            );
        }
        if !bd.max_stub.is_positive() {
            d.error(at, "backdrill `max_stub` must be positive");
        }
    }

    fn check_stackup(&self, d: &mut Diags) {
        let layers = &self.stackup.layers;
        let n_cu = layers.iter().filter(|l| l.kind == LayerKind::Copper).count();
        if n_cu == 0 {
            d.error("stackup", "no copper layers");
            return;
        }
        if n_cu > 2 && n_cu % 2 == 1 {
            d.warn("stackup", format!("{n_cu} copper layers, fabs build even counts"));
        }
        for (i, l) in layers.iter().enumerate() {
            let at = format!("stackup.layers[{i}]");
            if layers.iter().filter(|o| o.name == l.name).count() > 1 {
                d.error(&at, format!("layer name `{}` is used twice", l.name));
            }
            if (l.kind == LayerKind::Copper || l.kind.is_dielectric()) && !l.thickness.is_positive()
            {
                d.error(&at, format!("`{}` needs a positive `thickness`", l.name));
            }
            if l.kind.is_dielectric() && !(1.0..=20.0).contains(&l.er) {
                d.error(
                    &at,
                    format!("`{}` er {} is not a plausible dielectric constant", l.name, l.er),
                );
            }
        }
        let cu: Vec<usize> = layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.kind == LayerKind::Copper)
            .map(|(i, _)| i)
            .collect();
        for w in cu.windows(2) {
            if !layers[w[0] + 1..w[1]].iter().any(|l| l.kind.is_dielectric()) {
                d.error(
                    "stackup",
                    format!(
                        "no dielectric between `{}` and `{}`",
                        layers[w[0]].name, layers[w[1]].name
                    ),
                );
            }
        }
        let (first, last) = (cu[0], *cu.last().unwrap());
        for (i, l) in layers.iter().enumerate() {
            let outer = i < first || i > last;
            if matches!(l.kind, LayerKind::Mask | LayerKind::Silk | LayerKind::Paste) && !outer {
                d.error(
                    format!("stackup.layers[{i}]"),
                    format!("`{}` must sit outside the copper", l.name),
                );
            }
            if l.kind.is_dielectric() && outer {
                d.warn(
                    format!("stackup.layers[{i}]"),
                    format!("`{}` is outside the outer copper", l.name),
                );
            }
        }
        let copper = self.stackup.copper_names();
        let last = copper.len() - 1;
        for (i, step) in self.stackup.lamination.iter().enumerate() {
            let at = format!("stackup.lamination[{i}]");
            let (Some(a), Some(b)) = (
                copper.iter().position(|c| *c == step.from),
                copper.iter().position(|c| *c == step.to),
            ) else {
                d.error(&at, format!("`from`/`to` must name copper layers: {}", copper.join(", ")));
                continue;
            };
            if a >= b {
                d.error(&at, "`from` must be above `to` in the stackup");
            } else if step.drill_kind == DrillKind::ControlledDepth && (a == 0) == (b == last) {
                d.error(
                    &at,
                    format!(
                        "a controlled depth step drills from one outer layer to an inner layer, {} to {} does not",
                        step.from, step.to
                    ),
                );
            }
            if self.stackup.lamination[..i].contains(step) {
                d.warn(&at, "the same drill step is listed twice");
            }
        }
        let t = self.stackup.thickness();
        d.info("stackup", format!("{n_cu} copper layers, {t} finished thickness"));
    }
}

fn domain_voltage(d: &Domain, classes: &[Netclass]) -> Option<Voltage> {
    d.classes
        .iter()
        .filter_map(|c| classes.iter().find(|n| &n.name == c)?.voltage)
        .max_by(|a, b| a.peak().total_cmp(&b.peak()))
}

fn implicit_isolation(
    classes: &[Netclass],
    pollution_degree: u8,
    domains: &mut Vec<Domain>,
    barriers: &mut Vec<Barrier>,
) {
    if classes.iter().all(|c| c.voltage.is_none()) {
        return;
    }
    let explicit = domains.len();
    let claimed = |c: &str| domains[..explicit].iter().any(|d| d.classes.iter().any(|x| x == c));
    let mut ends: Vec<(Option<usize>, &str, Voltage, f64)> = Vec::new();
    for (i, d) in domains.iter().enumerate() {
        if let Some(v) = domain_voltage(d, classes) {
            let clear = d
                .classes
                .iter()
                .filter_map(|c| classes.iter().find(|n| &n.name == c))
                .map(|n| n.clearance.to_mm())
                .fold(0.0, f64::max);
            ends.push((Some(i), d.name.as_str(), v, clear));
        }
    }
    let free: Vec<&Netclass> = classes.iter().filter(|c| !claimed(&c.name)).collect();
    for c in &free {
        ends.push((None, c.name.as_str(), c.voltage.unwrap_or(Voltage::ZERO), c.clearance.to_mm()));
    }
    let mut plan: Vec<(usize, usize, crate::insulation::Spacing)> = Vec::new();
    for a in 0..ends.len() {
        for b in a + 1..ends.len() {
            let (ea, eb) = (&ends[a], &ends[b]);
            if let (Some(x), Some(y)) = (ea.0, eb.0)
                && barriers.iter().any(|z| z.between == [x, y] || z.between == [y, x])
            {
                continue;
            }
            let s = crate::insulation::spacing(ea.2, eb.2, pollution_degree);
            let own = ea.3.max(eb.3);
            let needed = s.grade == crate::insulation::Grade::Reinforced
                || s.clearance > own + 1e-9
                || s.creepage > own + 1e-9 && s.working.hazardous();
            if needed {
                plan.push((a, b, s));
            }
        }
    }
    let mut index: Vec<Option<usize>> = ends.iter().map(|e| e.0).collect();
    let names: Vec<String> = ends.iter().map(|e| e.1.to_string()).collect();
    let voltages: Vec<String> = ends
        .iter()
        .map(|e| match e.0 {
            None if classes.iter().any(|c| c.name == e.1 && c.voltage.is_none()) => {
                "no voltage, taken as 0V".to_string()
            }
            _ => e.2.to_string(),
        })
        .collect();
    for (a, b, s) in plan {
        for k in [a, b] {
            if index[k].is_none() {
                let taken = domains.iter().any(|d| d.name == names[k]);
                domains.push(Domain {
                    name: if taken { format!("class {}", names[k]) } else { names[k].clone() },
                    description: format!("netclass {} at {}", names[k], voltages[k]),
                    classes: vec![names[k].clone()],
                    nets: Vec::new(),
                    implicit: true,
                });
                index[k] = Some(domains.len() - 1);
            }
        }
        let grade = match s.grade {
            crate::insulation::Grade::Reinforced => "reinforced",
            crate::insulation::Grade::Functional => "functional",
        };
        barriers.push(Barrier {
            between: [index[a].unwrap(), index[b].unwrap()],
            description: format!(
                "{grade} insulation for {} between {} ({}) and {} ({})",
                s.working, names[a], voltages[a], names[b], voltages[b]
            ),
            clearance: Some(Length::mm(s.clearance)),
            creepage: Some(Length::mm(s.creepage)),
            pollution_degree,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(src: &str) -> (Board, Diags) {
        let f: BoardFile = toml::from_str(src).unwrap();
        let mut d = Diags::new("test");
        let b = f.resolve(&mut d);
        b.check(&mut d);
        (b, d)
    }

    #[test]
    fn jlc_4l_preset_names_and_geometry() {
        let (b, d) =
            board("name = \"x\"\nfab = \"jlcpcb\"\n[stackup]\npreset = \"jlcpcb-4l-1.6mm-7628\"\n");
        assert_eq!(b.stackup.copper_names(), ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]);
        assert!(matches!(b.stackup.geometry("F.Cu"), Some(TraceGeometry::Microstrip { .. })));
        assert!(matches!(b.stackup.geometry("In1.Cu"), Some(TraceGeometry::Stripline { .. })));
        assert!(!d.has_errors(), "{:?}", d.list);
        let t = b.stackup.thickness().to_mm();
        assert!((1.55..1.65).contains(&t), "{t}");
    }

    #[test]
    fn every_stackup_preset_resolves_clean() {
        for p in crate::stackups::stackup_presets() {
            let (b, d) = board(&format!("name = \"x\"\n[stackup]\npreset = \"{}\"\n", p.name));
            assert!(!d.has_errors(), "{}: {:?}", p.name, d.list);
            assert_eq!(b.stackup.copper_names().len(), p.copper_layers, "{}", p.name);
        }
    }

    #[test]
    fn netclass_checks_catch_undersized_power() {
        let (_, d) = board(
            r#"
name = "x"
[stackup]
preset = "jlcpcb-2l-1.6mm"
[[netclasses]]
name = "Default"
track_width = "0.2mm"
[[netclasses]]
name = "Power"
track_width = "0.2mm"
current = "3A"
"#,
        );
        assert!(d.list.iter().any(|x| x.message.contains("IPC-2221")), "{:?}", d.list);
    }

    #[test]
    fn impedance_width_is_solved_when_omitted() {
        let (b, d) = board(
            r#"
name = "x"
[stackup]
preset = "jlcpcb-4l-1.6mm-7628"
[[netclasses]]
name = "RF"
impedance = "50ohm"
layers = ["F.Cu"]
"#,
        );
        let w = b.netclasses[0].track_width.to_mm();
        assert!((0.3..0.42).contains(&w), "{w}");
        assert!(!d.has_errors(), "{:?}", d.list);
    }

    #[test]
    fn jlcpcb_rules_follow_layer_count_and_copper_weight() {
        let at = |layers, outer_oz, finish: &str| {
            fab_rules_for("jlcpcb", &FabSetup { layers, outer_oz, finish: finish.into() }).unwrap()
        };
        assert_eq!(at(2, 1.0, "HASL").min_track_width, Length::mm(0.10));
        assert_eq!(at(4, 1.0, "HASL").min_track_width, Length::mm(0.09));
        assert_eq!(at(2, 2.0, "HASL").min_clearance, Length::mm(0.16));
        assert_eq!(at(6, 2.0, "HASL").min_clearance, Length::mm(0.15));
        assert_eq!(at(2, 3.5, "HASL").min_track_width, Length::mm(0.25));
        assert_eq!(at(1, 1.0, "HASL").min_via_drill, Length::mm(0.3));
        assert_eq!(at(4, 1.0, "HASL").min_via_drill, Length::mm(0.15));
        assert_eq!(at(2, 1.0, "HASL").min_pth_annular_ring, Length::mm(0.18));
        assert_eq!(at(4, 1.0, "HASL").min_pth_annular_ring, Length::mm(0.15));
        assert_eq!(at(4, 2.0, "HASL").min_pth_annular_ring, Length::mm(0.254));
        assert_eq!(at(2, 1.0, "HASL").min_plated_slot_width, Length::mm(0.5));
        assert_eq!(at(4, 1.0, "HASL").min_plated_slot_width, Length::mm(0.35));
        assert_eq!(at(4, 1.0, "ENIG").min_bga_pad, Length::mm(0.2));
        assert_eq!(at(4, 1.0, "HASL").min_bga_pad, Length::mm(0.25));

        let (b, _) =
            board("name = \"x\"\nfab = \"jlcpcb\"\n[stackup]\npreset = \"jlcpcb-4l-1.6mm-7628\"\n");
        assert_eq!(b.rules.min_track_width, Length::mm(0.09));
        let (b, _) = board(
            "name = \"x\"\nfab = \"jlcpcb\"\n[stackup]\npreset = \"jlcpcb-4l-1.6mm-7628\"\n[rules]\nmin_track_width = \"0.2mm\"\nmax_aspect_ratio = 8\n",
        );
        assert_eq!(b.rules.min_track_width, Length::mm(0.2));
        assert_eq!(b.rules.max_aspect_ratio, 8.0);
    }

    #[test]
    fn drc_table_names_known_rules() {
        let (b, d) = board(
            r#"
name = "x"
[stackup]
preset = "jlcpcb-2l-1.6mm"
[[netclasses]]
name = "Default"
track_width = "0.2mm"
[drc]
disable = ["no-such-rule"]
severity = { "via-in-pad" = "error" }
"#,
        );
        assert_eq!(b.drc.severity["via-in-pad"], crate::diag::Severity::Error);
        assert!(d.list.iter().any(|x| x.message.contains("no DRC rule `no-such-rule`")));
        assert!(!d.list.iter().any(|x| x.message.contains("`via-in-pad`")), "{:?}", d.list);
        assert!(
            toml::from_str::<BoardFile>(
                "name = \"x\"\n[stackup]\npreset = \"a\"\n[drc]\nseverity = { a = \"loud\" }\n"
            )
            .is_err()
        );
    }

    fn hdi(vias: &str) -> (Board, Diags) {
        board(&format!(
            "name = \"x\"\nfab = \"hdi\"\n[stackup]\npreset = \"hdi-6l-1n1\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.1mm\"\n{vias}"
        ))
    }

    fn errors(d: &Diags) -> Vec<String> {
        d.list
            .iter()
            .filter(|x| x.severity == crate::diag::Severity::Error)
            .map(|x| x.message.clone())
            .collect()
    }

    #[test]
    fn via_type_is_inferred_from_the_span() {
        let (b, d) = hdi(
            "[[vias]]\nname = \"std\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\n[[vias]]\nname = \"bl\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\nfrom = \"F.Cu\"\nto = \"In1.Cu\"\ndrill_kind = \"controlled_depth\"\n[[vias]]\nname = \"bu\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\nfrom = \"In1.Cu\"\nto = \"In4.Cu\"\n",
        );
        assert!(errors(&d).is_empty(), "{:?}", d.list);
        let kinds: Vec<ViaKind> = b.vias.iter().map(|v| v.kind).collect();
        assert_eq!(kinds, [ViaKind::Through, ViaKind::Blind, ViaKind::Buried]);
        let cu = b.stackup.copper_names();
        assert_eq!(b.vias[2].copper_layers(&cu), ["In1.Cu", "In2.Cu", "In3.Cu", "In4.Cu"]);
    }

    #[test]
    fn jlcpcb_rejects_blind_vias_unless_hdi_is_set() {
        let src = "name = \"x\"\nfab = \"jlcpcb\"\n[stackup]\npreset = \"hdi-6l-1n1\"\n[[vias]]\nname = \"bl\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\ntype = \"blind\"\nfrom = \"F.Cu\"\nto = \"In1.Cu\"\ndrill_kind = \"controlled_depth\"\n";
        let (_, d) = board(src);
        assert!(
            errors(&d)
                .iter()
                .any(|m| m.contains("controlled depth, which fab `jlcpcb` does not build")),
            "{:?}",
            d.list
        );
        let (_, d) = board(&format!("{src}[rules]\nhdi = true\n"));
        assert!(errors(&d).is_empty(), "{:?}", d.list);
        let (_, d) = board(&src.replace("jlcpcb", "generic"));
        assert!(
            errors(&d)
                .iter()
                .any(|m| m.contains("controlled depth, which fab `generic` does not build")),
            "{:?}",
            d.list
        );
        for fab in ["generic", "jlcpcb"] {
            let r = fab_rules(fab).unwrap();
            assert!(!r.hdi && r.min_controlled_depth_drill == r.min_blind_via_drill);
        }
    }

    #[test]
    fn via_type_must_match_its_span() {
        let (_, d) = hdi(
            "[[vias]]\nname = \"a\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\ntype = \"blind\"\nfrom = \"In1.Cu\"\nto = \"In2.Cu\"\n[[vias]]\nname = \"b\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\ntype = \"through\"\nto = \"In4.Cu\"\n[[vias]]\nname = \"c\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\ntype = \"buried\"\nfrom = \"F.Cu\"\nto = \"In1.Cu\"\n",
        );
        let e = errors(&d);
        assert!(e.iter().any(|m| m.contains("exactly one outer layer")), "{e:?}");
        assert!(e.iter().any(|m| m.contains("a through via spans")), "{e:?}");
        assert!(e.iter().any(|m| m.contains("reaches an outer layer")), "{e:?}");
    }

    #[test]
    fn buried_span_must_match_the_lamination() {
        let (b, d) = hdi(
            "[[vias]]\nname = \"core\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\nfrom = \"In1.Cu\"\nto = \"In2.Cu\"\n[[vias]]\nname = \"split\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\nfrom = \"In2.Cu\"\nto = \"In3.Cu\"\n",
        );
        let e = errors(&d);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with("In2.Cu to In3.Cu cannot be drilled"), "{e:?}");
        assert!(e[0].contains("mechanical In1.Cu-In2.Cu, In3.Cu-In4.Cu, In1.Cu-In4.Cu"), "{e:?}");
        let st = &b.stackup;
        let mech = |a: &str, z: &str| st.drillable(a, z, DrillKind::Mechanical, false).is_ok();
        assert!(mech("In1.Cu", "In4.Cu") && mech("In3.Cu", "In4.Cu") && mech("F.Cu", "B.Cu"));
        assert!(!mech("In2.Cu", "In3.Cu") && !mech("F.Cu", "In2.Cu") && !mech("F.Cu", "In4.Cu"));
    }

    fn steps(st: &Stackup) -> Vec<String> {
        st.drill_steps()
            .iter()
            .map(|s| format!("{} {}-{}", s.drill_kind.name(), s.from, s.to))
            .filter(|s| !s.starts_with("controlled"))
            .collect()
    }

    #[test]
    fn lamination_follows_the_one_n_one_build() {
        let (b, _) = hdi("");
        assert_eq!(b.stackup.build_name(), "1+4+1");
        assert_eq!(
            steps(&b.stackup),
            [
                "mechanical In1.Cu-In2.Cu",
                "mechanical In3.Cu-In4.Cu",
                "mechanical In1.Cu-In4.Cu",
                "laser F.Cu-In1.Cu",
                "laser In4.Cu-B.Cu",
                "mechanical F.Cu-B.Cu",
            ]
        );
        let depth: Vec<DrillStep> = b
            .stackup
            .drill_steps()
            .into_iter()
            .filter(|s| s.drill_kind == DrillKind::ControlledDepth)
            .collect();
        assert_eq!(depth.len(), 8);
        assert!(depth.iter().all(|s| s.from == "F.Cu" || s.to == "B.Cu"));
    }

    #[test]
    fn lamination_of_two_n_two_adds_the_inner_build_up() {
        let (b, _) = board("name = \"x\"\nfab = \"hdi\"\n[stackup]\npreset = \"hdi-8l-2n2\"\n");
        assert_eq!(b.stackup.build_name(), "2+4+2");
        assert_eq!(
            steps(&b.stackup),
            [
                "mechanical In2.Cu-In3.Cu",
                "mechanical In4.Cu-In5.Cu",
                "mechanical In2.Cu-In5.Cu",
                "laser In1.Cu-In2.Cu",
                "laser In5.Cu-In6.Cu",
                "mechanical In1.Cu-In6.Cu",
                "laser F.Cu-In1.Cu",
                "laser In6.Cu-B.Cu",
                "laser F.Cu-In2.Cu",
                "laser In5.Cu-B.Cu",
                "mechanical F.Cu-B.Cu",
            ]
        );
        let st = &b.stackup;
        assert!(st.drillable("F.Cu", "In2.Cu", DrillKind::Laser, true).is_ok());
        assert!(st.drillable("F.Cu", "In3.Cu", DrillKind::Laser, true).is_err());
        let (b, _) = board("name = \"x\"\n[stackup]\npreset = \"jlcpcb-4l-1.6mm-7628\"\n");
        assert_eq!(b.stackup.build_name(), "1+2+1");
        let (b, _) = board("name = \"x\"\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n");
        assert_eq!(steps(&b.stackup), ["mechanical F.Cu-B.Cu"]);
    }

    fn layered(kinds: &[&str]) -> String {
        let mut s = "name = \"x\"\nfab = \"hdi\"\n[stackup]\n".to_string();
        for k in kinds {
            s += &format!("[[stackup.layers]]\nkind = \"{k}\"\nthickness = \"0.1mm\"\n");
        }
        s
    }

    #[test]
    fn an_asymmetric_build_counts_the_build_up_of_each_side() {
        let (b, d) = board(&layered(&[
            "copper", "prepreg", "copper", "core", "copper", "prepreg", "copper", "core", "copper",
            "prepreg", "copper", "prepreg", "copper",
        ]));
        assert!(!d.has_errors(), "{:?}", d.list);
        assert_eq!(b.stackup.build_up_layers(), (1, 2));
        assert_eq!(b.stackup.build_name(), "1+4+2");
        assert_eq!(
            steps(&b.stackup),
            [
                "mechanical In1.Cu-In2.Cu",
                "mechanical In3.Cu-In4.Cu",
                "mechanical In1.Cu-In4.Cu",
                "laser In4.Cu-In5.Cu",
                "mechanical In1.Cu-In5.Cu",
                "laser F.Cu-In1.Cu",
                "laser In5.Cu-B.Cu",
                "laser In4.Cu-B.Cu",
                "mechanical F.Cu-B.Cu",
            ]
        );
        let st = &b.stackup;
        assert!(st.drillable("In4.Cu", "B.Cu", DrillKind::Laser, true).is_ok());
        assert!(st.drillable("F.Cu", "In2.Cu", DrillKind::Laser, true).is_err());
    }

    #[test]
    fn a_stacked_microvia_is_built_from_the_inside_out() {
        let stack = "[[vias]]\nname = \"st\"\ntype = \"microvia\"\nstacked = true\nfill = \"filled_capped\"\ndrill = \"0.1mm\"\ndiameter = \"0.25mm\"\nfrom = \"F.Cu\"\nto = \"In2.Cu\"\n";
        let src = |first: &str, second: &str| {
            format!(
                "name = \"x\"\nfab = \"hdi\"\n[stackup]\npreset = \"hdi-8l-2n2\"\nlamination = [\n  {{ from = \"{first}\", to = \"{second}\", drill_kind = \"laser\" }},\n  {{ from = \"{}\", to = \"{}\", drill_kind = \"laser\" }},\n  {{ from = \"F.Cu\", to = \"B.Cu\", drill_kind = \"mechanical\" }},\n]\n{stack}",
                if first == "F.Cu" { "In1.Cu" } else { "F.Cu" },
                if first == "F.Cu" { "In2.Cu" } else { "In1.Cu" },
            )
        };
        let (_, d) = board(&src("In1.Cu", "In2.Cu"));
        assert!(errors(&d).is_empty(), "{:?}", d.list);
        let (_, d) = board(&src("F.Cu", "In1.Cu"));
        let e = errors(&d);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(
            e[0].contains(
                "the hop F.Cu-In1.Cu is drilled at lamination step 1, before the hop In1.Cu-In2.Cu under it at step 2"
            ),
            "{e:?}"
        );
    }

    #[test]
    fn explicit_lamination_overrides_the_derived_one() {
        let lam = "lamination = [\n  { from = \"In1.Cu\", to = \"In4.Cu\", drill_kind = \"mechanical\" },\n  { from = \"F.Cu\", to = \"In4.Cu\", drill_kind = \"mechanical\" },\n  { from = \"F.Cu\", to = \"B.Cu\", drill_kind = \"mechanical\" },\n]\n";
        let src = format!(
            "name = \"x\"\nfab = \"hdi\"\n[stackup]\npreset = \"hdi-6l-1n1\"\n{lam}[[vias]]\nname = \"bl\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\nfrom = \"F.Cu\"\nto = \"In4.Cu\"\n[[vias]]\nname = \"u\"\ndrill = \"0.1mm\"\ndiameter = \"0.25mm\"\ntype = \"microvia\"\nfrom = \"F.Cu\"\nto = \"In1.Cu\"\n"
        );
        let (b, d) = board(&src);
        let e = errors(&d);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with("F.Cu to In1.Cu cannot be drilled: no laser drill step"), "{e:?}");
        assert!(
            e[0].contains(
                "[stackup] lamination drills mechanical In1.Cu-In4.Cu, F.Cu-In4.Cu, F.Cu-B.Cu"
            ),
            "{e:?}"
        );
        assert_eq!(b.stackup.drill_steps().len(), 3);
        let (_, d) = board(&src.replace(
            "to = \"In4.Cu\", drill_kind = \"mechanical\" },\n  { from = \"F.Cu\"",
            "to = \"In4.Cu\", drill_kind = \"mechanical\" },\n  { from = \"In9.Cu\"",
        ));
        assert!(errors(&d).iter().any(|m| m.contains("must name copper layers")), "{:?}", d.list);
    }

    #[test]
    fn controlled_depth_vias_are_blind_shallow_and_drilled_from_one_side() {
        let via = |from: &str, to: &str, drill: &str| {
            hdi(&format!(
                "[[vias]]\nname = \"cd\"\ndrill = \"{drill}\"\ndiameter = \"0.45mm\"\nfrom = \"{from}\"\nto = \"{to}\"\ndrill_kind = \"controlled_depth\"\n"
            ))
        };
        let (b, d) = via("In4.Cu", "B.Cu", "0.15mm");
        assert!(errors(&d).is_empty(), "{:?}", d.list);
        assert_eq!(b.vias[0].kind, ViaKind::Blind);
        assert_eq!(b.vias[0].drill_kind, DrillKind::ControlledDepth);
        let (_, d) = via("F.Cu", "In2.Cu", "0.2mm");
        let e = errors(&d);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains(":1, over the fab's 1:1"), "{e:?}");
        let (_, d) = via("F.Cu", "In1.Cu", "0.12mm");
        let e = errors(&d);
        assert!(e.iter().any(|m| m.contains("under the fab minimum 0.15")), "{e:?}");
        let (_, d) = via("In1.Cu", "In4.Cu", "0.3mm");
        assert!(
            errors(&d)
                .iter()
                .any(|m| m.contains("a controlled depth via is drilled from one outer layer")),
            "{:?}",
            d.list
        );
        let (_, d) = hdi(
            "[[vias]]\nname = \"u\"\ndrill = \"0.1mm\"\ndiameter = \"0.25mm\"\ntype = \"microvia\"\nfrom = \"F.Cu\"\nto = \"In1.Cu\"\ndrill_kind = \"controlled_depth\"\n",
        );
        assert!(
            errors(&d).iter().any(|m| m.contains("a microvia is laser drilled")),
            "{:?}",
            d.list
        );
    }

    #[test]
    fn microvias_span_one_dielectric_within_the_aspect_ratio() {
        let (_, d) = hdi(
            "[[vias]]\nname = \"u\"\ndrill = \"0.1mm\"\ndiameter = \"0.25mm\"\ntype = \"microvia\"\nfrom = \"F.Cu\"\nto = \"In1.Cu\"\nfill = \"filled_capped\"\n",
        );
        assert!(errors(&d).is_empty(), "{:?}", d.list);
        let (_, d) = hdi(
            "[[vias]]\nname = \"u\"\ndrill = \"0.1mm\"\ndiameter = \"0.25mm\"\ntype = \"microvia\"\nfrom = \"F.Cu\"\nto = \"In2.Cu\"\n",
        );
        let e = errors(&d);
        assert!(e.iter().any(|m| m.contains("spans one dielectric")), "{e:?}");
        let (_, d) = hdi(
            "[[vias]]\nname = \"u\"\ndrill = \"0.08mm\"\ndiameter = \"0.2mm\"\ntype = \"microvia\"\nfrom = \"F.Cu\"\nto = \"In1.Cu\"\n",
        );
        let e = errors(&d);
        assert!(e.iter().any(|m| m.contains("under the fab minimum 0.1")), "{e:?}");
        assert!(e.iter().any(|m| m.contains(":1, over the fab's 0.8:1")), "{e:?}");
        let (_, d) = board(
            "name = \"x\"\nfab = \"hdi\"\n[stackup]\npreset = \"hdi-8l-2n2\"\n[[vias]]\nname = \"s\"\ndrill = \"0.1mm\"\ndiameter = \"0.25mm\"\ntype = \"microvia\"\nstacked = true\nfill = \"filled_capped\"\nfrom = \"F.Cu\"\nto = \"In2.Cu\"\n[rules]\nstacked_microvias = false\n",
        );
        let e = errors(&d);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains("does not stack microvias"));
    }

    #[test]
    fn backdrill_leaves_the_copper_above_its_stop_layer() {
        let (b, d) = hdi(
            "[[vias]]\nname = \"bd\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\nbackdrill = { from = \"B.Cu\", to = \"In2.Cu\", max_stub = \"0.2mm\" }\n",
        );
        assert!(errors(&d).is_empty(), "{:?}", d.list);
        let cu = b.stackup.copper_names();
        assert_eq!(b.vias[0].copper_layers(&cu), ["F.Cu", "In1.Cu", "In2.Cu"]);
        assert_eq!(b.vias[0].hole_layers(&cu).len(), 6);
        let bd = b.vias[0].backdrill.as_ref().unwrap();
        assert_eq!(bd.diameter, Length::mm(0.4));
        let (_, d) = hdi(
            "[[vias]]\nname = \"bd\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\nbackdrill = { from = \"In1.Cu\", to = \"In2.Cu\" }\n",
        );
        assert!(errors(&d).iter().any(|m| m.contains("outer layer the via reaches")));
    }

    #[test]
    fn class_vias_are_a_list_and_via_for_picks_one_that_reaches() {
        let (b, d) = hdi(
            "[[vias]]\nname = \"std\"\ndrill = \"0.2mm\"\ndiameter = \"0.45mm\"\n[[vias]]\nname = \"ub\"\ndrill = \"0.1mm\"\ndiameter = \"0.25mm\"\ntype = \"microvia\"\nfrom = \"In4.Cu\"\nto = \"B.Cu\"\n[[netclasses]]\nname = \"Fast\"\ntrack_width = \"0.1mm\"\nvia = [\"ub\", \"std\"]\n",
        );
        assert!(errors(&d).is_empty(), "{:?}", d.list);
        let fast = b.netclasses.iter().find(|c| c.name == "Fast");
        assert_eq!(fast.unwrap().via, ["ub", "std"]);
        assert_eq!(b.via_for(None, fast, &["B.Cu"]).unwrap().name, "ub");
        assert_eq!(b.via_for(None, fast, &["F.Cu"]).unwrap().name, "std");
        assert_eq!(b.via_for(Some("std"), fast, &[]).unwrap().name, "std");
        let both = ["std".to_string(), "ub".to_string()];
        assert_eq!(b.via_among(&both, None, &["B.Cu", "In4.Cu"]).unwrap().name, "std");
        assert_eq!(b.via_among(&both[1..], None, &["F.Cu"]).unwrap().name, "ub");
        let ub_first = ["ub".to_string(), "std".to_string()];
        assert_eq!(b.via_among(&ub_first, None, &["F.Cu"]).unwrap().name, "std");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(
            toml::from_str::<BoardFile>("name = \"x\"\n[stackup]\npreset = \"a\"\ntypo = 1\n")
                .is_err()
        );
    }
}
