use crate::calc::{self, Line, TraceGeometry};
use crate::diag::Diags;
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LayerFile>,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViaFile {
    pub name: String,
    pub drill: Length,
    pub diameter: Length,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
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
    pub track_width: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<String>,
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
        }

        #[derive(Clone, Debug, PartialEq, Serialize)]
        pub struct Rules {
            $(pub $field: Length,)*
        }

        impl Rules {
            pub fn overlay(&mut self, f: &RulesFile) {
                $(if let Some(v) = f.$field { self.$field = v; })*
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
    min_annular_ring: "copper left around a plated hole",
    min_hole_to_hole: "drill edge to drill edge",
    min_copper_to_edge: "copper to board outline",
    min_silk_width: "silkscreen line width",
    min_silk_text_height: "silkscreen text height",
}

pub const FAB_PRESETS: &[&str] = &["generic", "jlcpcb"];

pub fn fab_rules(name: &str) -> Option<Rules> {
    let mm = Length::mm;
    match name {
        "generic" => Some(Rules {
            min_track_width: mm(0.15),
            min_clearance: mm(0.15),
            min_drill: mm(0.3),
            min_via_drill: mm(0.3),
            min_via_diameter: mm(0.6),
            min_annular_ring: mm(0.15),
            min_hole_to_hole: mm(0.5),
            min_copper_to_edge: mm(0.5),
            min_silk_width: mm(0.12),
            min_silk_text_height: mm(1.0),
        }),
        "jlcpcb" => Some(Rules {
            min_track_width: mm(0.127),
            min_clearance: mm(0.127),
            min_drill: mm(0.3),
            min_via_drill: mm(0.3),
            min_via_diameter: mm(0.5),
            min_annular_ring: mm(0.1),
            min_hole_to_hole: mm(0.5),
            min_copper_to_edge: mm(0.3),
            min_silk_width: mm(0.15),
            min_silk_text_height: mm(1.0),
        }),
        _ => None,
    }
}

pub const STACKUP_PRESETS: &[(&str, &str)] = &[
    ("jlcpcb-2l-1.6mm", "2 layer FR4, 1.6 mm, 1 oz outer"),
    ("jlcpcb-4l-1.6mm-7628", "JLC04161H-7628: 4 layer, 7628 prepreg, 1 oz outer, 0.5 oz inner"),
    ("jlcpcb-4l-1.6mm-3313", "JLC04161H-3313: 4 layer, 3313 prepreg, 1 oz outer, 0.5 oz inner"),
];

pub fn stackup_preset(name: &str) -> Option<Vec<LayerFile>> {
    let l = |kind, t: f64, material: Option<&str>, er: Option<f64>, tan: Option<f64>| LayerFile {
        kind,
        name: None,
        thickness: Some(Length::mm(t)),
        material: material.map(str::to_string),
        er,
        loss_tangent: tan,
    };
    use LayerKind::*;
    let silk = || LayerFile {
        kind: Silk,
        name: None,
        thickness: None,
        material: None,
        er: None,
        loss_tangent: None,
    };
    let paste = || LayerFile { kind: Paste, ..silk() };
    let mask = || l(Mask, 0.0152, Some("LPI"), Some(3.8), None);
    let core = |t, er| l(Core, t, Some("FR4"), Some(er), Some(0.02));
    let pp = |t, er, m| l(Prepreg, t, Some(m), Some(er), Some(0.02));
    let cu = |t| l(Copper, t, None, None, None);
    let body = match name {
        "jlcpcb-2l-1.6mm" => vec![cu(0.035), core(1.51, 4.5), cu(0.035)],
        "jlcpcb-4l-1.6mm-7628" => vec![
            cu(0.035),
            pp(0.2104, 4.4, "7628"),
            cu(0.0152),
            core(1.065, 4.6),
            cu(0.0152),
            pp(0.2104, 4.4, "7628"),
            cu(0.035),
        ],
        "jlcpcb-4l-1.6mm-3313" => vec![
            cu(0.035),
            pp(0.0994, 4.1, "3313"),
            cu(0.0152),
            core(1.265, 4.6),
            cu(0.0152),
            pp(0.0994, 4.1, "3313"),
            cu(0.035),
        ],
        _ => return None,
    };
    let mut v = vec![silk(), paste(), mask()];
    v.extend(body);
    v.extend([mask(), paste(), silk()]);
    Some(v)
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
    pub layers: Vec<Layer>,
}

impl Stackup {
    pub fn copper(&self) -> impl Iterator<Item = (usize, &Layer)> {
        self.layers.iter().enumerate().filter(|(_, l)| l.kind == LayerKind::Copper)
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

#[derive(Clone, Debug, Serialize)]
pub struct Via {
    pub name: String,
    pub drill: Length,
    pub diameter: Length,
    pub from: String,
    pub to: String,
}

impl Via {
    pub fn annular_ring(&self) -> Length {
        (self.diameter - self.drill) / 2.0
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Netclass {
    pub name: String,
    pub description: String,
    pub track_width: Length,
    pub clearance: Length,
    pub via: Option<String>,
    pub current: Option<Amps>,
    pub max_temp_rise: Kelvin,
    pub impedance: Option<Ohms>,
    pub impedance_tolerance: Percent,
    pub solver: Solver,
    pub diff_gap: Option<Length>,
    pub coplanar_gap: Option<Length>,
    pub layers: Vec<String>,
}

impl Netclass {
    pub fn line(&self) -> Line {
        Line {
            diff_gap_mm: self.diff_gap.map(Length::to_mm),
            coplanar_gap_mm: self.coplanar_gap.map(Length::to_mm),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Board {
    pub name: String,
    pub description: String,
    pub fab: String,
    pub outline: Option<Outline>,
    pub stackup: Stackup,
    pub rules: Rules,
    pub vias: Vec<Via>,
    pub netclasses: Vec<Netclass>,
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
        let mut rules = fab_rules(&fab).unwrap_or_else(|| {
            d.error("fab", format!("unknown fab `{fab}`, use one of {}", FAB_PRESETS.join(", ")));
            fab_rules("generic").unwrap()
        });
        rules.overlay(&self.rules);

        let stackup = self.resolve_stackup(d);
        let copper = stackup.copper_names();
        let first = copper.first().cloned().unwrap_or_default();
        let last = copper.last().cloned().unwrap_or_default();

        let vias = self
            .vias
            .iter()
            .map(|v| Via {
                name: v.name.clone(),
                drill: v.drill,
                diameter: v.diameter,
                from: v.from.clone().unwrap_or_else(|| first.clone()),
                to: v.to.clone().unwrap_or_else(|| last.clone()),
            })
            .collect();

        let netclasses = self
            .netclasses
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let at = format!("netclasses[{i}]");
                if n.track_width.is_none() && n.impedance.is_none() {
                    d.error(&at, format!("netclass `{}` needs `track_width`", n.name));
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
                Netclass {
                    name: n.name.clone(),
                    description: n.description.clone(),
                    track_width,
                    clearance: n.clearance.unwrap_or(rules.min_clearance),
                    via: n.via.clone(),
                    current: n.current,
                    max_temp_rise: n.max_temp_rise.unwrap_or(Kelvin(10.0)),
                    impedance: n.impedance,
                    impedance_tolerance: n.impedance_tolerance.unwrap_or(Percent(10.0)),
                    solver: n.solver.unwrap_or_default(),
                    diff_gap: n.diff_gap,
                    coplanar_gap: n.coplanar_gap,
                    layers,
                }
            })
            .collect();

        let outline = self.outline.as_ref().and_then(|o| match (&o.points, o.size) {
            (Some(p), None) => Some(Outline::Polygon { points: p.clone() }),
            (None, Some(size)) => Some(Outline::Rect {
                origin: o.origin.unwrap_or(Point::ZERO),
                size,
                corner_radius: o.corner_radius.unwrap_or(Length::ZERO),
            }),
            _ => {
                d.error("outline", "give either `size` (a rectangle) or `points` (a polygon)");
                None
            }
        });

        Board {
            name: self.name.clone(),
            description: self.description.clone(),
            fab,
            outline,
            stackup,
            rules,
            vias,
            netclasses,
        }
    }

    fn resolve_stackup(&self, d: &mut Diags) -> Stackup {
        let s = &self.stackup;
        let files = match (&s.preset, s.layers.is_empty()) {
            (Some(p), true) => stackup_preset(p).unwrap_or_else(|| {
                let names: Vec<_> = STACKUP_PRESETS.iter().map(|p| p.0).collect();
                d.error(
                    "stackup.preset",
                    format!("unknown preset `{p}`, use one of {}", names.join(", ")),
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
        let layers = files
            .iter()
            .map(|f| {
                let side = if seen_cu == 0 { "F" } else { "B" };
                let name = f.name.clone().unwrap_or_else(|| match f.kind {
                    LayerKind::Copper => copper_name(seen_cu, n_cu),
                    LayerKind::Silk => format!("{side}.SilkS"),
                    LayerKind::Paste => format!("{side}.Paste"),
                    LayerKind::Mask => format!("{side}.Mask"),
                    LayerKind::Core => format!("core{}", seen_cu),
                    LayerKind::Prepreg => format!("prepreg{}", seen_cu),
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
            layers,
        }
    }
}

fn n_of<'a>(v: &'a [Netclass], name: &str) -> &'a Netclass {
    v.iter().find(|n| n.name == name).unwrap()
}

impl Board {
    pub fn analyze(&self) -> Vec<LayerAnalysis> {
        let mut out = Vec::new();
        for n in &self.netclasses {
            for layer in &n.layers {
                let Some(g) = self.stackup.geometry(layer) else { continue };
                let gap = n.line();
                let z = g.impedance(n.track_width.to_mm(), gap);
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
                    current_capacity: Amps(calc::ipc2221_current(
                        n.track_width.to_mm(),
                        rise,
                        cu,
                        ext,
                    )),
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
        if let Some(Outline::Polygon { points }) = &self.outline
            && points.len() < 3
        {
            d.error("outline.points", "a polygon outline needs at least three points");
        }
        if let Some(Outline::Rect { size, corner_radius, .. }) = &self.outline {
            if !size.0.is_positive() || !size.1.is_positive() {
                d.error("outline.size", "board size must be positive");
            }
            if *corner_radius * 2.0 > size.0.min(size.1) {
                d.error("outline.corner_radius", "corner radius is more than half the board");
            }
        }

        let copper = self.stackup.copper_names();
        for (i, v) in self.vias.iter().enumerate() {
            let at = format!("vias[{i}]");
            if self.vias.iter().filter(|o| o.name == v.name).count() > 1 {
                d.error(&at, format!("via name `{}` is used twice", v.name));
            }
            if v.drill < r.min_via_drill {
                d.error(
                    &at,
                    format!("drill {} is under the fab minimum {}", v.drill, r.min_via_drill),
                );
            }
            if v.diameter < r.min_via_diameter {
                d.error(
                    &at,
                    format!(
                        "diameter {} is under the fab minimum {}",
                        v.diameter, r.min_via_diameter
                    ),
                );
            }
            if v.annular_ring() < r.min_annular_ring {
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
            match (copper.iter().position(|c| *c == v.from), copper.iter().position(|c| *c == v.to))
            {
                (Some(a), Some(b)) if a < b => {}
                (Some(_), Some(_)) => d.error(&at, "`from` must be above `to` in the stackup"),
                _ => d.error(
                    &at,
                    format!("`from`/`to` must name copper layers: {}", copper.join(", ")),
                ),
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
            if let Some(v) = &n.via
                && !self.vias.iter().any(|x| &x.name == v)
            {
                d.error(&at, format!("via `{v}` is not defined in [[vias]]"));
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
                let hint = match a.width_for_impedance {
                    Some(w) => format!(", use track_width = \"{w}\""),
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
                && need > n.track_width
            {
                d.error(
                    at,
                    format!(
                        "{} carries only {} for a {} rise, {} needs {need} (IPC-2221)",
                        n.track_width, a.current_capacity, n.max_temp_rise, i
                    ),
                );
            }
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
        let t = self.stackup.thickness();
        d.info("stackup", format!("{n_cu} copper layers, {t} finished thickness"));
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
    fn unknown_keys_are_rejected() {
        assert!(
            toml::from_str::<BoardFile>("name = \"x\"\n[stackup]\npreset = \"a\"\ntypo = 1\n")
                .is_err()
        );
    }
}
