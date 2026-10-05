use crate::board::{Board, Netclass};
use crate::diag::Diags;
use crate::footprint::{Footprint, PadKind, PadShape};
use crate::geom::{self, P, Transform};
use crate::graphic::Bounds;
use crate::schematic::{Schematic, UnionFind};
use crate::units::{Length, Point};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

mod index;
mod pour;

pub use pour::{
    FillCase, FillFile, FillKey, ZonesCase, capture_fill_cases, take_fill_cases, take_zones_cases,
    to_file as fill_file, without_fills,
};

pub(crate) const DRC_EPSILON: f64 = 5e-4;

thread_local! {
    static UNCHECKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn without_checks<T>(f: impl FnOnce() -> T) -> T {
    let before = UNCHECKED.with(|u| u.replace(true));
    let out = f();
    UNCHECKED.with(|u| u.set(before));
    out
}

fn checking() -> bool {
    !UNCHECKED.with(|u| u.get())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardSide {
    #[default]
    Top,
    Bottom,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementFile {
    #[serde(rename = "ref")]
    pub reference: String,
    pub at: Point,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<BoardSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<LabelFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mlcc: Option<bool>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Length>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hide: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Label {
    pub at: P,
    pub rotation: f64,
    pub size: f64,
    pub hide: bool,
    pub moved: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SilkText {
    pub owner: String,
    pub part: usize,
    pub text: String,
    pub at: P,
    pub rotation: f64,
    pub size: f64,
    pub anchor: crate::graphic::Anchor,
    pub layer: String,
}

impl SilkText {
    pub fn outline(&self) -> Vec<P> {
        let pen = crate::font::default_thickness(self.size);
        let w = crate::font::ink_width(&self.text, self.size) + pen;
        let h = self.size + pen;
        let x0 = match self.anchor {
            crate::graphic::Anchor::Left => 0.0,
            crate::graphic::Anchor::Center => -w / 2.0,
            crate::graphic::Anchor::Right => -w,
        };
        [[x0, -h / 2.0], [x0 + w, -h / 2.0], [x0 + w, h / 2.0], [x0, h / 2.0]]
            .into_iter()
            .map(|q| {
                let [x, y] = geom::rotate(q, self.rotation);
                [x + self.at[0], y + self.at[1]]
            })
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackFile {
    pub net: String,
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Length>,
    pub points: Vec<Point>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViaFile {
    pub net: String,
    pub at: Point,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<Point>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneFile {
    pub net: String,
    pub layers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Vec<Point>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_width: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_island_area: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad_connection: Option<PadConnection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relief_gap: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoke_width: Option<Length>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub relief_tht_only: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PadConnection {
    #[default]
    Solid,
    Relief,
    None,
}

impl ZoneFile {
    pub fn connection_of(&self, part: &Placed, k: usize) -> PadConnection {
        let Some(pad) = part.footprint.pads.get(k) else { return PadConnection::Solid };
        if let Some(c) = pad.zone_connect {
            return c;
        }
        if pad.kind == PadKind::Smd
            && pad.shape == PadShape::Circle
            && part.footprint.is_ball_grid()
        {
            return PadConnection::Solid;
        }
        match self.pad_connection.unwrap_or_default() {
            PadConnection::Relief if self.relief_tht_only && pad.kind != PadKind::Tht => {
                PadConnection::Solid
            }
            c => c,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutoutFile {
    pub layers: Vec<String>,
    pub points: Vec<Point>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schematic: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footprints: Vec<PlacementFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<TrackFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vias: Vec<ViaFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<ZoneFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cutouts: Vec<CutoutFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pairs: Vec<PairFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_groups: Vec<MatchFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graphics: Vec<crate::graphic::GraphicFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artwork: Vec<ArtworkFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fanouts: Vec<FanoutFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stitching: Vec<StitchFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<crate::interface::InterfaceFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fills: Vec<FillFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<WatermarkFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<crate::testpoint::TestFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<crate::place::PlaceFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<crate::engine::EngineFile>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatermarkFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StitchFile {
    pub net: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Vec<Point>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin: Option<Length>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Length>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip_at: Vec<Point>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FanoutFile {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<crate::board::ViaNames>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_rings: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub always: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip_at: Vec<Point>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairFile {
    pub p: String,
    pub n: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_skew: Option<Length>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchFile {
    pub name: String,
    pub nets: Vec<String>,
    pub tolerance: Length,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Length>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Pair {
    pub p: usize,
    pub n: usize,
    pub skew_mm: f64,
    pub skew_ps: f64,
    pub limit_mm: Option<f64>,
    pub coupled_mm: f64,
    pub chain: Vec<(usize, usize)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MatchGroup {
    pub name: String,
    pub nets: Vec<usize>,
    pub target_mm: f64,
    pub tolerance_mm: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtworkFile {
    pub layer: String,
    pub at: Point,
    pub height: Length,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Artwork {
    pub name: String,
    pub layer: String,
    pub polygons: Vec<Vec<P>>,
}

pub const ART_LAYERS: [&str; 4] = ["F.SilkS", "B.SilkS", "F.Fab", "B.Fab"];

#[derive(Clone, Debug, Serialize)]
pub struct PlacedPad {
    pub number: String,
    pub net: Option<usize>,
    pub kind: PadKind,
    pub outlines: Vec<Vec<P>>,
    pub copper: Vec<String>,
    pub mask: Vec<String>,
    pub paste: Vec<String>,
    pub drill: Option<(P, [f64; 2], f64)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Placed {
    pub reference: String,
    pub value: String,
    pub footprint_name: String,
    #[serde(skip)]
    pub footprint: Footprint,
    pub at: Point,
    pub rotation: f64,
    pub bottom: bool,
    pub pads: Vec<PlacedPad>,
    pub label: Option<Label>,
    pub mlcc: Option<bool>,
}

impl Placed {
    pub fn silk_texts(&self, index: usize) -> Vec<SilkText> {
        let t = self.transform();
        let mut out = Vec::new();
        for g in &self.footprint.graphics {
            let layer = self.flip_layer(&g.layer);
            if !layer.ends_with(".SilkS") {
                continue;
            }
            let crate::graphic::Shape::Text { at, text, size, rotation, anchor } = &g.shape else {
                continue;
            };
            let is_ref = text.contains("${REFERENCE}");
            let text =
                text.replace("${REFERENCE}", &self.reference).replace("${VALUE}", &self.value);
            let mut st = SilkText {
                owner: self.reference.clone(),
                part: index,
                text,
                at: t.apply(at.to_mm()),
                rotation: readable(rotation + self.rotation),
                size: size.to_mm(),
                anchor: *anchor,
                layer,
            };
            if is_ref && let Some(l) = &self.label {
                if l.hide {
                    continue;
                }
                if l.moved {
                    st.at = l.at;
                    st.anchor = crate::graphic::Anchor::Center;
                }
                st.rotation = l.rotation;
                st.size = l.size;
            }
            out.push(st);
        }
        out
    }

    pub fn transform(&self) -> Transform {
        Transform { at: self.at.to_mm(), rotation: self.rotation, mirror: self.bottom }
    }

    pub fn flip_layer(&self, l: &str) -> String {
        flip(l, self.bottom)
    }
}

fn flip(l: &str, bottom: bool) -> String {
    if !bottom {
        return l.to_string();
    }
    if let Some(r) = l.strip_prefix("F.") {
        format!("B.{r}")
    } else if let Some(r) = l.strip_prefix("B.") {
        format!("F.{r}")
    } else {
        l.to_string()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Track {
    pub source: usize,
    pub net: usize,
    pub layer: String,
    pub width: f64,
    pub points: Vec<P>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum ViaSource {
    #[default]
    Generated,
    File {
        index: usize,
        k: u32,
    },
    Fanout(usize),
    Stitch(usize),
}

#[derive(Clone, Debug, Serialize)]
pub struct Via {
    #[serde(skip)]
    pub source: ViaSource,
    pub net: usize,
    pub at: P,
    pub drill: f64,
    pub diameter: f64,
    pub layers: Vec<String>,
    pub name: String,
    pub kind: crate::board::ViaKind,
    pub hole: Vec<String>,
    pub drill_kind: crate::board::DrillKind,
    pub stacked: bool,
    pub fill: Option<crate::board::ViaFill>,
    pub backdrill: Option<crate::board::Backdrill>,
}

impl Via {
    pub fn of(spec: &crate::board::Via, net: usize, at: P, copper: &[String]) -> Via {
        Via {
            source: ViaSource::Generated,
            net,
            at,
            drill: spec.drill.to_mm(),
            diameter: spec.diameter.to_mm(),
            layers: spec.copper_layers(copper),
            name: spec.name.clone(),
            kind: spec.kind,
            hole: spec.hole_layers(copper),
            drill_kind: spec.drill_kind,
            stacked: spec.stacked,
            fill: spec.fill,
            backdrill: spec.backdrill.clone(),
        }
    }

    pub fn span_of(&self, copper: &[String]) -> Option<(usize, usize)> {
        let a = copper.iter().position(|c| Some(c) == self.hole.first())?;
        let b = copper.iter().position(|c| Some(c) == self.hole.last())?;
        Some((a.min(b), a.max(b)))
    }

    pub fn shares_dielectric(&self, other: &Via, copper: &[String]) -> bool {
        match (self.span_of(copper), other.span_of(copper)) {
            (Some((a, b)), Some((c, d))) => a.max(c) < b.min(d),
            _ => true,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ZoneFill {
    pub net: usize,
    pub layer: String,
    pub origin: P,
    pub cell: f64,
    pub width: usize,
    pub height: usize,
    #[serde(skip)]
    pub mask: Vec<u8>,
    pub islands_removed: usize,
    pub min_width: f64,
    #[serde(skip)]
    pub rings: Vec<Vec<P>>,
    #[serde(skip)]
    pub triangles: Vec<[P; 3]>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LayoutNet {
    pub name: String,
    pub class: String,
    pub width: f64,
    pub clearance: f64,
    pub unrouted: usize,
    pub length_mm: f64,
    pub delay_ps: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Layout {
    pub name: String,
    pub board: String,
    pub schematic: String,
    pub outline: Vec<P>,
    pub board_cutouts: Vec<Vec<P>>,
    pub copper: Vec<String>,
    pub parts: Vec<Placed>,
    pub tracks: Vec<Track>,
    pub vias: Vec<Via>,
    pub zones: Vec<ZoneFill>,
    pub nets: Vec<LayoutNet>,
    pub ratsnest: Vec<(P, P, usize)>,
    pub cutouts: Vec<(Vec<String>, Vec<P>)>,
    pub graphics: Vec<crate::graphic::Graphic>,
    pub artwork: Vec<Artwork>,
    pub pairs: Vec<Pair>,
    pub match_groups: Vec<MatchGroup>,
    pub silk: Vec<SilkBox>,
    #[serde(skip)]
    pub label_fixes: Vec<LabelFix>,
    pub interfaces: Vec<crate::interface::Interface>,
    pub fill_keys: Vec<FillKey>,
    pub watermark: Option<SilkText>,
    pub watermark_problem: Option<String>,
    pub test: crate::testpoint::TestSpec,
    pub engine: crate::engine::EngineFile,
}

#[derive(Clone, Debug, Serialize)]
pub struct LabelFix {
    pub reference: String,
    pub at: Option<P>,
    pub rotation: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SilkBox {
    pub text: String,
    pub layer: String,
    pub outline: Vec<P>,
}

impl Layout {
    pub fn board_texts(&self) -> Vec<SilkText> {
        let mut out = board_texts(&self.graphics);
        out.extend(self.watermark.clone());
        out
    }

    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        self.outline.iter().for_each(|p| b.add(*p));
        for p in &self.parts {
            p.pads.iter().flat_map(|q| q.outlines.iter().flatten()).for_each(|q| b.add(*q));
        }
        b
    }

    pub fn edge(&self) -> geom::BoardEdge<'_> {
        geom::BoardEdge::new(&self.outline, &self.board_cutouts)
    }

    pub fn settle_labels(&self, board: &Board) -> (Vec<LabelFix>, Vec<String>) {
        let mut parts = self.parts.clone();
        let mut moved: BTreeMap<String, LabelFix> = BTreeMap::new();
        let mut tries: HashMap<String, usize> = HashMap::new();
        let rule_on = |id: &str| crate::drc::id_enabled(board, id);
        let mut failing = Vec::new();
        let mut stuck: Vec<String> = Vec::new();
        for _ in 0..LABEL_PASSES {
            let fixes = check_silk(
                &parts,
                &self.vias,
                &self.tracks,
                &self.graphics,
                &self.artwork,
                self.edge(),
                board.rules.min_silk_text_height.to_mm(),
                &rule_on,
                &|r| stuck.iter().any(|s| s == r),
                &mut crate::drc::Findings::default(),
            );
            failing = fixes.iter().map(|f| f.reference.clone()).collect();
            let mut changed = 0;
            for f in fixes {
                let n = tries.entry(f.reference.clone()).or_default();
                *n += 1;
                let Some(at) = f.at.filter(|_| *n <= LABEL_TRIES) else {
                    if !stuck.contains(&f.reference) {
                        stuck.push(f.reference.clone());
                    }
                    continue;
                };
                let Some(p) = parts.iter_mut().find(|p| p.reference == f.reference) else {
                    continue;
                };
                let size = p.label.as_ref().map_or(1.0, |l| l.size);
                p.label = Some(Label { at, rotation: f.rotation, size, hide: false, moved: true });
                moved.insert(f.reference.clone(), f);
                changed += 1;
            }
            if changed == 0 {
                break;
            }
        }
        (moved.into_values().collect(), failing)
    }

    pub fn unrouted(&self) -> usize {
        self.nets.iter().map(|n| n.unrouted).sum()
    }
}

#[derive(Clone, Debug)]
enum Shape {
    Poly(Vec<Vec<P>>),
    Seg(P, P, f64),
    Circle(P, f64),
}

impl Shape {
    fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        match self {
            Shape::Poly(v) => v.iter().flatten().for_each(|p| b.add(*p)),
            Shape::Seg(a, c, hw) => {
                b.add_circle(*a, *hw);
                b.add_circle(*c, *hw);
            }
            Shape::Circle(c, r) => b.add_circle(*c, *r),
        }
        b
    }

    fn probe(&self) -> P {
        match self {
            Shape::Seg(p, q, _) => [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0],
            Shape::Circle(c, _) => *c,
            Shape::Poly(v) => v.first().and_then(|r| r.first()).copied().unwrap_or([0.0, 0.0]),
        }
    }

    fn point_distance(&self, p: P) -> f64 {
        match self {
            Shape::Poly(v) => v
                .iter()
                .map(|poly| {
                    if geom::point_in_polygon(p, poly) {
                        0.0
                    } else {
                        geom::polyline_polygon_distance(&[p, p], poly)
                    }
                })
                .fold(f64::MAX, f64::min),
            Shape::Seg(a, b, hw) => geom::point_segment_distance(p, *a, *b) - hw,
            Shape::Circle(c, r) => geom::dist(p, *c) - r,
        }
    }

    fn distance(&self, o: &Shape) -> f64 {
        match (self, o) {
            (Shape::Seg(a, b, h1), Shape::Seg(c, d, h2)) => {
                geom::segment_segment_distance(*a, *b, *c, *d) - h1 - h2
            }
            (Shape::Seg(a, b, h), Shape::Circle(c, r))
            | (Shape::Circle(c, r), Shape::Seg(a, b, h)) => {
                geom::point_segment_distance(*c, *a, *b) - h - r
            }
            (Shape::Circle(a, r1), Shape::Circle(b, r2)) => geom::dist(*a, *b) - r1 - r2,
            (Shape::Poly(v), Shape::Seg(a, b, h)) | (Shape::Seg(a, b, h), Shape::Poly(v)) => v
                .iter()
                .map(|poly| {
                    if geom::point_in_polygon(*a, poly) || geom::point_in_polygon(*b, poly) {
                        -h
                    } else {
                        geom::polyline_polygon_distance(&[*a, *b], poly) - h
                    }
                })
                .fold(f64::MAX, f64::min),
            (Shape::Poly(v), Shape::Circle(c, r)) | (Shape::Circle(c, r), Shape::Poly(v)) => {
                Shape::Poly(v.clone()).point_distance(*c) - r
            }
            (Shape::Poly(a), Shape::Poly(b)) => a
                .iter()
                .flat_map(|p| b.iter().map(move |q| geom::polygon_distance(p, q)))
                .fold(f64::MAX, f64::min),
        }
    }
}

fn footprint_copper(p: &Placed, copper: &[String]) -> Vec<(String, Vec<Shape>)> {
    let tf = p.transform();
    let mut out: Vec<(String, Vec<Shape>)> = Vec::new();
    for g in &p.footprint.graphics {
        let layer = p.flip_layer(&g.layer);
        if !copper.contains(&layer) || matches!(g.shape, crate::graphic::Shape::Text { .. }) {
            continue;
        }
        let path: Vec<P> =
            crate::footprint::graphic_path(g).into_iter().map(|q| tf.apply(q)).collect();
        let hw = g.width.to_mm() / 2.0;
        let shapes: Vec<Shape> = if g.fill == crate::graphic::Fill::Solid && path.len() >= 3 {
            vec![Shape::Poly(vec![path])]
        } else if hw > 0.0 {
            path.windows(2).map(|w| Shape::Seg(w[0], w[1], hw)).collect()
        } else {
            Vec::new()
        };
        if shapes.is_empty() {
            continue;
        }
        match out.iter_mut().find(|(l, _)| *l == layer) {
            Some((_, v)) => v.extend(shapes),
            None => out.push((layer, shapes)),
        }
    }
    out
}

pub(crate) fn footprint_copper_joins(
    p: &Placed,
    copper: &[String],
    layer: &str,
    net: usize,
    at: P,
    r: f64,
) -> bool {
    let Some((_, shapes)) = footprint_copper(p, copper).into_iter().find(|(l, _)| l == layer)
    else {
        return false;
    };
    shapes.iter().any(|s| s.point_distance(at) <= r + 1e-6)
        && p.pads.iter().any(|q| {
            q.net == Some(net)
                && q.copper.iter().any(|l| l == layer)
                && shapes.iter().any(|s| s.distance(&Shape::Poly(q.outlines.clone())) <= 1e-6)
        })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Owner {
    Pad(usize, usize),
    Seg(usize),
    Via(usize),
    Hole,
    PadHole(usize, usize),
}

#[derive(Clone)]
struct Item {
    owner: Owner,
    net: Option<usize>,
    layers: Vec<String>,
    shape: Shape,
    bounds: Bounds,
    pour_gap: f64,
}

pub struct Context<'a> {
    pub dir: std::path::PathBuf,
    pub board: &'a Board,
    pub schematic: &'a Schematic,
    pub footprints: HashMap<&'a str, &'a Footprint>,
    pub heat: Vec<(String, f64)>,
}

fn skipped(at: &[Point], c: P) -> bool {
    at.iter().any(|q| geom::dist(q.to_mm(), c) < 1e-3)
}

fn max_clear_of(nets: &[LayoutNet], default: f64) -> f64 {
    nets.iter().map(|n| n.clearance).fold(default, f64::max)
}

pub(crate) fn class_of<'a>(board: &'a Board, name: &str) -> Option<&'a Netclass> {
    board
        .netclasses
        .iter()
        .find(|n| n.name == name)
        .or_else(|| board.netclasses.iter().find(|n| n.name == "Default"))
}

fn outline_of(board: &Board) -> Vec<P> {
    board.outline.as_ref().map(|o| o.points()).unwrap_or_default()
}

fn board_cutouts_of(board: &Board, outline: &[P], d: &mut Diags) -> Vec<Vec<P>> {
    let cutouts: Vec<Vec<P>> = board.cutouts.iter().map(|c| c.points()).collect();
    for (i, c) in cutouts.iter().enumerate() {
        if outline.len() >= 3 && c.iter().any(|p| !geom::point_in_polygon(*p, outline)) {
            d.error(
                format!("outline.cutouts[{i}]"),
                "the cutout runs past the board outline, draw it as part of the outline",
            );
        }
    }
    cutouts
}

fn edges(poly: &[P]) -> impl Iterator<Item = (P, P)> + '_ {
    (0..poly.len()).map(move |i| (poly[i], poly[(i + 1) % poly.len()]))
}

impl LayoutFile {
    pub fn resolve(&self, cx: &Context, d: &mut Diags) -> Layout {
        let board = cx.board;
        let sch = cx.schematic;
        let copper = board.stackup.copper_names();
        let outline = outline_of(board);
        if outline.is_empty() {
            d.error("board", format!("board `{}` has no outline to place parts in", board.name));
        }
        let board_cutouts = board_cutouts_of(board, &outline, d);
        let nets: Vec<LayoutNet> = sch
            .nets
            .iter()
            .map(|n| {
                let c = class_of(board, &n.class);
                LayoutNet {
                    name: n.name.clone(),
                    class: n.class.clone(),
                    width: c
                        .map(|c| c.track_width.to_mm())
                        .unwrap_or(board.rules.min_track_width.to_mm()),
                    clearance: c
                        .map(|c| c.clearance.to_mm())
                        .unwrap_or(board.rules.min_clearance.to_mm()),
                    unrouted: 0,
                    length_mm: 0.0,
                    delay_ps: 0.0,
                }
            })
            .collect();
        let net_index = |name: &str| sch.nets.iter().position(|n| n.name == name);
        let mut found = crate::drc::Findings::default();
        let default_clearance = class_of(board, "Default")
            .map(|c| c.clearance.to_mm())
            .unwrap_or(board.rules.min_clearance.to_mm());

        let mut parts = Vec::new();
        let mut placed_refs: HashMap<&str, usize> = HashMap::new();
        for (i, f) in self.footprints.iter().enumerate() {
            let at = format!("footprints[{i}] {}", f.reference);
            if let Some(j) = placed_refs.insert(&f.reference, i) {
                d.error(&at, format!("{} is already placed by footprints[{j}]", f.reference));
                continue;
            }
            let units: Vec<(usize, &crate::schematic::Part)> =
                sch.parts.iter().enumerate().filter(|(_, p)| p.reference == f.reference).collect();
            let Some((_, part)) = units.first() else {
                d.error(&at, format!("{} is not in schematic `{}`", f.reference, sch.name));
                continue;
            };
            let Some(fp_name) = &part.footprint else {
                d.error(&at, format!("{} has no footprint in the schematic", f.reference));
                continue;
            };
            let Some(fp) = cx.footprints.get(fp_name.as_str()) else {
                d.error(&at, format!("footprint `{fp_name}` is not in this project"));
                continue;
            };
            let bottom = f.side == Some(BoardSide::Bottom);
            let rotation = f.rotation.unwrap_or(0.0);
            let t = Transform { at: f.at.to_mm(), rotation, mirror: bottom };
            let pads = fp
                .pads
                .iter()
                .map(|pad| {
                    let mut layers: Vec<String> = Vec::new();
                    for l in &pad.layers {
                        if l == "*.Cu" {
                            layers.extend(copper.iter().cloned());
                        } else if l.ends_with(".Cu") {
                            layers.push(flip(l, bottom));
                        }
                    }
                    layers.dedup();
                    if pad.kind == PadKind::Npth {
                        layers.clear();
                    }
                    let net = units.iter().find_map(|(pi, p)| {
                        p.symbol.pins.iter().position(|n| n.number == pad.number).and_then(|ni| {
                            sch.net_of(crate::schematic::PinRef { part: *pi, pin: ni })
                        })
                    });
                    PlacedPad {
                        number: pad.number.clone(),
                        net,
                        kind: pad.kind,
                        outlines: pad
                            .outlines()
                            .into_iter()
                            .map(|o| o.into_iter().map(|p| t.apply(p)).collect())
                            .collect(),
                        copper: layers,
                        mask: ["F.Mask", "B.Mask"]
                            .into_iter()
                            .filter(|m| pad.on_layer(m))
                            .map(|m| flip(m, bottom))
                            .collect(),
                        paste: ["F.Paste", "B.Paste"]
                            .into_iter()
                            .filter(|m| pad.on_layer(m))
                            .map(|m| flip(m, bottom))
                            .collect(),
                        drill: pad.drill.map(|dr| {
                            let s = dr.size().to_mm();
                            let o = geom::rotate(pad.drill_offset.to_mm(), pad.rotation);
                            let at = pad.at.to_mm();
                            (t.apply([at[0] + o[0], at[1] + o[1]]), s, pad.rotation + rotation)
                        }),
                    }
                })
                .collect();
            parts.push(Placed {
                reference: f.reference.clone(),
                value: part.value.clone(),
                footprint_name: fp_name.clone(),
                footprint: (*fp).clone(),
                at: f.at,
                rotation,
                bottom,
                pads,
                label: f.label.as_ref().map(|l| Label {
                    at: l.at.map(|p| p.to_mm()).unwrap_or(f.at.to_mm()),
                    rotation: l.rotation.unwrap_or(0.0),
                    size: l.size.map(Length::to_mm).unwrap_or(1.0),
                    hide: l.hide,
                    moved: l.at.is_some(),
                }),
                mlcc: f.mlcc.or(fp.mlcc),
            });
        }
        for r in sch.references() {
            let has_fp = sch.parts.iter().any(|p| p.reference == r && p.footprint.is_some());
            if has_fp && !placed_refs.contains_key(r) {
                d.error("footprints", format!("{r} is not placed"));
            }
        }

        let mut tracks = Vec::new();
        for (i, t) in self.tracks.iter().enumerate() {
            let at = format!("tracks[{i}] {}", t.net);
            let Some(net) = net_index(&t.net) else {
                d.error(&at, format!("net `{}` is not in the schematic", t.net));
                continue;
            };
            if !copper.contains(&t.layer) {
                d.error(
                    &at,
                    format!("`{}` is not a copper layer ({})", t.layer, copper.join(", ")),
                );
                continue;
            }
            if t.points.len() < 2 {
                d.error(&at, "a track needs at least two points");
                continue;
            }
            let width = t.width.map(Length::to_mm).unwrap_or_else(|| {
                class_of(board, &nets[net].class)
                    .map(|c| c.width_on(&t.layer).to_mm())
                    .unwrap_or(nets[net].width)
            });
            tracks.push(Track {
                source: i,
                net,
                layer: t.layer.clone(),
                width,
                points: t.points.iter().map(|p| p.to_mm()).collect(),
            });
        }

        let mut vias = Vec::new();
        for (i, v) in self.vias.iter().enumerate() {
            let at = format!("vias[{i}] {}", v.net);
            let Some(net) = net_index(&v.net) else {
                d.error(&at, format!("net `{}` is not in the schematic", v.net));
                continue;
            };
            let Some(spec) =
                board.via_for(v.via.as_deref(), class_of(board, &nets[net].class), &[])
            else {
                d.error(&at, "the board defines no [[vias]]");
                continue;
            };
            let count = v.count.unwrap_or(1).max(1);
            if count > 1 && v.pitch.is_none() {
                d.error(&at, "`count` needs a `pitch`");
            }
            let pitch = v.pitch.unwrap_or(Point::ZERO).to_mm();
            for k in 0..count {
                let [x, y] = v.at.to_mm();
                vias.push(Via {
                    source: ViaSource::File { index: i, k },
                    ..Via::of(
                        spec,
                        net,
                        [x + pitch[0] * k as f64, y + pitch[1] * k as f64],
                        &copper,
                    )
                });
            }
        }

        for (i, f) in self.fanouts.iter().enumerate() {
            let at = format!("fanouts[{i}] {}", f.reference);
            let matched: Vec<&Placed> = parts
                .iter()
                .filter(|p| glob(&f.reference, &p.reference))
                .filter(|p| !f.exclude.iter().any(|x| glob(x, &p.reference)))
                .filter(|p| p.reference == f.reference || !crate::testpoint::is_test_point(p))
                .collect();
            if matched.is_empty() {
                d.error(&at, format!("no placed part matches `{}`", f.reference));
                continue;
            }
            for n in &f.always {
                if !nets.iter().any(|m| glob(n, &m.name)) {
                    d.error(&at, format!("no net matches `{n}`"));
                }
            }
            let mut placed = 0;
            for part in matched {
                let pads: Vec<(&PlacedPad, P)> = part
                    .pads
                    .iter()
                    .filter(|p| !p.copper.is_empty() && p.kind != PadKind::Tht)
                    .map(|p| {
                        let mut b = Bounds::EMPTY;
                        p.outlines.iter().flatten().for_each(|q| b.add(*q));
                        (p, b.center())
                    })
                    .collect();
                let mut grid = Bounds::EMPTY;
                pads.iter().for_each(|(_, c)| grid.add(*c));
                let pitch = pads
                    .iter()
                    .flat_map(|(_, a)| pads.iter().map(move |(_, b)| geom::dist(*a, *b)))
                    .filter(|d| *d > 1e-3)
                    .fold(f64::MAX, f64::min);
                let rings = f.skip_rings.unwrap_or(0) as f64;
                for (pad, c) in &pads {
                    let Some(net) = pad.net else { continue };
                    if f.skip.contains(&pad.number) || skipped(&f.skip_at, *c) {
                        continue;
                    }
                    if !f.nets.is_empty() && !f.nets.iter().any(|g| glob(g, &nets[net].name)) {
                        continue;
                    }
                    if vias.iter().any(|v: &Via| geom::dist(v.at, *c) < 1e-6) {
                        continue;
                    }
                    let edge = (c[0] - grid.min[0])
                        .min(grid.max[0] - c[0])
                        .min(c[1] - grid.min[1])
                        .min(grid.max[1] - c[1]);
                    let ring = (edge / pitch).round();
                    if ring < rings && !f.always.iter().any(|g| glob(g, &nets[net].name)) {
                        continue;
                    }
                    let reach: Vec<&str> = pad.copper.iter().map(String::as_str).collect();
                    let names =
                        f.via.as_ref().map(crate::board::ViaNames::list).unwrap_or_default();
                    let Some(spec) =
                        board.via_among(&names, class_of(board, &nets[net].class), &reach)
                    else {
                        d.error(&at, "the board defines no [[vias]]");
                        break;
                    };
                    vias.push(Via {
                        source: ViaSource::Fanout(i),
                        ..Via::of(spec, net, *c, &copper)
                    });
                    placed += 1;
                }
            }
            if placed == 0 {
                found.add(
                    "fanout-empty",
                    &at,
                    "placed no vias: no pad with a net is left after the skips",
                );
            }
        }

        let hole_to_copper = board.rules.min_via_hole_to_copper.to_mm();
        let smd_gap = board.rules.min_hole_to_smd_pad.to_mm();
        let npth_gap = board.rules.min_npth_to_copper.to_mm();
        let via_pour_gap = |v: &Via| (v.drill / 2.0 + hole_to_copper - v.diameter / 2.0).max(0.0);
        let pth_gap = board.rules.min_pth_hole_to_copper.to_mm();
        let inner_pth_gap = pth_gap.max(board.rules.min_inner_pth_hole_to_copper.to_mm());
        let mut hole_layers: Vec<(f64, Vec<String>)> = Vec::new();
        for (i, l) in copper.iter().enumerate() {
            let gap = if i == 0 || i + 1 == copper.len() { pth_gap } else { inner_pth_gap };
            match hole_layers.iter_mut().find(|(g, _)| *g == gap) {
                Some((_, ls)) => ls.push(l.clone()),
                None => hole_layers.push((gap, vec![l.clone()])),
            }
        }
        let mut items: Vec<Item> = Vec::new();
        for (pi, p) in parts.iter().enumerate() {
            let t = p.transform();
            for (k, pad) in p.pads.iter().enumerate() {
                if pad.kind != PadKind::Npth
                    && let Some((c, size, _)) = pad.drill
                {
                    let rot = p.footprint.pads.get(k).map(|f| f.rotation).unwrap_or(0.0);
                    let long = if size[0] >= size[1] { [1.0, 0.0] } else { [0.0, 1.0] };
                    let u = t.direction(geom::rotate(long, rot));
                    let half = (size[0] - size[1]).abs() / 2.0;
                    let shape = Shape::Seg(
                        [c[0] - u[0] * half, c[1] - u[1] * half],
                        [c[0] + u[0] * half, c[1] + u[1] * half],
                        size[0].min(size[1]) / 2.0,
                    );
                    for (gap, layers) in &hole_layers {
                        items.push(Item {
                            owner: Owner::PadHole(pi, k),
                            net: pad.net,
                            layers: layers.clone(),
                            bounds: shape.bounds(),
                            shape: shape.clone(),
                            pour_gap: *gap,
                        });
                    }
                }
                if !pad.copper.is_empty() {
                    let shape = Shape::Poly(pad.outlines.clone());
                    items.push(Item {
                        owner: Owner::Pad(pi, k),
                        net: pad.net,
                        layers: pad.copper.clone(),
                        bounds: shape.bounds(),
                        shape,
                        pour_gap: 0.0,
                    });
                }
                if pad.kind == PadKind::Npth
                    && let Some((c, s, _)) = pad.drill
                {
                    let shape = Shape::Circle(c, s[0].max(s[1]) / 2.0);
                    items.push(Item {
                        owner: Owner::Hole,
                        net: None,
                        layers: copper.clone(),
                        bounds: shape.bounds(),
                        shape,
                        pour_gap: npth_gap,
                    });
                }
            }
        }
        let mut seg_track = Vec::new();
        for (ti, t) in tracks.iter().enumerate() {
            let pour_gap = class_of(board, &nets[t.net].class)
                .and_then(|c| c.coplanar_gap)
                .map_or(0.0, Length::to_mm);
            for w in t.points.windows(2) {
                let shape = Shape::Seg(w[0], w[1], t.width / 2.0);
                seg_track.push(ti);
                items.push(Item {
                    owner: Owner::Seg(seg_track.len() - 1),
                    net: Some(t.net),
                    layers: vec![t.layer.clone()],
                    bounds: shape.bounds(),
                    shape,
                    pour_gap,
                });
            }
        }
        for (vi, v) in vias.iter().enumerate() {
            let shape = Shape::Circle(v.at, v.diameter / 2.0);
            items.push(Item {
                owner: Owner::Via(vi),
                net: Some(v.net),
                layers: v.layers.clone(),
                bounds: shape.bounds(),
                shape,
                pour_gap: via_pour_gap(v),
            });
        }

        for (i, st) in self.stitching.iter().enumerate() {
            let at = format!("stitching[{i}] {}", st.net);
            let Some(net) = net_index(&st.net) else {
                d.error(&at, format!("net `{}` is not in the schematic", st.net));
                continue;
            };
            let Some(spec) =
                board.via_for(st.via.as_deref(), class_of(board, &nets[net].class), &[])
            else {
                d.error(&at, "the board defines no [[vias]]");
                continue;
            };
            let layers = spec.copper_layers(&copper);
            let (r, drill) = (spec.diameter.to_mm() / 2.0, spec.drill.to_mm());
            let mut candidates: Vec<P> = Vec::new();
            if st.fence.is_empty() {
                let pitch = st.pitch.map(Length::to_mm).unwrap_or(2.5);
                let area: Vec<P> = st
                    .outline
                    .as_ref()
                    .map(|o| o.iter().map(|p| p.to_mm()).collect())
                    .unwrap_or_else(|| outline.clone());
                let mut b = Bounds::EMPTY;
                area.iter().for_each(|q| b.add(*q));
                let mut y = (b.min[1] / pitch).ceil() * pitch;
                while y <= b.max[1] {
                    let mut x = (b.min[0] / pitch).ceil() * pitch;
                    while x <= b.max[0] {
                        if geom::point_in_polygon([x, y], &area) {
                            candidates.push([x, y]);
                        }
                        x += pitch;
                    }
                    y += pitch;
                }
            } else {
                let pitch = st.pitch.map(Length::to_mm).unwrap_or(1.0);
                for t in
                    tracks.iter().filter(|t| st.fence.iter().any(|g| glob(g, &nets[t.net].name)))
                {
                    let gap = class_of(board, &nets[t.net].class)
                        .and_then(|c| c.coplanar_gap.map(Length::to_mm))
                        .unwrap_or(nets[t.net].clearance);
                    let off =
                        st.offset.map(Length::to_mm).unwrap_or(t.width / 2.0 + gap + r + 0.05);
                    let mut carry = 0.0;
                    for w in t.points.windows(2) {
                        let l = geom::dist(w[0], w[1]);
                        if l < 1e-9 {
                            continue;
                        }
                        let u = [(w[1][0] - w[0][0]) / l, (w[1][1] - w[0][1]) / l];
                        let n = [-u[1], u[0]];
                        let mut s = carry;
                        while s <= l {
                            for side in [1.0, -1.0] {
                                candidates.push([
                                    w[0][0] + u[0] * s + n[0] * off * side,
                                    w[0][1] + u[1] * s + n[1] * off * side,
                                ]);
                            }
                            s += pitch;
                        }
                        carry = s - l;
                    }
                }
            }
            let margin = st.margin.map(Length::to_mm).unwrap_or(0.0);
            let edge = board.rules.min_copper_to_edge.to_mm().max(margin) + r;
            let hole_gap = board.rules.min_hole_to_hole.to_mm();
            let through = (0, copper.len().saturating_sub(1));
            let span = spec.span(&copper);
            let mut drills: Vec<(P, f64, (usize, usize))> = vias
                .iter()
                .map(|v| (v.at, v.drill / 2.0, v.span_of(&copper).unwrap_or(through)))
                .collect();
            for p in &parts {
                for pad in &p.pads {
                    if let Some((c, sz, _)) = pad.drill {
                        drills.push((c, sz[0].max(sz[1]) / 2.0, through));
                    }
                }
            }
            let zoned: Vec<Vec<P>> = self
                .zones
                .iter()
                .filter(|z| z.net == st.net && z.layers.iter().any(|l| layers.contains(l)))
                .map(|z| {
                    z.outline
                        .as_ref()
                        .map(|o| o.iter().map(|p| p.to_mm()).collect())
                        .unwrap_or_else(|| outline.clone())
                })
                .collect();
            let own = nets[net].clearance;
            let mut placed = 0;
            for c in candidates.into_iter().filter(|c| !skipped(&st.skip_at, *c)) {
                let board_edge = geom::BoardEdge::new(&outline, &board_cutouts);
                if !board_edge.contains(c)
                    || board_edge.distance(c) < edge - 1e-9
                    || !zoned.iter().any(|z| geom::point_in_polygon(c, z))
                    || drills.iter().any(|(q, dr, (a, b))| {
                        span.0.max(*a) < span.1.min(*b)
                            && geom::dist(c, *q) - dr - drill / 2.0 < hole_gap - 1e-9
                    })
                {
                    continue;
                }
                let probe = Shape::Circle(c, r);
                let reach = r + max_clear_of(&nets, default_clearance).max(smd_gap);
                let near_smd = items.iter().any(|it| {
                    let Owner::Pad(pi, k) = it.owner else { return false };
                    parts[pi].pads[k].kind == PadKind::Smd
                        && it.layers.iter().any(|l| layers.contains(l))
                        && it.shape.point_distance(c) < (drill / 2.0 + smd_gap).max(r) - 1e-9
                });
                if near_smd {
                    continue;
                }
                let self_gap = (drill / 2.0 + hole_to_copper - r).max(0.0);
                let blocked = items.iter().any(|it| {
                    it.net != Some(net)
                        && it.layers.iter().any(|l| layers.contains(l))
                        && it.bounds.min[0] <= c[0] + reach
                        && it.bounds.max[0] >= c[0] - reach
                        && it.bounds.min[1] <= c[1] + reach
                        && it.bounds.max[1] >= c[1] - reach
                        && it.shape.distance(&probe)
                            < own
                                .max(it.net.map(|n| nets[n].clearance).unwrap_or(default_clearance))
                                .max(self_gap)
                                .max(it.pour_gap)
                                - 1e-9
                });
                if blocked {
                    continue;
                }
                vias.push(Via { source: ViaSource::Stitch(i), ..Via::of(spec, net, c, &copper) });
                items.push(Item {
                    owner: Owner::Via(vias.len() - 1),
                    net: Some(net),
                    layers: layers.clone(),
                    bounds: probe.bounds(),
                    shape: probe,
                    pour_gap: via_pour_gap(vias.last().unwrap()),
                });
                drills.push((c, drill / 2.0, span));
                placed += 1;
            }
            found.add("stitching", &at, format!("{placed} stitching vias"));
            if placed == 0 {
                found.add(
                    "stitching-empty",
                    &at,
                    "placed no vias: every spot is blocked, or outside a zone of the net",
                );
            }
        }

        let clearance_of =
            |n: Option<usize>| n.map(|n| nets[n].clearance).unwrap_or(default_clearance);
        let name_of = |it: &Item| -> String {
            match it.owner {
                Owner::Pad(pi, k) => {
                    format!("{}.{}", parts[pi].reference, parts[pi].pads[k].number)
                }
                Owner::Seg(s) => {
                    format!("track {} ({})", seg_track[s], nets[tracks[seg_track[s]].net].name)
                }
                Owner::Via(v) => format!(
                    "via at [{:.3}, {:.3}] ({})",
                    vias[v].at[0], vias[v].at[1], nets[vias[v].net].name
                ),
                Owner::Hole => "hole".into(),
                Owner::PadHole(pi, k) => {
                    format!("{}.{} hole", parts[pi].reference, parts[pi].pads[k].number)
                }
            }
        };

        let rule_on = |id: &str| crate::drc::id_enabled(board, id);
        let copper_rules = rule_on("short") || rule_on("clearance");
        let min_clearance = board.rules.min_clearance.to_mm();
        let footprint_clearance = |it: &Item| match it.owner {
            Owner::Pad(pi, _) => parts[pi].footprint.clearance,
            _ => None,
        };
        let net_tied = |pad: &Item, other: &Item| {
            let Owner::Pad(pi, k) = pad.owner else { return false };
            let p = &parts[pi];
            let Some(group) = p.footprint.net_tie_group(&p.pads[k].number) else { return false };
            other.net.is_some()
                && p.pads.iter().any(|q| q.net == other.net && group.contains(&q.number))
        };
        let mut uf = UnionFind::new(items.len());
        let mut shorts = Vec::new();
        let mut tight = Vec::new();
        let max_clear = nets.iter().map(|n| n.clearance).fold(default_clearance, f64::max);
        for i in 0..items.len() {
            for j in i + 1..items.len() {
                let (a, b) = (&items[i], &items[j]);
                if matches!(a.owner, Owner::PadHole(..)) || matches!(b.owner, Owner::PadHole(..)) {
                    continue;
                }
                if let (Owner::Pad(p1, _), Owner::Pad(p2, _)) = (a.owner, b.owner)
                    && p1 == p2
                {
                    if a.net.is_some() && a.net == b.net {
                        uf.union(i, j);
                    }
                    continue;
                }
                if !a.layers.iter().any(|l| b.layers.contains(l)) {
                    continue;
                }
                let mut grown = a.bounds;
                grown.add([grown.min[0] - max_clear, grown.min[1] - max_clear]);
                grown.add([grown.max[0] + max_clear, grown.max[1] + max_clear]);
                if !grown.overlaps(&b.bounds) && !grown.contains(&b.bounds) {
                    continue;
                }
                let same = a.net.is_some() && a.net == b.net;
                if !same && (!copper_rules || net_tied(a, b) || net_tied(b, a)) {
                    continue;
                }
                let dist = a.shape.distance(&b.shape);
                if same {
                    if dist <= 1e-6 {
                        uf.union(i, j);
                    }
                    continue;
                }
                if a.owner == Owner::Hole || b.owner == Owner::Hole {
                    if dist <= 0.0 {
                        tight.push(format!(
                            "{} runs into a hole",
                            if a.owner == Owner::Hole { name_of(b) } else { name_of(a) }
                        ));
                    }
                    continue;
                }
                let need = match (footprint_clearance(a), footprint_clearance(b)) {
                    (None, None) => clearance_of(a.net).max(clearance_of(b.net)),
                    (x, y) => x.unwrap_or(0.0).max(y.unwrap_or(0.0)).max(min_clearance),
                };
                if dist <= 1e-6 {
                    shorts.push(format!("{} touches {}", name_of(a), name_of(b)));
                } else if dist + DRC_EPSILON < need {
                    tight.push(format!(
                        "{} is {} from {}, needs {}",
                        name_of(a),
                        Length::mm(dist),
                        name_of(b),
                        Length::mm(need)
                    ));
                }
            }
        }
        for (pi, p) in parts.iter().enumerate() {
            for (layer, shapes) in footprint_copper(p, &copper) {
                let mut cb = Bounds::EMPTY;
                shapes.iter().for_each(|sh| cb.union(&sh.bounds()));
                let near = |it: &Item| {
                    let mut grown = cb;
                    grown.add([cb.min[0] - max_clear, cb.min[1] - max_clear]);
                    grown.add([cb.max[0] + max_clear, cb.max[1] + max_clear]);
                    it.layers.contains(&layer)
                        && (grown.overlaps(&it.bounds) || grown.contains(&it.bounds))
                };
                let gap = |it: &Item| {
                    shapes.iter().map(|sh| sh.distance(&it.shape)).fold(f64::MAX, f64::min)
                };
                let own: Vec<usize> = (0..items.len())
                    .filter(|&i| {
                        matches!(items[i].owner, Owner::Pad(q, _) if q == pi)
                            && near(&items[i])
                            && gap(&items[i]) <= 1e-6
                    })
                    .collect();
                for w in own.windows(2) {
                    uf.union(w[0], w[1]);
                }
                let tie: Vec<usize> = own.iter().filter_map(|&i| items[i].net).collect();
                let name = format!("{} copper on {layer}", p.reference);
                for (j, it) in items.iter().enumerate() {
                    let tied = it.net.is_some_and(|n| tie.contains(&n));
                    if matches!(it.owner, Owner::Pad(q, _) if q == pi)
                        || matches!(it.owner, Owner::PadHole(..))
                        || !(tied || copper_rules)
                        || !near(it)
                    {
                        continue;
                    }
                    let dist = gap(it);
                    if it.owner == Owner::Hole {
                        if dist <= 0.0 {
                            tight.push(format!("{name} runs into a hole"));
                        }
                        continue;
                    }
                    if tied {
                        if dist <= 1e-6 {
                            uf.union(own[0], j);
                        }
                        continue;
                    }
                    let need = tie
                        .iter()
                        .map(|&n| clearance_of(Some(n)))
                        .fold(clearance_of(None), f64::max)
                        .max(clearance_of(it.net));
                    if dist <= 1e-6 {
                        shorts.push(format!("{name} touches {}", name_of(it)));
                    } else if dist + DRC_EPSILON < need {
                        tight.push(format!(
                            "{name} is {} from {}, needs {}",
                            Length::mm(dist),
                            name_of(it),
                            Length::mm(need)
                        ));
                    }
                }
            }
        }
        for s in shorts {
            found.add("short", "short", s);
        }
        for s in tight {
            found.add("clearance", "clearance", s);
        }

        let edge_clear = board.rules.min_copper_to_edge.to_mm();

        let cutouts: Vec<(Vec<String>, Vec<P>)> = self
            .cutouts
            .iter()
            .map(|c| (c.layers.clone(), c.points.iter().map(|p| p.to_mm()).collect()))
            .collect();
        let mut zones = Vec::new();
        let mut fill_keys: Vec<FillKey> = Vec::new();
        let mut island_nodes: Vec<Vec<usize>> = Vec::new();
        let zone_area = |z: &ZoneFile| {
            let pts: Vec<P> = match &z.outline {
                Some(p) => p.iter().map(|q| q.to_mm()).collect(),
                None => outline.clone(),
            };
            geom::signed_area(&pts).abs()
        };
        let mut order: Vec<usize> = (0..self.zones.len()).collect();
        order.sort_by(|&a, &b| {
            let (za, zb) = (&self.zones[a], &self.zones[b]);
            zb.priority
                .unwrap_or(0)
                .cmp(&za.priority.unwrap_or(0))
                .then(zone_area(za).total_cmp(&zone_area(zb)))
        });
        for zi in order {
            let z = &self.zones[zi];
            let at = format!("zones[{zi}] {}", z.net);
            let Some(net) = net_index(&z.net) else {
                d.error(&at, format!("net `{}` is not in the schematic", z.net));
                continue;
            };
            let poly: Vec<P> = match &z.outline {
                Some(p) => p.iter().map(|q| q.to_mm()).collect(),
                None => outline.clone(),
            };
            if poly.len() < 3 {
                d.error(&at, "zone outline needs three points, or leave it out to fill the board");
                continue;
            }
            let clearance = z.clearance.map(Length::to_mm).unwrap_or(nets[net].clearance);
            for layer in &z.layers {
                if !copper.contains(layer) {
                    d.error(&at, format!("`{layer}` is not a copper layer"));
                    continue;
                }
                let min_width = z.min_width.map(Length::to_mm).unwrap_or(0.25);
                let reliefs = relief_cutouts(
                    &items,
                    &parts,
                    net,
                    layer,
                    &poly,
                    ReliefSpec {
                        zone: z,
                        gap: z.relief_gap.map(Length::to_mm).unwrap_or(clearance),
                        clearance,
                        spoke: z
                            .spoke_width
                            .map(Length::to_mm)
                            .unwrap_or(nets[net].width.max(min_width)),
                    },
                );
                let layer_cutouts: Vec<&Vec<P>> = cutouts
                    .iter()
                    .filter(|(ls, _)| ls.contains(layer))
                    .map(|(_, p)| p)
                    .chain(&reliefs)
                    .collect();
                let (blockers, blocker_hashes): (Vec<&ZoneFill>, Vec<u64>) = zones
                    .iter()
                    .zip(&fill_keys)
                    .filter(|(f, _): &(&ZoneFill, &FillKey)| &f.layer == layer && f.net != net)
                    .map(|(f, k)| (f, k.hash))
                    .unzip();
                let spec = pour::FillSpec {
                    net,
                    net_name: &z.net,
                    layer,
                    poly: &poly,
                    board: geom::BoardEdge::new(&outline, &board_cutouts),
                    edge_clear,
                    clearance,
                    cutouts: &layer_cutouts,
                    min_width: z.min_width.map(Length::to_mm).unwrap_or(0.25),
                    min_island_area: z.min_island_area.unwrap_or(2.0),
                };
                let hash = spec.hash(&items, &clearance_of, &blocker_hashes);
                let stored = self.fills.iter().find(|f| f.zone == zi && &f.layer == layer);
                let fresh = stored.filter(|f| f.hash == format!("{hash:016x}"));
                if pour::capturing() {
                    pour::capture(|| FillCase {
                        name: format!("zone{zi} {} {layer}", z.net),
                        net,
                        layer: layer.clone(),
                        poly: poly.clone(),
                        board: outline.clone(),
                        board_cutouts: board_cutouts.clone(),
                        edge_clear,
                        clearance,
                        items: items
                            .iter()
                            .filter(|it| it.layers.iter().any(|l| l == layer))
                            .cloned()
                            .collect(),
                        net_clearance: nets.iter().map(|n| n.clearance).collect(),
                        default_clearance,
                        cutouts: layer_cutouts.iter().map(|c| c.to_vec()).collect(),
                        blockers: blockers.iter().map(|b| (*b).clone()).collect(),
                        min_width: spec.min_width,
                        min_island_area: spec.min_island_area,
                    });
                }
                let hit = if fresh.is_none() { pour::cached(hash) } else { None };
                let (fill, touched) = match fresh.or(hit.as_ref()) {
                    Some(f) => spec.stored(&items, f),
                    None if pour::capturing() => spec.stored(&items, &FillFile::default()),
                    None if pour::unfilled() => {
                        spec.stored(&items, stored.unwrap_or(&FillFile::default()))
                    }
                    None => fill_zone(
                        net,
                        layer,
                        &poly,
                        geom::BoardEdge::new(&outline, &board_cutouts),
                        edge_clear,
                        clearance,
                        &items,
                        &clearance_of,
                        &layer_cutouts,
                        &blockers,
                        spec.min_width,
                        spec.min_island_area,
                    ),
                };
                if fresh.is_none() && hit.is_none() && !pour::capturing() && !pour::unfilled() {
                    pour::cache(hash, &fill);
                }
                if stored.is_some() && fresh.is_none() {
                    d.info(
                        &at,
                        format!("the stored fill on {layer} is stale, `agentee fill` refreshes it"),
                    );
                }
                fill_keys.push(FillKey {
                    zone: zi,
                    layer: layer.clone(),
                    hash,
                    stored: fresh.is_some(),
                    stale: stored.is_some() && fresh.is_none(),
                });
                if fill.islands_removed > 0 {
                    found.add(
                        "zone-islands",
                        &at,
                        format!(
                            "{} copper islands on {layer} reach nothing of {} and were removed",
                            fill.islands_removed, z.net
                        ),
                    );
                }
                zones.push(fill);
                island_nodes.extend(touched);
            }
        }
        for touched in &island_nodes {
            for group in touched.windows(2) {
                uf.union(group[0], group[1]);
            }
        }
        pour::capture_zones(|| pour::ZonesCase {
            zones: zones.clone(),
            items: items.clone(),
            nets: nets.clone(),
            default_clearance,
        });
        check_zones(&zones, &items, &nets, &clearance_of, &rule_on, &mut found);

        let mut ratsnest = Vec::new();
        let mut stats: Vec<(usize, f64)> = Vec::new();
        for (ni, n) in nets.iter().enumerate() {
            let pads: Vec<(usize, P)> = items
                .iter()
                .enumerate()
                .filter(|(_, it)| it.net == Some(ni) && matches!(it.owner, Owner::Pad(..)))
                .map(|(i, it)| (i, it.bounds.center()))
                .collect();
            let mut groups: BTreeMap<usize, Vec<(usize, P)>> = BTreeMap::new();
            for (i, c) in &pads {
                groups.entry(uf.find(*i)).or_default().push((*i, *c));
            }
            let groups: Vec<Vec<(usize, P)>> = groups.into_values().collect();
            let unrouted = groups.len().saturating_sub(1);
            if groups.len() > 1 {
                let mut joined = vec![false; groups.len()];
                joined[0] = true;
                for _ in 1..groups.len() {
                    let mut best: Option<(f64, usize, P, P)> = None;
                    for g in groups.iter().enumerate().filter(|(gi, _)| joined[*gi]).map(|(_, g)| g)
                    {
                        for (hi, h) in groups.iter().enumerate().filter(|(hi, _)| !joined[*hi]) {
                            for (_, a) in g {
                                for (_, b) in h {
                                    let dd = geom::dist(*a, *b);
                                    if best.is_none_or(|x| dd < x.0) {
                                        best = Some((dd, hi, *a, *b));
                                    }
                                }
                            }
                        }
                    }
                    let (_, hi, a, b) = best.unwrap();
                    joined[hi] = true;
                    ratsnest.push((a, b, ni));
                }
                let names: Vec<String> = groups
                    .iter()
                    .map(|g| {
                        g.iter().map(|(i, _)| name_of(&items[*i])).collect::<Vec<_>>().join("+")
                    })
                    .collect();
                found.add(
                    "unrouted",
                    format!("net {}", n.name),
                    format!("{unrouted} unrouted: {} are not joined", names.join(" | ")),
                );
            }
            let length = tracks
                .iter()
                .filter(|t| t.net == ni)
                .flat_map(|t| t.points.windows(2).map(|w| geom::dist(w[0], w[1])))
                .sum();
            stats.push((unrouted, length));
        }

        let (graphics, artwork) = self.artwork_of(&cx.dir, d);
        let silk: Vec<SilkBox> = parts
            .iter()
            .enumerate()
            .flat_map(|(i, p)| p.silk_texts(i))
            .chain(board_texts(&graphics))
            .map(|t| SilkBox { outline: t.outline(), text: t.text, layer: t.layer })
            .collect();
        let label_fixes = if checking() {
            check_silk(
                &parts,
                &vias,
                &tracks,
                &graphics,
                &artwork,
                geom::BoardEdge::new(&outline, &board_cutouts),
                board.rules.min_silk_text_height.to_mm(),
                &rule_on,
                &|_| false,
                &mut found,
            )
        } else {
            Vec::new()
        };

        let (watermark, watermark_problem) = place_watermark(
            self.watermark.as_ref(),
            board,
            &parts,
            &vias,
            &graphics,
            &artwork,
            geom::BoardEdge::new(&outline, &board_cutouts),
            &mut found,
        );

        let mut nets = nets;
        for (n, (unrouted, length)) in nets.iter_mut().zip(stats) {
            n.unrouted = unrouted;
            n.length_mm = length;
        }
        for (ni, n) in nets.iter_mut().enumerate() {
            n.delay_ps = tracks
                .iter()
                .filter(|t| t.net == ni)
                .map(|t| {
                    let len: f64 = t.points.windows(2).map(|w| geom::dist(w[0], w[1])).sum();
                    len * delay_per_mm(board, &t.layer, t.width)
                })
                .sum();
        }
        let pairs = self.pairs_of(board, &nets, &tracks, &parts, d, &mut found);
        let match_groups = self.matches_of(&nets, d, &mut found);
        let interfaces = crate::interface::check(
            &self.interfaces,
            &crate::interface::Ctx {
                board,
                copper: &copper,
                nets: &nets,
                tracks: &tracks,
                vias: &vias,
                parts: &parts,
                zones: &zones,
                pairs: &pairs,
            },
            d,
            &mut found,
        );
        let test = self.test.clone().unwrap_or_default().resolve(d);
        let engine = self.engine.clone().unwrap_or_default();
        engine.check(d);
        if checking() {
            crate::drc::run(
                &crate::drc::Ctx::new(
                    board,
                    &copper,
                    &outline,
                    &board_cutouts,
                    &parts,
                    &tracks,
                    &vias,
                    &zones,
                    &nets,
                )
                .with_signals(&graphics, &pairs, &match_groups, &interfaces)
                .with_test(&test)
                .with_heat(&cx.heat)
                .with_found(&found),
                d,
            );
        }
        Layout {
            name: self.name.clone(),
            board: board.name.clone(),
            schematic: sch.name.clone(),
            outline,
            board_cutouts,
            copper,
            parts,
            tracks,
            vias,
            zones,
            nets,
            ratsnest,
            cutouts,
            graphics,
            artwork,
            pairs,
            match_groups,
            silk,
            label_fixes,
            interfaces,
            fill_keys,
            watermark,
            watermark_problem,
            test,
            engine,
        }
    }

    fn pairs_of(
        &self,
        board: &Board,
        nets: &[LayoutNet],
        tracks: &[Track],
        parts: &[Placed],
        d: &mut Diags,
        findings: &mut crate::drc::Findings,
    ) -> Vec<Pair> {
        let index = |n: &str| nets.iter().position(|x| x.name == n);
        let mut found: Vec<(usize, usize, Option<f64>)> = Vec::new();
        for (i, pf) in self.pairs.iter().enumerate() {
            match (index(&pf.p), index(&pf.n)) {
                (Some(a), Some(b)) => found.push((a, b, pf.max_skew.map(Length::to_mm))),
                _ => d.error(
                    format!("pairs[{i}]"),
                    format!("`{}` / `{}` are not nets of the layout", pf.p, pf.n),
                ),
            }
        }
        for (a, na) in nets.iter().enumerate() {
            let Some(base) = pair_base(&na.name, true) else { continue };
            if found.iter().any(|f| f.0 == a || f.1 == a) {
                continue;
            }
            let pair = class_of(board, &na.class).is_some_and(|c| c.diff_gap.is_some());
            if let Some(b) = nets
                .iter()
                .position(|nb| pair_base(&nb.name, false).as_deref() == Some(base.as_str()))
                && pair
            {
                found.push((a, b, None));
            }
        }
        let series: Vec<(usize, usize)> = parts
            .iter()
            .filter_map(|p| {
                let mut nets = p.pads.iter().filter(|q| !q.copper.is_empty()).map(|q| q.net);
                match (nets.next(), nets.next(), nets.next()) {
                    (Some(Some(a)), Some(Some(b)), None) if a != b => Some((a, b)),
                    _ => None,
                }
            })
            .collect();
        let joined =
            |x: usize, y: usize| series.iter().any(|&(u, v)| (u, v) == (x, y) || (v, u) == (x, y));
        let mut group: Vec<usize> = (0..found.len()).collect();
        fn root(g: &mut [usize], i: usize) -> usize {
            if g[i] != i {
                let r = root(g, g[i]);
                g[i] = r;
            }
            g[i]
        }
        for i in 0..found.len() {
            for j in i + 1..found.len() {
                if joined(found[i].0, found[j].0) && joined(found[i].1, found[j].1) {
                    let (ri, rj) = (root(&mut group, i), root(&mut group, j));
                    group[rj] = ri;
                }
            }
        }
        type Skew = (f64, f64, Option<f64>, Vec<(usize, usize)>);
        let mut skews: std::collections::HashMap<usize, Skew> = std::collections::HashMap::new();
        for (i, &(a, b, skew_limit)) in found.iter().enumerate() {
            let r = root(&mut group, i);
            let class = class_of(board, &nets[a].class);
            let limit = skew_limit.or(class.and_then(|c| c.max_skew.map(Length::to_mm)));
            let e = skews.entry(r).or_insert((0.0, 0.0, None, Vec::new()));
            e.0 += nets[a].length_mm - nets[b].length_mm;
            e.1 += nets[a].delay_ps - nets[b].delay_ps;
            e.2 = match (e.2, limit) {
                (Some(x), Some(y)) => Some(x.min(y)),
                (x, y) => x.or(y),
            };
            e.3.push((a, b));
        }
        let mut reported = std::collections::HashSet::new();
        let mut out = Vec::new();
        for (i, (a, b, _)) in found.clone().into_iter().enumerate() {
            let (pa, pb) = (&nets[a], &nets[b]);
            let class = class_of(board, &pa.class);
            let gap = class.and_then(|c| c.diff_gap.map(Length::to_mm));
            let at = format!("pair {}/{}", pa.name, pb.name);
            let r = root(&mut group, i);
            let (skew_mm, skew_ps, limit, chain) = skews[&r].clone();
            if reported.insert(r) {
                let names = |pick: fn(&(usize, usize)) -> usize| {
                    chain.iter().map(|m| nets[pick(m)].name.as_str()).collect::<Vec<_>>().join("+")
                };
                let (at, short) = if chain.len() > 1 {
                    (
                        format!("pair {}/{}", names(|m| m.0), names(|m| m.1)),
                        if skew_mm > 0.0 { names(|m| m.1) } else { names(|m| m.0) },
                    )
                } else {
                    (at.clone(), if skew_mm > 0.0 { pb.name.clone() } else { pa.name.clone() })
                };
                match limit {
                    Some(l) if skew_mm.abs() > l + 1e-9 => findings.add(
                        "pair-skew",
                        &at,
                        format!(
                            "skew {:.3} mm ({:.2} ps), limit {l} mm: lengthen {} by {:.3} mm",
                            skew_mm,
                            skew_ps,
                            short,
                            skew_mm.abs() - l
                        ),
                    ),
                    _ => findings.add(
                        "pair-skew-info",
                        &at,
                        format!("skew {:.3} mm ({:.2} ps)", skew_mm, skew_ps),
                    ),
                }
            }
            let mut coupled = 0.0;
            let mut wrong: Option<(f64, f64)> = None;
            let mut uncoupled = 0.0;
            let budget = class.and_then(|c| c.max_uncoupled.map(Length::to_mm)).unwrap_or(0.0);
            if let Some(g) = gap {
                for ta in tracks.iter().filter(|t| t.net == a) {
                    for tb in tracks.iter().filter(|t| t.net == b && t.layer == ta.layer) {
                        for sa in ta.points.windows(2) {
                            for sb in tb.points.windows(2) {
                                if let Some((overlap, sep)) =
                                    parallel_overlap(sa[0], sa[1], sb[0], sb[1])
                                {
                                    let want = g + (ta.width + tb.width) / 2.0;
                                    if (sep - want).abs() <= 0.1 * g + 0.005 {
                                        coupled += overlap;
                                    } else if sep < want + 2.0 * g && overlap > 0.05 {
                                        uncoupled += overlap;
                                        wrong = Some((sep - (ta.width + tb.width) / 2.0, overlap));
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some((got, len)) = wrong.filter(|_| uncoupled > budget) {
                    findings.add(
                        "pair-gap",
                        &at,
                        format!("runs {len:.2} mm at a {got:.3} mm gap, the class wants {g} mm"),
                    );
                }
                let longest = pa.length_mm.max(pb.length_mm);
                if longest > 0.0 && coupled < 0.8 * longest {
                    findings.add(
                        "pair-coupling",
                        &at,
                        format!(
                            "only {coupled:.2} of {longest:.2} mm run side by side at the pair gap"
                        ),
                    );
                }
            }
            out.push(Pair {
                p: a,
                n: b,
                skew_mm,
                skew_ps,
                limit_mm: limit,
                coupled_mm: coupled,
                chain,
            });
        }
        out
    }

    fn matches_of(
        &self,
        nets: &[LayoutNet],
        d: &mut Diags,
        found: &mut crate::drc::Findings,
    ) -> Vec<MatchGroup> {
        let mut out = Vec::new();
        for (i, m) in self.match_groups.iter().enumerate() {
            let at = format!("match_groups[{i}] {}", m.name);
            let members: Vec<usize> = nets
                .iter()
                .enumerate()
                .filter(|(_, n)| m.nets.iter().any(|pat| glob(pat, &n.name)))
                .map(|(k, _)| k)
                .collect();
            if members.len() < 2 {
                d.error(&at, "matches fewer than two nets");
                continue;
            }
            let target = m
                .target
                .map(Length::to_mm)
                .unwrap_or_else(|| members.iter().map(|k| nets[*k].length_mm).fold(0.0, f64::max));
            let tol = m.tolerance.to_mm();
            for k in &members {
                let n = &nets[*k];
                let off = n.length_mm - target;
                if off.abs() > tol + 1e-9 {
                    found.add(
                        "match-length",
                        &at,
                        format!(
                            "{} is {:.3} mm, {:.3} mm {} the {:.3} mm target",
                            n.name,
                            n.length_mm,
                            off.abs(),
                            if off < 0.0 { "short of" } else { "over" },
                            target
                        ),
                    );
                }
            }
            out.push(MatchGroup {
                name: m.name.clone(),
                nets: members,
                target_mm: target,
                tolerance_mm: tol,
            });
        }
        out
    }

    pub fn artwork_of(
        &self,
        dir: &std::path::Path,
        d: &mut Diags,
    ) -> (Vec<crate::graphic::Graphic>, Vec<Artwork>) {
        let def = crate::graphic::GraphicDefaults {
            width: Length::mm(0.15),
            text_size: Length::mm(1.0),
            layer: Some("F.SilkS"),
        };
        let mut graphics = Vec::new();
        for (i, g) in self.graphics.iter().enumerate() {
            let at = format!("graphics[{i}]");
            let Some(g) = crate::graphic::resolve(g, &at, d, &def) else { continue };
            if !ART_LAYERS.contains(&g.layer.as_str()) {
                d.error(
                    &at,
                    format!("layer `{}` is not one of {}", g.layer, ART_LAYERS.join(", ")),
                );
                continue;
            }
            graphics.push(g);
        }
        let mut artwork = Vec::new();
        for (i, a) in self.artwork.iter().enumerate() {
            let at = format!("artwork[{i}]");
            if !ART_LAYERS.contains(&a.layer.as_str()) {
                d.error(
                    &at,
                    format!("layer `{}` is not one of {}", a.layer, ART_LAYERS.join(", ")),
                );
                continue;
            }
            let (name, svg) = match (&a.icon, &a.file) {
                (Some(n), None) => match crate::artwork::icon(n) {
                    Some(s) => (n.clone(), s.to_string()),
                    None => {
                        d.error(
                            &at,
                            format!(
                                "no icon `{n}`, there is {}",
                                crate::artwork::icon_names().join(", ")
                            ),
                        );
                        continue;
                    }
                },
                (None, Some(f)) => match std::fs::read_to_string(dir.join(f)) {
                    Ok(s) => (f.clone(), s),
                    Err(e) => {
                        d.error(&at, format!("cannot read {f}: {e}"));
                        continue;
                    }
                },
                _ => {
                    d.error(&at, "give either `icon` or `file` (an SVG)");
                    continue;
                }
            };
            if !a.height.is_positive() {
                d.error(&at, "`height` must be positive");
                continue;
            }
            match crate::artwork::svg_polygons(&svg, a.height.to_mm()) {
                Ok(polys) => artwork.push(Artwork {
                    name,
                    layer: a.layer.clone(),
                    polygons: crate::artwork::place(
                        &polys,
                        a.at.to_mm(),
                        a.rotation.unwrap_or(0.0),
                        a.layer.starts_with("B."),
                    ),
                }),
                Err(e) => d.error(&at, format!("{name}: {e}")),
            }
        }
        (graphics, artwork)
    }
}

pub fn board_texts(graphics: &[crate::graphic::Graphic]) -> Vec<SilkText> {
    graphics
        .iter()
        .filter(|g| g.layer.ends_with(".SilkS"))
        .filter_map(|g| match &g.shape {
            crate::graphic::Shape::Text { at, text, size, rotation, anchor } => Some(SilkText {
                owner: format!("board text `{text}`"),
                part: usize::MAX,
                text: text.clone(),
                at: at.to_mm(),
                rotation: *rotation,
                size: size.to_mm(),
                anchor: *anchor,
                layer: g.layer.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn readable(deg: f64) -> f64 {
    let a = deg.rem_euclid(360.0);
    if a > 90.0 && a <= 270.0 { a - 180.0 } else { a }
}

#[allow(clippy::too_many_arguments)]
fn check_silk(
    parts: &[Placed],
    vias: &[Via],
    tracks: &[Track],
    graphics: &[crate::graphic::Graphic],
    artwork: &[Artwork],
    board_edge: geom::BoardEdge,
    min_height: f64,
    rule_on: &dyn Fn(&str) -> bool,
    stuck: &dyn Fn(&str) -> bool,
    d: &mut crate::drc::Findings,
) -> Vec<LabelFix> {
    let mut fixes = Vec::new();
    let artwork_on = rule_on("silk-artwork");
    let height_on = rule_on("silk-text-height");
    let texts_on = rule_on("silk-text") || rule_on("silk-hidden");
    if !(artwork_on || height_on || texts_on) {
        return fixes;
    }
    let mut texts: Vec<SilkText> =
        parts.iter().enumerate().flat_map(|(i, p)| p.silk_texts(i)).collect();
    texts.extend(board_texts(graphics));
    let boxes: Vec<Vec<P>> = texts.iter().map(|t| t.outline()).collect();
    let spans = part_spans(parts);
    for a in artwork.iter().filter(|a| artwork_on && a.layer.ends_with(".SilkS")) {
        let at = format!("silk {}", a.name);
        let cu = format!("{}.Cu", a.layer.trim_end_matches(".SilkS"));
        let over: Vec<String> = parts
            .iter()
            .flat_map(|p| p.pads.iter().map(move |q| (p, q)))
            .filter(|(_, q)| {
                q.copper.contains(&cu)
                    && q.outlines
                        .iter()
                        .any(|o| a.polygons.iter().any(|r| geom::polygon_distance(o, r) <= 0.0))
            })
            .map(|(p, q)| format!("{}.{}", p.reference, q.number))
            .collect();
        if !over.is_empty() {
            d.add(
                "silk-artwork",
                &at,
                format!("`{}` sits on pads {}, it will be clipped", a.name, over.join(", ")),
            );
        }
        let hit: Vec<&str> = texts
            .iter()
            .zip(&boxes)
            .filter(|(t, b)| {
                t.layer == a.layer && a.polygons.iter().any(|r| geom::polygon_distance(r, b) <= 0.0)
            })
            .map(|(t, _)| t.text.as_str())
            .collect();
        if !hit.is_empty() {
            d.add("silk-artwork", &at, format!("`{}` overlaps {}", a.name, hit.join(", ")));
        }
        if board_edge.is_closed() && a.polygons.iter().any(|r| !board_edge.holds(r)) {
            d.add("silk-artwork", &at, format!("`{}` runs off the board", a.name));
        }
    }
    for (i, t) in texts.iter().enumerate() {
        let who = &t.owner;
        let at = format!("silk {who}");
        if height_on && t.size + 1e-9 < min_height {
            d.add(
                "silk-text-height",
                &at,
                format!(
                    "`{}` is {} tall, under the fab minimum {}",
                    t.text,
                    Length::mm(t.size),
                    Length::mm(min_height)
                ),
            );
        }
        if !texts_on {
            continue;
        }
        let found = silk_issues(t, &boxes[i], i, &texts, &boxes, parts, &spans, vias, board_edge);
        if found.is_empty() {
            continue;
        }
        let hint = match (t.part != usize::MAX).then(|| parts.get(t.part)).flatten() {
            Some(p) if t.text == p.reference => {
                let spot = if stuck(&p.reference) {
                    None
                } else {
                    let scene = (parts, spans.as_slice(), vias, tracks);
                    free_spot(t, p, &texts, &boxes, i, scene, board_edge)
                };
                fixes.push(LabelFix {
                    reference: p.reference.clone(),
                    at: spot.map(|s| s.0),
                    rotation: spot.map(|s| s.1).unwrap_or(0.0),
                });
                spot.map(|(at, rot)| {
                    let r = if rot != 0.0 { format!(", rotation = {rot}") } else { String::new() };
                    format!("; label = {{ at = [{:.2}, {:.2}]{r} }} is clear", at[0], at[1])
                })
                .unwrap_or_default()
            }
            _ => String::new(),
        };
        let text: Vec<&str> = found.iter().map(|(_, s)| s.as_str()).collect();
        let message = format!("`{}` {}{hint}", t.text, text.join(", "));
        let rule = if found.iter().any(|(error, _)| *error) { "silk-text" } else { "silk-hidden" };
        d.add(rule, &at, message);
    }
    fixes
}

const SILK_GAP: f64 = 0.4;
const LABEL_PASSES: usize = 8;
const LABEL_TRIES: usize = 3;
pub(crate) const NECKDOWN: f64 = 0.5;

#[allow(clippy::too_many_arguments)]
fn silk_issues(
    t: &SilkText,
    bx: &[P],
    me: usize,
    texts: &[SilkText],
    boxes: &[Vec<P>],
    parts: &[Placed],
    spans: &[Bounds],
    vias: &[Via],
    board_edge: geom::BoardEdge,
) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    let reach = ring_bounds(std::slice::from_ref(&bx.to_vec()));
    let near = |pts: &mut dyn Iterator<Item = P>, gap: f64| {
        let mut b = Bounds::EMPTY;
        pts.for_each(|q| b.add(q));
        !b.is_empty()
            && b.max[0] + gap >= reach.min[0]
            && b.min[0] - gap <= reach.max[0]
            && b.max[1] + gap >= reach.min[1]
            && b.min[1] - gap <= reach.max[1]
    };
    for (j, u) in texts.iter().enumerate() {
        if j != me
            && u.layer == t.layer
            && near(&mut boxes[j].iter().copied(), SILK_GAP)
            && geom::polygon_distance(bx, &boxes[j]) < SILK_GAP
        {
            out.push((true, format!("crowds `{}` of {}", u.text, u.owner)));
        }
    }
    let close = |k: usize| {
        let b = &spans[k];
        let m = SILK_GAP + 0.1;
        !b.is_empty()
            && b.max[0] + m >= reach.min[0]
            && b.min[0] - m <= reach.max[0]
            && b.max[1] + m >= reach.min[1]
            && b.min[1] - m <= reach.max[1]
    };
    let side = t.layer.trim_end_matches(".SilkS");
    let cu = format!("{side}.Cu");
    let pads: Vec<String> = parts
        .iter()
        .enumerate()
        .filter(|(k, _)| close(*k))
        .flat_map(|(_, p)| p.pads.iter().map(move |q| (p, q)))
        .filter(|(_, q)| {
            (q.copper.contains(&cu) || q.drill.is_some())
                && q.outlines.iter().any(|o| {
                    near(&mut o.iter().copied(), 0.0) && geom::polygon_distance(o, bx) <= 0.0
                })
        })
        .map(|(p, q)| format!("{}.{}", p.reference, q.number))
        .collect();
    if !pads.is_empty() {
        out.push((true, format!("sits on pads {}, it will be clipped", pads.join(", "))));
    }
    let face = if t.layer.starts_with("B.") { "B.Cu" } else { "F.Cu" };
    let on_vias = vias
        .iter()
        .filter(|v| {
            v.hole.iter().any(|l| l == face)
                && near(&mut std::iter::once(v.at), v.diameter / 2.0)
                && (geom::point_in_polygon(v.at, bx)
                    || geom::polyline_polygon_distance(&[v.at, v.at], bx) < v.diameter / 2.0)
        })
        .count();
    if on_vias > 0 {
        out.push((
            true,
            format!("prints over {on_vias} via{}", if on_vias == 1 { "" } else { "s" }),
        ));
    }
    let flipped = flip(&t.layer, true);
    let crossed: Vec<&str> = parts
        .iter()
        .enumerate()
        .filter(|(k, _)| close(*k))
        .map(|(_, p)| p)
        .filter(|p| {
            let tf = p.transform();
            let layer = if p.bottom { &flipped } else { &t.layer };
            p.footprint.graphics.iter().any(|g| {
                g.layer == *layer && !matches!(g.shape, crate::graphic::Shape::Text { .. }) && {
                    let reach = g.width.to_mm() / 2.0 + SILK_GAP / 2.0;
                    let b = g.bounds();
                    let corners = [b.min, [b.max[0], b.min[1]], b.max, [b.min[0], b.max[1]]];
                    near(&mut corners.map(|q| tf.apply(q)).into_iter(), reach) && {
                        let path: Vec<P> = crate::footprint::graphic_path(g)
                            .into_iter()
                            .map(|q| tf.apply(q))
                            .collect();
                        let gap = g.width.to_mm() / 2.0 + SILK_GAP / 2.0;
                        path.len() >= 2
                            && near(&mut path.iter().copied(), gap)
                            && geom::polyline_polygon_distance(&path, bx)
                                < g.width.to_mm() / 2.0 + SILK_GAP / 2.0
                    }
                }
            })
        })
        .map(|p| p.reference.as_str())
        .collect();
    if !crossed.is_empty() {
        out.push((true, format!("crosses the silk outline of {}", crossed.join(", "))));
    }
    if board_edge.is_closed() && !board_edge.holds(bx) {
        out.push((true, "runs off the board".into()));
    }
    let side = if t.layer.starts_with("B.") { "B" } else { "F" };
    let hidden: Vec<&str> = parts
        .iter()
        .enumerate()
        .filter(|(k, p)| {
            *k != t.part
                && close(*k)
                && body_box(p, side).is_some_and(|b| {
                    near(&mut b.iter().copied(), 0.0) && geom::polygon_distance(&b, bx) <= 0.0
                })
        })
        .map(|(_, p)| p.reference.as_str())
        .collect();
    if !hidden.is_empty() {
        out.push((false, format!("hides under the body of {}", hidden.join(", "))));
    }
    out
}

fn part_spans(parts: &[Placed]) -> Vec<Bounds> {
    parts
        .iter()
        .map(|p| {
            let mut b = Bounds::EMPTY;
            p.pads.iter().flat_map(|q| q.outlines.iter().flatten()).for_each(|q| b.add(*q));
            let tf = p.transform();
            for g in &p.footprint.graphics {
                let gb = g.bounds();
                if gb.is_empty() {
                    continue;
                }
                let w = g.width.to_mm() / 2.0;
                for c in [gb.min, [gb.max[0], gb.min[1]], gb.max, [gb.min[0], gb.max[1]]] {
                    b.add_circle(tf.apply(c), w);
                }
            }
            b
        })
        .collect()
}

pub(crate) fn body_box(p: &Placed, side: &str) -> Option<Vec<P>> {
    let layer =
        format!("{}.Fab", if p.bottom { if side == "F" { "B" } else { "F" } } else { side });
    let mut b = Bounds::EMPTY;
    for g in p
        .footprint
        .graphics
        .iter()
        .filter(|g| g.layer == layer && !matches!(g.shape, crate::graphic::Shape::Text { .. }))
    {
        b.union(&g.bounds());
    }
    if b.is_empty() || (p.bottom != (side == "B")) {
        return None;
    }
    let tf = p.transform();
    Some(
        [b.min, [b.max[0], b.min[1]], b.max, [b.min[0], b.max[1]]]
            .into_iter()
            .map(|q| tf.apply(q))
            .collect(),
    )
}

#[allow(clippy::too_many_arguments)]
fn free_spot(
    t: &SilkText,
    part: &Placed,
    texts: &[SilkText],
    boxes: &[Vec<P>],
    me: usize,
    scene: (&[Placed], &[Bounds], &[Via], &[Track]),
    board_edge: geom::BoardEdge,
) -> Option<(P, f64)> {
    let (parts, spans, vias, tracks) = scene;
    let mut b = Bounds::EMPTY;
    for q in &part.pads {
        q.outlines.iter().flatten().for_each(|p| b.add(*p));
    }
    let side = if t.layer.starts_with("B.") { "B" } else { "F" };
    let court = part.footprint.courtyard(if part.bottom {
        if side == "F" { "B" } else { "F" }
    } else {
        side
    });
    if !court.is_empty() {
        let tf = part.transform();
        for c in [court.min, court.max, [court.min[0], court.max[1]], [court.max[0], court.min[1]]]
        {
            b.add(tf.apply(c));
        }
    }
    if b.is_empty() {
        return None;
    }
    let c = b.center();
    let half = [(b.max[0] - b.min[0]) / 2.0, (b.max[1] - b.min[1]) / 2.0];
    let pen = crate::font::default_thickness(t.size);
    let w = crate::font::ink_width(&t.text, t.size) + pen;
    let h = t.size + pen;
    let mut candidates: Vec<(P, f64)> = Vec::new();
    let slides: Vec<f64> = std::iter::once(0.0)
        .chain((1..=8).flat_map(|k| [k as f64 * 0.25, -(k as f64) * 0.25]))
        .collect();
    for step in 0..6 {
        let gap = SILK_GAP + step as f64 * 0.35;
        for &o in &slides {
            candidates.push(([c[0] + o, b.min[1] - gap - h / 2.0], 0.0));
            candidates.push(([c[0] + o, b.max[1] + gap + h / 2.0], 0.0));
            candidates.push(([b.max[0] + gap + w / 2.0, c[1] + o], 0.0));
            candidates.push(([b.min[0] - gap - w / 2.0, c[1] + o], 0.0));
            candidates.push(([b.max[0] + gap + h / 2.0, c[1] + o], 90.0));
            candidates.push(([b.min[0] - gap - h / 2.0, c[1] + o], 90.0));
        }
        for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            candidates.push((
                [c[0] + sx * (half[0] + gap + w / 2.0), c[1] + sy * (half[1] + gap + h / 2.0)],
                0.0,
            ));
        }
    }
    let cu = format!("{side}.Cu");
    let round = |v: f64| (v * 100.0).round() / 100.0;
    candidates.into_iter().map(|(at, rot)| ([round(at[0]), round(at[1])], rot)).find(|(at, rot)| {
        let trial = SilkText {
            at: *at,
            rotation: *rot,
            anchor: crate::graphic::Anchor::Center,
            ..t.clone()
        };
        let bx = trial.outline();
        silk_issues(&trial, &bx, me, texts, boxes, parts, spans, vias, board_edge).is_empty()
            && !tracks.iter().any(|tr| {
                tr.layer == cu
                    && geom::polyline_polygon_distance(&tr.points, &bx) < tr.width / 2.0 + 0.1
            })
    })
}

const WATERMARK_CELL: f64 = 0.25;

struct Occupancy {
    origin: P,
    w: usize,
    h: usize,
    blocked: Vec<bool>,
    sum: Vec<u32>,
}

impl Occupancy {
    fn new(b: &Bounds) -> Occupancy {
        let w = ((b.max[0] - b.min[0]) / WATERMARK_CELL).ceil().max(0.0) as usize;
        let h = ((b.max[1] - b.min[1]) / WATERMARK_CELL).ceil().max(0.0) as usize;
        Occupancy { origin: b.min, w, h, blocked: vec![false; w * h], sum: Vec::new() }
    }

    fn center(&self, i: usize, j: usize) -> P {
        [
            self.origin[0] + (i as f64 + 0.5) * WATERMARK_CELL,
            self.origin[1] + (j as f64 + 0.5) * WATERMARK_CELL,
        ]
    }

    fn block(&mut self, b: &Bounds, reach: f64) {
        if b.is_empty() || self.w == 0 || self.h == 0 {
            return;
        }
        let cell = |v: f64, o: f64, n: usize| {
            ((v - o) / WATERMARK_CELL).floor().clamp(0.0, n as f64 - 1.0) as usize
        };
        if b.max[0] + reach < self.origin[0] || b.max[1] + reach < self.origin[1] {
            return;
        }
        let (x0, x1) = (
            cell(b.min[0] - reach, self.origin[0], self.w),
            cell(b.max[0] + reach, self.origin[0], self.w),
        );
        let (y0, y1) = (
            cell(b.min[1] - reach, self.origin[1], self.h),
            cell(b.max[1] + reach, self.origin[1], self.h),
        );
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.blocked[y * self.w + x] = true;
            }
        }
    }

    fn block_path(&mut self, path: &[P], reach: f64) {
        for s in path.windows(2) {
            let mut b = Bounds::EMPTY;
            b.add(s[0]);
            b.add(s[1]);
            self.block(&b, reach);
        }
    }

    fn finish(&mut self) {
        let stride = self.w + 1;
        self.sum = vec![0; stride * (self.h + 1)];
        for y in 0..self.h {
            for x in 0..self.w {
                self.sum[(y + 1) * stride + x + 1] = self.blocked[y * self.w + x] as u32
                    + self.sum[y * stride + x + 1]
                    + self.sum[(y + 1) * stride + x]
                    - self.sum[y * stride + x];
            }
        }
    }

    fn count(&self, x: usize, y: usize, nw: usize, nh: usize) -> u32 {
        let stride = self.w + 1;
        let (x1, y1) = (x + nw, y + nh);
        self.sum[y1 * stride + x1] + self.sum[y * stride + x]
            - self.sum[y * stride + x1]
            - self.sum[y1 * stride + x]
    }
}

#[allow(clippy::too_many_arguments)]
fn watermark_occupancy(
    layer: &str,
    parts: &[Placed],
    vias: &[Via],
    texts: &[SilkText],
    boxes: &[Vec<P>],
    graphics: &[crate::graphic::Graphic],
    artwork: &[Artwork],
    board_edge: geom::BoardEdge,
    margin: f64,
) -> Occupancy {
    let mut ob = Bounds::EMPTY;
    board_edge.outline.iter().for_each(|p| ob.add(*p));
    let mut occ = Occupancy::new(&ob);
    for j in 0..occ.h {
        for i in 0..occ.w {
            let c = occ.center(i, j);
            if !board_edge.contains(c) || board_edge.distance(c) < margin + WATERMARK_CELL {
                occ.blocked[j * occ.w + i] = true;
            }
        }
    }
    let side = if layer.starts_with("B.") { "B" } else { "F" };
    let cu = format!("{side}.Cu");
    for q in parts.iter().flat_map(|p| p.pads.iter()) {
        if q.copper.contains(&cu) || q.drill.is_some() {
            occ.block(&crate::drc::rings_bounds(&q.outlines), 0.05);
        }
    }
    for v in vias.iter().filter(|v| v.hole.contains(&cu)) {
        let mut b = Bounds::EMPTY;
        b.add_circle(v.at, v.diameter / 2.0);
        occ.block(&b, 0.05);
    }
    for (t, bx) in texts.iter().zip(boxes) {
        if t.layer == layer {
            let mut b = Bounds::EMPTY;
            bx.iter().for_each(|p| b.add(*p));
            occ.block(&b, SILK_GAP);
        }
    }
    for p in parts {
        let tf = p.transform();
        for g in &p.footprint.graphics {
            if p.flip_layer(&g.layer) == layer
                && !matches!(g.shape, crate::graphic::Shape::Text { .. })
            {
                let path: Vec<P> =
                    crate::footprint::graphic_path(g).into_iter().map(|q| tf.apply(q)).collect();
                occ.block_path(&path, g.width.to_mm() / 2.0 + SILK_GAP / 2.0);
            }
        }
        if let Some(b) = body_box(p, side) {
            let mut bb = Bounds::EMPTY;
            b.iter().for_each(|q| bb.add(*q));
            occ.block(&bb, 0.0);
        }
    }
    for g in graphics.iter().filter(|g| g.layer == layer) {
        if matches!(g.shape, crate::graphic::Shape::Text { .. }) {
            continue;
        }
        let path = crate::footprint::graphic_path(g);
        if g.fill != crate::graphic::Fill::None {
            let mut b = Bounds::EMPTY;
            path.iter().for_each(|q| b.add(*q));
            occ.block(&b, g.width.to_mm() / 2.0 + SILK_GAP / 2.0);
        } else {
            occ.block_path(&path, g.width.to_mm() / 2.0 + SILK_GAP / 2.0);
        }
    }
    for a in artwork.iter().filter(|a| a.layer == layer) {
        occ.block(&crate::drc::rings_bounds(&a.polygons), SILK_GAP);
    }
    occ.finish();
    occ
}

#[allow(clippy::too_many_arguments)]
fn place_watermark(
    spec: Option<&WatermarkFile>,
    board: &Board,
    parts: &[Placed],
    vias: &[Via],
    graphics: &[crate::graphic::Graphic],
    artwork: &[Artwork],
    board_edge: geom::BoardEdge,
    found: &mut crate::drc::Findings,
) -> (Option<SilkText>, Option<String>) {
    let text = crate::version::watermark();
    let size = board.rules.min_silk_text_height.to_mm();
    let margin = board.rules.min_copper_to_edge.to_mm();
    let silk_layers: Vec<String> = board
        .stackup
        .layers
        .iter()
        .filter(|l| l.kind == crate::board::LayerKind::Silk)
        .map(|l| l.name.clone())
        .collect();
    let spec = spec.cloned().unwrap_or_default();
    let layers: Vec<String> = match &spec.layer {
        Some(l) if silk_layers.contains(l) => vec![l.clone()],
        Some(l) => {
            found.add(
                "watermark",
                "watermark",
                format!(
                    "layer `{l}` is not a silk layer of the board, use {}",
                    silk_layers.join(" or ")
                ),
            );
            return (None, Some(format!("[watermark] layer `{l}` is not a silk layer")));
        }
        None => ["B.SilkS", "F.SilkS"]
            .into_iter()
            .filter(|l| silk_layers.iter().any(|s| s == l))
            .map(String::from)
            .collect(),
    };
    let Some(first) = layers.first() else {
        let msg = "the board has no silk layer for the watermark: add a `[[stackup.layers]]` with `kind = \"silk\"` outside the mask at the top and bottom of the board's stackup".to_string();
        found.add("watermark", "watermark", &msg);
        return (None, Some(msg));
    };
    let mut texts: Vec<SilkText> =
        parts.iter().enumerate().flat_map(|(i, p)| p.silk_texts(i)).collect();
    texts.extend(board_texts(graphics));
    let boxes: Vec<Vec<P>> = texts.iter().map(|t| t.outline()).collect();
    let spans = part_spans(parts);
    let make = |at: P, rotation: f64, layer: &str| SilkText {
        owner: "watermark".into(),
        part: usize::MAX,
        text: text.clone(),
        at,
        rotation,
        size,
        anchor: crate::graphic::Anchor::Center,
        layer: layer.into(),
    };
    let problems = |t: &SilkText| -> Vec<String> {
        let bx = t.outline();
        let mut out: Vec<String> =
            silk_issues(t, &bx, usize::MAX, &texts, &boxes, parts, &spans, vias, board_edge)
                .into_iter()
                .map(|(_, s)| s)
                .collect();
        let lines = graphics.iter().any(|g| {
            g.layer == t.layer && !matches!(g.shape, crate::graphic::Shape::Text { .. }) && {
                let path = crate::footprint::graphic_path(g);
                path.len() >= 2
                    && geom::polyline_polygon_distance(&path, &bx)
                        < g.width.to_mm() / 2.0 + SILK_GAP / 2.0
            }
        });
        if lines {
            out.push("crosses board silk lines".into());
        }
        if artwork.iter().any(|a| {
            a.layer == t.layer
                && a.polygons.iter().any(|r| geom::polygon_distance(r, &bx) < SILK_GAP)
        }) {
            out.push("overlaps silk artwork".into());
        }
        if board_edge.is_closed()
            && board_edge.holds(&bx)
            && bx.iter().any(|c| board_edge.distance(*c) < margin)
        {
            out.push(format!("sits closer than {} to the board edge", Length::mm(margin)));
        }
        out
    };
    if let Some(at) = spec.at {
        let t = make(at.to_mm(), spec.rotation.unwrap_or(0.0), first);
        let p = problems(&t);
        if !p.is_empty() {
            found.add(
                "watermark",
                "watermark",
                format!(
                    "`{text}` at [{:.2}, {:.2}] on {} {}",
                    t.at[0],
                    t.at[1],
                    t.layer,
                    p.join(", ")
                ),
            );
        }
        return (Some(t), None);
    }
    let pen = crate::font::default_thickness(size);
    let (w, h) = (crate::font::ink_width(&text, size) + pen, size + pen);
    let rotations: Vec<f64> = match spec.rotation {
        Some(r) => vec![r],
        None => vec![0.0, 90.0],
    };
    let mut ob = Bounds::EMPTY;
    board_edge.outline.iter().for_each(|p| ob.add(*p));
    let corner = [ob.min[0], ob.max[1]];
    let round = |v: f64| (v * 100.0).round() / 100.0;
    let mut least: Option<(u32, SilkText)> = None;
    for layer in &layers {
        let occ = watermark_occupancy(
            layer, parts, vias, &texts, &boxes, graphics, artwork, board_edge, margin,
        );
        for &rot in &rotations {
            let (sin, cos) = rot.to_radians().sin_cos();
            let bw = w * cos.abs() + h * sin.abs();
            let bh = w * sin.abs() + h * cos.abs();
            let nw = (bw / WATERMARK_CELL).ceil() as usize + 1;
            let nh = (bh / WATERMARK_CELL).ceil() as usize + 1;
            if nw > occ.w || nh > occ.h {
                continue;
            }
            let at_of = |i: usize, j: usize| {
                [
                    round(occ.origin[0] + (i as f64 + nw as f64 / 2.0) * WATERMARK_CELL),
                    round(occ.origin[1] + (j as f64 + nh as f64 / 2.0) * WATERMARK_CELL),
                ]
            };
            let mut spots: Vec<(u32, f64, P)> = Vec::new();
            for j in 0..=occ.h - nh {
                for i in 0..=occ.w - nw {
                    let at = at_of(i, j);
                    spots.push((occ.count(i, j, nw, nh), geom::dist(at, corner), at));
                }
            }
            spots.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
            for &(n, _, at) in spots.iter().take(256) {
                let t = make(at, rot, layer);
                if problems(&t).is_empty() {
                    return (Some(t), None);
                }
                if least.as_ref().is_none_or(|l| n < l.0) {
                    least = Some((n, t));
                }
            }
        }
    }
    let hint = least
        .map(|(_, t)| {
            let r = if t.rotation != 0.0 { format!(", rotation = {}", t.rotation) } else { String::new() };
            format!(
                "; the least crowded spot is [watermark] at = [{:.2}, {:.2}], layer = \"{}\"{r}, where it {}",
                t.at[0],
                t.at[1],
                t.layer,
                problems(&t).join(", ")
            )
        })
        .unwrap_or_default();
    let msg = format!(
        "no clear spot for the `{text}` watermark ({w:.2} x {h:.2} mm at the {} minimum silk text height) on {}: clear an area that size off pads, vias, silk and part bodies, {} inside the board edge, and set [watermark] at = [x, y] there{hint}",
        Length::mm(size),
        layers.join(" or "),
        Length::mm(margin),
    );
    found.add("watermark", "watermark", &msg);
    (None, Some(msg))
}

impl ZoneFill {
    pub fn filled(&self, p: P) -> bool {
        let x = ((p[0] - self.origin[0]) / self.cell).floor();
        let y = ((p[1] - self.origin[1]) / self.cell).floor();
        if x < 0.0 || y < 0.0 || x as usize >= self.width || y as usize >= self.height {
            return false;
        }
        self.mask[y as usize * self.width + x as usize] != 0
    }
}

fn scan_spans(
    poly: &[P],
    origin: P,
    cell: f64,
    w: usize,
    h: usize,
    mut span: impl FnMut(usize, usize, usize),
) {
    let mut rows: Vec<Vec<f64>> = vec![Vec::new(); h];
    for (a, c) in edges(poly) {
        let lo = (((a[1].min(c[1]) - origin[1]) / cell) - 0.5).ceil().max(0.0) as usize;
        let hi = (((a[1].max(c[1]) - origin[1]) / cell) - 0.5).floor();
        if hi < 0.0 {
            continue;
        }
        for (y, row) in rows.iter_mut().enumerate().take((hi as usize + 1).min(h)).skip(lo) {
            let py = origin[1] + (y as f64 + 0.5) * cell;
            if (a[1] > py) != (c[1] > py) {
                row.push(a[0] + (py - a[1]) / (c[1] - a[1]) * (c[0] - a[0]));
            }
        }
    }
    for (y, xs) in rows.iter_mut().enumerate() {
        xs.sort_by(|a, b| a.total_cmp(b));
        for pair in xs.chunks(2) {
            if pair.len() < 2 {
                continue;
            }
            let x0 = (((pair[0] - origin[0]) / cell) - 0.5).ceil().max(0.0) as usize;
            let x1 = (((pair[1] - origin[0]) / cell) - 0.5).floor().min(w as f64 - 1.0);
            if x1 < 0.0 || x0 > x1 as usize {
                continue;
            }
            span(y, x0, x1 as usize);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_zone(
    net: usize,
    layer: &str,
    poly: &[P],
    board: geom::BoardEdge,
    edge_clear: f64,
    clearance: f64,
    items: &[Item],
    clearance_of: &dyn Fn(Option<usize>) -> f64,
    cutouts: &[&Vec<P>],
    blockers: &[&ZoneFill],
    min_width: f64,
    min_island_area: f64,
) -> (ZoneFill, Vec<Vec<usize>>) {
    let (origin, cell, w, h) = pour::raster_grid(poly);
    let center = |x: usize, y: usize| {
        [origin[0] + (x as f64 + 0.5) * cell, origin[1] + (y as f64 + 0.5) * cell]
    };
    let mut mask = vec![0u8; w * h];
    scan_spans(poly, origin, cell, w, h, |y, x0, x1| mask[y * w + x0..=y * w + x1].fill(1));
    let margin = 1.75 * cell;
    let clear_near = |mask: &mut Vec<u8>, bb: Bounds, grow: f64, test: &dyn Fn(P) -> bool| {
        let grow = grow + margin;
        let x0 = (((bb.min[0] - grow - origin[0]) / cell).floor().max(0.0)) as usize;
        let y0 = (((bb.min[1] - grow - origin[1]) / cell).floor().max(0.0)) as usize;
        let x1 =
            ((((bb.max[0] + grow - origin[0]) / cell).ceil()) as usize).min(w.saturating_sub(1));
        let y1 =
            ((((bb.max[1] + grow - origin[1]) / cell).ceil()) as usize).min(h.saturating_sub(1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                if mask[y * w + x] != 0 && test(center(x, y)) {
                    mask[y * w + x] = 0;
                }
            }
        }
    };
    if board.is_closed() {
        for (a, c) in board.segments() {
            let mut bb = Bounds::EMPTY;
            bb.add(a);
            bb.add(c);
            clear_near(&mut mask, bb, edge_clear, &|p| {
                geom::point_segment_distance(p, a, c) < edge_clear + margin
            });
        }
        let mut on_board = vec![0u8; w * h];
        scan_spans(board.outline, origin, cell, w, h, |y, x0, x1| {
            on_board[y * w + x0..=y * w + x1].fill(1)
        });
        for c in board.cutouts.iter().filter(|c| c.len() >= 3) {
            scan_spans(c, origin, cell, w, h, |y, x0, x1| {
                on_board[y * w + x0..=y * w + x1].fill(0)
            });
        }
        mask.iter_mut().zip(&on_board).for_each(|(m, b)| *m &= b);
    }
    for c in cutouts {
        let mut bb = Bounds::EMPTY;
        c.iter().for_each(|p| bb.add(*p));
        clear_near(&mut mask, bb, 0.0, &|p| {
            geom::point_in_polygon(p, c)
                || edges(c).any(|(a, b)| geom::point_segment_distance(p, a, b) < margin)
        });
    }
    for it in items.iter().filter(|it| it.layers.iter().any(|l| l == layer)) {
        if it.net == Some(net) && it.owner != Owner::Hole {
            continue;
        }
        let gap = clearance.max(clearance_of(it.net)).max(it.pour_gap);
        let shape = &it.shape;
        clear_near(&mut mask, it.bounds, gap, &|p| shape.point_distance(p) < gap + margin);
    }
    for z in blockers {
        let gap = clearance.max(clearance_of(Some(z.net)));
        let mut bb = Bounds::EMPTY;
        z.rings.iter().flatten().for_each(|p| bb.add(*p));
        if bb.is_empty() {
            continue;
        }
        clear_near(&mut mask, bb, 0.0, &|p| z.filled(p));
        for (a, c) in z.rings.iter().flat_map(|r| edges(r)) {
            let mut eb = Bounds::EMPTY;
            eb.add(a);
            eb.add(c);
            clear_near(&mut mask, eb, gap, &|p| {
                geom::point_segment_distance(p, a, c) < gap + margin
            });
        }
    }

    let mut label = vec![0u32; w * h];
    let mut next = 0u32;
    let mut stack = Vec::new();
    for start in 0..w * h {
        if mask[start] == 0 || label[start] != 0 {
            continue;
        }
        next += 1;
        label[start] = next;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let mut visit = |j: usize| {
                if mask[j] != 0 && label[j] == 0 {
                    label[j] = next;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
    }
    let mut island_items: HashMap<u32, Vec<usize>> = HashMap::new();
    for (ii, it) in items.iter().enumerate() {
        if it.net != Some(net) || !it.layers.iter().any(|l| l == layer) {
            continue;
        }
        let bb = it.bounds;
        let x0 = (((bb.min[0] - origin[0]) / cell).floor().max(0.0)) as usize;
        let y0 = (((bb.min[1] - origin[1]) / cell).floor().max(0.0)) as usize;
        let x1 = ((((bb.max[0] - origin[0]) / cell).ceil()) as usize).min(w.saturating_sub(1));
        let y1 = ((((bb.max[1] - origin[1]) / cell).ceil()) as usize).min(h.saturating_sub(1));
        if bb.max[0] < origin[0] || bb.max[1] < origin[1] {
            continue;
        }
        let mut seen = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let l = label[y * w + x];
                if l != 0 && !seen.contains(&l) && it.shape.point_distance(center(x, y)) <= 0.0 {
                    seen.push(l);
                }
            }
        }
        for l in seen {
            island_items.entry(l).or_default().push(ii);
        }
    }
    let mut removed = 0;
    for l in 1..=next {
        if !island_items.contains_key(&l) {
            removed += 1;
        }
    }
    for i in 0..w * h {
        if label[i] != 0 && !island_items.contains_key(&label[i]) {
            mask[i] = 0;
        }
    }
    let touched: Vec<Vec<usize>> = island_items.into_values().collect();
    let mut fill = ZoneFill {
        net,
        layer: layer.to_string(),
        origin,
        cell,
        width: w,
        height: h,
        mask,
        islands_removed: removed,
        min_width,
        rings: Vec::new(),
        triangles: Vec::new(),
    };
    fill.rings = vector_fill(
        &fill,
        poly,
        board,
        edge_clear,
        clearance,
        items,
        clearance_of,
        cutouts,
        blockers,
        min_width,
    );
    let _ = touched;
    pour::snap(&mut fill.rings);
    let (rings, touched, dropped) = keep_connected(&fill.rings, net, layer, items, min_island_area);
    fill.rings = rings;
    fill.islands_removed += dropped;
    rasterize(&mut fill);
    fill.triangles = crate::contour::triangles(&fill.rings);
    (fill, touched)
}

fn keep_connected(
    rings: &[Vec<P>],
    net: usize,
    layer: &str,
    items: &[Item],
    min_island_area: f64,
) -> (Vec<Vec<P>>, Vec<Vec<usize>>, usize) {
    let mut shapes: Vec<Vec<&Vec<P>>> = Vec::new();
    for r in rings {
        if geom::signed_area(r) > 0.0 || shapes.is_empty() {
            shapes.push(vec![r]);
        } else {
            shapes.last_mut().unwrap().push(r);
        }
    }
    let own: Vec<usize> = (0..items.len())
        .filter(|&i| {
            items[i].net == Some(net)
                && !matches!(items[i].owner, Owner::Hole | Owner::PadHole(..))
                && items[i].layers.iter().any(|l| l == layer)
        })
        .collect();
    let (mut kept, mut touched, mut dropped) = (Vec::new(), Vec::new(), 0);
    for shape in shapes {
        let outer = shape[0];
        let b = ring_bounds(std::slice::from_ref(outer));
        let body = index::Winding::new(std::slice::from_ref(outer));
        let rim = EdgeBins::new(std::slice::from_ref(outer), 1.0);
        let holes: Vec<(Bounds, &Vec<P>)> =
            shape[1..].iter().map(|h| (ring_bounds(std::slice::from_ref(*h)), *h)).collect();
        let hits: Vec<usize> = own
            .iter()
            .copied()
            .filter(|&i| {
                let it = &items[i];
                if it.bounds.max[0] < b.min[0]
                    || it.bounds.min[0] > b.max[0]
                    || it.bounds.max[1] < b.min[1]
                    || it.bounds.min[1] > b.max[1]
                {
                    return false;
                }
                let touches = body.inside(it.shape.probe())
                    || rim
                        .near(&it.bounds, 1e-6)
                        .into_iter()
                        .any(|(p, q)| it.shape.distance(&Shape::Seg(p, q, 0.0)) <= 1e-6);
                if !touches {
                    return false;
                }
                let corners = [
                    it.bounds.min,
                    it.bounds.max,
                    [it.bounds.min[0], it.bounds.max[1]],
                    [it.bounds.max[0], it.bounds.min[1]],
                ];
                !holes.iter().any(|(hb, h)| {
                    corners.iter().all(|c| {
                        c[0] >= hb.min[0]
                            && c[0] <= hb.max[0]
                            && c[1] >= hb.min[1]
                            && c[1] <= hb.max[1]
                            && geom::point_in_polygon(*c, h)
                    })
                })
            })
            .collect();
        let area: f64 = shape.iter().map(|r| geom::signed_area(r)).sum();
        if hits.len() >= 2 || (hits.len() == 1 && area >= min_island_area) {
            kept.extend(shape.into_iter().cloned());
            touched.push(hits);
        } else {
            dropped += 1;
        }
    }
    (kept, touched, dropped)
}

fn rasterize(fill: &mut ZoneFill) {
    let (w, h, cell, origin) = (fill.width, fill.height, fill.cell, fill.origin);
    fill.mask.iter_mut().for_each(|m| *m = 0);
    for y in 0..h {
        let py = origin[1] + (y as f64 + 0.5) * cell;
        let mut cross: Vec<(f64, i32)> = Vec::new();
        for r in &fill.rings {
            for (a, c) in edges(r) {
                if (a[1] > py) != (c[1] > py) {
                    let x = a[0] + (py - a[1]) / (c[1] - a[1]) * (c[0] - a[0]);
                    cross.push((x, if c[1] > a[1] { 1 } else { -1 }));
                }
            }
        }
        cross.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut wind = 0;
        for k in 0..cross.len() {
            wind += cross[k].1;
            if wind != 0 && k + 1 < cross.len() {
                let x0 = (((cross[k].0 - origin[0]) / cell) - 0.5).ceil().max(0.0) as usize;
                let x1 = (((cross[k + 1].0 - origin[0]) / cell) - 0.5).floor();
                if x1 < 0.0 {
                    continue;
                }
                for x in x0..=(x1 as usize).min(w - 1) {
                    fill.mask[y * w + x] = 1;
                }
            }
        }
    }
}

fn spans(shape: &[Vec<P>], at: f64, axis: usize) -> Vec<(f64, f64)> {
    let o = 1 - axis;
    let mut cross: Vec<(f64, i32)> = Vec::new();
    for r in shape {
        for (a, c) in edges(r) {
            if (a[o] > at) != (c[o] > at) {
                let v = a[axis] + (at - a[o]) / (c[o] - a[o]) * (c[axis] - a[axis]);
                cross.push((v, if c[o] > a[o] { 1 } else { -1 }));
            }
        }
    }
    cross.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut wind = 0;
    let mut out = Vec::new();
    for w in cross.windows(2) {
        wind += w[0].1;
        if wind != 0 && w[1].0 > w[0].0 {
            out.push((w[0].0, w[1].0));
        }
    }
    out
}

fn interior_point(shape: &[Vec<P>]) -> Option<P> {
    let b = ring_bounds(&shape[..1.min(shape.len())]);
    if b.is_empty() {
        return None;
    }
    let mut best: Option<(f64, P)> = None;
    let lines = 16;
    for k in 0..lines {
        let y = b.min[1] + (b.max[1] - b.min[1]) * (k as f64 + 0.5) / lines as f64;
        let mut row = spans(shape, y, 0);
        row.sort_by(|a, b| (b.1 - b.0).total_cmp(&(a.1 - a.0)));
        for (x0, x1) in row.into_iter().take(4) {
            let x = (x0 + x1) / 2.0;
            let Some((y0, y1)) = spans(shape, x, 1).into_iter().find(|s| s.0 <= y && y <= s.1)
            else {
                continue;
            };
            let p = [x, (y0 + y1) / 2.0];
            let depth = ((x1 - x0) / 2.0).min((y1 - y0) / 2.0);
            if best.is_none_or(|(d, _)| depth > d) {
                best = Some((depth, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn ring_shapes(rings: &[Vec<P>]) -> Vec<Vec<Vec<P>>> {
    let mut shapes: Vec<Vec<Vec<P>>> = Vec::new();
    for r in rings {
        if geom::signed_area(r) > 0.0 || shapes.is_empty() {
            shapes.push(vec![r.clone()]);
        } else if let Some(last) = shapes.last_mut() {
            last.push(r.clone());
        }
    }
    shapes
}

#[allow(clippy::ptr_arg)]
fn grown(shapes: &Vec<Vec<Vec<P>>>, gap: f64) -> Vec<Vec<Vec<P>>> {
    use i_overlay::mesh::float::outline::offset::OutlineOffset;
    use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};
    if shapes.is_empty() || gap <= 0.0 {
        return shapes.clone();
    }
    let step = 2.0 * (1.0 - 0.002f64.min(gap * 0.5) / gap).acos();
    let out = gap / (step / 2.0).cos();
    shapes.outline(&OutlineStyle::new(out).line_join(LineJoin::Round(step)))
}

fn arc_steps(r: f64) -> usize {
    let tol = 0.002f64.min(r * 0.5);
    ((std::f64::consts::PI / (1.0 - tol / r).clamp(-1.0, 1.0).acos()).ceil() as usize)
        .clamp(12, 180)
}

struct ReliefSpec<'a> {
    zone: &'a ZoneFile,
    gap: f64,
    clearance: f64,
    spoke: f64,
}

fn relief_cutouts(
    items: &[Item],
    parts: &[Placed],
    net: usize,
    layer: &str,
    poly: &[P],
    spec: ReliefSpec,
) -> Vec<Vec<P>> {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;
    use i_overlay::mesh::float::outline::offset::OutlineOffset;
    use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};
    let mut zone = Bounds::EMPTY;
    poly.iter().for_each(|p| zone.add(*p));
    let ccw = |mut r: Vec<P>| {
        if geom::signed_area(&r) < 0.0 {
            r.reverse();
        }
        r
    };
    let mut out = Vec::new();
    for it in items {
        let (Owner::Pad(pi, k), Shape::Poly(rings)) = (it.owner, &it.shape) else { continue };
        let part = &parts[pi];
        if it.net != Some(net)
            || !matches!(part.pads[k].kind, PadKind::Smd | PadKind::Tht)
            || !it.layers.iter().any(|l| l == layer)
            || !(zone.overlaps(&it.bounds) || zone.contains(&it.bounds))
        {
            continue;
        }
        let connection = spec.zone.connection_of(part, k);
        let gap = match connection {
            PadConnection::Solid => continue,
            PadConnection::Relief => spec.gap,
            PadConnection::None => spec.clearance,
        };
        if gap <= 0.0 {
            continue;
        }
        let pad: Vec<Vec<P>> = rings.iter().cloned().map(ccw).collect();
        let step = 2.0 * (1.0 - 0.002f64.min(gap * 0.5) / gap).acos();
        let ring = pad.outline(&OutlineStyle::new(gap).line_join(LineJoin::Round(step)));
        if connection == PadConnection::None {
            out.extend(ring.into_iter().flatten().filter(|r| r.len() >= 3).map(ccw));
            continue;
        }
        let pad_rotation = part.footprint.pads.get(k).map(|f| f.rotation).unwrap_or(0.0);
        let u = part.transform().direction(geom::rotate([1.0, 0.0], pad_rotation));
        let v = [-u[1], u[0]];
        let c = it.bounds.center();
        let [w, h] = it.bounds.size();
        let reach = w.max(h) + gap + 1.0;
        let half = spec.spoke / 2.0;
        let bar = |d: P, n: P| {
            ccw(vec![
                [c[0] - d[0] * reach - n[0] * half, c[1] - d[1] * reach - n[1] * half],
                [c[0] + d[0] * reach - n[0] * half, c[1] + d[1] * reach - n[1] * half],
                [c[0] + d[0] * reach + n[0] * half, c[1] + d[1] * reach + n[1] * half],
                [c[0] - d[0] * reach + n[0] * half, c[1] - d[1] * reach + n[1] * half],
            ])
        };
        let mut clip = pad;
        clip.push(bar(u, v));
        clip.push(bar(v, u));
        let pieces = ring.overlay(&clip, OverlayRule::Difference, FillRule::NonZero);
        out.extend(pieces.into_iter().flatten().filter(|r| r.len() >= 3).map(ccw));
    }
    out
}

fn capsule(a: P, b: P, r: f64) -> Vec<P> {
    let n = arc_steps(r);
    let r = r / (std::f64::consts::PI / (n / 2 * 2) as f64).cos();
    let ang = (b[1] - a[1]).atan2(b[0] - a[0]);
    let mut out = Vec::with_capacity(n + 2);
    for (c, start) in
        [(b, ang - std::f64::consts::FRAC_PI_2), (a, ang + std::f64::consts::FRAC_PI_2)]
    {
        for k in 0..=n / 2 {
            let t = start + std::f64::consts::PI * k as f64 / (n / 2) as f64;
            out.push([c[0] + r * t.cos(), c[1] + r * t.sin()]);
        }
    }
    out
}

fn inflated(shape: &Shape, gap: f64) -> Vec<Vec<P>> {
    match shape {
        Shape::Circle(c, r) => {
            let r = r + gap;
            let n = arc_steps(r);
            let r = r / (std::f64::consts::PI / n as f64).cos();
            vec![
                (0..n)
                    .map(|k| {
                        let t = std::f64::consts::TAU * k as f64 / n as f64;
                        [c[0] + r * t.cos(), c[1] + r * t.sin()]
                    })
                    .collect(),
            ]
        }
        Shape::Seg(a, b, hw) => vec![capsule(*a, *b, hw + gap)],
        Shape::Poly(rings) => {
            let mut out = Vec::new();
            for ring in rings {
                out.push(ring.clone());
                if gap > 0.0 {
                    for (a, b) in edges(ring) {
                        out.push(capsule(a, b, gap));
                    }
                }
            }
            out
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_clip(
    layer: &str,
    net: usize,
    poly: &[P],
    board: geom::BoardEdge,
    edge_clear: f64,
    clearance: f64,
    items: &[Item],
    clearance_of: &dyn Fn(Option<usize>) -> f64,
    cutouts: &[&Vec<P>],
    blockers: &[&ZoneFill],
    min_width: f64,
) -> (Vec<Vec<P>>, Vec<Vec<P>>) {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;
    let subject: Vec<Vec<P>> = if board.is_closed() && poly.len() >= 3 {
        poly.to_vec()
            .overlay(&board.outline.to_vec(), OverlayRule::Intersect, FillRule::NonZero)
            .into_iter()
            .flatten()
            .collect()
    } else {
        vec![poly.to_vec()]
    };
    let reach = ring_bounds(&subject);
    let near = |b: Bounds, grow: f64| {
        b.max[0] + grow + min_width >= reach.min[0]
            && b.min[0] - grow - min_width <= reach.max[0]
            && b.max[1] + grow + min_width >= reach.min[1]
            && b.min[1] - grow - min_width <= reach.max[1]
    };
    let edge_near = |a: P, b: P, grow: f64| {
        let mut bb = Bounds::EMPTY;
        bb.add(a);
        bb.add(b);
        near(bb, grow)
    };
    let mut clip: Vec<Vec<P>> = Vec::new();
    if board.is_closed() && edge_clear > 0.0 {
        for (a, b) in board.segments().filter(|(a, b)| edge_near(*a, *b, edge_clear)) {
            clip.push(capsule(a, b, edge_clear + pour::SNAP_MARGIN));
        }
    }
    if board.is_closed() {
        for c in board.cutouts.iter().filter(|c| c.len() >= 3) {
            if near(ring_bounds(std::slice::from_ref(c)), 0.0) {
                clip.push(c.to_vec());
            }
        }
    }
    for c in cutouts.iter().filter(|c| near(ring_bounds(std::slice::from_ref(*c)), 0.0)) {
        clip.push(c.to_vec());
    }
    let keepouts: Vec<(&Shape, f64)> = items
        .iter()
        .filter(|it| it.layers.iter().any(|l| l == layer))
        .filter(|it| it.net != Some(net) || it.owner == Owner::Hole)
        .map(|it| (it, clearance.max(clearance_of(it.net)).max(it.pour_gap)))
        .filter(|(it, gap)| near(it.bounds, *gap))
        .map(|(it, gap)| (&it.shape, gap))
        .collect();
    for (shape, gap) in &keepouts {
        clip.extend(inflated(shape, *gap + pour::SNAP_MARGIN));
    }
    if min_width > 0.0 {
        clip.extend(gap_bridges(&keepouts, min_width));
        let mut walls: Vec<Wall> = Vec::new();
        if board.is_closed() && edge_clear > 0.0 {
            for (k, ring) in board.rings().enumerate() {
                let group = if k == 0 { 0 } else { cutouts.len() + k };
                walls.extend(
                    edges(ring).filter(|(a, b)| edge_near(*a, *b, edge_clear)).map(|(a, b)| Wall {
                        a,
                        b,
                        gap: edge_clear,
                        group,
                    }),
                );
            }
        }
        for (k, c) in cutouts.iter().enumerate() {
            walls.extend(edges(c).filter(|(a, b)| edge_near(*a, *b, 0.0)).map(|(a, b)| Wall {
                a,
                b,
                gap: 0.0,
                group: 1 + k,
            }));
        }
        for z in blockers {
            let gap = clearance.max(clearance_of(Some(z.net)));
            for r in &z.rings {
                let group = 1 + cutouts.len() + board.cutouts.len() + walls.len();
                walls.extend(edges(r).filter(|(a, b)| edge_near(*a, *b, gap)).map(|(a, b)| Wall {
                    a,
                    b,
                    gap,
                    group,
                }));
            }
        }
        clip.extend(wall_bridges(&keepouts, &walls, min_width));
    }
    for z in blockers {
        let gap = clearance.max(clearance_of(Some(z.net)));
        let shapes: Vec<Vec<Vec<P>>> =
            ring_shapes(&z.rings).into_iter().filter(|s| near(ring_bounds(s), gap)).collect();
        clip.extend(grown(&shapes, gap + pour::SNAP_MARGIN).into_iter().flatten());
    }
    (subject, clip)
}

#[allow(clippy::too_many_arguments)]
fn vector_fill(
    raster: &ZoneFill,
    poly: &[P],
    board: geom::BoardEdge,
    edge_clear: f64,
    clearance: f64,
    items: &[Item],
    clearance_of: &dyn Fn(Option<usize>) -> f64,
    cutouts: &[&Vec<P>],
    blockers: &[&ZoneFill],
    min_width: f64,
) -> Vec<Vec<P>> {
    let (subject, clip) = fill_clip(
        &raster.layer,
        raster.net,
        poly,
        board,
        edge_clear,
        clearance,
        items,
        clearance_of,
        cutouts,
        blockers,
        min_width,
    );
    let shapes = clip_overlay(&subject, &clip);
    let shapes = open_to_width(shapes, min_width);
    probe_shapes(shapes, raster)
}

#[allow(clippy::ptr_arg)]
fn clip_overlay(subject: &Vec<Vec<P>>, clip: &Vec<Vec<P>>) -> Vec<Vec<Vec<P>>> {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;
    subject.overlay(clip, OverlayRule::Difference, FillRule::NonZero)
}

fn open_to_width(shapes: Vec<Vec<Vec<P>>>, min_width: f64) -> Vec<Vec<Vec<P>>> {
    if min_width <= 0.0 {
        return shapes;
    }
    use i_overlay::mesh::float::outline::offset::OutlineOffset;
    use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};
    let r = min_width / 2.0;
    let step = 2.0 * (1.0 - 0.002f64.min(r * 0.5) / r).acos();
    let eroded = shapes.outline(&OutlineStyle::new(-r).line_join(LineJoin::Round(step)));
    eroded.outline(&OutlineStyle::new(r).line_join(LineJoin::Round(step)))
}

fn probe_shapes(shapes: Vec<Vec<Vec<P>>>, raster: &ZoneFill) -> Vec<Vec<P>> {
    let area_of = |r: &[P]| crate::contour::area(r);
    let mut rings = Vec::new();
    for shape in shapes {
        if !interior_point(&shape).is_some_and(|p| raster.filled(p)) {
            continue;
        }
        for (k, mut r) in shape.into_iter().enumerate() {
            let outer = k == 0;
            if (area_of(&r) > 0.0) != outer {
                r.reverse();
            }
            rings.push(r);
        }
    }
    rings
}

fn closest_on_segment(p: P, a: P, b: P) -> P {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return a;
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0);
    [a[0] + t * dx, a[1] + t * dy]
}

fn core_of(shape: &Shape) -> (Vec<(P, P)>, f64) {
    match shape {
        Shape::Circle(c, r) => (vec![(*c, *c)], *r),
        Shape::Seg(a, b, hw) => (vec![(*a, *b)], *hw),
        Shape::Poly(rings) => (rings.iter().flat_map(|r| edges(r)).collect(), 0.0),
    }
}

fn nearest_points(a: &[(P, P)], b: &[(P, P)]) -> Option<(P, P)> {
    let mut best: Option<(f64, P, P)> = None;
    for (p, q) in a {
        for (r, s) in b {
            if geom::segments_intersect(*p, *q, *r, *s) {
                return None;
            }
            for (x, y) in [
                (*p, closest_on_segment(*p, *r, *s)),
                (*q, closest_on_segment(*q, *r, *s)),
                (closest_on_segment(*r, *p, *q), *r),
                (closest_on_segment(*s, *p, *q), *s),
            ] {
                let d = geom::dist(x, y);
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, x, y));
                }
            }
        }
    }
    best.map(|(_, x, y)| (x, y))
}

fn cap_points(rings: &[Vec<P>], m: P, rho: f64, out: &mut Vec<P>) {
    for r in rings {
        for (a, b) in edges(r) {
            if geom::dist(a, m) <= rho {
                out.push(a);
            }
            let d = [b[0] - a[0], b[1] - a[1]];
            let f = [a[0] - m[0], a[1] - m[1]];
            let qa = d[0] * d[0] + d[1] * d[1];
            let qb = 2.0 * (f[0] * d[0] + f[1] * d[1]);
            let qc = f[0] * f[0] + f[1] * f[1] - rho * rho;
            let disc = qb * qb - 4.0 * qa * qc;
            if qa == 0.0 || disc < 0.0 {
                continue;
            }
            for t in [(-qb - disc.sqrt()) / (2.0 * qa), (-qb + disc.sqrt()) / (2.0 * qa)] {
                if (0.0..=1.0).contains(&t) {
                    out.push([a[0] + t * d[0], a[1] + t * d[1]]);
                }
            }
        }
    }
}

fn convex_hull(mut pts: Vec<P>) -> Vec<P> {
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup_by(|a, b| geom::dist(*a, *b) < 1e-9);
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: P, a: P, b: P| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut hull: Vec<P> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &P>> =
            if pass == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
        for p in iter {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], *p) <= 0.0
            {
                hull.pop();
            }
            hull.push(*p);
        }
        hull.pop();
    }
    hull
}

fn gap_bridges(keepouts: &[(&Shape, f64)], min_width: f64) -> Vec<Vec<P>> {
    let mut order: Vec<(Bounds, usize)> = keepouts
        .iter()
        .enumerate()
        .map(|(i, (s, gap))| {
            let mut b = s.bounds();
            b.add([b.min[0] - gap, b.min[1] - gap]);
            b.add([b.max[0] + gap, b.max[1] + gap]);
            (b, i)
        })
        .collect();
    order.sort_by(|a, b| a.0.min[0].total_cmp(&b.0.min[0]));
    let mut out = Vec::new();
    for (k, (ba, i)) in order.iter().enumerate() {
        for (bb, j) in &order[k + 1..] {
            if bb.min[0] > ba.max[0] + min_width {
                break;
            }
            if bb.min[1] > ba.max[1] + min_width || bb.max[1] < ba.min[1] - min_width {
                continue;
            }
            let ((sa, ga), (sb, gb)) = (keepouts[*i], keepouts[*j]);
            let apart = sa.distance(sb) - ga - gb;
            if apart <= 1e-6 || apart >= min_width {
                continue;
            }
            let ((ca, ra), (cb, rb)) = (core_of(sa), core_of(sb));
            let Some((qa, qb)) = nearest_points(&ca, &cb) else { continue };
            let core = geom::dist(qa, qb);
            if core < 1e-9 {
                continue;
            }
            let dir = [(qb[0] - qa[0]) / core, (qb[1] - qa[1]) / core];
            let (ea, eb) = (ra + ga, rb + gb);
            let m = [
                (qa[0] + dir[0] * ea + qb[0] - dir[0] * eb) / 2.0,
                (qa[1] + dir[1] * ea + qb[1] - dir[1] * eb) / 2.0,
            ];
            let rho = apart / 2.0 + 1.5 * min_width;
            let mut pts = Vec::new();
            cap_points(&inflated(sa, ga), m, rho, &mut pts);
            cap_points(&inflated(sb, gb), m, rho, &mut pts);
            let hull = convex_hull(pts);
            if hull.len() >= 3 {
                out.push(hull);
            }
        }
    }
    out
}

struct Wall {
    a: P,
    b: P,
    gap: f64,
    group: usize,
}

fn wall_bridges(keepouts: &[(&Shape, f64)], walls: &[Wall], min_width: f64) -> Vec<Vec<P>> {
    let cell = 1.0;
    let key = |p: P| ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64);
    let mut bins: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (k, w) in walls.iter().enumerate() {
        let (lo, hi) = (
            key([w.a[0].min(w.b[0]) - w.gap, w.a[1].min(w.b[1]) - w.gap]),
            key([w.a[0].max(w.b[0]) + w.gap, w.a[1].max(w.b[1]) + w.gap]),
        );
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                bins.entry((x, y)).or_default().push(k);
            }
        }
    }
    let side = |w: &Wall| {
        if w.gap > 0.0 { vec![capsule(w.a, w.b, w.gap)] } else { vec![vec![w.a, w.b]] }
    };
    let mut out = Vec::new();
    let mut near: Vec<usize> = Vec::new();
    for (shape, gap) in keepouts {
        let b = shape.bounds();
        let reach = gap + min_width;
        let (lo, hi) =
            (key([b.min[0] - reach, b.min[1] - reach]), key([b.max[0] + reach, b.max[1] + reach]));
        near.clear();
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                near.extend(bins.get(&(x, y)).into_iter().flatten());
            }
        }
        near.sort_unstable_by_key(|&k| (walls[k].group, k));
        near.dedup();
        for group in near.chunk_by(|&x, &y| walls[x].group == walls[y].group) {
            let mut best: Option<(f64, usize)> = None;
            for &k in group {
                let w = &walls[k];
                let apart = shape.distance(&Shape::Seg(w.a, w.b, 0.0)) - gap - w.gap;
                if best.is_none_or(|x| apart < x.0) {
                    best = Some((apart, k));
                }
            }
            let Some((apart, k)) = best else { continue };
            if apart <= 1e-6 || apart >= min_width {
                continue;
            }
            let w = &walls[k];
            let (ca, ra) = core_of(shape);
            let Some((qa, qb)) = nearest_points(&ca, &[(w.a, w.b)]) else { continue };
            let core = geom::dist(qa, qb);
            if core < 1e-9 {
                continue;
            }
            let dir = [(qb[0] - qa[0]) / core, (qb[1] - qa[1]) / core];
            let (ea, eb) = (ra + gap, w.gap);
            let m = [
                (qa[0] + dir[0] * ea + qb[0] - dir[0] * eb) / 2.0,
                (qa[1] + dir[1] * ea + qb[1] - dir[1] * eb) / 2.0,
            ];
            let rho = apart / 2.0 + 1.5 * min_width;
            let mut pts = Vec::new();
            cap_points(&inflated(shape, *gap), m, rho, &mut pts);
            for &j in group {
                cap_points(&side(&walls[j]), m, rho, &mut pts);
            }
            let hull = convex_hull(pts);
            if hull.len() >= 3 {
                out.push(hull);
            }
        }
    }
    out
}

fn pair_base(name: &str, positive: bool) -> Option<String> {
    let upper = name.to_uppercase();
    for (p, n) in [("_DP", "_DN"), ("_P", "_N"), ("+", "-"), ("P", "N")] {
        let suffix = if positive { p } else { n };
        if upper.ends_with(suffix) && name.len() > suffix.len() {
            return Some(format!("{}|{p}", &name[..name.len() - suffix.len()]));
        }
    }
    None
}

pub(crate) fn parallel_overlap(a0: P, a1: P, b0: P, b1: P) -> Option<(f64, f64)> {
    let da = [a1[0] - a0[0], a1[1] - a0[1]];
    let la = (da[0] * da[0] + da[1] * da[1]).sqrt();
    let db = [b1[0] - b0[0], b1[1] - b0[1]];
    let lb = (db[0] * db[0] + db[1] * db[1]).sqrt();
    if la < 1e-9 || lb < 1e-9 {
        return None;
    }
    let u = [da[0] / la, da[1] / la];
    let cross = (u[0] * db[1] - u[1] * db[0]).abs() / lb;
    if cross > 0.02 {
        return None;
    }
    let proj = |p: P| (p[0] - a0[0]) * u[0] + (p[1] - a0[1]) * u[1];
    let (s0, s1) = (proj(b0).min(proj(b1)), proj(b0).max(proj(b1)));
    let overlap = s1.min(la) - s0.max(0.0);
    if overlap <= 0.0 {
        return None;
    }
    let mid = [b0[0] - a0[0], b0[1] - a0[1]];
    Some((overlap, (u[0] * mid[1] - u[1] * mid[0]).abs()))
}

fn ring_bounds(rings: &[Vec<P>]) -> Bounds {
    let mut b = Bounds::EMPTY;
    rings.iter().flatten().for_each(|p| b.add(*p));
    b
}

struct EdgeBins {
    size: f64,
    bins: HashMap<(i64, i64), Vec<(P, P)>>,
}

impl EdgeBins {
    fn new(rings: &[Vec<P>], size: f64) -> EdgeBins {
        let mut bins: HashMap<(i64, i64), Vec<(P, P)>> = HashMap::new();
        for r in rings {
            for (a, b) in edges(r) {
                let (x0, x1) = (
                    (a[0].min(b[0]) / size).floor() as i64,
                    (a[0].max(b[0]) / size).floor() as i64,
                );
                let (y0, y1) = (
                    (a[1].min(b[1]) / size).floor() as i64,
                    (a[1].max(b[1]) / size).floor() as i64,
                );
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        bins.entry((x, y)).or_default().push((a, b));
                    }
                }
            }
        }
        EdgeBins { size, bins }
    }

    fn near(&self, b: &Bounds, grow: f64) -> Vec<(P, P)> {
        let (x0, x1) = (
            ((b.min[0] - grow) / self.size).floor() as i64,
            ((b.max[0] + grow) / self.size).floor() as i64,
        );
        let (y0, y1) = (
            ((b.min[1] - grow) / self.size).floor() as i64,
            ((b.max[1] + grow) / self.size).floor() as i64,
        );
        let mut out = Vec::new();
        for x in x0..=x1 {
            for y in y0..=y1 {
                if let Some(v) = self.bins.get(&(x, y)) {
                    out.extend_from_slice(v);
                }
            }
        }
        out
    }
}

fn check_zones(
    zones: &[ZoneFill],
    items: &[Item],
    nets: &[LayoutNet],
    clearance_of: &dyn Fn(Option<usize>) -> f64,
    rule_on: &dyn Fn(&str) -> bool,
    d: &mut crate::drc::Findings,
) {
    const TOL: f64 = 3e-3;
    let (tips_on, overlap_on, gap_on, clear_on) = (
        rule_on("zone-tips"),
        rule_on("zone-overlap"),
        rule_on("zone-to-zone"),
        rule_on("zone-clearance"),
    );
    if !(tips_on || overlap_on || gap_on || clear_on) {
        return;
    }
    let bins: Vec<EdgeBins> = zones.iter().map(|z| EdgeBins::new(&z.rings, 1.0)).collect();
    let bounds: Vec<Bounds> = zones.iter().map(|z| ring_bounds(&z.rings)).collect();
    let inside: Vec<index::Winding> = zones.iter().map(|z| index::Winding::new(&z.rings)).collect();
    let at = |z: &ZoneFill| format!("zone {} on {}", nets[z.net].name, z.layer);
    for (i, a) in zones.iter().enumerate() {
        if a.rings.is_empty() {
            continue;
        }
        let mut tips = Vec::new();
        for r in a.rings.iter().filter(|_| tips_on) {
            let n = r.len();
            for k in 0..n {
                let (p, q, s) = (r[(k + n - 1) % n], r[k], r[(k + 1) % n]);
                let (Some(u), Some(v)) = (unit(p, q), unit(q, s)) else { continue };
                let cross = u[0] * v[1] - u[1] * v[0];
                let turn = (u[0] * v[0] + u[1] * v[1]).clamp(-1.0, 1.0).acos().to_degrees();
                if cross > 0.0 && 180.0 - turn < 30.0 {
                    tips.push(q);
                }
            }
        }
        if let Some(p) = tips.first() {
            d.add(
                "zone-tips",
                at(a),
                format!(
                    "{} sharp copper tips under 30 degrees, first at [{:.3}, {:.3}]; they etch unevenly and can lift, raise the zone's min_width",
                    tips.len(),
                    p[0],
                    p[1]
                ),
            );
        }
        for (j, b) in zones.iter().enumerate().skip(i + 1) {
            if !(overlap_on || gap_on) || a.layer != b.layer || a.net == b.net || b.rings.is_empty()
            {
                continue;
            }
            let need = clearance_of(Some(a.net)).max(clearance_of(Some(b.net)));
            let (ba, bb) = (&bounds[i], &bounds[j]);
            if ba.max[0] + need < bb.min[0]
                || bb.max[0] + need < ba.min[0]
                || ba.max[1] + need < bb.min[1]
                || bb.max[1] + need < ba.min[1]
            {
                continue;
            }
            let over = a
                .rings
                .iter()
                .flatten()
                .find(|p| inside[j].inside(**p))
                .or_else(|| b.rings.iter().flatten().find(|p| inside[i].inside(**p)));
            if let Some(p) = over {
                d.add(
                    "zone-overlap",
                    at(a),
                    format!(
                        "overlaps the {} zone at [{:.3}, {:.3}], a short",
                        nets[b.net].name, p[0], p[1]
                    ),
                );
                continue;
            }
            let mut worst: Option<(f64, P)> = None;
            for r in a.rings.iter().filter(|_| gap_on) {
                for (p, q) in edges(r) {
                    let mut eb = Bounds::EMPTY;
                    eb.add(p);
                    eb.add(q);
                    for (s, t) in bins[j].near(&eb, need) {
                        let gap = geom::segment_segment_distance(p, q, s, t);
                        if gap < need - TOL && worst.is_none_or(|w| gap < w.0) {
                            worst = Some((gap, p));
                        }
                    }
                }
            }
            if let Some((gap, p)) = worst {
                d.add(
                    "zone-to-zone",
                    at(a),
                    format!(
                        "comes {gap:.3} mm from the {} zone at [{:.3}, {:.3}], needs {need} mm",
                        nets[b.net].name, p[0], p[1]
                    ),
                );
            }
        }
        let mut hits = 0;
        let mut first: Option<(String, f64, P)> = None;
        for it in items.iter().filter(|it| {
            clear_on
                && it.net != Some(a.net)
                && it.owner != Owner::Hole
                && it.layers.contains(&a.layer)
        }) {
            let need = clearance_of(Some(a.net)).max(clearance_of(it.net)).max(it.pour_gap);
            let mut hit: Option<(f64, P)> = None;
            let probe = match &it.shape {
                Shape::Seg(p, q, _) => [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0],
                Shape::Circle(c, _) => *c,
                Shape::Poly(v) => v.first().and_then(|r| r.first()).copied().unwrap_or([0.0, 0.0]),
            };
            if inside[i].inside(probe) {
                hit = Some((-1.0, probe));
            } else {
                for (p, q) in bins[i].near(&it.bounds, need) {
                    let gap = it.shape.distance(&Shape::Seg(p, q, 0.0));
                    if gap < need - TOL && hit.is_none_or(|h| gap < h.0) {
                        hit = Some((gap, p));
                    }
                }
            }
            if let Some((gap, p)) = hit {
                hits += 1;
                if first.is_none() {
                    let name = it
                        .net
                        .map(|n| nets[n].name.clone())
                        .unwrap_or_else(|| "unconnected".into());
                    first = Some((name, gap, p));
                }
            }
        }
        if let Some((name, gap, p)) = first {
            let what =
                if gap < 0.0 { "covers".to_string() } else { format!("comes {gap:.3} mm from") };
            d.add(
                "zone-clearance",
                at(a),
                format!("{what} {name} copper at [{:.3}, {:.3}] ({hits} places), the fill must clear other nets", p[0], p[1]),
            );
        }
    }
}

pub(crate) fn unit(a: P, b: P) -> Option<P> {
    let l = geom::dist(a, b);
    (l > 1e-9).then(|| [(b[0] - a[0]) / l, (b[1] - a[1]) / l])
}

pub fn glob(pat: &str, s: &str) -> bool {
    fn go(p: &[char], s: &[char]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], s) || (!s.is_empty() && go(p, &s[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &s[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &s[1..]),
            _ => false,
        }
    }
    go(&pat.chars().collect::<Vec<_>>(), &s.chars().collect::<Vec<_>>())
}

pub fn delay_per_mm(board: &Board, layer: &str, width: f64) -> f64 {
    let eeff = board.stackup.geometry(layer).map(|g| g.eeff(width)).unwrap_or(1.0);
    eeff.sqrt() / 0.299_792_458
}

pub fn serpentine(a: P, b: P, add: f64, amplitude: f64, pitch: f64) -> Result<Vec<P>, String> {
    let l = geom::dist(a, b);
    if add <= 0.0 {
        return Ok(vec![a, b]);
    }
    let bumps = (add / (2.0 * amplitude)).ceil().max(1.0);
    let height = add / (2.0 * bumps);
    let run = 2.0 * pitch * bumps;
    if run > l {
        return Err(format!(
            "{bumps} bumps at a {pitch} mm pitch need {run:.3} mm of straight track, the segment is {l:.3} mm"
        ));
    }
    let u = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
    let n = [-u[1], u[0]];
    let at = |s: f64, h: f64| [a[0] + u[0] * s + n[0] * h, a[1] + u[1] * s + n[1] * h];
    let mut s = (l - run) / 2.0;
    let mut out = vec![a, at(s, 0.0)];
    for _ in 0..bumps as usize {
        out.push(at(s, height));
        s += pitch;
        out.push(at(s, height));
        out.push(at(s, 0.0));
        s += pitch;
        out.push(at(s, 0.0));
    }
    out.push(b);
    out.dedup_by(|x, y| geom::dist(*x, *y) < 1e-9);
    Ok(out)
}

#[cfg(test)]
mod pair_tests {
    use super::*;

    #[test]
    fn pair_names_meet_their_partner() {
        for (p, n) in [("USB_DP", "USB_DN"), ("CLK_P", "CLK_N"), ("RX+", "RX-"), ("TXP", "TXN")] {
            assert_eq!(pair_base(p, true), pair_base(n, false), "{p} {n}");
        }
        assert_ne!(pair_base("USB_DP", true), pair_base("CLK_N", false));
    }

    fn via(net: usize, c: P) -> Item {
        let shape = Shape::Circle(c, 0.175);
        Item {
            owner: Owner::Via(0),
            net: Some(net),
            layers: vec!["In6.Cu".into()],
            bounds: shape.bounds(),
            shape,
            pour_gap: 0.0,
        }
    }

    #[test]
    fn a_pour_leaves_no_stubs_between_antipads_closer_than_min_width() {
        let items = [via(1, [-0.375, -0.075]), via(1, [0.375, 0.075]), via(0, [1.5, 1.5])];
        let square = [[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]];
        let (fill, _) = fill_zone(
            0,
            "In6.Cu",
            &square,
            geom::BoardEdge::default(),
            0.0,
            0.1,
            &items,
            &|_| 0.1,
            &[],
            &[],
            0.25,
            0.0,
        );
        let copper =
            |p: P| fill.rings.iter().filter(|r| geom::point_in_polygon(p, r)).count() % 2 == 1;
        let across = [-0.196, 0.981];
        for k in 0..=26 {
            let s = k as f64 * 0.01;
            for side in [1.0, -1.0] {
                let p = [across[0] * s * side, across[1] * s * side];
                assert!(!copper(p), "copper at {p:?}");
            }
        }
        assert!(copper([across[0] * 0.6, across[1] * 0.6]));
        assert!(copper([0.0, 1.0]) && copper([-1.0, 0.0]));
    }

    fn gap_is_cut(fill: &ZoneFill, y: f64) {
        let copper =
            |p: P| fill.rings.iter().filter(|r| geom::point_in_polygon(p, r)).count() % 2 == 1;
        for k in -30..=30 {
            let p = [k as f64 * 0.01, y];
            assert!(!copper(p), "copper at {p:?}");
        }
        assert!(copper([-1.0, 1.0]) && copper([1.0, 0.0]));
    }

    #[test]
    fn a_pour_leaves_no_stubs_between_an_antipad_and_the_board_edge() {
        let items = [via(1, [0.0, 1.3]), via(0, [-1.5, -1.5])];
        let square = [[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]];
        let (fill, _) = fill_zone(
            0,
            "In6.Cu",
            &square,
            geom::BoardEdge::new(&square, &[]),
            0.2,
            0.1,
            &items,
            &|_| 0.1,
            &[],
            &[],
            0.25,
            0.0,
        );
        gap_is_cut(&fill, (1.575 + 1.8) / 2.0);
    }

    #[test]
    fn a_pour_leaves_no_stubs_between_an_antipad_and_a_cutout() {
        let items = [via(1, [0.0, 1.3]), via(0, [-1.5, -1.5])];
        let square = [[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]];
        let cut = vec![[-3.0, 1.8], [3.0, 1.8], [3.0, 3.0], [-3.0, 3.0]];
        let (fill, _) = fill_zone(
            0,
            "In6.Cu",
            &square,
            geom::BoardEdge::default(),
            0.0,
            0.1,
            &items,
            &|_| 0.1,
            &[&cut],
            &[],
            0.25,
            0.0,
        );
        gap_is_cut(&fill, (1.575 + 1.8) / 2.0);
    }

    #[test]
    fn a_pour_clears_a_board_cutout_by_the_edge_clearance() {
        let items = [via(0, [-1.5, -1.5])];
        let square = vec![[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]];
        let slot = vec![vec![[-0.5, -0.25], [0.5, -0.25], [0.5, 0.25], [-0.5, 0.25]]];
        let (fill, _) = fill_zone(
            0,
            "In6.Cu",
            &square,
            geom::BoardEdge::new(&square, &slot),
            0.3,
            0.1,
            &items,
            &|_| 0.1,
            &[],
            &[],
            0.25,
            0.0,
        );
        let copper =
            |p: P| fill.rings.iter().filter(|r| geom::point_in_polygon(p, r)).count() % 2 == 1;
        for p in [[0.0, 0.0], [0.0, 0.5], [0.75, 0.0], [-0.75, 0.0], [0.0, -0.5]] {
            assert!(!copper(p), "copper at {p:?}");
        }
        for p in [[0.0, 0.6], [0.85, 0.0], [0.0, -1.0]] {
            assert!(copper(p), "no copper at {p:?}");
        }
        let edge = geom::BoardEdge::new(&square, &slot);
        let closest =
            fill.rings.iter().flatten().map(|p| edge.distance(*p)).fold(f64::MAX, f64::min);
        assert!(closest > 0.3 - 1e-6, "fill {closest} from the edge");
    }

    #[test]
    fn a_pour_leaves_no_stubs_between_an_antipad_and_another_fill() {
        let items = [via(1, [0.0, 1.3]), via(0, [-1.5, -1.5])];
        let square = [[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]];
        let other = ZoneFill {
            net: 2,
            layer: "In6.Cu".into(),
            origin: [0.0, 0.0],
            cell: 1.0,
            width: 0,
            height: 0,
            mask: Vec::new(),
            islands_removed: 0,
            min_width: 0.25,
            rings: vec![vec![[-2.0, 1.8], [2.0, 1.8], [2.0, 2.0], [-2.0, 2.0]]],
            triangles: Vec::new(),
        };
        let (fill, _) = fill_zone(
            0,
            "In6.Cu",
            &square,
            geom::BoardEdge::default(),
            0.0,
            0.1,
            &items,
            &|_| 0.1,
            &[],
            &[&other],
            0.25,
            0.0,
        );
        gap_is_cut(&fill, (1.575 + 1.7) / 2.0);
    }

    #[test]
    fn a_serpentine_adds_exactly_what_was_asked() {
        let pts = serpentine([0.0, 0.0], [10.0, 0.0], 2.5, 0.6, 0.4).unwrap();
        let len: f64 = pts.windows(2).map(|w| geom::dist(w[0], w[1])).sum();
        assert!((len - 12.5).abs() < 1e-9, "{len}");
        assert!(serpentine([0.0, 0.0], [1.0, 0.0], 5.0, 0.3, 0.4).is_err());
        assert!(glob("DQ?", "DQ7") && glob("DQ*", "DQ15") && !glob("DQ?", "DQS0"));
    }
}
