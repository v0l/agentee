use crate::board::{Board, Netclass};
use crate::diag::Diags;
use crate::footprint::{Footprint, PadKind};
use crate::geom::{self, P, Transform};
use crate::graphic::Bounds;
use crate::schematic::{Schematic, UnionFind};
use crate::units::{Length, Point};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

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
    pub coupled_mm: f64,
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
    pub net: usize,
    pub layer: String,
    pub width: f64,
    pub points: Vec<P>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Via {
    pub net: usize,
    pub at: P,
    pub drill: f64,
    pub diameter: f64,
    pub layers: Vec<String>,
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
}

#[derive(Clone, Debug, Serialize)]
pub struct SilkBox {
    pub text: String,
    pub layer: String,
    pub outline: Vec<P>,
}

impl Layout {
    pub fn board_texts(&self) -> Vec<SilkText> {
        board_texts(&self.graphics)
    }

    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        self.outline.iter().for_each(|p| b.add(*p));
        for p in &self.parts {
            p.pads.iter().flat_map(|q| q.outlines.iter().flatten()).for_each(|q| b.add(*q));
        }
        b
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Owner {
    Pad(usize, usize),
    Seg(usize),
    Via(usize),
    Hole,
}

struct Item {
    owner: Owner,
    net: Option<usize>,
    layers: Vec<String>,
    shape: Shape,
    bounds: Bounds,
}

pub struct Context<'a> {
    pub dir: std::path::PathBuf,
    pub board: &'a Board,
    pub schematic: &'a Schematic,
    pub footprints: HashMap<&'a str, &'a Footprint>,
}

fn class_of<'a>(board: &'a Board, name: &str) -> Option<&'a Netclass> {
    board
        .netclasses
        .iter()
        .find(|n| n.name == name)
        .or_else(|| board.netclasses.iter().find(|n| n.name == "Default"))
}

fn outline_of(board: &Board) -> Vec<P> {
    use crate::board::Outline;
    match &board.outline {
        Some(Outline::Rect { origin, size, corner_radius }) => {
            let [w, h] = size.to_mm();
            let [x0, y0] = origin.to_mm();
            geom::rounded_rect(w, h, corner_radius.to_mm(), 8)
                .into_iter()
                .map(|q| [q[0] + x0 + w / 2.0, q[1] + y0 + h / 2.0])
                .collect()
        }
        Some(Outline::Polygon { points }) => points.iter().map(|p| p.to_mm()).collect(),
        None => Vec::new(),
    }
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
                            (t.apply(pad.at.to_mm()), s, pad.rotation + rotation)
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
            let class_w = nets[net].width;
            let width = t.width.map(Length::to_mm).unwrap_or(class_w);
            if width + 1e-6 < class_w {
                d.error(
                    &at,
                    format!(
                        "{} is narrower than the {} class width {}",
                        Length::mm(width),
                        nets[net].class,
                        Length::mm(class_w)
                    ),
                );
            }
            if let Some(c) = class_of(board, &nets[net].class)
                && c.impedance.is_some()
                && (width - class_w).abs() > 1e-3
            {
                d.warn(
                    &at,
                    format!(
                        "{} differs from the {} class width {}, its impedance moves",
                        Length::mm(width),
                        c.name,
                        Length::mm(class_w)
                    ),
                );
            }
            tracks.push(Track {
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
            let kind = v
                .via
                .clone()
                .or_else(|| class_of(board, &nets[net].class).and_then(|c| c.via.clone()));
            let Some(spec) = kind
                .as_ref()
                .and_then(|k| board.vias.iter().find(|x| &x.name == k))
                .or(board.vias.first())
            else {
                d.error(&at, "the board defines no [[vias]]");
                continue;
            };
            let (a, b) = (
                copper.iter().position(|c| *c == spec.from).unwrap_or(0),
                copper.iter().position(|c| *c == spec.to).unwrap_or(copper.len().saturating_sub(1)),
            );
            let count = v.count.unwrap_or(1).max(1);
            if count > 1 && v.pitch.is_none() {
                d.error(&at, "`count` needs a `pitch`");
            }
            let pitch = v.pitch.unwrap_or(Point::ZERO).to_mm();
            for k in 0..count {
                let [x, y] = v.at.to_mm();
                vias.push(Via {
                    net,
                    at: [x + pitch[0] * k as f64, y + pitch[1] * k as f64],
                    drill: spec.drill.to_mm(),
                    diameter: spec.diameter.to_mm(),
                    layers: copper[a..=b].to_vec(),
                });
            }
        }

        let mut items: Vec<Item> = Vec::new();
        for (pi, p) in parts.iter().enumerate() {
            for (k, pad) in p.pads.iter().enumerate() {
                if !pad.copper.is_empty() {
                    let shape = Shape::Poly(pad.outlines.clone());
                    items.push(Item {
                        owner: Owner::Pad(pi, k),
                        net: pad.net,
                        layers: pad.copper.clone(),
                        bounds: shape.bounds(),
                        shape,
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
                    });
                }
            }
        }
        let mut seg_track = Vec::new();
        for (ti, t) in tracks.iter().enumerate() {
            for w in t.points.windows(2) {
                let shape = Shape::Seg(w[0], w[1], t.width / 2.0);
                seg_track.push(ti);
                items.push(Item {
                    owner: Owner::Seg(seg_track.len() - 1),
                    net: Some(t.net),
                    layers: vec![t.layer.clone()],
                    bounds: shape.bounds(),
                    shape,
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
            });
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
            }
        };

        let mut uf = UnionFind::new(items.len());
        let mut shorts = Vec::new();
        let mut tight = Vec::new();
        let max_clear = nets.iter().map(|n| n.clearance).fold(default_clearance, f64::max);
        for i in 0..items.len() {
            for j in i + 1..items.len() {
                let (a, b) = (&items[i], &items[j]);
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
                let dist = a.shape.distance(&b.shape);
                let same = a.net.is_some() && a.net == b.net;
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
                let need = clearance_of(a.net).max(clearance_of(b.net));
                if dist <= 1e-6 {
                    shorts.push(format!("{} touches {}", name_of(a), name_of(b)));
                } else if dist + 1e-6 < need {
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
        for s in &shorts {
            d.error("short", s.clone());
        }
        for s in &tight {
            d.error("clearance", s.clone());
        }

        let edge_clear = board.rules.min_copper_to_edge.to_mm();
        if outline.len() >= 3 {
            for it in &items {
                if !matches!(it.owner, Owner::Seg(_) | Owner::Via(_)) {
                    continue;
                }
                let inside = match &it.shape {
                    Shape::Seg(a, b, _) => {
                        geom::point_in_polygon(*a, &outline) && geom::point_in_polygon(*b, &outline)
                    }
                    Shape::Circle(c, _) => geom::point_in_polygon(*c, &outline),
                    Shape::Poly(_) => true,
                };
                let to_edge = edges(&outline)
                    .map(|(a, b)| it.shape.distance(&Shape::Seg(a, b, 0.0)))
                    .fold(f64::MAX, f64::min);
                if !inside {
                    d.error("edge", format!("{} leaves the board", name_of(it)));
                } else if to_edge + 1e-6 < edge_clear {
                    d.error(
                        "edge",
                        format!(
                            "{} is {} from the board edge, needs {}",
                            name_of(it),
                            Length::mm(to_edge),
                            Length::mm(edge_clear)
                        ),
                    );
                }
            }
            for p in &parts {
                let off: Vec<&str> = p
                    .pads
                    .iter()
                    .filter(|q| {
                        q.outlines.iter().flatten().any(|c| {
                            !geom::point_in_polygon(*c, &outline)
                                && edges(&outline)
                                    .all(|(a, b)| geom::point_segment_distance(*c, a, b) > 1e-3)
                        })
                    })
                    .map(|q| q.number.as_str())
                    .collect();
                if !off.is_empty() {
                    d.error(
                        format!("part {}", p.reference),
                        format!("pads {} hang off the board", off.join(", ")),
                    );
                }
            }
        }

        let courtyards: Vec<(Bounds, bool)> = parts
            .iter()
            .map(|p| {
                let mut cy = p.footprint.courtyard("F");
                if cy.is_empty() {
                    cy = p.footprint.courtyard("B");
                }
                let t = p.transform();
                let mut b = Bounds::EMPTY;
                if !cy.is_empty() {
                    for c in [cy.min, cy.max, [cy.min[0], cy.max[1]], [cy.max[0], cy.min[1]]] {
                        b.add(t.apply(c));
                    }
                }
                (b, p.bottom)
            })
            .collect();
        for i in 0..parts.len() {
            for j in i + 1..parts.len() {
                let ((a, sa), (b, sb)) = (&courtyards[i], &courtyards[j]);
                if sa == sb && !a.is_empty() && !b.is_empty() && a.overlaps(b) {
                    d.error(
                        format!("part {}", parts[i].reference),
                        format!("courtyard overlaps {}", parts[j].reference),
                    );
                }
            }
        }

        let cutouts: Vec<(Vec<String>, Vec<P>)> = self
            .cutouts
            .iter()
            .map(|c| (c.layers.clone(), c.points.iter().map(|p| p.to_mm()).collect()))
            .collect();
        let mut zones = Vec::new();
        let mut island_nodes: Vec<Vec<usize>> = Vec::new();
        for (zi, z) in self.zones.iter().enumerate() {
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
                let layer_cutouts: Vec<&Vec<P>> =
                    cutouts.iter().filter(|(ls, _)| ls.contains(layer)).map(|(_, p)| p).collect();
                let (fill, touched) = fill_zone(
                    net,
                    layer,
                    &poly,
                    &outline,
                    edge_clear,
                    clearance,
                    &items,
                    &clearance_of,
                    &layer_cutouts,
                );
                if fill.islands_removed > 0 {
                    d.info(
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
                d.error(
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

        for (ti, t) in tracks.iter().enumerate() {
            for end in [t.points[0], *t.points.last().unwrap()] {
                let probe = Shape::Circle(end, t.width / 2.0);
                let touches = items.iter().any(|it| {
                    it.net == Some(t.net)
                        && it.layers.contains(&t.layer)
                        && match it.owner {
                            Owner::Seg(s) => seg_track[s] != ti,
                            _ => true,
                        }
                        && it.shape.distance(&probe) <= 1e-6
                }) || t.points.len() > 2 && is_interior_join(t, end)
                    || zones.iter().any(|z| z.net == t.net && z.layer == t.layer && z.filled(end));
                if !touches {
                    d.warn(
                        format!("tracks[{ti}] {}", nets[t.net].name),
                        format!("end at [{:.3}, {:.3}] connects to nothing", end[0], end[1]),
                    );
                }
            }
        }

        let (graphics, artwork) = self.artwork_of(&cx.dir, d);
        let silk: Vec<SilkBox> = parts
            .iter()
            .enumerate()
            .flat_map(|(i, p)| p.silk_texts(i))
            .chain(board_texts(&graphics))
            .map(|t| SilkBox { outline: t.outline(), text: t.text, layer: t.layer })
            .collect();
        check_silk(
            &parts,
            &vias,
            &tracks,
            &graphics,
            &artwork,
            &outline,
            board.rules.min_silk_text_height.to_mm(),
            d,
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
        let pairs = self.pairs_of(board, &nets, &tracks, d);
        let match_groups = self.matches_of(&nets, d);
        Layout {
            name: self.name.clone(),
            board: board.name.clone(),
            schematic: sch.name.clone(),
            outline,
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
        }
    }

    fn pairs_of(
        &self,
        board: &Board,
        nets: &[LayoutNet],
        tracks: &[Track],
        d: &mut Diags,
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
        let mut out = Vec::new();
        for (a, b, skew_limit) in found {
            let (pa, pb) = (&nets[a], &nets[b]);
            let class = class_of(board, &pa.class);
            let gap = class.and_then(|c| c.diff_gap.map(Length::to_mm));
            let at = format!("pair {}/{}", pa.name, pb.name);
            let skew_mm = pa.length_mm - pb.length_mm;
            let skew_ps = pa.delay_ps - pb.delay_ps;
            let limit = skew_limit.or(class.and_then(|c| c.max_skew.map(Length::to_mm)));
            match limit {
                Some(l) if skew_mm.abs() > l + 1e-9 => d.error(
                    &at,
                    format!(
                        "skew {:.3} mm ({:.2} ps), limit {l} mm: lengthen {} by {:.3} mm",
                        skew_mm,
                        skew_ps,
                        if skew_mm > 0.0 { &pb.name } else { &pa.name },
                        skew_mm.abs() - l
                    ),
                ),
                _ => d.info(&at, format!("skew {:.3} mm ({:.2} ps)", skew_mm, skew_ps)),
            }
            let mut coupled = 0.0;
            let mut wrong: Option<(f64, f64)> = None;
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
                                        wrong = Some((sep - (ta.width + tb.width) / 2.0, overlap));
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some((got, len)) = wrong {
                    d.error(
                        &at,
                        format!("runs {len:.2} mm at a {got:.3} mm gap, the class wants {g} mm"),
                    );
                }
                let longest = pa.length_mm.max(pb.length_mm);
                if longest > 0.0 && coupled < 0.8 * longest {
                    d.warn(
                        &at,
                        format!(
                            "only {coupled:.2} of {longest:.2} mm run side by side at the pair gap"
                        ),
                    );
                }
            }
            out.push(Pair { p: a, n: b, skew_mm, skew_ps, coupled_mm: coupled });
        }
        out
    }

    fn matches_of(&self, nets: &[LayoutNet], d: &mut Diags) -> Vec<MatchGroup> {
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
                    d.error(
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

    fn artwork_of(
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

fn board_texts(graphics: &[crate::graphic::Graphic]) -> Vec<SilkText> {
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
    outline: &[P],
    min_height: f64,
    d: &mut Diags,
) {
    let mut texts: Vec<SilkText> =
        parts.iter().enumerate().flat_map(|(i, p)| p.silk_texts(i)).collect();
    texts.extend(board_texts(graphics));
    let boxes: Vec<Vec<P>> = texts.iter().map(|t| t.outline()).collect();
    for a in artwork.iter().filter(|a| a.layer.ends_with(".SilkS")) {
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
            d.warn(
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
            d.warn(&at, format!("`{}` overlaps {}", a.name, hit.join(", ")));
        }
        if outline.len() >= 3
            && a.polygons.iter().flatten().any(|c| !geom::point_in_polygon(*c, outline))
        {
            d.warn(&at, format!("`{}` runs off the board", a.name));
        }
    }
    for (i, t) in texts.iter().enumerate() {
        let who = &t.owner;
        let at = format!("silk {who}");
        if t.size + 1e-9 < min_height {
            d.warn(
                &at,
                format!(
                    "`{}` is {} tall, under the fab minimum {}",
                    t.text,
                    Length::mm(t.size),
                    Length::mm(min_height)
                ),
            );
        }
        let found = silk_issues(t, &boxes[i], i, &texts, &boxes, parts, vias, outline);
        if found.is_empty() {
            continue;
        }
        let hint = match (t.part != usize::MAX).then(|| parts.get(t.part)).flatten() {
            Some(p) if t.text == p.reference => {
                free_spot(t, p, &texts, &boxes, i, parts, vias, tracks, outline)
                    .map(|(at, rot)| {
                        let r =
                            if rot != 0.0 { format!(", rotation = {rot}") } else { String::new() };
                        format!("; label = {{ at = [{:.2}, {:.2}]{r} }} is clear", at[0], at[1])
                    })
                    .unwrap_or_default()
            }
            _ => String::new(),
        };
        d.warn(&at, format!("`{}` {}{hint}", t.text, found.join(", ")));
    }
}

const SILK_GAP: f64 = 0.4;

#[allow(clippy::too_many_arguments)]
fn silk_issues(
    t: &SilkText,
    bx: &[P],
    me: usize,
    texts: &[SilkText],
    boxes: &[Vec<P>],
    parts: &[Placed],
    vias: &[Via],
    outline: &[P],
) -> Vec<String> {
    let mut out = Vec::new();
    for (j, u) in texts.iter().enumerate() {
        if j != me && u.layer == t.layer && geom::polygon_distance(bx, &boxes[j]) < SILK_GAP {
            out.push(format!("crowds `{}` of {}", u.text, u.owner));
        }
    }
    let side = t.layer.trim_end_matches(".SilkS");
    let cu = format!("{side}.Cu");
    let pads: Vec<String> = parts
        .iter()
        .flat_map(|p| p.pads.iter().map(move |q| (p, q)))
        .filter(|(_, q)| {
            (q.copper.contains(&cu) || q.drill.is_some())
                && q.outlines.iter().any(|o| geom::polygon_distance(o, bx) <= 0.0)
        })
        .map(|(p, q)| format!("{}.{}", p.reference, q.number))
        .collect();
    if !pads.is_empty() {
        out.push(format!("sits on pads {}, it will be clipped", pads.join(", ")));
    }
    let on_vias = vias
        .iter()
        .filter(|v| {
            geom::point_in_polygon(v.at, bx)
                || geom::polyline_polygon_distance(&[v.at, v.at], bx) < v.diameter / 2.0
        })
        .count();
    if on_vias > 0 {
        out.push(format!("prints over {on_vias} via{}", if on_vias == 1 { "" } else { "s" }));
    }
    let crossed: Vec<&str> = parts
        .iter()
        .filter(|p| {
            let tf = p.transform();
            p.footprint.graphics.iter().any(|g| {
                p.flip_layer(&g.layer) == t.layer
                    && !matches!(g.shape, crate::graphic::Shape::Text { .. })
                    && {
                        let path: Vec<P> = crate::footprint::graphic_path(g)
                            .into_iter()
                            .map(|q| tf.apply(q))
                            .collect();
                        path.len() >= 2
                            && geom::polyline_polygon_distance(&path, bx)
                                < g.width.to_mm() / 2.0 + SILK_GAP / 2.0
                    }
            })
        })
        .map(|p| p.reference.as_str())
        .collect();
    if !crossed.is_empty() {
        out.push(format!("crosses the silk outline of {}", crossed.join(", ")));
    }
    if outline.len() >= 3 && bx.iter().any(|c| !geom::point_in_polygon(*c, outline)) {
        out.push("runs off the board".into());
    }
    let side = if t.layer.starts_with("B.") { "B" } else { "F" };
    let hidden: Vec<&str> = parts
        .iter()
        .enumerate()
        .filter(|(k, p)| {
            *k != t.part && body_box(p, side).is_some_and(|b| geom::polygon_distance(&b, bx) <= 0.0)
        })
        .map(|(_, p)| p.reference.as_str())
        .collect();
    if !hidden.is_empty() {
        out.push(format!("hides under the body of {}", hidden.join(", ")));
    }
    out
}

fn body_box(p: &Placed, side: &str) -> Option<Vec<P>> {
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
    parts: &[Placed],
    vias: &[Via],
    tracks: &[Track],
    outline: &[P],
) -> Option<(P, f64)> {
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
    for step in 0..6 {
        let gap = SILK_GAP + step as f64 * 0.35;
        candidates.push(([c[0], b.min[1] - gap - h / 2.0], 0.0));
        candidates.push(([c[0], b.max[1] + gap + h / 2.0], 0.0));
        candidates.push(([b.max[0] + gap + w / 2.0, c[1]], 0.0));
        candidates.push(([b.min[0] - gap - w / 2.0, c[1]], 0.0));
        candidates.push(([b.max[0] + gap + h / 2.0, c[1]], 90.0));
        candidates.push(([b.min[0] - gap - h / 2.0, c[1]], 90.0));
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
        silk_issues(&trial, &bx, me, texts, boxes, parts, vias, outline).is_empty()
            && !tracks.iter().any(|tr| {
                tr.layer == cu
                    && geom::polyline_polygon_distance(&tr.points, &bx) < tr.width / 2.0 + 0.1
            })
    })
}

fn is_interior_join(t: &Track, end: P) -> bool {
    t.points[1..t.points.len() - 1].iter().any(|p| geom::dist(*p, end) < 1e-9)
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

#[allow(clippy::too_many_arguments)]
fn fill_zone(
    net: usize,
    layer: &str,
    poly: &[P],
    board: &[P],
    edge_clear: f64,
    clearance: f64,
    items: &[Item],
    clearance_of: &dyn Fn(Option<usize>) -> f64,
    cutouts: &[&Vec<P>],
) -> (ZoneFill, Vec<Vec<usize>>) {
    let mut b = Bounds::EMPTY;
    poly.iter().for_each(|p| b.add(*p));
    let [sw, sh] = b.size();
    let cell = (sw.max(sh) / 1600.0).max(0.02);
    let (w, h) = ((sw / cell).ceil() as usize + 1, (sh / cell).ceil() as usize + 1);
    let origin = b.min;
    let center = |x: usize, y: usize| {
        [origin[0] + (x as f64 + 0.5) * cell, origin[1] + (y as f64 + 0.5) * cell]
    };
    let mut mask = vec![0u8; w * h];
    for y in 0..h {
        let py = origin[1] + (y as f64 + 0.5) * cell;
        let mut xs = Vec::new();
        for (a, c) in edges(poly) {
            if (a[1] > py) != (c[1] > py) {
                xs.push(a[0] + (py - a[1]) / (c[1] - a[1]) * (c[0] - a[0]));
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in xs.chunks(2) {
            if pair.len() < 2 {
                continue;
            }
            let x0 = (((pair[0] - origin[0]) / cell) - 0.5).ceil().max(0.0) as usize;
            let x1 = (((pair[1] - origin[0]) / cell) - 0.5).floor().min(w as f64 - 1.0);
            if x1 < 0.0 {
                continue;
            }
            for x in x0..=(x1 as usize) {
                mask[y * w + x] = 1;
            }
        }
    }
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
    if board.len() >= 3 {
        for (a, c) in edges(board) {
            let mut bb = Bounds::EMPTY;
            bb.add(a);
            bb.add(c);
            clear_near(&mut mask, bb, edge_clear, &|p| {
                geom::point_segment_distance(p, a, c) < edge_clear + margin
            });
        }
        for y in 0..h {
            for x in 0..w {
                if mask[y * w + x] != 0 && !geom::point_in_polygon(center(x, y), board) {
                    mask[y * w + x] = 0;
                }
            }
        }
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
        let gap = clearance.max(clearance_of(it.net));
        let shape = &it.shape;
        clear_near(&mut mask, it.bounds, gap, &|p| shape.point_distance(p) < gap + margin);
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
        rings: Vec::new(),
        triangles: Vec::new(),
    };
    fill.rings =
        vector_fill(&fill, poly, board, edge_clear, clearance, items, clearance_of, cutouts);
    fill.triangles = crate::contour::triangles(&fill.rings);
    (fill, touched)
}

fn arc_steps(r: f64) -> usize {
    let tol = 0.002f64.min(r * 0.5);
    ((std::f64::consts::PI / (1.0 - tol / r).clamp(-1.0, 1.0).acos()).ceil() as usize)
        .clamp(12, 180)
}

fn capsule(a: P, b: P, r: f64) -> Vec<P> {
    let n = arc_steps(r);
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
fn vector_fill(
    raster: &ZoneFill,
    poly: &[P],
    board: &[P],
    edge_clear: f64,
    clearance: f64,
    items: &[Item],
    clearance_of: &dyn Fn(Option<usize>) -> f64,
    cutouts: &[&Vec<P>],
) -> Vec<Vec<P>> {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;
    let area_of = |r: &[P]| crate::contour::area(r);
    let subject: Vec<Vec<P>> = if board.len() >= 3 && poly.len() >= 3 {
        poly.to_vec()
            .overlay(&board.to_vec(), OverlayRule::Intersect, FillRule::NonZero)
            .into_iter()
            .flatten()
            .collect()
    } else {
        vec![poly.to_vec()]
    };
    let mut clip: Vec<Vec<P>> = Vec::new();
    if board.len() >= 3 && edge_clear > 0.0 {
        for (a, b) in edges(board) {
            clip.push(capsule(a, b, edge_clear));
        }
    }
    for c in cutouts {
        clip.push(c.to_vec());
    }
    for it in items.iter().filter(|it| it.layers.iter().any(|l| l == &raster.layer)) {
        if it.net == Some(raster.net) && it.owner != Owner::Hole {
            continue;
        }
        let gap = clearance.max(clearance_of(it.net));
        clip.extend(inflated(&it.shape, gap));
    }
    let shapes = subject.overlay(&clip, OverlayRule::Difference, FillRule::NonZero);
    let mut rings = Vec::new();
    for shape in shapes {
        let tris = crate::contour::triangles(&shape);
        let probe = tris
            .iter()
            .max_by(|a, b| area_of(a.as_ref()).abs().total_cmp(&area_of(b.as_ref()).abs()))
            .map(|t| [(t[0][0] + t[1][0] + t[2][0]) / 3.0, (t[0][1] + t[1][1] + t[2][1]) / 3.0]);
        if !probe.is_some_and(|p| raster.filled(p)) {
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

fn parallel_overlap(a0: P, a1: P, b0: P, b1: P) -> Option<(f64, f64)> {
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

fn glob(pat: &str, s: &str) -> bool {
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

    #[test]
    fn a_serpentine_adds_exactly_what_was_asked() {
        let pts = serpentine([0.0, 0.0], [10.0, 0.0], 2.5, 0.6, 0.4).unwrap();
        let len: f64 = pts.windows(2).map(|w| geom::dist(w[0], w[1])).sum();
        assert!((len - 12.5).abs() < 1e-9, "{len}");
        assert!(serpentine([0.0, 0.0], [1.0, 0.0], 5.0, 0.3, 0.4).is_err());
        assert!(glob("DQ?", "DQ7") && glob("DQ*", "DQ15") && !glob("DQ?", "DQS0"));
    }
}
