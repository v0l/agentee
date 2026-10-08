mod rf;

use crate::board::Board;
use crate::footprint::{Footprint, PadKind};
use crate::geom::{self, P, Transform};
use crate::graphic::{Anchor, Bounds, Graphic, Shape};
use crate::layout::{Artwork, BoardSide, PlacementFile, SilkText, glob};
use crate::schematic::{PinRef, Schematic};
use crate::units::{Length, Point};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const GRID: f64 = 0.05;
pub const FIDUCIAL_TO_EDGE: f64 = 3.0;
const HOT_GAP: f64 = 10.0;
const QUIET_GAP: f64 = 8.0;
pub const HOT_WATTS: f64 = 0.25;
const EPS: f64 = 1e-6;
const LARGE_MLCC: f64 = 1.8;
const FLEX_LARGE: f64 = 20.0;
const FLEX_POINTING: f64 = 3.0;
const FLEX_REACH: f64 = 12.0;
const STARTS: u64 = 8;
const KEPT_STARTS: usize = 3;
const CENTRE_PULL: f64 = 2.0;
const CENTRE_BLEND: f64 = 0.5;
const MOVES_PER_PART: f64 = 1500.0;
const ANNEAL_HEAT: f64 = 1.0 / 3.0;
const QUENCH_PER_PART: f64 = 500.0;
const QUENCH_HEAT: f64 = 0.03;
const CROSSING_WEIGHT: f64 = 4.0;
const SILK_ROOM: f64 = 0.2;
const LABEL_SHARE: f64 = 0.25;
const LABEL_REACH: usize = 60;
const LABEL_WEIGHT: f64 = 0.2;
const TEXT_REACH: f64 = 10.0;
const HOT_SHARE: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

const EDGES: [Edge; 4] = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom];

impl Edge {
    fn normal(self) -> P {
        match self {
            Edge::Left => [-1.0, 0.0],
            Edge::Right => [1.0, 0.0],
            Edge::Top => [0.0, -1.0],
            Edge::Bottom => [0.0, 1.0],
        }
    }

    fn tangent(self) -> P {
        match self {
            Edge::Left | Edge::Right => [0.0, 1.0],
            Edge::Top | Edge::Bottom => [1.0, 0.0],
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceFile {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub edges: BTreeMap<String, Edge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keepouts: Vec<Vec<Point>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoupling_distance: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crystal_distance: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_spread: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hot_distance: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_edge: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_centre: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sides {
    Top,
    Bottom,
    Both,
}

#[derive(Clone, Debug)]
pub struct PlaceOptions {
    pub parts: Vec<String>,
    pub keep_placed: bool,
    pub sides: Sides,
    pub seed: u64,
    pub spacing: f64,
    pub standoff: f64,
}

impl Default for PlaceOptions {
    fn default() -> Self {
        PlaceOptions {
            parts: Vec::new(),
            keep_placed: false,
            sides: Sides::Top,
            seed: 1,
            spacing: 0.0,
            standoff: 0.0,
        }
    }
}

pub struct PlaceInput<'a> {
    pub board: &'a Board,
    pub outline: &'a [P],
    pub cutouts: &'a [Vec<P>],
    pub schematic: &'a Schematic,
    pub footprints: &'a HashMap<&'a str, &'a Footprint>,
    pub placements: &'a [PlacementFile],
    pub spec: &'a PlaceFile,
    pub fast_nets: Vec<String>,
    pub heat: Vec<(String, f64)>,
    pub silk: Vec<SilkArea>,
    pub texts: Vec<SilkText>,
    pub tracks: &'a [crate::layout::TrackFile],
    pub vias: &'a [crate::layout::ViaFile],
}

#[derive(Clone, Debug)]
pub struct SilkArea {
    pub bottom: bool,
    pub poly: Vec<P>,
}

pub fn movable_texts(graphics: &[Graphic]) -> Vec<SilkText> {
    let unlocked: Vec<Graphic> = graphics.iter().filter(|g| !g.locked).cloned().collect();
    crate::layout::board_texts(&unlocked)
}

pub fn board_silk(graphics: &[Graphic], artwork: &[Artwork]) -> Vec<SilkArea> {
    let side = |layer: &str| layer.starts_with("B.");
    let locked: Vec<Graphic> = graphics.iter().filter(|g| g.locked).cloned().collect();
    let mut out: Vec<SilkArea> = crate::layout::board_texts(&locked)
        .iter()
        .filter(|t| t.part == usize::MAX && t.owner != "watermark")
        .map(|t| SilkArea { bottom: side(&t.layer), poly: grow(&t.outline(), SILK_ROOM) })
        .collect();
    for g in graphics.iter().filter(|g| g.layer.ends_with(".SilkS")) {
        if matches!(g.shape, Shape::Text { .. }) {
            continue;
        }
        let half = g.width.to_mm() / 2.0 + SILK_ROOM;
        let path = crate::footprint::graphic_path(g);
        for w in path.windows(2) {
            let (a, b) = (w[0], w[1]);
            let l = geom::dist(a, b);
            let (u, n) = if l < 1e-9 {
                ([half, 0.0], [0.0, half])
            } else {
                let d = [(b[0] - a[0]) / l * half, (b[1] - a[1]) / l * half];
                (d, [-d[1], d[0]])
            };
            let poly = vec![
                [a[0] - u[0] - n[0], a[1] - u[1] - n[1]],
                [b[0] + u[0] - n[0], b[1] + u[1] - n[1]],
                [b[0] + u[0] + n[0], b[1] + u[1] + n[1]],
                [a[0] - u[0] + n[0], a[1] - u[1] + n[1]],
            ];
            out.push(SilkArea { bottom: side(&g.layer), poly });
        }
    }
    for a in artwork.iter().filter(|a| a.layer.ends_with(".SilkS")) {
        for poly in &a.polygons {
            out.push(SilkArea { bottom: side(&a.layer), poly: poly.clone() });
        }
    }
    out
}

fn grow(rect: &[P], by: f64) -> Vec<P> {
    let unit = |a: P, b: P| {
        let l = geom::dist(a, b).max(1e-9);
        [(b[0] - a[0]) / l * by, (b[1] - a[1]) / l * by]
    };
    let (u, v) = (unit(rect[0], rect[1]), unit(rect[0], rect[3]));
    let signs = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    rect.iter()
        .zip(signs)
        .map(|(q, (a, b))| [q[0] + a * u[0] + b * v[0], q[1] + a * u[1] + b * v[1]])
        .collect()
}

#[derive(Clone)]
struct LabelBox {
    poly: Vec<P>,
    at: P,
    rotation: f64,
}

fn label_room(
    fp: &Footprint,
    reference: &str,
    placed: Option<&PlacementFile>,
    centre: P,
    inward: bool,
) -> Option<LabelBox> {
    let file = placed.and_then(|f| f.label.as_ref());
    if file.is_some_and(|l| l.hide) {
        return None;
    }
    let (layer, at, text, size, rotation, anchor) =
        fp.graphics.iter().find_map(|g| match &g.shape {
            Shape::Text { at, text, size, rotation, anchor }
                if g.layer.ends_with(".SilkS") && text.contains("${REFERENCE}") =>
            {
                Some((&g.layer, at.to_mm(), text, size.to_mm(), *rotation, *anchor))
            }
            _ => None,
        })?;
    let size = file.and_then(|l| l.size).map_or(size, |s| s.to_mm());
    let text = text.replace("${REFERENCE}", reference);
    let pen = crate::font::default_thickness(size);
    let (w, h) = (crate::font::ink_width(&text, size) + pen, size + pen);
    let shift = match anchor {
        Anchor::Left => w / 2.0,
        Anchor::Center => 0.0,
        Anchor::Right => -w / 2.0,
    };
    let c0 = geom::rotate([shift, 0.0], rotation);
    let c0 = [at[0] + c0[0], at[1] + c0[1]];
    let rect = |c: P, m: f64| -> Vec<P> {
        [
            [-w / 2.0 - m, -h / 2.0 - m],
            [w / 2.0 + m, -h / 2.0 - m],
            [w / 2.0 + m, h / 2.0 + m],
            [-w / 2.0 - m, h / 2.0 + m],
        ]
        .into_iter()
        .map(|q| {
            let [x, y] = geom::rotate(q, rotation);
            [x + c[0], y + c[1]]
        })
        .collect()
    };
    let pads: Vec<Vec<P>> = fp.pads.iter().flat_map(|q| q.outlines()).collect();
    let silk: Vec<(Vec<P>, f64)> = fp
        .graphics
        .iter()
        .filter(|g| g.layer == *layer && !matches!(g.shape, Shape::Text { .. }))
        .map(|g| (crate::footprint::graphic_path(g), g.width.to_mm() / 2.0 + SILK_ROOM))
        .filter(|(path, _)| path.len() >= 2)
        .collect();
    let clear = |c: P| {
        let bx = rect(c, 0.0);
        pads.iter().all(|o| geom::polygon_distance(o, &bx) > GRID)
            && silk
                .iter()
                .all(|(path, gap)| geom::polyline_polygon_distance(path, &bx) > gap + GRID)
    };
    let d = [c0[0] - centre[0], c0[1] - centre[1]];
    let l = d[0].hypot(d[1]);
    let dir = if l < 1e-6 { [0.0, -1.0] } else { [d[0] / l, d[1] / l] };
    let dir = if inward { [-dir[0], -dir[1]] } else { dir };
    let at = (0..=LABEL_REACH)
        .map(|k| [c0[0] + dir[0] * k as f64 * GRID, c0[1] + dir[1] * k as f64 * GRID])
        .find(|c| clear(*c))?;
    Some(LabelBox { poly: rect(at, SILK_ROOM), at, rotation })
}

fn upright(deg: f64) -> f64 {
    let a = deg.rem_euclid(360.0);
    if a > 90.0 && a <= 270.0 { a - 180.0 } else { a }
}

#[derive(Clone, Debug, Serialize)]
pub struct Placement {
    pub reference: String,
    pub at: P,
    pub rotation: f64,
    pub bottom: bool,
    pub label: Option<(P, f64)>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Metrics {
    pub hpwl_mm: f64,
    pub hpwl_all_mm: f64,
    pub crossings: usize,
    pub overlaps: usize,
    pub decap_mean_mm: f64,
    pub decap_max_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClusterSummary {
    pub anchor: String,
    pub members: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlaceResult {
    pub placements: Vec<Placement>,
    pub kept: Vec<String>,
    pub failed: Vec<String>,
    pub edges: BTreeMap<String, Edge>,
    pub clusters: Vec<ClusterSummary>,
    pub before: Option<Metrics>,
    pub after: Metrics,
    pub moves: usize,
    pub labels: LabelRoom,
    pub texts_moved: Vec<TextMove>,
    pub texts_stuck: Vec<String>,
    pub hot_spread: HotSpread,
}

#[derive(Clone, Debug, Serialize)]
pub struct HotSpread {
    pub applied: bool,
    pub share: f64,
    pub threshold: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TextMove {
    pub text: String,
    pub layer: String,
    pub from: P,
    pub to: P,
}

#[derive(Clone, Debug, Serialize)]
pub struct LabelRoom {
    pub reserved: bool,
    pub weighed: bool,
    pub label_mm2: f64,
    pub free_mm2: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Hole,
    Fiducial,
    TestPoint,
    Connector,
    Crystal,
    Passive,
    Chip,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnKind {
    Rf,
    Usb,
    Other,
}

fn ref_prefix(reference: &str) -> String {
    reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_uppercase()
}

pub fn copper_pad_numbers(fp: &Footprint) -> usize {
    let mut v: Vec<&str> = fp
        .pads
        .iter()
        .filter(|p| p.is_copper() && !p.number.is_empty())
        .map(|p| p.number.as_str())
        .collect();
    v.sort_unstable();
    v.dedup();
    v.len()
}

pub fn edge_mount(fp: &Footprint) -> bool {
    fp.overhang || fp.pads.iter().any(|p| p.edge)
}

pub fn role_of(reference: &str, fp_name: &str, fp: &Footprint) -> Role {
    let name = fp_name.to_ascii_lowercase();
    let prefix = ref_prefix(reference);
    if name.contains("mountinghole") || prefix == "MH" {
        return Role::Hole;
    }
    if name.contains("fiducial") || prefix == "FID" {
        return Role::Fiducial;
    }
    if prefix == "TP" || name.starts_with("testpoint") {
        return Role::TestPoint;
    }
    if edge_mount(fp) {
        return Role::Connector;
    }
    let pogo = ["tag-connect", "tc2030", "tc2050", "u.fl", "ufl", "testpoint"];
    if matches!(prefix.as_str(), "J" | "P" | "CN" | "CON" | "USB")
        && !pogo.iter().any(|w| name.contains(w))
    {
        return Role::Connector;
    }
    if prefix == "Y"
        || ["crystal", "oscillator", "xtal", "tcxo", "resonator"].iter().any(|w| name.contains(w))
    {
        return Role::Crystal;
    }
    let numbers = copper_pad_numbers(fp);
    if matches!(prefix.as_str(), "R" | "C" | "L" | "FB" | "D" | "LED" | "F")
        && (1..=4).contains(&numbers)
    {
        return Role::Passive;
    }
    match numbers {
        0 => Role::Other,
        1 | 2 => Role::Passive,
        _ => Role::Chip,
    }
}

pub fn conn_kind(fp_name: &str) -> ConnKind {
    let n = fp_name.to_ascii_lowercase();
    if n.contains("usb") {
        ConnKind::Usb
    } else if ["sma", "smb", "bnc", "coax", "mmcx", "u.fl", "ufl", "n_type"]
        .iter()
        .any(|w| n.contains(w))
    {
        ConnKind::Rf
    } else {
        ConnKind::Other
    }
}

pub fn watts(v: &str) -> Option<f64> {
    let s = v.trim().to_ascii_lowercase();
    if let Some(x) = s.strip_suffix("mw") {
        return x.trim().parse::<f64>().ok().map(|x| x * 1e-3);
    }
    s.strip_suffix('w').unwrap_or(&s).trim().parse().ok()
}

pub fn thermal_heat<'a>(
    sims: impl IntoIterator<Item = &'a crate::sim::SimFile>,
    layout: &str,
) -> Vec<(String, f64)> {
    let mut heat: Vec<(String, f64)> = Vec::new();
    for f in sims {
        let mine = f.layout.as_deref().is_none_or(|l| l == layout);
        if f.kind != Some(crate::sim::SimKind::Thermal) || !mine {
            continue;
        }
        for h in &f.sources {
            if let Some(w) = watts(&h.power)
                && !heat.iter().any(|(r, _)| *r == h.reference)
            {
                heat.push((h.reference.clone(), w));
            }
        }
    }
    heat
}

pub fn is_ground(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    u.starts_with("GND") || u.ends_with("GND") || u.starts_with("VSS") || u == "0V"
}

pub fn is_power_net(board: &Board, name: &str, class: &str) -> bool {
    let c = board.netclasses.iter().find(|c| c.name == class);
    is_ground(name) || c.is_some_and(|c| c.current.is_some()) || crate::testpoint::is_rail(name)
}

pub fn is_fast_class(board: &Board, class: &str) -> bool {
    board
        .netclasses
        .iter()
        .find(|c| c.name == class)
        .is_some_and(|c| c.impedance.is_some() || c.diff_gap.is_some())
}

pub fn is_rf_class(board: &Board, class: &str) -> bool {
    board
        .netclasses
        .iter()
        .find(|c| c.name == class)
        .is_some_and(|c| c.impedance.is_some() && c.diff_gap.is_none())
}

pub fn cap_farads(value: &str) -> Option<f64> {
    let v: String = value.trim().replace('\u{b5}', "u").chars().filter(|c| *c != ' ').collect();
    let v = v.trim_end_matches(['F', 'f']);
    let pos = v.find(['p', 'n', 'u', 'm'])?;
    let scale = match &v[pos..pos + 1] {
        "p" => 1e-12,
        "n" => 1e-9,
        "u" => 1e-6,
        _ => 1e-3,
    };
    let (a, b) = (&v[..pos], &v[pos + 1..]);
    let text = if b.is_empty() { a.to_string() } else { format!("{a}.{b}") };
    text.parse::<f64>().ok().map(|x| x * scale)
}

pub fn is_capacitor(reference: &str, fp_name: &str) -> bool {
    let n = fp_name.to_ascii_lowercase();
    ref_prefix(reference) == "C" || n.starts_with("c_") || n.starts_with("cp_")
}

pub fn chip_length(fp_name: &str, fp: &Footprint) -> Option<f64> {
    if let Some((_, length)) = crate::drc::case_of(fp_name) {
        return Some(length);
    }
    let centres: Vec<P> = fp
        .pads
        .iter()
        .filter(|q| q.is_copper())
        .map(|q| {
            let mut b = Bounds::EMPTY;
            q.outlines().iter().flatten().for_each(|v| b.add(*v));
            if b.is_empty() { q.at.to_mm() } else { b.center() }
        })
        .collect();
    let d = match centres[..] {
        [a, b] => geom::dist(a, b),
        _ => return None,
    };
    (d > 1e-6).then_some(d)
}

fn grow_loop(poly: &[P], d: f64) -> Vec<P> {
    let n = poly.len();
    if d <= 0.0 || n < 3 {
        return poly.to_vec();
    }
    let area: f64 = (0..n)
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };
    let normal = |a: P, b: P| {
        let l = geom::dist(a, b).max(1e-12);
        [sign * (b[1] - a[1]) / l, -sign * (b[0] - a[0]) / l]
    };
    (0..n)
        .map(|i| {
            let (prev, here, next) = (poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]);
            let (u, v) = (normal(prev, here), normal(here, next));
            let k = (1.0 + u[0] * v[0] + u[1] * v[1]).max(0.2);
            [here[0] + d * (u[0] + v[0]) / k, here[1] + d * (u[1] + v[1]) / k]
        })
        .collect()
}

pub fn courtyard_loops(fp: &Footprint, layer: &str) -> Vec<Vec<P>> {
    let near = |a: P, b: P| geom::dist(a, b) < 1e-3;
    let mut closed = Vec::new();
    let mut open: Vec<Vec<P>> = Vec::new();
    for g in fp.graphics.iter().filter(|g| g.layer == layer) {
        let mut path = crate::footprint::graphic_path(g);
        if path.len() < 2 {
            continue;
        }
        let shut = match &g.shape {
            Shape::Rect { .. } | Shape::Circle { .. } => true,
            Shape::Polyline { closed, .. } => *closed,
            _ => false,
        };
        if shut {
            if near(path[0], *path.last().unwrap()) {
                path.pop();
            }
            closed.push(path);
        } else {
            open.push(path);
        }
    }
    while let Some(mut cur) = open.pop() {
        loop {
            let end = *cur.last().unwrap();
            if cur.len() > 2 && near(cur[0], end) {
                cur.pop();
                break;
            }
            if let Some(j) = open.iter().position(|p| near(p[0], end)) {
                let next = open.swap_remove(j);
                cur.extend_from_slice(&next[1..]);
            } else if let Some(j) = open.iter().position(|p| near(*p.last().unwrap(), end)) {
                let next = open.swap_remove(j);
                cur.extend(next.into_iter().rev().skip(1));
            } else {
                break;
            }
        }
        if cur.len() >= 3 {
            closed.push(cur);
        }
    }
    closed.retain(|c| c.len() >= 3);
    closed
}

fn poly_bounds(poly: &[P]) -> Bounds {
    let mut b = Bounds::EMPTY;
    poly.iter().for_each(|p| b.add(*p));
    b
}

fn is_axis_rect(poly: &[P]) -> bool {
    if poly.len() != 4 {
        return false;
    }
    (0..4).all(|i| {
        let (a, b) = (poly[i], poly[(i + 1) % 4]);
        (a[0] - b[0]).abs() < 1e-9 || (a[1] - b[1]).abs() < 1e-9
    })
}

fn strict_overlap(a: &Bounds, b: &Bounds) -> bool {
    a.min[0] < b.max[0] - EPS
        && b.min[0] < a.max[0] - EPS
        && a.min[1] < b.max[1] - EPS
        && b.min[1] < a.max[1] - EPS
}

fn near_box(b: &Bounds, v: P) -> bool {
    v[0] >= b.min[0] - EPS
        && v[0] <= b.max[0] + EPS
        && v[1] >= b.min[1] - EPS
        && v[1] <= b.max[1] + EPS
}

fn polys_overlap(a: &[P], b: &[P]) -> bool {
    let (box_a, box_b) = (poly_bounds(a), poly_bounds(b));
    let edges = |poly: &[P], other: &Bounds| -> Vec<(P, P)> {
        let n = poly.len();
        (0..n)
            .map(|i| (poly[i], poly[(i + 1) % n]))
            .filter(|(p, q)| {
                p[0].max(q[0]) >= other.min[0] - EPS
                    && p[0].min(q[0]) <= other.max[0] + EPS
                    && p[1].max(q[1]) >= other.min[1] - EPS
                    && p[1].min(q[1]) <= other.max[1] + EPS
            })
            .collect()
    };
    let near_b = edges(b, &box_a);
    if !near_b.is_empty() {
        for (p, q) in edges(a, &box_b) {
            if near_b.iter().any(|(u, v)| geom::segments_intersect(p, q, *u, *v)) {
                return true;
            }
        }
    }
    let inner = |p: P, q: &[P], c: P| {
        let s = [p[0] + (c[0] - p[0]) * 1e-4, p[1] + (c[1] - p[1]) * 1e-4];
        geom::point_in_polygon(s, q)
    };
    let (ca, cb) = (box_a.center(), box_b.center());
    a.iter().any(|p| inner(*p, b, cb)) || b.iter().any(|p| inner(*p, a, ca))
}

fn snap(v: f64) -> f64 {
    (v / GRID).round() * GRID
}

fn snap_p(p: P) -> P {
    [snap(p[0]), snap(p[1])]
}

fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

type Trial = (f64, Vec<(usize, St)>);

type Start<'a> = (f64, Placer<'a>, Vec<String>, BTreeMap<String, Edge>, usize);

type Rough<'a> = (f64, Placer<'a>, Vec<String>, BTreeMap<String, Edge>);

fn in_parallel<T: Send, R: Send>(items: Vec<T>, f: impl Fn(T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let threads = threads.clamp(1, items.len().max(1));
    let mut lanes: Vec<Vec<(usize, T)>> = (0..threads).map(|_| Vec::new()).collect();
    for (x, item) in items.into_iter().enumerate() {
        lanes[x % threads].push((x, item));
    }
    let mut out: Vec<(usize, R)> = std::thread::scope(|scope| {
        let handles: Vec<_> = lanes
            .into_iter()
            .map(|lane| {
                let f = &f;
                scope.spawn(move || lane.into_iter().map(|(x, t)| (x, f(t))).collect::<Vec<_>>())
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    out.sort_by_key(|r| r.0);
    out.into_iter().map(|r| r.1).collect()
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct St {
    at: P,
    rot: f64,
    bottom: bool,
}

impl St {
    fn transform(&self) -> Transform {
        Transform { at: self.at, rotation: self.rot, mirror: self.bottom }
    }
}

#[derive(Clone)]
struct LPad {
    c: P,
    outline: Vec<Vec<P>>,
    net: Option<usize>,
    edge: bool,
}

#[derive(Clone)]
struct WShape {
    side: u8,
    poly: Vec<P>,
    b: Bounds,
    rect: bool,
    label: bool,
}

#[derive(Clone)]
struct Part<'a> {
    reference: String,
    fp: &'a Footprint,
    value: String,
    role: Role,
    conn: ConnKind,
    pads: Vec<LPad>,
    ends: Option<(usize, usize)>,
    loops: Vec<(bool, Vec<P>)>,
    through: bool,
    pads_in_court: bool,
    local: Bounds,
    extent: Bounds,
    label: Option<LabelBox>,
    label_inward: Option<LabelBox>,
    over_silk: bool,
    area: f64,
    pins: usize,
    large: bool,
    heat: f64,
    mlcc_len: Option<f64>,
    fixed: bool,
    active: bool,
    placed: bool,
    st: St,
    sch_at: P,
    pin_edge: Option<Edge>,
    sensitive: bool,
    switcher: bool,
    hot: bool,
    spread_hot: bool,
}

#[derive(Clone)]
struct NetInfo {
    pins: Vec<(usize, usize)>,
    weight: f64,
    power: bool,
    ground: bool,
    rf: bool,
}

type Target = (usize, P, Option<(usize, P)>);

#[derive(Clone, Copy)]
enum End {
    Pad(usize, usize),
    Centre(usize),
}

#[derive(Clone, Copy)]
enum LinkKind {
    Pull { w: f64, thr: f64, extra: f64, near: f64 },
    Repel { w: f64, thr: f64 },
}

#[derive(Clone, Copy)]
struct Link {
    a: End,
    b: End,
    kind: LinkKind,
}

#[derive(Clone)]
struct Cluster {
    anchor: usize,
    members: Vec<usize>,
    served: HashMap<usize, (usize, usize)>,
}

#[derive(Clone)]
struct Board2 {
    outline: Vec<P>,
    cutouts: Vec<Vec<P>>,
    bounds: Bounds,
    centre: P,
    keepouts: Vec<Vec<P>>,
    body_edge: f64,
    part_edge: f64,
    copper_edge: f64,
    flex: f64,
    decap: f64,
    crystal: f64,
    spread: f64,
    standoff: f64,
    depth: EdgeDepth,
    edges: std::sync::Arc<EdgeIndex>,
    flex_edges: std::sync::Arc<EdgeIndex>,
    silk: Vec<WShape>,
}

#[derive(Clone)]
struct Placer<'a> {
    parts: Vec<Part<'a>>,
    nets: Vec<NetInfo>,
    part_nets: Vec<Vec<usize>>,
    links: Vec<Link>,
    part_links: Vec<Vec<usize>>,
    two_pin: Vec<usize>,
    is_two_pin: Vec<bool>,
    clusters: Vec<Cluster>,
    cluster_of: Vec<Option<usize>>,
    b: Board2,
    sides: Sides,
    grid: PartGrid,
    pad_at: Vec<Vec<P>>,
    segments: Vec<Option<(P, P)>>,
    two_pin_at: Vec<usize>,
    cache: Vec<Vec<WShape>>,
    offsets: Vec<P>,
    net_stamp: Vec<u32>,
    link_stamp: Vec<u32>,
    stamp: u32,
    holes: Vec<(P, f64)>,
    cross_w: f64,
    soft_labels: bool,
    hot_gap: f64,
    label_at: Vec<Option<(u8, Bounds)>>,
    copper: Option<(&'a Copper<'a>, Vec<Option<usize>>)>,
    reach: f64,
}

const CELL: f64 = 2.0;

const FREE_CELL: f64 = 0.5;
const MACRO_ROOM: f64 = 1.3;
const CLUSTER_NUDGE: f64 = 0.35;
const SPREAD_ROUNDS: usize = 24;
const SPREAD_GROWTH: f64 = 1.3;
const SOLVE_SWEEPS: usize = 12;
const SPECTRAL_ROUNDS: usize = 60;
const GLOBAL_CROSSING: f64 = 0.02;
const DEPTH_CELL: f64 = 0.5;
const EDGE_CELL: f64 = 1.0;
const FLEX_RAD: f64 = 6.0;
const GRID_MARGIN: f64 = 20.0;

#[derive(Clone, Copy)]
struct MacroEdge {
    to: usize,
    w: f64,
    pin: Option<usize>,
    far: Option<usize>,
}

struct MacroGraph {
    adj: Vec<Vec<MacroEdge>>,
    pairs: Vec<(usize, Option<usize>, usize, Option<usize>)>,
    fixed: Vec<Vec<(f64, P, Option<usize>)>>,
}

struct FreeGrid {
    origin: P,
    nx: usize,
    ny: usize,
    sum: Vec<f64>,
}

impl FreeGrid {
    fn index(&self, v: f64, axis: usize) -> usize {
        let n = if axis == 0 { self.nx } else { self.ny };
        (((v - self.origin[axis]) / FREE_CELL).round().max(0.0) as usize).min(n)
    }

    fn free(&self, r: &Bounds) -> f64 {
        let (x0, x1) = (self.index(r.min[0], 0), self.index(r.max[0], 0));
        let (y0, y1) = (self.index(r.min[1], 1), self.index(r.max[1], 1));
        if x1 <= x0 || y1 <= y0 {
            return 0.0;
        }
        let w = self.nx + 1;
        (self.sum[y1 * w + x1] - self.sum[y0 * w + x1] - self.sum[y1 * w + x0]
            + self.sum[y0 * w + x0])
            * FREE_CELL
            * FREE_CELL
    }
}

#[derive(Clone)]
struct PartGrid {
    origin: P,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<usize>>,
}

impl PartGrid {
    fn new(bb: &Bounds) -> PartGrid {
        let origin = [bb.min[0] - GRID_MARGIN, bb.min[1] - GRID_MARGIN];
        let nx = ((bb.size()[0] + 2.0 * GRID_MARGIN) / CELL).ceil() as usize + 1;
        let ny = ((bb.size()[1] + 2.0 * GRID_MARGIN) / CELL).ceil() as usize + 1;
        PartGrid { origin, nx, ny, cells: vec![Vec::new(); nx * ny] }
    }

    fn cell_of(&self, p: P) -> usize {
        let at = |v: f64, axis: usize, n: usize| {
            ((v - self.origin[axis]) / CELL).floor().clamp(0.0, (n - 1) as f64) as usize
        };
        at(p[1], 1, self.ny) * self.nx + at(p[0], 0, self.nx)
    }

    fn cells(&self, b: &Bounds) -> impl Iterator<Item = usize> + use<> {
        let at = |v: f64, axis: usize, n: usize| {
            ((v - self.origin[axis]) / CELL).floor().clamp(0.0, (n - 1) as f64) as usize
        };
        let (x0, x1) = (at(b.min[0], 0, self.nx), at(b.max[0], 0, self.nx));
        let (y0, y1) = (at(b.min[1], 1, self.ny), at(b.max[1], 1, self.ny));
        let nx = self.nx;
        (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| y * nx + x))
    }
}

#[derive(Clone)]
struct EdgeDepth {
    origin: P,
    nx: usize,
    ny: usize,
    depth: Vec<f64>,
}

impl EdgeDepth {
    fn new(outline: &[P], cutouts: &[Vec<P>], bb: &Bounds) -> EdgeDepth {
        let edge = geom::BoardEdge::new(outline, cutouts);
        let nx = ((bb.size()[0] / DEPTH_CELL).ceil() as usize).max(1);
        let ny = ((bb.size()[1] / DEPTH_CELL).ceil() as usize).max(1);
        let half = DEPTH_CELL * std::f64::consts::FRAC_1_SQRT_2;
        let mut depth = vec![0.0; nx * ny];
        for y in 0..ny {
            for x in 0..nx {
                let q = [
                    bb.min[0] + (x as f64 + 0.5) * DEPTH_CELL,
                    bb.min[1] + (y as f64 + 0.5) * DEPTH_CELL,
                ];
                if edge.contains(q) {
                    depth[y * nx + x] = (edge.distance(q) - half).max(0.0);
                }
            }
        }
        EdgeDepth { origin: bb.min, nx, ny, depth }
    }

    fn cell(&self, v: f64, axis: usize) -> Option<usize> {
        let n = if axis == 0 { self.nx } else { self.ny };
        let k = ((v - self.origin[axis]) / DEPTH_CELL).floor();
        (k >= 0.0 && k < n as f64).then_some(k as usize)
    }

    fn placeable(&self, margin: f64) -> f64 {
        self.depth.iter().filter(|d| **d > margin).count() as f64 * DEPTH_CELL * DEPTH_CELL
    }

    fn at(&self, p: P) -> f64 {
        match (self.cell(p[0], 0), self.cell(p[1], 1)) {
            (Some(x), Some(y)) => self.depth[y * self.nx + x],
            _ => 0.0,
        }
    }

    fn least(&self, b: &Bounds) -> f64 {
        let (Some(x0), Some(x1), Some(y0), Some(y1)) = (
            self.cell(b.min[0], 0),
            self.cell(b.max[0], 0),
            self.cell(b.min[1], 1),
            self.cell(b.max[1], 1),
        ) else {
            return 0.0;
        };
        let mut low = f64::MAX;
        for y in y0..=y1 {
            for x in x0..=x1 {
                low = low.min(self.depth[y * self.nx + x]);
                if low <= 0.0 {
                    return 0.0;
                }
            }
        }
        low
    }
}

struct EdgeIndex {
    origin: P,
    nx: usize,
    ny: usize,
    reach: f64,
    near: Vec<Vec<(P, P)>>,
    rows: Vec<Vec<(usize, P, P)>>,
    rings: usize,
}

impl EdgeIndex {
    fn new(edge: geom::BoardEdge, bb: &Bounds, reach: f64) -> EdgeIndex {
        let origin = [bb.min[0] - reach - EDGE_CELL, bb.min[1] - reach - EDGE_CELL];
        let span = |axis: usize| {
            ((bb.max[axis] - bb.min[axis] + 2.0 * (reach + EDGE_CELL)) / EDGE_CELL).ceil() as usize
                + 1
        };
        let (nx, ny) = (span(0), span(1));
        let mut near = vec![Vec::new(); nx * ny];
        let mut rows = vec![Vec::new(); ny];
        let cell = |v: f64, axis: usize, n: usize| {
            ((v - origin[axis]) / EDGE_CELL).floor().clamp(0.0, (n - 1) as f64) as usize
        };
        let mut rings = 0;
        for (r, ring) in edge.rings().enumerate() {
            rings = r + 1;
            let n = ring.len();
            for k in 0..n {
                let (a, b) = (ring[k], ring[(k + n - 1) % n]);
                let e = ring[(k + 1) % n];
                let (x0, x1) = (a[0].min(e[0]) - reach, a[0].max(e[0]) + reach);
                let (y0, y1) = (a[1].min(e[1]) - reach, a[1].max(e[1]) + reach);
                for y in cell(y0, 1, ny)..=cell(y1, 1, ny) {
                    for x in cell(x0, 0, nx)..=cell(x1, 0, nx) {
                        near[y * nx + x].push((a, e));
                    }
                }
                for row in rows.iter_mut().take(cell(a[1].max(b[1]), 1, ny) + 1).skip(cell(
                    a[1].min(b[1]),
                    1,
                    ny,
                )) {
                    row.push((r, a, b));
                }
            }
        }
        EdgeIndex { origin, nx, ny, reach, near, rows, rings }
    }

    fn cell(&self, v: f64, axis: usize) -> Option<usize> {
        let n = if axis == 0 { self.nx } else { self.ny };
        let k = ((v - self.origin[axis]) / EDGE_CELL).floor();
        (k >= 1.0 && k < (n - 1) as f64).then_some(k as usize)
    }

    fn contains(&self, p: P) -> Option<bool> {
        if self.rings > 64 {
            return None;
        }
        let row = &self.rows[self.cell(p[1], 1)?];
        let mut odd = 0u64;
        for (r, a, b) in row {
            if (a[1] > p[1]) != (b[1] > p[1])
                && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
            {
                odd ^= 1 << r;
            }
        }
        Some(odd == 1)
    }

    fn near(&self, p: P, reach: f64) -> Option<&[(P, P)]> {
        if reach > self.reach {
            return None;
        }
        let (x, y) = (self.cell(p[0], 0)?, self.cell(p[1], 1)?);
        Some(&self.near[y * self.nx + x])
    }

    fn clear(&self, p: P, gap: f64) -> Option<bool> {
        let near = self.near(p, gap)?;
        Some(near.iter().all(|(a, b)| geom::point_segment_distance(p, *a, *b) >= gap))
    }
}

fn spectral(adj: &[Vec<MacroEdge>], variant: u64, wide: bool) -> Vec<P> {
    let m = adj.len();
    let mut comp = vec![usize::MAX; m];
    let mut best: Vec<usize> = Vec::new();
    for s in 0..m {
        if comp[s] != usize::MAX {
            continue;
        }
        let mut members = vec![s];
        comp[s] = s;
        let mut k = 0;
        while k < members.len() {
            for e in &adj[members[k]] {
                if comp[e.to] == usize::MAX {
                    comp[e.to] = s;
                    members.push(e.to);
                }
            }
            k += 1;
        }
        if members.len() > best.len() {
            best = members;
        }
    }
    let mut out = vec![[0.0, 0.0]; m];
    if best.len() < 3 {
        return out;
    }
    best.sort_unstable();
    let local: HashMap<usize, usize> = best.iter().enumerate().map(|(x, a)| (*a, x)).collect();
    let c = best.len();
    let deg: Vec<f64> = best.iter().map(|a| adj[*a].iter().map(|e| e.w).sum()).collect();
    let eps = 1e-3 * deg.iter().sum::<f64>() / c as f64;
    let mut rng = Rng(0x5eed);
    let mut v: Vec<Vec<f64>> = (0..2).map(|_| (0..c).map(|_| rng.unit() - 0.5).collect()).collect();
    let tidy = |v: &mut Vec<Vec<f64>>| {
        for k in 0..2 {
            let mean = v[k].iter().sum::<f64>() / c as f64;
            v[k].iter_mut().for_each(|x| *x -= mean);
            if k == 1 {
                let (first, rest) = v.split_at_mut(1);
                let d: f64 = first[0].iter().zip(&rest[0]).map(|(p, q)| p * q).sum();
                rest[0].iter_mut().zip(&first[0]).for_each(|(q, p)| *q -= d * p);
            }
            let norm = v[k].iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
            v[k].iter_mut().for_each(|x| *x /= norm);
        }
    };
    tidy(&mut v);
    for _ in 0..SPECTRAL_ROUNDS {
        for vk in v.iter_mut() {
            let b = vk.clone();
            for _ in 0..4 {
                for x in 0..c {
                    let s: f64 = adj[best[x]].iter().map(|e| e.w * vk[local[&e.to]]).sum();
                    vk[x] = (b[x] + s) / (deg[x] + eps);
                }
            }
        }
        tidy(&mut v);
    }
    let rank = |v: &[f64]| -> Vec<f64> {
        let mut order: Vec<usize> = (0..c).collect();
        order.sort_by(|a, b| v[*a].total_cmp(&v[*b]).then(a.cmp(b)));
        let mut r = vec![0.0; c];
        for (k, x) in order.into_iter().enumerate() {
            r[x] = 2.0 * k as f64 / (c - 1) as f64 - 1.0;
        }
        r
    };
    let (r0, r1) = (rank(&v[0]), rank(&v[1]));
    let flip =
        [if variant & 1 == 1 { -1.0 } else { 1.0 }, if variant & 2 == 2 { -1.0 } else { 1.0 }];
    let swap = (variant & 4 == 4) == wide;
    for (x, a) in best.iter().enumerate() {
        let (p, q) = (r0[x], r1[x]);
        let (px, py) = if swap { (q, p) } else { (p, q) };
        out[*a] = [px * flip[0], py * flip[1]];
    }
    out
}

impl<'a> Placer<'a> {
    fn shapes(&self, i: usize, st: St) -> Vec<WShape> {
        let p = &self.parts[i];
        let t = st.transform();
        let shape = |poly: &[P], side: u8, label: bool| {
            let world: Vec<P> = poly.iter().map(|q| t.apply(*q)).collect();
            WShape { side, b: poly_bounds(&world), rect: is_axis_rect(&world), poly: world, label }
        };
        let mut out: Vec<WShape> = p
            .loops
            .iter()
            .map(|(back, poly)| {
                let side = if p.through {
                    3
                } else if *back != st.bottom {
                    2
                } else {
                    1
                };
                shape(poly, side, false)
            })
            .collect();
        if let Some(label) = p.label.as_ref().filter(|_| !self.soft_labels) {
            out.push(shape(&label.poly, if st.bottom { 2 } else { 1 }, true));
        }
        out
    }

    fn soft_label(&self, i: usize, st: St) -> Option<(u8, Bounds)> {
        let label = self.parts[i].label.as_ref().filter(|_| self.soft_labels)?;
        let t = st.transform();
        let mut b = Bounds::EMPTY;
        label.poly.iter().for_each(|q| b.add(t.apply(*q)));
        Some((if st.bottom { 2 } else { 1 }, b))
    }

    fn label_overlap(&self, i: usize) -> f64 {
        let grid = &self.grid;
        let area = |c: usize, a: &Bounds, b: &Bounds| {
            let lo = [a.min[0].max(b.min[0]), a.min[1].max(b.min[1])];
            let w = a.max[0].min(b.max[0]) - lo[0];
            let h = a.max[1].min(b.max[1]) - lo[1];
            if w > 0.0 && h > 0.0 && grid.cell_of(lo) == c { w * h } else { 0.0 }
        };
        let mut sum = 0.0;
        if let Some((side, lb)) = self.label_at[i] {
            for c in grid.cells(&lb) {
                for &j in grid.cells[c].iter().filter(|j| **j != i) {
                    for t in self.cache[j].iter().filter(|t| t.side & side != 0) {
                        sum += area(c, &lb, &t.b);
                    }
                    if let Some((sj, bj)) = self.label_at[j]
                        && sj == side
                    {
                        sum += area(c, &lb, &bj);
                    }
                }
            }
        }
        for s in &self.cache[i] {
            for c in grid.cells(&s.b) {
                for &j in grid.cells[c].iter().filter(|j| **j != i) {
                    if let Some((sj, bj)) = self.label_at[j]
                        && s.side & sj != 0
                    {
                        sum += area(c, &s.b, &bj);
                    }
                }
            }
        }
        sum
    }

    fn insert(&mut self, i: usize) {
        let sh = self.shapes(i, self.parts[i].st);
        for s in &sh {
            for c in self.grid.cells(&s.b) {
                let v = &mut self.grid.cells[c];
                if !v.contains(&i) {
                    v.push(i);
                }
            }
        }
        self.cache[i] = sh;
        self.label_at[i] = self.soft_label(i, self.parts[i].st);
        if let Some((_, b)) = self.label_at[i] {
            for c in self.grid.cells(&b) {
                let v = &mut self.grid.cells[c];
                if !v.contains(&i) {
                    v.push(i);
                }
            }
        }
        let t = self.parts[i].st.transform();
        self.pad_at[i] = self.parts[i].pads.iter().map(|q| t.apply(q.c)).collect();
        self.parts[i].placed = true;
        self.update_segments(i);
    }

    fn remove(&mut self, i: usize) {
        for s in std::mem::take(&mut self.cache[i]) {
            for c in self.grid.cells(&s.b) {
                self.grid.cells[c].retain(|j| *j != i);
            }
        }
        if let Some((_, b)) = self.label_at[i].take() {
            for c in self.grid.cells(&b) {
                self.grid.cells[c].retain(|j| *j != i);
            }
        }
        self.parts[i].placed = false;
        self.update_segments(i);
    }

    fn update_segments(&mut self, i: usize) {
        for k in 0..self.part_nets[i].len() {
            let n = self.part_nets[i][k];
            if self.is_two_pin[n] {
                let pins = &self.nets[n].pins;
                let (a, b) = (pins[0], pins[1]);
                let seg = (self.parts[a.0].placed && self.parts[b.0].placed)
                    .then(|| (self.pad_pos(a.0, a.1), self.pad_pos(b.0, b.1)));
                self.segments[self.two_pin_at[n]] = seg;
            }
        }
    }

    fn board_edge(&self) -> geom::BoardEdge<'_> {
        geom::BoardEdge::new(&self.b.outline, &self.b.cutouts)
    }

    fn edge_gap(&self, p: P) -> f64 {
        self.board_edge().distance(p)
    }

    fn clear_of_cutouts(&self, poly: &[P], min: f64) -> bool {
        let b = poly_bounds(poly);
        self.b.cutouts.iter().all(|c| {
            let cb = poly_bounds(c);
            b.min[0] > cb.max[0] + min
                || cb.min[0] > b.max[0] + min
                || b.min[1] > cb.max[1] + min
                || cb.min[1] > b.max[1] + min
                || {
                    let d = geom::polygon_distance(poly, c);
                    d > 0.0 && d >= min - 1e-6
                }
        })
    }

    fn edge_clear(&self, edge: geom::BoardEdge, v: P, min: f64) -> bool {
        let depth = self.b.depth.at(v);
        if depth > 0.0 && depth >= min - 1e-6 {
            return true;
        }
        self.on_board(edge, v)
            && self.b.edges.clear(v, min - 1e-6).unwrap_or_else(|| edge.distance(v) >= min - 1e-6)
    }

    fn on_board(&self, edge: geom::BoardEdge, v: P) -> bool {
        self.b.edges.contains(v).unwrap_or_else(|| edge.contains(v))
    }

    fn inside(&self, i: usize, st: St, sh: &[WShape]) -> bool {
        let part = &self.parts[i];
        let o = &self.b.outline;
        if o.len() < 3 {
            return true;
        }
        let t = st.transform();
        let edge = self.board_edge();
        let pads_in = |min: f64, skip_edge: bool| {
            part.pads.iter().filter(|q| !(skip_edge && q.edge)).all(|q| {
                q.outline.iter().all(|ring| {
                    let w: Vec<P> = ring.iter().map(|v| t.apply(*v)).collect();
                    w.iter().all(|v| self.edge_clear(edge, *v, min))
                        && self.clear_of_cutouts(&w, min)
                })
            })
        };
        let body_in = |min: f64| {
            sh.iter().filter(|s| !s.label).all(|s| {
                s.poly.iter().all(|v| self.edge_clear(edge, *v, min))
                    && !o.iter().any(|v| near_box(&s.b, *v) && geom::point_in_polygon(*v, &s.poly))
                    && self.clear_of_cutouts(&s.poly, min)
            })
        };
        let loose = !matches!(part.role, Role::Connector | Role::Hole | Role::Fiducial);
        let deep = loose && {
            let mut wb = Bounds::EMPTY;
            let e = &part.extent;
            for q in [e.min, [e.max[0], e.min[1]], e.max, [e.min[0], e.max[1]]] {
                wb.add(t.apply(q));
            }
            self.b.depth.least(&wb) > self.b.body_edge.max(self.b.part_edge) + EPS
        };
        let ok = deep
            || match part.role {
                Role::Connector if edge_mount(part.fp) => {
                    part.pads.iter().filter(|q| !q.edge).all(|q| self.on_board(edge, t.apply(q.c)))
                }
                Role::Hole => body_in(0.0) && pads_in(self.b.copper_edge, false),
                Role::Fiducial => body_in(0.0) && pads_in(FIDUCIAL_TO_EDGE, false),
                _ => {
                    body_in(self.b.body_edge)
                        && ((part.pads_in_court && self.b.body_edge >= self.b.part_edge)
                            || pads_in(self.b.part_edge, true))
                }
            };
        let label_in = deep
            || sh.iter().filter(|s| s.label).all(|s| {
                s.poly.iter().all(|v| self.on_board(edge, *v))
                    && self.clear_of_cutouts(&s.poly, 0.0)
            });
        ok && label_in
            && !sh.iter().any(|s| {
                (!s.label
                    && self.b.keepouts.iter().any(|k| {
                        strict_overlap(&s.b, &poly_bounds(k)) && polys_overlap(&s.poly, k)
                    }))
                    || !part.over_silk
                        && self.b.silk.iter().any(|k| {
                            s.side & k.side != 0
                                && strict_overlap(&s.b, &k.b)
                                && ((s.rect && k.rect) || polys_overlap(&s.poly, &k.poly))
                        })
            })
    }

    fn clashes(&self, i: usize, sh: &[WShape], skip: &[usize]) -> bool {
        for s in sh {
            for c in self.grid.cells(&s.b) {
                let v = &self.grid.cells[c];
                for &j in v {
                    if j == i || skip.contains(&j) {
                        continue;
                    }
                    for t in &self.cache[j] {
                        if s.side & t.side != 0
                            && strict_overlap(&s.b, &t.b)
                            && ((s.rect && t.rect) || polys_overlap(&s.poly, &t.poly))
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn legal_shape(&self, i: usize, st: St, skip: &[usize]) -> bool {
        let sh = self.shapes(i, st);
        self.inside(i, st, &sh) && !self.clashes(i, &sh, skip) && !self.too_hot(i, &sh, skip)
    }

    fn legal(&self, i: usize, st: St, skip: &[usize]) -> bool {
        let sh = self.shapes(i, st);
        self.inside(i, st, &sh)
            && !self.clashes(i, &sh, skip)
            && !self.too_hot(i, &sh, skip)
            && self.copper_ok(i, st, &sh, skip)
    }

    fn copper_ok(&self, i: usize, st: St, sh: &[WShape], skip: &[usize]) -> bool {
        let Some((cu, at)) = &self.copper else { return true };
        let Some(me) = at[i] else { return true };
        let mv = |k: usize, st: St| crate::rules::Move {
            part: k,
            at: st.at,
            rotation: st.rot,
            bottom: st.bottom,
        };
        let mut own = Bounds::EMPTY;
        sh.iter().filter(|s| !s.label).for_each(|s| own.union(&s.b));
        let reach = self.reach + 1.0;
        let near = Bounds {
            min: [own.min[0] - reach, own.min[1] - reach],
            max: [own.max[0] + reach, own.max[1] + reach],
        };
        if !cu.separates {
            let t = st.transform();
            let mut pads = Bounds::EMPTY;
            for q in &self.parts[i].pads {
                q.outline.iter().flatten().for_each(|p| pads.add(t.apply(*p)));
            }
            if pads.is_empty() || !cu.fixed_near(&pads, self.reach) {
                return true;
            }
        }
        let mut kept = crate::rules::Plan::default();
        let mut seen = Vec::new();
        for c in self.grid.cells(&near) {
            for &j in &self.grid.cells[c] {
                if j == i || skip.contains(&j) || seen.contains(&j) {
                    continue;
                }
                seen.push(j);
                let p = &self.parts[j];
                if let (true, false, Some(k)) = (p.placed, p.fixed, at[j]) {
                    kept.parts.push(mv(k, p.st));
                }
            }
        }
        let plan = crate::rules::Plan { parts: vec![mv(me, st)], ..Default::default() };
        let planned = crate::rules::Planned::hiding(&cu.base, &cu.hidden, &kept, plan);
        crate::rules::legal(&planned).is_ok()
    }

    fn too_hot(&self, i: usize, sh: &[WShape], skip: &[usize]) -> bool {
        if !self.parts[i].spread_hot {
            return false;
        }
        let mut own = Bounds::EMPTY;
        sh.iter().filter(|s| !s.label).for_each(|s| own.union(&s.b));
        (0..self.parts.len()).any(|j| {
            if j == i || skip.contains(&j) || !self.parts[j].placed || !self.parts[j].hot {
                return false;
            }
            let mut other = Bounds::EMPTY;
            self.cache[j].iter().filter(|s| !s.label).for_each(|s| other.union(&s.b));
            let dx = (own.min[0] - other.max[0]).max(other.min[0] - own.max[0]).max(0.0);
            let dy = (own.min[1] - other.max[1]).max(other.min[1] - own.max[1]).max(0.0);
            dx.hypot(dy) < self.hot_gap + GRID
        })
    }

    fn nearest(
        &self,
        i: usize,
        target: P,
        rot: f64,
        bottom: bool,
        max_r: f64,
        extra: &dyn Fn(St) -> f64,
    ) -> Option<St> {
        let target = snap_p(target);
        let rel = self.shapes(i, St { at: [0.0, 0.0], rot, bottom });
        let mut hull = Bounds::EMPTY;
        rel.iter().filter(|s| !s.label).for_each(|s| hull.union(&s.b));
        let bb = &self.b.bounds;
        let loose = !matches!(self.parts[i].role, Role::Connector | Role::Hole | Role::Fiducial);
        let margin = if loose { self.b.body_edge } else { 0.0 };
        let mut best: Option<(f64, St)> = None;
        let mut limit = f64::MAX;
        for off in &self.offsets {
            let r = off[0].hypot(off[1]);
            if r > limit || r > max_r {
                break;
            }
            let at = snap_p([target[0] + off[0], target[1] + off[1]]);
            if loose
                && (at[0] + hull.min[0] < bb.min[0] + margin - EPS
                    || at[0] + hull.max[0] > bb.max[0] - margin + EPS
                    || at[1] + hull.min[1] < bb.min[1] + margin - EPS
                    || at[1] + hull.max[1] > bb.max[1] - margin + EPS)
            {
                continue;
            }
            if self.clashes_boxes(i, &rel, at) {
                continue;
            }
            let st = St { at, rot, bottom };
            if !self.legal(i, st, &[]) {
                continue;
            }
            let c = r + extra(st);
            if best.as_ref().is_none_or(|b| c < b.0) {
                best = Some((c, st));
            }
            if limit == f64::MAX {
                limit = r + 0.6;
            }
        }
        best.map(|b| b.1)
    }

    fn clashes_boxes(&self, i: usize, rel: &[WShape], at: P) -> bool {
        for s in rel.iter().filter(|s| s.rect) {
            let mut b = s.b;
            b.min = [b.min[0] + at[0], b.min[1] + at[1]];
            b.max = [b.max[0] + at[0], b.max[1] + at[1]];
            for c in self.grid.cells(&b) {
                let v = &self.grid.cells[c];
                for &j in v {
                    if j != i
                        && self.cache[j]
                            .iter()
                            .any(|t| s.side & t.side != 0 && t.rect && strict_overlap(&b, &t.b))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn pad_pos(&self, i: usize, k: usize) -> P {
        if self.parts[i].placed {
            self.pad_at[i][k]
        } else {
            self.parts[i].st.transform().apply(self.parts[i].pads[k].c)
        }
    }

    fn centre(&self, i: usize) -> P {
        let p = &self.parts[i];
        p.st.transform().apply(p.local.center())
    }

    fn end_pos(&self, e: End) -> P {
        match e {
            End::Pad(i, k) => self.pad_pos(i, k),
            End::Centre(i) => self.centre(i),
        }
    }

    fn end_part(e: End) -> usize {
        match e {
            End::Pad(i, _) | End::Centre(i) => i,
        }
    }

    fn net_hpwl(&self, n: usize) -> f64 {
        let mut b = Bounds::EMPTY;
        let mut count = 0;
        for &(i, k) in &self.nets[n].pins {
            if self.parts[i].placed {
                b.add(self.pad_pos(i, k));
                count += 1;
            }
        }
        if count < 2 { 0.0 } else { b.size()[0] + b.size()[1] }
    }

    fn crossings_of(&self, n: usize) -> usize {
        let own = self.two_pin_at[n];
        let Some((a, b)) = self.segments[own] else { return 0 };
        let mut bb = Bounds::EMPTY;
        bb.add(a);
        bb.add(b);
        let mut count = 0;
        for (x, seg) in self.segments.iter().enumerate() {
            if x == own {
                continue;
            }
            let Some((c, d)) = *seg else { continue };
            if c[0].max(d[0]) < bb.min[0]
                || c[0].min(d[0]) > bb.max[0]
                || c[1].max(d[1]) < bb.min[1]
                || c[1].min(d[1]) > bb.max[1]
            {
                continue;
            }
            if geom::segments_intersect(a, b, c, d) {
                count += 1;
            }
        }
        count
    }

    fn link_cost(&self, l: &Link) -> f64 {
        if !self.parts[Self::end_part(l.a)].placed || !self.parts[Self::end_part(l.b)].placed {
            return 0.0;
        }
        let d = geom::dist(self.end_pos(l.a), self.end_pos(l.b));
        match l.kind {
            LinkKind::Pull { w, thr, extra, near } => {
                w * (d - near).max(0.0) + extra * (d - thr).max(0.0)
            }
            LinkKind::Repel { w, thr } => w * (thr - d).max(0.0),
        }
    }

    fn unary(&self, i: usize) -> f64 {
        let p = &self.parts[i];
        if !p.placed {
            return 0.0;
        }
        let c = self.centre(i);
        let mut cost = 0.0;
        if p.large {
            cost += 0.5 * geom::dist(c, self.b.centre);
        }
        cost + self.flex_penalty(i, p.st)
    }

    fn hole_of(&self, i: usize) -> (P, f64) {
        let p = &self.parts[i];
        let t = p.st.transform();
        p.fp.pads
            .iter()
            .filter_map(|q| {
                let d = q.drill.as_ref()?.min().to_mm();
                let at = q.at.to_mm();
                let off = q.drill_offset.to_mm();
                Some((t.apply([at[0] + off[0], at[1] + off[1]]), d / 2.0))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((self.centre(i), 0.0))
    }

    fn flex_hit(&self, i: usize, st: St) -> Option<bool> {
        let p = &self.parts[i];
        p.mlcc_len?;
        let (ka, kb) = p.ends?;
        let t = st.transform();
        let (a, b) = (t.apply(p.pads[ka].c), t.apply(p.pads[kb].c));
        let c = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let reach = geom::dist(a, b) + p.local.size()[0].max(p.local.size()[1]);
        let zone = self.b.flex;
        let bb = &self.b.bounds;
        let deep = c[0] - bb.min[0] > zone + reach
            && bb.max[0] - c[0] > zone + reach
            && c[1] - bb.min[1] > zone + reach
            && bb.max[1] - c[1] > zone + reach
            && self.b.cutouts.is_empty();
        let clear = deep
            || self.b.depth.at(c) > zone + reach
            || match self.b.flex_edges.near(c, zone + reach) {
                Some(near) => {
                    near.iter().all(|(s, e)| geom::point_segment_distance(c, *s, *e) > zone + reach)
                }
                None => self.edge_gap(c) > zone + reach,
            };
        if clear && self.holes.iter().all(|(h, r)| geom::dist(*h, c) > zone + reach + r) {
            return None;
        }
        let rings: Vec<Vec<P>> = [ka, kb]
            .iter()
            .flat_map(|k| p.pads[*k].outline.iter())
            .map(|r| r.iter().map(|v| t.apply(*v)).collect())
            .collect();
        let rad = rings.iter().flatten().map(|v| geom::dist(*v, c)).fold(0.0, f64::max);
        let mut best: Option<(f64, P)> = None;
        let segments: Vec<(P, P)> = match self.b.flex_edges.near(c, zone + rad) {
            Some(near) => near.to_vec(),
            None => self.board_edge().segments().collect(),
        };
        for (s, e) in segments {
            let bound = geom::point_segment_distance(c, s, e) - rad;
            if bound >= best.map_or(zone, |b| b.0.min(zone)) {
                continue;
            }
            let gap = rings
                .iter()
                .flat_map(|o| (0..o.len()).map(move |k| (o[k], o[(k + 1) % o.len()])))
                .map(|(u, v)| geom::segment_segment_distance(u, v, s, e))
                .fold(f64::MAX, f64::min);
            if best.is_none_or(|b| gap < b.0) {
                let (dx, dy) = (e[0] - s[0], e[1] - s[1]);
                let l2 = dx * dx + dy * dy;
                let f = if l2 == 0.0 {
                    0.0
                } else {
                    (((c[0] - s[0]) * dx + (c[1] - s[1]) * dy) / l2).clamp(0.0, 1.0)
                };
                best = Some((gap, [s[0] + f * dx, s[1] + f * dy]));
            }
        }
        for &(h, r) in &self.holes {
            if geom::dist(h, c) - rad - r >= best.map_or(zone, |b| b.0.min(zone)) {
                continue;
            }
            let gap = rings
                .iter()
                .map(|o| geom::polyline_polygon_distance(&[h, h], o) - r)
                .fold(f64::MAX, f64::min)
                .max(0.0);
            if best.is_none_or(|b| gap < b.0) {
                best = Some((gap, h));
            }
        }
        let (gap, at) = best?;
        if gap + 1e-6 >= zone {
            return None;
        }
        let d = geom::dist(at, c);
        let l = geom::dist(a, b).max(1e-9);
        let axis = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
        Some(
            d > 1e-6
                && dot([at[0] - c[0], at[1] - c[1]], axis).abs() / d
                    > std::f64::consts::FRAC_1_SQRT_2 - 0.05,
        )
    }

    fn axis(&self, i: usize) -> Option<P> {
        let (ka, kb) = self.parts[i].ends?;
        let (a, b) = (self.pad_pos(i, ka), self.pad_pos(i, kb));
        let d = geom::dist(a, b);
        (d > 1e-6).then(|| [(b[0] - a[0]) / d, (b[1] - a[1]) / d])
    }

    fn local_cost(&mut self, set: &[usize]) -> f64 {
        self.stamp += 1;
        let s = self.stamp;
        let mut cost = 0.0;
        for &i in set {
            for k in 0..self.part_nets[i].len() {
                let n = self.part_nets[i][k];
                if self.net_stamp[n] == s {
                    continue;
                }
                self.net_stamp[n] = s;
                let net = &self.nets[n];
                if net.power || net.weight == 0.0 {
                    continue;
                }
                cost += net.weight * self.net_hpwl(n);
                if self.is_two_pin[n] {
                    cost += self.cross_w * self.crossings_of(n) as f64;
                }
            }
            for k in 0..self.part_links[i].len() {
                let l = self.part_links[i][k];
                if self.link_stamp[l] == s {
                    continue;
                }
                self.link_stamp[l] = s;
                cost += self.link_cost(&self.links[l]);
            }
            cost += self.unary(i);
            if self.soft_labels {
                cost += LABEL_WEIGHT * self.label_overlap(i);
            }
        }
        cost
    }

    fn metrics(&self) -> Metrics {
        let mut m = Metrics::default();
        for n in 0..self.nets.len() {
            let h = self.net_hpwl(n);
            m.hpwl_all_mm += h;
            if !self.nets[n].power {
                m.hpwl_mm += h;
            }
        }
        let mut c = 0;
        for &n in &self.two_pin {
            c += self.crossings_of(n);
        }
        m.crossings = c / 2;
        for i in 0..self.parts.len() {
            if !self.parts[i].placed {
                continue;
            }
            for j in i + 1..self.parts.len() {
                if !self.parts[j].placed {
                    continue;
                }
                let hit = self.cache[i].iter().filter(|s| !s.label).any(|s| {
                    self.cache[j].iter().filter(|t| !t.label).any(|t| {
                        s.side & t.side != 0
                            && strict_overlap(&s.b, &t.b)
                            && ((s.rect && t.rect) || polys_overlap(&s.poly, &t.poly))
                    })
                });
                if hit {
                    m.overlaps += 1;
                }
            }
        }
        let mut ds = Vec::new();
        for cl in &self.clusters {
            for (&cap, &(k, pin)) in &cl.served {
                if self.parts[cap].placed && self.parts[cl.anchor].placed {
                    ds.push(geom::dist(self.pad_pos(cap, k), self.pad_pos(cl.anchor, pin)));
                }
            }
        }
        if !ds.is_empty() {
            m.decap_mean_mm = ds.iter().sum::<f64>() / ds.len() as f64;
            m.decap_max_mm = ds.iter().copied().fold(0.0, f64::max);
        }
        m.hpwl_mm = (m.hpwl_mm * 10.0).round() / 10.0;
        m.hpwl_all_mm = (m.hpwl_all_mm * 10.0).round() / 10.0;
        m.decap_mean_mm = (m.decap_mean_mm * 100.0).round() / 100.0;
        m.decap_max_mm = (m.decap_max_mm * 100.0).round() / 100.0;
        m
    }
}

fn offsets(reach: f64) -> Vec<P> {
    let mut out: Vec<P> = Vec::new();
    let rings = [(0.25, 6.0), (0.5, 20.0), (1.0, reach.max(20.0))];
    let mut inner = 0.0;
    for (step, r) in rings {
        let n = (r / step).ceil() as i64;
        for x in -n..=n {
            for y in -n..=n {
                let p = [x as f64 * step, y as f64 * step];
                let d = p[0].hypot(p[1]);
                if d <= r && (d > inner || (inner == 0.0 && d == 0.0)) {
                    out.push(p);
                }
            }
        }
        inner = r;
    }
    out.sort_by(|a, b| {
        let (da, db) = (a[0].hypot(a[1]), b[0].hypot(b[1]));
        da.total_cmp(&db).then(a[1].atan2(a[0]).total_cmp(&b[1].atan2(b[0])))
    });
    out.dedup();
    out
}

struct EdgeGeom {
    u: P,
    e_loc: f64,
}

fn edge_geom(p: &Part, part_edge: f64, body_edge: f64) -> EdgeGeom {
    let fp = p.fp;
    let cu: Vec<&LPad> = p.pads.iter().filter(|q| !q.outline.is_empty()).collect();
    let mut pc = [0.0, 0.0];
    for q in &cu {
        pc[0] += q.c[0] / cu.len().max(1) as f64;
        pc[1] += q.c[1] / cu.len().max(1) as f64;
    }
    let axis_dir = |d: P| -> P {
        if d[0].abs() >= d[1].abs() { [d[0].signum(), 0.0] } else { [0.0, d[1].signum()] }
    };
    let max_along =
        |u: P, pts: &mut dyn Iterator<Item = P>| pts.map(|v| dot(v, u)).fold(f64::MIN, f64::max);
    let edge_text = fp.graphics.iter().find_map(|g| match &g.shape {
        Shape::Text { at, text, .. }
            if g.layer == "Dwgs.User" && text.to_ascii_lowercase().contains("edge") =>
        {
            Some(at.to_mm())
        }
        _ => None,
    });
    if let Some(tp) = edge_text {
        let line = fp
            .graphics
            .iter()
            .filter(|g| g.layer == "Dwgs.User")
            .filter_map(|g| match &g.shape {
                Shape::Line { start, end } => Some((start.to_mm(), end.to_mm())),
                _ => None,
            })
            .filter(|(a, b)| (a[0] - b[0]).abs() < 1e-6 || (a[1] - b[1]).abs() < 1e-6)
            .min_by(|x, y| {
                geom::point_segment_distance(tp, x.0, x.1)
                    .total_cmp(&geom::point_segment_distance(tp, y.0, y.1))
            });
        if let Some((a, b)) = line {
            let horizontal = (a[1] - b[1]).abs() < 1e-6;
            let (c, u) = if horizontal {
                (a[1], [0.0, (a[1] - pc[1]).signum()])
            } else {
                (a[0], [(a[0] - pc[0]).signum(), 0.0])
            };
            if u != [0.0, 0.0] {
                let e_loc = if horizontal { c * u[1] } else { c * u[0] };
                return EdgeGeom { u, e_loc };
            }
        }
    }
    let cc = p.local.center();
    let mut u = axis_dir([cc[0] - pc[0], cc[1] - pc[1]]);
    if u == [0.0, 0.0] || (cc[0] - pc[0]).hypot(cc[1] - pc[1]) < 0.2 {
        let s = p.local.size();
        u = if s[0] >= s[1] { [0.0, 1.0] } else { [1.0, 0.0] };
    }
    let snap_up = |v: f64| (v / GRID).ceil() * GRID;
    if p.pads.iter().any(|q| q.edge) {
        let e_loc = max_along(
            u,
            &mut p.pads.iter().filter(|q| q.edge).flat_map(|q| q.outline.iter().flatten().copied()),
        );
        return EdgeGeom { u, e_loc };
    }
    if fp.overhang {
        let e_loc =
            max_along(u, &mut p.pads.iter().flat_map(|q| q.outline.iter().flatten().copied()))
                + part_edge;
        return EdgeGeom { u, e_loc: snap_up(e_loc) };
    }
    let e_loc = max_along(u, &mut p.loops.iter().flat_map(|l| l.1.iter().copied())) + body_edge;
    EdgeGeom { u, e_loc: snap_up(e_loc) }
}

fn edge_level(o: &[P], n: P, tg: P, lo: f64, hi: f64) -> Option<f64> {
    let mut samples = vec![lo, hi];
    samples.extend(o.iter().map(|q| dot(*q, tg)).filter(|t| *t > lo && *t < hi));
    let mut level = f64::MAX;
    for t in samples {
        let mut out = f64::MIN;
        for k in 0..o.len() {
            let (a, b) = (o[k], o[(k + 1) % o.len()]);
            let (ta, tb) = (dot(a, tg), dot(b, tg));
            if (ta - t).abs() < EPS && (tb - t).abs() < EPS {
                out = out.max(dot(a, n)).max(dot(b, n));
            } else if (ta - t) * (tb - t) <= 0.0 {
                let f = (t - ta) / (tb - ta);
                out = out.max(dot([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f], n));
            }
        }
        if out == f64::MIN {
            return None;
        }
        level = level.min(out);
    }
    Some(level)
}

fn rotation_for(u: P, n: P, bottom: bool) -> f64 {
    for r in [0.0, 90.0, 180.0, 270.0] {
        let t = Transform { at: [0.0, 0.0], rotation: r, mirror: bottom };
        let w = t.direction(u);
        if (w[0] - n[0]).abs() < 1e-6 && (w[1] - n[1]).abs() < 1e-6 {
            return r;
        }
    }
    0.0
}

pub fn place<'a>(input: &PlaceInput<'a>, opts: &PlaceOptions) -> Result<PlaceResult, String> {
    let geo = rf::geometry(input);
    let chains = rf::chains(input, &geo);
    if chains.is_empty() {
        return place_once(input, opts);
    }
    let chosen = |r: &str| opts.parts.is_empty() || opts.parts.iter().any(|g| glob(g, r));
    let fixed = |r: &str| {
        input
            .placements
            .iter()
            .find(|f| f.reference == r)
            .is_some_and(|f| f.locked || opts.keep_placed || !chosen(r))
            || (input.placements.iter().all(|f| f.reference != r) && !chosen(r))
    };
    let mut spec = input.spec.clone();
    for c in &chains {
        for (r, e) in rf::edges_for(c, input, &geo) {
            if !fixed(&r) {
                spec.edges.insert(r, e);
            }
        }
    }
    let first = place_once(&with_spec(input, input.placements, &spec), opts)?;
    let pose = |r: &str| -> Option<rf::Pose> {
        if let Some(p) = first.placements.iter().find(|p| p.reference == r) {
            return Some(rf::Pose { at: p.at, rot: p.rotation, movable: !p.bottom && !fixed(r) });
        }
        input.placements.iter().find(|f| f.reference == r).map(|f| rf::Pose {
            at: f.at.to_mm(),
            rot: f.rotation.unwrap_or(0.0),
            movable: false,
        })
    };
    let mut taken: Vec<Bounds> = input
        .placements
        .iter()
        .filter(|f| fixed(&f.reference) && f.side != Some(BoardSide::Bottom))
        .filter_map(|f| {
            rf::footprint_box(&geo, &f.reference, f.at.to_mm(), f.rotation.unwrap_or(0.0))
        })
        .collect();
    let mut laid: Vec<Placement> = Vec::new();
    for c in &chains {
        let one = rf::lay_out(c, input, &geo, &pose, input.outline);
        laid.extend(rf::prune(one, &geo, &mut taken, input));
    }
    if laid.is_empty() {
        return Ok(first);
    }
    let mut placements: Vec<PlacementFile> = input
        .placements
        .iter()
        .filter(|f| !laid.iter().any(|l| l.reference == f.reference))
        .cloned()
        .collect();
    for l in &laid {
        placements.push(PlacementFile {
            reference: l.reference.clone(),
            at: crate::units::Point::mm(l.at[0], l.at[1]),
            rotation: Some(l.rotation),
            side: None,
            label: None,
            mlcc: None,
            locked: true,
        });
    }
    let mut second = place_once(&with_spec(input, &placements, &spec), opts)?;
    second.kept.retain(|k| !laid.iter().any(|l| &l.reference == k));
    second.placements.retain(|p| !laid.iter().any(|l| l.reference == p.reference));
    for l in &mut laid {
        let Some(was) = first.placements.iter().find(|p| p.reference == l.reference) else {
            continue;
        };
        l.label = was.label.map(|(at, rot)| {
            let off =
                geom::rotate([at[0] - was.at[0], at[1] - was.at[1]], l.rotation - was.rotation);
            let snap = |v: f64| (v / GRID).round() * GRID;
            (
                [snap(l.at[0] + off[0]), snap(l.at[1] + off[1])],
                (rot + l.rotation - was.rotation).rem_euclid(180.0),
            )
        });
        if let Some(e) = first.edges.get(&l.reference) {
            second.edges.insert(l.reference.clone(), *e);
        }
    }
    second.placements.extend(laid);
    Ok(second)
}

fn with_spec<'b>(
    input: &PlaceInput<'b>,
    placements: &'b [PlacementFile],
    spec: &'b PlaceFile,
) -> PlaceInput<'b> {
    PlaceInput {
        board: input.board,
        outline: input.outline,
        cutouts: input.cutouts,
        schematic: input.schematic,
        footprints: input.footprints,
        placements,
        spec,
        fast_nets: input.fast_nets.clone(),
        heat: input.heat.clone(),
        silk: input.silk.clone(),
        texts: input.texts.clone(),
        tracks: input.tracks,
        vias: input.vias,
    }
}

struct Copper<'a> {
    base: crate::rules::Placed<'a>,
    index: HashMap<String, usize>,
    hidden: Vec<usize>,
    hide: Vec<bool>,
    separates: bool,
}

impl Copper<'_> {
    fn fixed_near(&self, b: &Bounds, reach: f64) -> bool {
        use crate::drc::{HoleOf, Owner};
        use crate::rules::Context;
        let cx = &self.base;
        cx.items_near(b, reach)
            .into_iter()
            .any(|i| !matches!(cx.item(i).owner, Owner::Pad(p, _) if self.hide[p]))
            || cx
                .holes_near(b, reach)
                .into_iter()
                .any(|i| !matches!(cx.hole(i).of, HoleOf::Pad(p, _) if self.hide[p]))
    }

    fn reach(&self) -> f64 {
        use crate::rules::Context;
        let cx = &self.base;
        let nets = (0..cx.nets().len()).map(|n| cx.spacing().reach(Some(n)));
        let barriers = cx
            .board()
            .barriers
            .iter()
            .flat_map(|b| [b.clearance, b.creepage])
            .flatten()
            .map(Length::to_mm);
        crate::rules::clearance::reach(cx).max(nets.chain(barriers).fold(0.0, f64::max))
    }
}

fn moving(input: &PlaceInput, opts: &PlaceOptions, reference: &str) -> bool {
    let chosen = opts.parts.is_empty() || opts.parts.iter().any(|g| glob(g, reference));
    !input
        .placements
        .iter()
        .find(|f| f.reference == reference)
        .is_some_and(|f| f.locked || opts.keep_placed || !chosen)
}

fn place_once(input: &PlaceInput, opts: &PlaceOptions) -> Result<PlaceResult, String> {
    let mut footprints = input.placements.to_vec();
    for r in input.schematic.references() {
        let has_fp =
            input.schematic.parts.iter().any(|p| p.reference == r && p.footprint.is_some());
        if has_fp && footprints.iter().all(|f| f.reference != r) {
            footprints.push(PlacementFile {
                reference: r.to_string(),
                at: Point::mm(0.0, 0.0),
                rotation: None,
                side: None,
                label: None,
                mlcc: None,
                locked: false,
            });
        }
    }
    let file = crate::layout::LayoutFile {
        footprints,
        tracks: input.tracks.to_vec(),
        vias: input.vias.to_vec(),
        ..Default::default()
    };
    let cx = crate::layout::Context {
        dir: std::path::PathBuf::new(),
        board: input.board,
        schematic: input.schematic,
        footprints: input.footprints.clone(),
        heat: Vec::new(),
    };
    let layout = crate::layout::without_checks(|| {
        crate::layout::without_fills(|| file.resolve(&cx, &mut crate::diag::Diags::new("")))
    });
    let hidden: Vec<usize> = (0..layout.parts.len())
        .filter(|&i| moving(input, opts, &layout.parts[i].reference))
        .collect();
    let stale: std::collections::HashSet<usize> =
        hidden.iter().flat_map(|&i| layout.parts[i].pads.iter().filter_map(|q| q.net)).collect();
    let tracks: Vec<crate::layout::Track> =
        layout.tracks.iter().filter(|t| !stale.contains(&t.net)).cloned().collect();
    let vias: Vec<crate::layout::Via> =
        layout.vias.iter().filter(|v| !stale.contains(&v.net)).cloned().collect();
    let world = crate::drc::Ctx::new(
        input.board,
        &layout.copper,
        &layout.outline,
        &layout.board_cutouts,
        &layout.parts,
        &tracks,
        &vias,
        &[],
        &layout.nets,
    );
    let base = crate::rules::Placed::new(&world);
    let separates = crate::rules::Context::spacing(&base).isolation.separates();
    let mut hide = vec![false; layout.parts.len()];
    hidden.iter().for_each(|&i| hide[i] = true);
    let matters = separates || hide.iter().any(|h| !h) || !tracks.is_empty() || !vias.is_empty();
    let copper = Copper {
        base,
        index: layout.parts.iter().enumerate().map(|(i, p)| (p.reference.clone(), i)).collect(),
        hidden,
        hide,
        separates,
    };
    place_with(input, opts, (matters && layout.outline.len() >= 3).then_some(&copper))
}

fn place_with<'a>(
    input: &PlaceInput<'a>,
    opts: &PlaceOptions,
    copper: Option<&'a Copper<'a>>,
) -> Result<PlaceResult, String> {
    let board = input.board;
    let sch = input.schematic;
    if input.outline.len() < 3 {
        return Err(format!("board `{}` has no outline to place parts in", board.name));
    }
    let mut ob = Bounds::EMPTY;
    input.outline.iter().for_each(|p| ob.add(*p));
    let pd = input.board.drc.placement.clone().unwrap_or_default();
    let b = Board2 {
        outline: input.outline.to_vec(),
        cutouts: input.cutouts.iter().filter(|c| c.len() >= 3).cloned().collect(),
        centre: ob.center(),
        bounds: ob,
        keepouts: input
            .spec
            .keepouts
            .iter()
            .map(|k| k.iter().map(|p| p.to_mm()).collect())
            .filter(|k: &Vec<P>| k.len() >= 3)
            .collect(),
        body_edge: board.rules.min_body_to_edge.to_mm(),
        part_edge: board.rules.min_part_to_edge.to_mm(),
        copper_edge: board.rules.min_copper_to_edge.to_mm(),
        flex: board.rules.flex_zone.to_mm(),
        decap: pd.decoupling_distance.map(|l| l.to_mm()).unwrap_or(DECOUPLING_DISTANCE),
        crystal: pd.crystal_distance.map(|l| l.to_mm()).unwrap_or(CRYSTAL_DISTANCE),
        spread: pd.cluster_spread.map(|l| l.to_mm()).unwrap_or(CLUSTER_SPREAD),
        standoff: opts.standoff,
        depth: EdgeDepth::new(input.outline, input.cutouts, &ob),
        edges: std::sync::Arc::new(EdgeIndex::new(
            geom::BoardEdge::new(input.outline, input.cutouts),
            &ob,
            board
                .rules
                .min_body_to_edge
                .to_mm()
                .max(board.rules.min_part_to_edge.to_mm())
                .max(board.rules.min_copper_to_edge.to_mm())
                .max(FIDUCIAL_TO_EDGE)
                + EDGE_CELL,
        )),
        flex_edges: std::sync::Arc::new(EdgeIndex::new(
            geom::BoardEdge::new(input.outline, input.cutouts),
            &ob,
            board.rules.flex_zone.to_mm() + FLEX_RAD,
        )),
        silk: input
            .silk
            .iter()
            .filter(|a| a.poly.len() >= 3)
            .map(|a| WShape {
                side: if a.bottom { 2 } else { 1 },
                b: poly_bounds(&a.poly),
                rect: is_axis_rect(&a.poly),
                poly: a.poly.clone(),
                label: false,
            })
            .collect(),
    };

    let mut refs: Vec<&str> = Vec::new();
    for p in &sch.parts {
        if p.footprint.is_some() && !refs.contains(&p.reference.as_str()) {
            refs.push(&p.reference);
        }
    }
    let heat: HashMap<&str, f64> = input.heat.iter().map(|(r, w)| (r.as_str(), *w)).collect();
    let mut parts: Vec<Part> = Vec::new();
    for r in &refs {
        let units: Vec<(usize, &crate::schematic::Part)> =
            sch.parts.iter().enumerate().filter(|(_, p)| p.reference == *r).collect();
        let first = units[0].1;
        let fp_name = first.footprint.clone().unwrap_or_default();
        let Some(fp) = input.footprints.get(fp_name.as_str()).copied() else {
            return Err(format!("{r}: footprint `{fp_name}` is not in this project"));
        };
        let pads: Vec<LPad> = fp
            .pads
            .iter()
            .map(|pad| {
                let net = units.iter().find_map(|(pi, p)| {
                    p.symbol
                        .pins
                        .iter()
                        .position(|n| n.number == pad.number)
                        .and_then(|ni| sch.net_of(PinRef { part: *pi, pin: ni }))
                });
                let outline = if pad.is_copper() || pad.kind == PadKind::Npth {
                    pad.outlines()
                } else {
                    Vec::new()
                };
                let mut bb = Bounds::EMPTY;
                outline.iter().flatten().for_each(|q| bb.add(*q));
                let c = if bb.is_empty() { pad.at.to_mm() } else { bb.center() };
                LPad { c, outline, net, edge: pad.edge }
            })
            .collect();
        let mut loops: Vec<(bool, Vec<P>)> = Vec::new();
        for l in courtyard_loops(fp, "F.CrtYd") {
            loops.push((false, grow_loop(&l, opts.spacing / 2.0)));
        }
        for l in courtyard_loops(fp, "B.CrtYd") {
            loops.push((true, grow_loop(&l, opts.spacing / 2.0)));
        }
        if loops.is_empty() {
            let mut bb = Bounds::EMPTY;
            pads.iter().flat_map(|q| q.outline.iter().flatten()).for_each(|q| bb.add(*q));
            for g in fp.graphics.iter().filter(|g| g.layer == "F.Fab") {
                bb.union(&g.bounds());
            }
            if bb.is_empty() {
                bb.add_circle([0.0, 0.0], 0.5);
            }
            let m = 0.25 + opts.spacing / 2.0;
            let (lo, hi) = ([bb.min[0] - m, bb.min[1] - m], [bb.max[0] + m, bb.max[1] + m]);
            loops.push((false, vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]]));
        }
        let mut local = Bounds::EMPTY;
        loops.iter().flat_map(|l| l.1.iter()).for_each(|q| local.add(*q));
        let mut extent = local;
        pads.iter().flat_map(|q| q.outline.iter().flatten()).for_each(|q| extent.add(*q));
        let role = role_of(r, &fp_name, fp);
        let placed = input.placements.iter().find(|f| f.reference == *r);
        let label = label_room(fp, r, placed, local.center(), false);
        let label_inward = if matches!(role, Role::Connector | Role::Hole | Role::Fiducial) {
            label_room(fp, r, placed, local.center(), true)
        } else {
            None
        };
        label.iter().chain(&label_inward).flat_map(|l| l.poly.iter()).for_each(|q| extent.add(*q));
        let through = fp.pads.iter().any(|q| q.kind == PadKind::Tht || q.kind == PadKind::Npth);
        let mut pad_box = Bounds::EMPTY;
        pads.iter().flat_map(|q| q.outline.iter().flatten()).for_each(|q| pad_box.add(*q));
        let pads_in_court = pad_box.is_empty() || local.contains(&pad_box);
        let pins = copper_pad_numbers(fp);
        let s = local.size();
        let area = s[0] * s[1];
        let large = fp_name.to_ascii_lowercase().contains("bga") || (pins >= 16 && area >= 25.0);
        let mlcc = role == Role::Passive
            && is_capacitor(r, &fp_name)
            && !fp_name.to_ascii_lowercase().starts_with("cp_");
        let mlcc_len = if fp.mlcc == Some(false) {
            None
        } else if mlcc || fp.mlcc == Some(true) {
            chip_length(&fp_name, fp).or(Some(1.0))
        } else {
            None
        };
        let cu: Vec<usize> = (0..pads.len()).filter(|k| !pads[*k].outline.is_empty()).collect();
        let numbered = |n: &str| fp.pads.iter().position(|q| q.number == n && q.is_copper());
        let ends = match (numbered("1"), numbered("2")) {
            (Some(a), Some(b)) => Some((a, b)),
            _ if cu.len() == 2 => Some((cu[0], cu[1])),
            _ => None,
        };
        parts.push(Part {
            reference: r.to_string(),
            ends,
            fp,
            conn: conn_kind(&fp_name),
            value: first.value.clone(),
            role,
            pads,
            loops,
            through,
            pads_in_court,
            local,
            extent,
            label,
            label_inward,
            over_silk: false,
            area,
            pins,
            large,
            heat: heat.get(r).copied().unwrap_or(0.0),
            mlcc_len,
            fixed: false,
            active: false,
            placed: false,
            st: St { at: [0.0, 0.0], rot: 0.0, bottom: false },
            sch_at: first.at.to_mm(),
            pin_edge: input.spec.edges.get(*r).copied(),
            sensitive: false,
            switcher: false,
            hot: false,
            spread_hot: false,
        });
    }
    for r in input.spec.edges.keys() {
        if !refs.contains(&r.as_str()) {
            return Err(format!("[place] edges names {r}, which is not a part of the schematic"));
        }
    }
    let max_pins = parts.iter().filter(|p| p.role == Role::Chip).map(|p| p.pins).max().unwrap_or(0);
    if !parts.iter().any(|p| p.large) && max_pins >= 8 {
        for p in parts.iter_mut().filter(|p| p.role == Role::Chip && p.pins == max_pins) {
            p.large = true;
        }
    }
    for p in parts.iter_mut() {
        p.hot = p.heat >= HOT_WATTS || (p.role == Role::Chip && p.large && p.area >= 49.0);
        if p.large && p.heat == 0.0 && p.area >= 49.0 {
            p.heat = 1.0;
        }
    }

    let fast: Vec<bool> = sch
        .nets
        .iter()
        .map(|n| {
            is_fast_class(board, &n.class)
                || input.fast_nets.iter().any(|g| glob(g, &n.name) || g == &n.name)
        })
        .collect();
    let mut nets: Vec<NetInfo> = sch
        .nets
        .iter()
        .enumerate()
        .map(|(ni, n)| {
            let power = is_power_net(board, &n.name, &n.class);
            NetInfo {
                pins: Vec::new(),
                weight: if power {
                    0.0
                } else if fast[ni] {
                    3.0
                } else {
                    1.0
                },
                power,
                ground: is_ground(&n.name),
                rf: is_rf_class(board, &n.class),
            }
        })
        .collect();
    let mut part_nets: Vec<Vec<usize>> = vec![Vec::new(); parts.len()];
    for (i, p) in parts.iter().enumerate() {
        for (k, q) in p.pads.iter().enumerate() {
            if let Some(n) = q.net
                && !q.outline.is_empty()
            {
                nets[n].pins.push((i, k));
                if !part_nets[i].contains(&n) {
                    part_nets[i].push(n);
                }
            }
        }
    }
    for n in nets.iter_mut() {
        let mut seen: Vec<usize> = Vec::new();
        n.pins.retain(|(i, _)| {
            if seen.contains(i) {
                false
            } else {
                seen.push(*i);
                true
            }
        });
    }
    let two_pin: Vec<usize> =
        (0..nets.len()).filter(|n| !nets[*n].power && nets[*n].pins.len() == 2).collect();
    let mut is_two_pin = vec![false; nets.len()];
    two_pin.iter().for_each(|n| is_two_pin[*n] = true);

    let chosen = |r: &str| opts.parts.is_empty() || opts.parts.iter().any(|g| glob(g, r));
    let mut kept = Vec::new();
    let mut before_all = true;
    for p in parts.iter_mut() {
        let f = input.placements.iter().find(|f| f.reference == p.reference);
        if let Some(f) = f {
            p.st = St {
                at: f.at.to_mm(),
                rot: f.rotation.unwrap_or(0.0),
                bottom: f.side == Some(BoardSide::Bottom),
            };
        } else {
            before_all = false;
        }
        let keep = f.is_some_and(|f| f.locked || opts.keep_placed || !chosen(&p.reference));
        if keep {
            p.fixed = true;
            kept.push(p.reference.clone());
        } else if chosen(&p.reference) {
            p.active = true;
        }
    }
    for p in parts.iter() {
        if p.pin_edge.is_some() && p.role != Role::Connector && p.active {
            return Err(format!("[place] edges pins {}, which is not a connector", p.reference));
        }
    }

    let sides = if opts.sides == Sides::Both { 2.0 } else { 1.0 };
    let room = b.depth.placeable(b.body_edge) * sides;
    let used: f64 = parts.iter().map(|p| p.area).sum();
    let label_area: f64 = parts
        .iter()
        .filter(|p| p.active)
        .filter_map(|p| p.label.as_ref())
        .map(|l| {
            let s = poly_bounds(&l.poly).size();
            s[0] * s[1]
        })
        .sum();
    let hot_gap = pd.hot_distance.map(|l| l.to_mm()).unwrap_or(HOT_DISTANCE);
    let hot_area: f64 = parts
        .iter()
        .filter(|p| p.hot && p.active)
        .map(|p| {
            let s = p.local.size();
            (s[0] + hot_gap) * (s[1] + hot_gap)
        })
        .sum();
    let hot_share = hot_area / b.depth.placeable(b.body_edge).max(1e-9);
    let spread = parts.iter().filter(|p| p.hot && p.active).count() >= 2 && hot_share <= HOT_SHARE;
    for p in parts.iter_mut() {
        p.spread_hot = spread && p.hot;
    }
    let hot_spread = HotSpread {
        applied: spread,
        share: (hot_share * 1000.0).round() / 1000.0,
        threshold: HOT_SHARE,
    };
    let label_room = label_area <= LABEL_SHARE * (room - used);
    let soft_labels = !label_room && label_area <= room - used;
    for p in parts.iter_mut().filter(|p| !p.active || !(label_room || soft_labels)) {
        p.label = None;
        p.label_inward = None;
    }
    let labels = LabelRoom {
        reserved: label_room,
        weighed: soft_labels,
        label_mm2: (label_area * 10.0).round() / 10.0,
        free_mm2: ((room - used) * 10.0).round() / 10.0,
    };

    let n_parts = parts.len();
    let mut two_pin_at = vec![usize::MAX; nets.len()];
    two_pin.iter().enumerate().for_each(|(x, n)| two_pin_at[*n] = x);
    let reach = ob.size()[0].hypot(ob.size()[1]) + 10.0;
    let pl_refs: Vec<String> = parts.iter().map(|p| p.reference.clone()).collect();
    let mut pl = Placer {
        parts,
        part_links: vec![Vec::new(); n_parts],
        cluster_of: vec![None; n_parts],
        cache: vec![Vec::new(); n_parts],
        net_stamp: vec![0; nets.len()],
        nets,
        part_nets,
        links: Vec::new(),
        two_pin,
        is_two_pin,
        clusters: Vec::new(),
        b,
        sides: opts.sides,
        grid: PartGrid::new(&ob),
        pad_at: vec![Vec::new(); n_parts],
        segments: vec![None; two_pin_at.iter().filter(|x| **x != usize::MAX).count()],
        two_pin_at,
        offsets: offsets(reach),
        link_stamp: Vec::new(),
        stamp: 0,
        holes: Vec::new(),
        cross_w: CROSSING_WEIGHT,
        soft_labels,
        hot_gap,
        label_at: vec![None; n_parts],
        copper: copper.map(|c| {
            let at: Vec<Option<usize>> =
                pl_refs.iter().map(|r| c.index.get(r.as_str()).copied()).collect();
            (c, at)
        }),
        reach: copper.map_or(0.0, Copper::reach),
    };

    pl.build_clusters();
    pl.link_stamp = vec![0; pl.links.len()];
    let before = if before_all {
        for i in 0..n_parts {
            pl.insert(i);
        }
        let m = pl.metrics();
        for i in 0..n_parts {
            pl.remove(i);
        }
        Some(m)
    } else {
        None
    };

    for i in 0..n_parts {
        if pl.parts[i].fixed {
            pl.insert(i);
        }
    }
    pl.holes = (0..n_parts)
        .filter(|i| pl.parts[*i].role == Role::Hole && pl.parts[*i].fixed)
        .map(|i| pl.hole_of(i))
        .collect();

    let rough_in = |start: u64| -> Rough {
        let mut cand = pl.clone();
        let mut failed = Vec::new();
        cand.place_corners(&mut failed);
        let rough = cand.global(start, None);
        let edges = cand.place_connectors(&rough, &mut failed);
        let pos = cand.global(start, Some(&rough));
        cand.legalise(&pos, &mut failed);
        let mut seen = std::collections::HashSet::new();
        failed.retain(|r| seen.insert(r.clone()));
        let all: Vec<usize> = (0..n_parts).collect();
        let score = cand.local_cost(&all) + 1e4 * failed.len() as f64;
        (score, cand, failed, edges)
    };
    let finish = |start: u64, rough: Rough<'a>| -> Start<'a> {
        let (_, mut cand, failed, edges) = rough;
        let mut rng = Rng(opts.seed.wrapping_mul(0x9e37_79b9).wrapping_add(start));
        cand.rearrange_clusters();
        let mut moves = cand.refine(&mut rng, MOVES_PER_PART, ANNEAL_HEAT);
        cand.share_rotation();
        moves += cand.refine(&mut rng, QUENCH_PER_PART, QUENCH_HEAT);
        cand.clear_flex_zone();
        let all: Vec<usize> = (0..n_parts).collect();
        let score = cand.local_cost(&all) + 1e4 * failed.len() as f64;
        (score, cand, failed, edges, moves)
    };
    let mut roughs = in_parallel((0..STARTS).collect(), |s| (s, rough_in(s)));
    roughs.sort_by(|a, b| a.1.0.total_cmp(&b.1.0).then(a.0.cmp(&b.0)));
    roughs.truncate(KEPT_STARTS);
    let runs = in_parallel(roughs, |(s, r)| (s, finish(s, r)));
    let best =
        runs.into_iter().min_by(|a, b| a.1.0.total_cmp(&b.1.0).then(a.0.cmp(&b.0))).map(|r| r.1);
    let Some((_, mut pl, failed, edges, moves)) = best else {
        return Err("no placement".into());
    };

    let mut x = ob.max[0] + 5.0;
    for i in 0..n_parts {
        if pl.parts[i].active && !pl.parts[i].placed {
            let s = pl.parts[i].local.size();
            pl.parts[i].st = St {
                at: snap_p([x + s[0] / 2.0, ob.min[1] + s[1] / 2.0]),
                rot: 0.0,
                bottom: false,
            };
            x += s[0] + 1.0;
        }
    }
    let after = pl.metrics();
    let clear: Vec<bool> = (0..n_parts)
        .map(|i| match pl.label_at[i] {
            Some((side, b)) => {
                pl.label_overlap(i) == 0.0
                    && pl.b.depth.least(&b) > 0.0
                    && !pl.b.silk.iter().any(|k| k.side & side != 0 && strict_overlap(&b, &k.b))
            }
            None => !pl.soft_labels,
        })
        .collect();
    let placements = pl
        .parts
        .iter()
        .enumerate()
        .filter(|(_, p)| p.active)
        .map(|(i, p)| Placement {
            reference: p.reference.clone(),
            at: [(p.st.at[0] * 1e4).round() / 1e4, (p.st.at[1] * 1e4).round() / 1e4],
            rotation: p.st.rot,
            bottom: p.st.bottom,
            label: p.label.as_ref().filter(|_| p.placed && clear[i]).map(|l| {
                let at = p.st.transform().apply(l.at);
                (
                    [(at[0] * 1e4).round() / 1e4, (at[1] * 1e4).round() / 1e4],
                    upright(l.rotation + p.st.rot),
                )
            }),
        })
        .collect();
    let clusters = pl
        .clusters
        .iter()
        .filter(|c| !c.members.is_empty())
        .map(|c| ClusterSummary {
            anchor: pl.parts[c.anchor].reference.clone(),
            members: c.members.iter().map(|m| pl.parts[*m].reference.clone()).collect(),
        })
        .collect();
    let (texts_moved, texts_stuck) = pl.move_texts(&input.texts);
    Ok(PlaceResult {
        placements,
        kept,
        failed,
        edges,
        clusters,
        before,
        after,
        moves,
        labels,
        texts_moved,
        texts_stuck,
        hot_spread,
    })
}

pub const DECOUPLING_DISTANCE: f64 = 3.0;
pub const CRYSTAL_DISTANCE: f64 = 5.0;
pub const CLUSTER_SPREAD: f64 = 10.0;
pub const OFF_CENTRE: f64 = 0.6;
pub const HOT_DISTANCE: f64 = 5.0;
pub const CONNECTOR_EDGE: f64 = 3.0;

impl<'a> Placer<'a> {
    fn anchors_on(&self, n: usize, skip: usize) -> Vec<usize> {
        let mut v: Vec<usize> = self.nets[n]
            .pins
            .iter()
            .map(|(i, _)| *i)
            .filter(|i| *i != skip && matches!(self.parts[*i].role, Role::Chip | Role::Connector))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    fn signal_nets(&self, i: usize) -> Vec<usize> {
        self.part_nets[i].iter().copied().filter(|n| !self.nets[*n].power).collect()
    }

    fn add_link(&mut self, a: End, b: End, kind: LinkKind) {
        let l = self.links.len();
        self.links.push(Link { a, b, kind });
        let (pa, pb) = (Self::end_part(a), Self::end_part(b));
        self.part_links[pa].push(l);
        if pb != pa {
            self.part_links[pb].push(l);
        }
    }

    #[allow(clippy::needless_range_loop)]
    fn build_clusters(&mut self) {
        let n = self.parts.len();
        let mut anchor: Vec<Option<usize>> = vec![None; n];
        let mut served: HashMap<usize, (usize, usize)> = HashMap::new();
        let mut load: HashMap<(usize, usize), usize> = HashMap::new();
        let followers = |r: Role| matches!(r, Role::Passive | Role::Crystal | Role::TestPoint);
        for i in 0..n {
            if !followers(self.parts[i].role) {
                continue;
            }
            let sig = self.signal_nets(i);
            if sig.is_empty() {
                let rails: Vec<usize> =
                    self.part_nets[i].iter().copied().filter(|x| !self.nets[*x].ground).collect();
                let has_ground = self.part_nets[i].iter().any(|x| self.nets[*x].ground);
                let Some(&rail) = rails.first() else { continue };
                if !has_ground && rails.len() < 2 {
                    continue;
                }
                let me = self.parts[i].sch_at;
                let chips: Vec<usize> = self
                    .anchors_on(rail, i)
                    .into_iter()
                    .filter(|c| self.parts[*c].role == Role::Chip)
                    .collect();
                let Some(&best) = chips.iter().min_by(|a, b| {
                    geom::dist(self.parts[**a].sch_at, me)
                        .total_cmp(&geom::dist(self.parts[**b].sch_at, me))
                }) else {
                    continue;
                };
                anchor[i] = Some(best);
                let Some(k) = self.parts[i].pads.iter().position(|q| q.net == Some(rail)) else {
                    continue;
                };
                let pins: Vec<usize> = (0..self.parts[best].pads.len())
                    .filter(|k| {
                        self.parts[best].pads[*k].net == Some(rail)
                            && !self.parts[best].pads[*k].outline.is_empty()
                    })
                    .collect();
                if let Some(&pin) =
                    pins.iter().min_by_key(|p| load.get(&(best, **p)).copied().unwrap_or(0))
                {
                    *load.entry((best, pin)).or_default() += 1;
                    served.insert(i, (k, pin));
                }
                continue;
            }
            let mut a: Vec<usize> = sig.iter().flat_map(|x| self.anchors_on(*x, i)).collect();
            a.sort_unstable();
            a.dedup();
            if a.len() == 1 {
                anchor[i] = Some(a[0]);
            }
        }
        for _ in 0..3 {
            for i in 0..n {
                if anchor[i].is_some() || !followers(self.parts[i].role) {
                    continue;
                }
                let sig = self.signal_nets(i);
                let mut direct: Vec<usize> =
                    sig.iter().flat_map(|x| self.anchors_on(*x, i)).collect();
                if !direct.is_empty() {
                    continue;
                }
                let mut via: Vec<Option<usize>> = sig
                    .iter()
                    .flat_map(|x| self.nets[*x].pins.iter().map(|p| p.0))
                    .filter(|j| *j != i)
                    .map(|j| anchor[j])
                    .collect();
                via.sort_unstable();
                via.dedup();
                if via.len() == 1
                    && let Some(a) = via[0]
                {
                    anchor[i] = Some(a);
                }
                direct.clear();
            }
        }
        let mut cl_index: HashMap<usize, usize> = HashMap::new();
        for (i, a) in anchor.iter().enumerate() {
            let Some(a) = *a else { continue };
            let c = *cl_index.entry(a).or_insert_with(|| {
                self.clusters.push(Cluster {
                    anchor: a,
                    members: Vec::new(),
                    served: HashMap::new(),
                });
                self.clusters.len() - 1
            });
            self.clusters[c].members.push(i);
            if let Some(s) = served.get(&i) {
                self.clusters[c].served.insert(i, *s);
            }
            self.cluster_of[i] = Some(c);
        }
        for (a, c) in cl_index.iter() {
            self.cluster_of[*a] = Some(*c);
        }
        self.mark_switchers();
        let mut links: Vec<(End, End, LinkKind)> = Vec::new();
        for c in &self.clusters {
            for &m in &c.members {
                let p = &self.parts[m];
                if let Some(&(k, pin)) = c.served.get(&m) {
                    let bulk = cap_farads(&p.value).is_some_and(|f| f > 1.1e-6);
                    let w = if self.parts[c.anchor].switcher {
                        4.0
                    } else if bulk {
                        0.5
                    } else {
                        2.0
                    };
                    let thr = if bulk { self.b.decap * 2.5 } else { self.b.decap };
                    links.push((
                        End::Pad(m, k),
                        End::Pad(c.anchor, pin),
                        LinkKind::Pull { w, thr, extra: 5.0, near: self.b.standoff },
                    ));
                } else if p.role == Role::Crystal {
                    for n in self.signal_nets(m) {
                        let (Some(k), Some(pin)) = (
                            p.pads.iter().position(|q| q.net == Some(n)),
                            self.parts[c.anchor].pads.iter().position(|q| q.net == Some(n)),
                        ) else {
                            continue;
                        };
                        links.push((
                            End::Pad(m, k),
                            End::Pad(c.anchor, pin),
                            LinkKind::Pull { w: 3.0, thr: self.b.crystal, extra: 5.0, near: 0.0 },
                        ));
                    }
                } else {
                    let w = if p.switcher { 4.0 } else { 0.0 };
                    links.push((
                        End::Centre(m),
                        End::Centre(c.anchor),
                        LinkKind::Pull { w, thr: self.b.spread / 2.0, extra: 1.0, near: 0.0 },
                    ));
                }
            }
        }
        let hot: Vec<usize> = (0..n).filter(|i| self.parts[*i].heat >= HOT_WATTS).collect();
        for (x, &i) in hot.iter().enumerate() {
            for &j in &hot[x + 1..] {
                let thr =
                    HOT_GAP + (self.parts[i].local.size()[0] + self.parts[j].local.size()[0]) / 2.0;
                links.push((End::Centre(i), End::Centre(j), LinkKind::Repel { w: 2.0, thr }));
            }
        }
        let loud: Vec<usize> = (0..n).filter(|i| self.parts[*i].switcher).collect();
        let quiet: Vec<usize> = (0..n).filter(|i| self.parts[*i].sensitive).collect();
        for &i in &loud {
            for &j in &quiet {
                if i != j {
                    links.push((
                        End::Centre(i),
                        End::Centre(j),
                        LinkKind::Repel { w: 2.0, thr: QUIET_GAP },
                    ));
                }
            }
        }
        for (a, b, k) in links {
            self.add_link(a, b, k);
        }
    }

    fn mark_switchers(&mut self) {
        let n = self.parts.len();
        for i in 0..n {
            let p = &self.parts[i];
            if p.role == Role::Crystal {
                self.parts[i].sensitive = true;
                continue;
            }
            if self.part_nets[i].iter().any(|x| self.nets[*x].rf) {
                self.parts[i].sensitive = true;
            }
        }
        for i in 0..n {
            if self.parts[i].role != Role::Chip {
                continue;
            }
            let mut found = Vec::new();
            for &net in &self.signal_nets(i) {
                for &(j, _) in &self.nets[net].pins {
                    let q = &self.parts[j];
                    if j == i || q.role != Role::Passive || ref_prefix(&q.reference) != "L" {
                        continue;
                    }
                    if self.part_nets[j]
                        .iter()
                        .any(|x| *x != net && self.nets[*x].power && !self.nets[*x].ground)
                    {
                        found.push(j);
                    }
                }
            }
            if !found.is_empty() && !self.part_nets[i].iter().any(|x| self.nets[*x].rf) {
                self.parts[i].switcher = true;
                self.parts[i].sensitive = false;
                for j in found {
                    self.parts[j].switcher = true;
                    self.parts[j].sensitive = false;
                }
            }
        }
    }

    fn place_corners(&mut self, failed: &mut Vec<String>) {
        let bb = self.b.bounds;
        let corners = [
            ([bb.min[0], bb.min[1]], [1.0, 1.0]),
            ([bb.max[0], bb.max[1]], [-1.0, -1.0]),
            ([bb.max[0], bb.min[1]], [-1.0, 1.0]),
            ([bb.min[0], bb.max[1]], [1.0, -1.0]),
        ];
        let mut used = [false; 4];
        for (k, (c, _)) in corners.iter().enumerate() {
            used[k] = self.parts.iter().enumerate().any(|(i, p)| {
                p.fixed
                    && matches!(p.role, Role::Hole | Role::Fiducial)
                    && geom::dist(self.centre(i), *c) < 10.0
            });
        }
        let bottom = self.sides == Sides::Bottom;
        for role in [Role::Hole, Role::Fiducial] {
            let list: Vec<usize> = (0..self.parts.len())
                .filter(|i| self.parts[*i].active && self.parts[*i].role == role)
                .collect();
            for i in list {
                let s = self.parts[i].local.size();
                let half = s[0].max(s[1]) / 2.0;
                let k = (0..4).find(|k| !used[*k]);
                let (c, d) = match k {
                    Some(k) => {
                        used[k] = true;
                        corners[k]
                    }
                    None => {
                        corners
                            [self.parts.iter().filter(|p| p.placed && p.role == role).count() % 4]
                    }
                };
                let inset = if role == Role::Fiducial { FIDUCIAL_TO_EDGE + half } else { half };
                let lc = self.parts[i].local.center();
                let target = [c[0] + d[0] * inset - lc[0], c[1] + d[1] * inset - lc[1]];
                let fid_bottom = bottom && role == Role::Fiducial;
                let labels = self.take_labels(i);
                let found =
                    self.nearest(i, target, 0.0, fid_bottom, 40.0, &|_| 0.0).or_else(|| {
                        self.parts[i].over_silk = true;
                        self.nearest(i, target, 0.0, fid_bottom, 40.0, &|_| 0.0)
                    });
                match found {
                    Some(st) => {
                        self.parts[i].st = st;
                        self.label_where_it_fits(i, st, labels);
                        self.insert(i);
                        if role == Role::Hole {
                            let hole = self.hole_of(i);
                            self.holes.push(hole);
                        }
                    }
                    None => failed.push(self.parts[i].reference.clone()),
                }
            }
        }
    }

    fn macro_of(&self) -> (Vec<Vec<usize>>, Vec<Option<usize>>) {
        let n = self.parts.len();
        let mut macros: Vec<Vec<usize>> = Vec::new();
        let mut of = vec![None; n];
        for i in 0..n {
            let p = &self.parts[i];
            if !p.active || p.placed || matches!(p.role, Role::Hole | Role::Fiducial) {
                continue;
            }
            if let Some(c) = self.cluster_of[i] {
                let a = self.clusters[c].anchor;
                if a != i && self.parts[a].active && !self.parts[a].placed {
                    continue;
                }
                if a == i {
                    let mut v = vec![i];
                    v.extend(
                        self.clusters[c]
                            .members
                            .iter()
                            .copied()
                            .filter(|m| self.parts[*m].active && !self.parts[*m].placed),
                    );
                    for m in &v {
                        of[*m] = Some(macros.len());
                    }
                    macros.push(v);
                    continue;
                }
            }
            of[i] = Some(macros.len());
            macros.push(vec![i]);
        }
        (macros, of)
    }

    fn macro_graph(&self, macros: &[Vec<usize>], of: &[Option<usize>]) -> MacroGraph {
        let m = macros.len();
        let mut g =
            MacroGraph { adj: vec![Vec::new(); m], pairs: Vec::new(), fixed: vec![Vec::new(); m] };
        for net in &self.nets {
            if net.power || net.weight == 0.0 {
                continue;
            }
            let mut ms: Vec<(usize, Option<usize>)> = Vec::new();
            let mut fixed_pts: Vec<P> = Vec::new();
            for &(i, k) in &net.pins {
                if let Some(x) = of[i] {
                    let pin = (macros[x][0] == i).then_some(k);
                    match ms.iter_mut().find(|e| e.0 == x) {
                        Some(e) => {
                            if e.1.is_none() {
                                e.1 = pin;
                            }
                        }
                        None => ms.push((x, pin)),
                    }
                } else if self.parts[i].placed {
                    fixed_pts.push(self.pad_pos(i, k));
                }
            }
            let k = ms.len() + fixed_pts.len();
            if k < 2 {
                continue;
            }
            if net.pins.len() == 2 && ms.len() == 2 {
                g.pairs.push((ms[0].0, ms[0].1, ms[1].0, ms[1].1));
            }
            let w = net.weight / (k - 1) as f64;
            for (x, &(a, ka)) in ms.iter().enumerate() {
                for &(b, kb) in &ms[x + 1..] {
                    g.adj[a].push(MacroEdge { to: b, w, pin: ka, far: kb });
                    g.adj[b].push(MacroEdge { to: a, w, pin: kb, far: ka });
                }
                for p in &fixed_pts {
                    g.fixed[a].push((w, *p, ka));
                }
            }
        }
        g
    }

    fn free_grid(&self) -> FreeGrid {
        let bb = self.b.bounds;
        let nx = ((bb.size()[0] / FREE_CELL).ceil() as usize).max(1);
        let ny = ((bb.size()[1] / FREE_CELL).ceil() as usize).max(1);
        let mut free = vec![false; nx * ny];
        let edge = self.board_edge();
        let centre = |x: usize, y: usize| {
            [bb.min[0] + (x as f64 + 0.5) * FREE_CELL, bb.min[1] + (y as f64 + 0.5) * FREE_CELL]
        };
        for y in 0..ny {
            for x in 0..nx {
                let q = centre(x, y);
                free[y * nx + x] = edge.contains(q)
                    && edge.distance(q) >= self.b.body_edge
                    && !self.b.keepouts.iter().any(|k| geom::point_in_polygon(q, k));
            }
        }
        for i in (0..self.parts.len()).filter(|i| self.parts[*i].placed) {
            for s in &self.cache[i] {
                let lo = |v: f64, o: f64| ((v - o) / FREE_CELL - 0.5).ceil().max(0.0) as usize;
                let hi = |v: f64, o: f64, n: usize| {
                    (((v - o) / FREE_CELL - 0.5).floor() + 1.0).clamp(0.0, n as f64) as usize
                };
                for y in lo(s.b.min[1], bb.min[1])..hi(s.b.max[1], bb.min[1], ny) {
                    for x in lo(s.b.min[0], bb.min[0])..hi(s.b.max[0], bb.min[0], nx) {
                        free[y * nx + x] = false;
                    }
                }
            }
        }
        let mut sum = vec![0.0; (nx + 1) * (ny + 1)];
        for y in 0..ny {
            for x in 0..nx {
                let v = if free[y * nx + x] { 1.0 } else { 0.0 };
                sum[(y + 1) * (nx + 1) + x + 1] =
                    v + sum[y * (nx + 1) + x + 1] + sum[(y + 1) * (nx + 1) + x]
                        - sum[y * (nx + 1) + x];
            }
        }
        FreeGrid { origin: bb.min, nx, ny, sum }
    }

    fn global(&mut self, variant: u64, init: Option<&[P]>) -> Vec<P> {
        let n = self.parts.len();
        let (macros, of) = self.macro_of();
        let m = macros.len();
        let bb = self.b.bounds;
        let centre = self.b.centre;
        let mut out = init.map(|v| v.to_vec()).unwrap_or_else(|| vec![centre; n]);
        if m == 0 {
            return out;
        }
        let both = self.sides == Sides::Both;
        let area: Vec<f64> = macros
            .iter()
            .map(|v| {
                let a: f64 = v
                    .iter()
                    .map(|i| {
                        let a = self.parts[*i].area.max(0.5);
                        if both && *i != v[0] { 0.5 * a } else { a }
                    })
                    .sum();
                a * MACRO_ROOM
            })
            .collect();
        let large: Vec<bool> = macros.iter().map(|v| self.parts[v[0]].large).collect();
        let conn: Vec<Option<Option<Edge>>> = macros
            .iter()
            .map(|v| {
                let p = &self.parts[v[0]];
                (p.role == Role::Connector).then_some(p.pin_edge)
            })
            .collect();
        let graph = self.macro_graph(&macros, &of);
        let bottom = self.sides == Sides::Bottom;
        let turnable: Vec<bool> = macros
            .iter()
            .map(|v| self.parts[v[0]].role == Role::Chip && self.parts[v[0]].pins >= 8)
            .collect();
        let mut rot: Vec<f64> = macros.iter().map(|v| self.parts[v[0]].st.rot).collect();
        let offset = |x: usize, r: f64, k: Option<usize>| -> P {
            let Some(k) = k else { return [0.0, 0.0] };
            let p = &self.parts[macros[x][0]];
            let t = Transform { at: [0.0, 0.0], rotation: r, mirror: bottom };
            let (q, c) = (t.apply(p.pads[k].c), t.apply(p.local.center()));
            [q[0] - c[0], q[1] - c[1]]
        };
        let half = [bb.size()[0] / 2.0, bb.size()[1] / 2.0];
        let mut pos: Vec<P> = match init {
            Some(p) => macros.iter().map(|v| p[v[0]]).collect(),
            None => spectral(&graph.adj, variant, bb.size()[0] >= bb.size()[1])
                .into_iter()
                .map(|v| [centre[0] + v[0] * half[0] * 0.8, centre[1] + v[1] * half[1] * 0.8])
                .collect(),
        };
        let reserve = self
            .parts
            .iter()
            .filter(|p| p.role == Role::Hole)
            .map(|p| p.local.size()[0].max(p.local.size()[1]) + 0.5)
            .fold(1.0, f64::max);
        let project = |p: P, pinned: Option<Edge>| -> P {
            let gaps = [
                (p[0] - bb.min[0], Edge::Left),
                (bb.max[0] - p[0], Edge::Right),
                (p[1] - bb.min[1], Edge::Top),
                (bb.max[1] - p[1], Edge::Bottom),
            ];
            let e = pinned.unwrap_or_else(|| {
                gaps.iter().min_by(|a, b| a.0.total_cmp(&b.0)).map(|g| g.1).unwrap_or(Edge::Left)
            });
            let x = p[0].clamp(bb.min[0] + reserve, (bb.max[0] - reserve).max(bb.min[0] + reserve));
            let y = p[1].clamp(bb.min[1] + reserve, (bb.max[1] - reserve).max(bb.min[1] + reserve));
            match e {
                Edge::Left => [bb.min[0], y],
                Edge::Right => [bb.max[0], y],
                Edge::Top => [x, bb.min[1]],
                Edge::Bottom => [x, bb.max[1]],
            }
        };
        let grid = self.free_grid();
        let inner: Vec<usize> = (0..m).filter(|a| conn[*a].is_none()).collect();
        let limit = [half[0] * OFF_CENTRE, half[1] * OFF_CENTRE];
        let mut target = pos.clone();
        for round in 0..SPREAD_ROUNDS {
            self.spread(&grid, &pos, &area, inner.clone(), bb, &mut target);
            let alpha = if init.is_some() { 0.3 } else { 0.03 } * SPREAD_GROWTH.powi(round as i32);
            for a in (0..m).filter(|a| turnable[*a]) {
                let cost = |r: f64| -> f64 {
                    let mut c = 0.0;
                    for e in &graph.adj[a] {
                        let (o, f) = (offset(a, r, e.pin), offset(e.to, rot[e.to], e.far));
                        let d = [
                            pos[a][0] + o[0] - pos[e.to][0] - f[0],
                            pos[a][1] + o[1] - pos[e.to][1] - f[1],
                        ];
                        c += e.w * (d[0] * d[0] + d[1] * d[1]);
                    }
                    for (w, q, k) in &graph.fixed[a] {
                        let o = offset(a, r, *k);
                        let d = [pos[a][0] + o[0] - q[0], pos[a][1] + o[1] - q[1]];
                        c += w * (d[0] * d[0] + d[1] * d[1]);
                    }
                    c
                };
                let late = round * 2 >= SPREAD_ROUNDS;
                let crossings = |r: f64| -> f64 {
                    if !late {
                        return 0.0;
                    }
                    let end = |x: usize, k: Option<usize>| {
                        let o = offset(x, if x == a { r } else { rot[x] }, k);
                        [pos[x][0] + o[0], pos[x][1] + o[1]]
                    };
                    let segs: Vec<(P, P, bool)> = graph
                        .pairs
                        .iter()
                        .map(|&(x, kx, y, ky)| (end(x, kx), end(y, ky), x == a || y == a))
                        .collect();
                    let mut c = 0;
                    for s in segs.iter().filter(|s| s.2) {
                        c += segs
                            .iter()
                            .filter(|t| geom::segments_intersect(s.0, s.1, t.0, t.1))
                            .count();
                    }
                    c as f64
                };
                let costs: Vec<(f64, f64)> =
                    [0.0, 90.0, 180.0, 270.0].iter().map(|r| (cost(*r), crossings(*r))).collect();
                let low = costs.iter().map(|c| c.0).fold(f64::MAX, f64::min).max(1e-9);
                let mut best = (f64::MAX, rot[a]);
                for (k, r) in [0.0, 90.0, 180.0, 270.0].iter().enumerate() {
                    let c = costs[k].0 / low + GLOBAL_CROSSING * costs[k].1;
                    let c = if *r == rot[a] { c - 1e-9 } else { c };
                    if c < best.0 {
                        best = (c, *r);
                    }
                }
                rot[a] = best.1;
            }
            for _ in 0..SOLVE_SWEEPS {
                for a in 0..m {
                    let (mut sw, mut sx, mut sy) = (0.0, 0.0, 0.0);
                    for e in &graph.adj[a] {
                        let (o, f) = (offset(a, rot[a], e.pin), offset(e.to, rot[e.to], e.far));
                        sw += e.w;
                        sx += e.w * (pos[e.to][0] + f[0] - o[0]);
                        sy += e.w * (pos[e.to][1] + f[1] - o[1]);
                    }
                    for (w, q, k) in &graph.fixed[a] {
                        let o = offset(a, rot[a], *k);
                        sw += w;
                        sx += w * (q[0] - o[0]);
                        sy += w * (q[1] - o[1]);
                    }
                    let base = sw.max(0.2);
                    let (t, tw, g) = match conn[a] {
                        Some(pinned) => (project(pos[a], pinned), 2.0 * base, 0.0),
                        None => {
                            let g = if large[a] { CENTRE_PULL * base } else { 0.02 * base };
                            (target[a], alpha * base, g)
                        }
                    };
                    let den = sw + tw + g;
                    pos[a] = [
                        (sx + tw * t[0] + g * centre[0]) / den,
                        (sy + tw * t[1] + g * centre[1]) / den,
                    ];
                    if large[a] {
                        pos[a] = [
                            pos[a][0].clamp(centre[0] - limit[0], centre[0] + limit[0]),
                            pos[a][1].clamp(centre[1] - limit[1], centre[1] + limit[1]),
                        ];
                    }
                }
            }
        }
        self.spread(&grid, &pos, &area, inner, bb, &mut target);
        for (x, v) in macros.iter().enumerate() {
            let p = if conn[x].is_some() { pos[x] } else { target[x] };
            for &i in v {
                out[i] = p;
                let r = if i == v[0] { rot[x] } else { self.parts[i].st.rot };
                let lc = Transform { at: [0.0, 0.0], rotation: r, mirror: bottom }
                    .apply(self.parts[i].local.center());
                self.parts[i].st = St { at: [p[0] - lc[0], p[1] - lc[1]], rot: r, bottom };
            }
        }
        out
    }

    #[allow(clippy::only_used_in_recursion)]
    fn spread(
        &self,
        grid: &FreeGrid,
        pos: &[P],
        area: &[f64],
        mut items: Vec<usize>,
        region: Bounds,
        out: &mut [P],
    ) {
        if items.is_empty() {
            return;
        }
        if items.len() == 1 {
            let a = items[0];
            let h = area[a].sqrt() / 2.0;
            let fit = |v: f64, lo: f64, hi: f64| {
                if hi - lo < 2.0 * h { (lo + hi) / 2.0 } else { v.clamp(lo + h, hi - h) }
            };
            out[a] = [
                fit(pos[a][0], region.min[0], region.max[0]),
                fit(pos[a][1], region.min[1], region.max[1]),
            ];
            return;
        }
        let s = region.size();
        let axis = if s[0] >= s[1] { 0 } else { 1 };
        items.sort_by(|a, b| pos[*a][axis].total_cmp(&pos[*b][axis]).then(a.cmp(b)));
        let total: f64 = items.iter().map(|i| area[*i]).sum();
        let mut k = 1;
        let mut acc = area[items[0]];
        while k < items.len() - 1 && acc + area[items[k]] / 2.0 < total / 2.0 {
            acc += area[items[k]];
            k += 1;
        }
        let f = acc / total;
        let free = grid.free(&region);
        let (lo, hi) = (region.min[axis], region.max[axis]);
        let cut = if free <= 0.0 {
            lo + (hi - lo) * f
        } else {
            let (mut a, mut b) = (lo, hi);
            for _ in 0..30 {
                let mid = (a + b) / 2.0;
                let mut r = region;
                r.max[axis] = mid;
                if grid.free(&r) < f * free {
                    a = mid;
                } else {
                    b = mid;
                }
            }
            (a + b) / 2.0
        };
        let (mut ra, mut rb) = (region, region);
        ra.max[axis] = cut;
        rb.min[axis] = cut;
        let right = items.split_off(k);
        self.spread(grid, pos, area, items, ra, out);
        self.spread(grid, pos, area, right, rb, out);
    }

    fn place_connectors(&mut self, pos: &[P], failed: &mut Vec<String>) -> BTreeMap<String, Edge> {
        let bb = self.b.bounds;
        let list: Vec<usize> = (0..self.parts.len())
            .filter(|i| {
                self.parts[*i].active
                    && !self.parts[*i].placed
                    && self.parts[*i].role == Role::Connector
            })
            .collect();
        let bottom = self.sides == Sides::Bottom;
        let geo: Vec<EdgeGeom> = list
            .iter()
            .map(|i| edge_geom(&self.parts[*i], self.b.part_edge, self.b.body_edge))
            .collect();
        let spans_of: Vec<Vec<(f64, f64)>> = list
            .iter()
            .enumerate()
            .map(|(x, i)| {
                EDGES
                    .iter()
                    .map(|e| {
                        let rot = rotation_for(geo[x].u, e.normal(), bottom);
                        let t = Transform { at: [0.0, 0.0], rotation: rot, mirror: bottom };
                        let tg = e.tangent();
                        let vals: Vec<f64> = self.parts[*i]
                            .loops
                            .iter()
                            .flat_map(|l| l.1.iter())
                            .map(|q| dot(t.apply(*q), tg))
                            .collect();
                        (
                            vals.iter().copied().fold(f64::MAX, f64::min),
                            vals.iter().copied().fold(f64::MIN, f64::max),
                        )
                    })
                    .collect()
            })
            .collect();
        let span = |x: usize, e: Edge| -> (f64, f64) {
            spans_of[x][EDGES.iter().position(|f| *f == e).unwrap_or(0)]
        };
        let conn_of: Vec<ConnKind> = list.iter().map(|i| self.parts[*i].conn).collect();
        let pins_of: Vec<Option<Edge>> = list.iter().map(|i| self.parts[*i].pin_edge).collect();
        let reserve = self
            .parts
            .iter()
            .filter(|p| p.role == Role::Hole)
            .map(|p| p.local.size()[0].max(p.local.size()[1]) + 0.5)
            .fold(1.0, f64::max);
        let range = |e: Edge| -> (f64, f64) {
            match e {
                Edge::Left | Edge::Right => (bb.min[1] + reserve, bb.max[1] - reserve),
                Edge::Top | Edge::Bottom => (bb.min[0] + reserve, bb.max[0] - reserve),
            }
        };
        let dist_to = |x: usize, e: Edge| -> f64 {
            let p = pos[list[x]];
            match e {
                Edge::Left => p[0] - bb.min[0],
                Edge::Right => bb.max[0] - p[0],
                Edge::Top => p[1] - bb.min[1],
                Edge::Bottom => bb.max[1] - p[1],
            }
        };
        let cost_of = |assign: &[Option<Edge>]| -> f64 {
            let mut c = 0.0;
            for (x, e) in assign.iter().enumerate() {
                let Some(e) = e else { continue };
                if let Some(pinned) = pins_of[x]
                    && pinned != *e
                {
                    return f64::MAX;
                }
                c += dist_to(x, *e);
            }
            for e in EDGES {
                let on: Vec<usize> = (0..assign.len()).filter(|x| assign[*x] == Some(e)).collect();
                let used: f64 = on
                    .iter()
                    .map(|x| {
                        let (lo, hi) = span(*x, e);
                        hi - lo + 1.0
                    })
                    .sum();
                let (lo, hi) = range(e);
                if used > hi - lo {
                    c += 1e5 * (used - (hi - lo));
                }
                let rf = on.iter().any(|x| conn_of[*x] == ConnKind::Rf);
                let usb = on.iter().any(|x| conn_of[*x] == ConnKind::Usb);
                if rf && usb {
                    c += 1e4;
                }
            }
            c
        };
        let k = list.len();
        let mut best: Vec<Option<Edge>> = vec![None; k];
        if k <= 7 {
            let mut best_c = f64::MAX;
            for code in 0..4usize.pow(k as u32) {
                let mut c = code;
                let assign: Vec<Option<Edge>> = (0..k)
                    .map(|_| {
                        let e = EDGES[c % 4];
                        c /= 4;
                        Some(e)
                    })
                    .collect();
                let cost = cost_of(&assign);
                if cost < best_c {
                    best_c = cost;
                    best = assign;
                }
            }
        } else {
            let mut order: Vec<usize> = (0..k).collect();
            let areas: Vec<f64> = list.iter().map(|i| self.parts[*i].area).collect();
            order.sort_by(|a, b| areas[*b].total_cmp(&areas[*a]).then(a.cmp(b)));
            for x in order {
                let mut bc = f64::MAX;
                let mut be = Edge::Left;
                for e in EDGES {
                    best[x] = Some(e);
                    let c = cost_of(&best);
                    if c < bc {
                        bc = c;
                        be = e;
                    }
                }
                best[x] = Some(be);
            }
        }
        let mut out = BTreeMap::new();
        for e in EDGES {
            let mut on: Vec<usize> = (0..k).filter(|x| best[*x] == Some(e)).collect();
            let tg = e.tangent();
            on.sort_by(|a, b| {
                dot(pos[list[*a]], tg).total_cmp(&dot(pos[list[*b]], tg)).then(a.cmp(b))
            });
            let (lo, hi) = range(e);
            let spans: Vec<(f64, f64)> = on.iter().map(|x| span(*x, e)).collect();
            let total: f64 = spans.iter().map(|s| s.1 - s.0).sum();
            let gap = ((hi - lo - total) / (on.len() + 1) as f64).max(1.0);
            let mut coords: Vec<f64> = Vec::new();
            let mut used = 0.0;
            for (y, x) in on.iter().enumerate() {
                let (s0, s1) = spans[y];
                let even = lo + gap * (y + 1) as f64 + used - s0;
                used += s1 - s0;
                let free = dot(pos[list[*x]], tg);
                let want = if on.len() > 1 { 0.5 * even + 0.5 * free } else { free };
                let floor = match coords.last() {
                    Some(prev) => prev + spans[y - 1].1 + 1.0 - s0,
                    None => lo - s0,
                };
                coords.push(want.max(floor));
            }
            for y in (0..on.len()).rev() {
                let ceil = match coords.get(y + 1) {
                    Some(next) => next + spans[y + 1].0 - 1.0 - spans[y].1,
                    None => hi - spans[y].1,
                };
                coords[y] = coords[y].min(ceil);
            }
            let level = self.b.outline.iter().map(|q| dot(*q, e.normal())).fold(f64::MIN, f64::max);
            for (y, &x) in on.iter().enumerate() {
                let i = list[x];
                let rot = rotation_for(geo[x].u, e.normal(), bottom);
                let nrm = e.normal();
                let depth = level - geo[x].e_loc;
                let target = {
                    let t = snap(coords[y]);
                    [tg[0] * t + nrm[0] * depth, tg[1] * t + nrm[1] * depth]
                };
                let labels = self.take_labels(i);
                let mut edge = e;
                let mut slid = self.slide_to_edge(i, e, &geo[x], spans[y], coords[y], (lo, hi));
                if slid.is_none() && pins_of[x].is_none() {
                    let mut others: Vec<Edge> = EDGES.into_iter().filter(|o| *o != e).collect();
                    others.sort_by(|a, b| dist_to(x, *a).total_cmp(&dist_to(x, *b)));
                    for o in others {
                        let (olo, ohi) = range(o);
                        let (a0, a1) = span(x, o);
                        let start =
                            dot(pos[i], o.tangent()).clamp(olo - a0, (ohi - a1).max(olo - a0));
                        slid = self.slide_to_edge(i, o, &geo[x], (a0, a1), start, (olo, ohi));
                        if slid.is_some() {
                            edge = o;
                            break;
                        }
                    }
                }
                let found = slid.or_else(|| {
                    self.nearest(i, target, rot, bottom, 60.0, &|_| 0.0).or_else(|| {
                        self.parts[i].over_silk = true;
                        self.nearest(i, target, rot, bottom, 60.0, &|_| 0.0)
                    })
                });
                match found {
                    Some(st) => {
                        self.parts[i].st = st;
                        self.label_where_it_fits(i, st, labels);
                        self.insert(i);
                    }
                    None => {
                        self.parts[i].label = labels.0;
                        failed.push(self.parts[i].reference.clone());
                    }
                }
                out.insert(self.parts[i].reference.clone(), edge);
            }
        }
        out
    }

    fn slide_to_edge(
        &self,
        i: usize,
        e: Edge,
        geo: &EdgeGeom,
        span: (f64, f64),
        start: f64,
        (lo, hi): (f64, f64),
    ) -> Option<St> {
        let bottom = self.sides == Sides::Bottom;
        let rot = rotation_for(geo.u, e.normal(), bottom);
        let (tg, nrm) = (e.tangent(), e.normal());
        let mut steps = 0i64;
        while (steps as f64) * GRID < (hi - lo) {
            for sgn in [1.0, -1.0] {
                let t = snap(start + sgn * steps as f64 * GRID);
                if let Some(level) = edge_level(&self.b.outline, nrm, tg, t + span.0, t + span.1) {
                    let depth = level - geo.e_loc;
                    let at = [tg[0] * t + nrm[0] * depth, tg[1] * t + nrm[1] * depth];
                    let st = St { at, rot, bottom };
                    if self.legal(i, st, &[]) {
                        return Some(st);
                    }
                }
                if steps == 0 {
                    break;
                }
            }
            steps += 1;
        }
        None
    }

    fn on_parts(&self, side: u8, poly: &[P]) -> bool {
        let b = poly_bounds(poly);
        self.grid.cells(&b).any(|c| {
            self.grid.cells[c].iter().any(|&j| {
                self.parts[j].placed
                    && (self.cache[j].iter().any(|s| {
                        s.side & side != 0
                            && strict_overlap(&s.b, &b)
                            && polys_overlap(&s.poly, poly)
                    }) || self.label_at[j]
                        .is_some_and(|(ls, lb)| ls & side != 0 && strict_overlap(&lb, &b)))
            })
        })
    }

    fn move_texts(&self, texts: &[SilkText]) -> (Vec<TextMove>, Vec<String>) {
        let side = |t: &SilkText| if t.layer.starts_with("B.") { 2u8 } else { 1u8 };
        let mut taken: Vec<Vec<P>> = texts.iter().map(|t| grow(&t.outline(), SILK_ROOM)).collect();
        let edge = self.board_edge();
        let mut moved = Vec::new();
        let mut stuck = Vec::new();
        for (k, t) in texts.iter().enumerate() {
            let sd = side(t);
            if !self.on_parts(sd, &taken[k]) {
                continue;
            }
            let base = grow(&t.outline(), SILK_ROOM);
            let clear = |poly: &[P]| {
                let b = poly_bounds(poly);
                poly.iter().all(|v| self.edge_clear(edge, *v, 0.0))
                    && !self.on_parts(sd, poly)
                    && !self.b.silk.iter().any(|a| {
                        a.side & sd != 0 && strict_overlap(&a.b, &b) && polys_overlap(&a.poly, poly)
                    })
                    && texts.iter().zip(&taken).enumerate().all(|(x, (o, q))| {
                        x == k
                            || side(o) != sd
                            || !(strict_overlap(&poly_bounds(q), &b) && polys_overlap(q, poly))
                    })
            };
            let found = self
                .offsets
                .iter()
                .take_while(|o| o[0].hypot(o[1]) <= TEXT_REACH)
                .map(|o| (*o, base.iter().map(|q| [q[0] + o[0], q[1] + o[1]]).collect::<Vec<P>>()))
                .find(|(_, poly)| clear(poly));
            match found {
                Some((o, poly)) => {
                    taken[k] = poly;
                    let to = [
                        ((t.at[0] + o[0]) * 1e4).round() / 1e4,
                        ((t.at[1] + o[1]) * 1e4).round() / 1e4,
                    ];
                    moved.push(TextMove {
                        text: t.text.clone(),
                        layer: t.layer.clone(),
                        from: t.at,
                        to,
                    });
                }
                None => stuck.push(t.text.clone()),
            }
        }
        (moved, stuck)
    }

    fn take_labels(&mut self, i: usize) -> (Option<LabelBox>, Option<LabelBox>) {
        (self.parts[i].label.take(), self.parts[i].label_inward.take())
    }

    fn label_where_it_fits(
        &mut self,
        i: usize,
        st: St,
        labels: (Option<LabelBox>, Option<LabelBox>),
    ) {
        if self.soft_labels {
            self.parts[i].label = labels.0;
            return;
        }
        for l in [labels.0, labels.1].into_iter().flatten() {
            self.parts[i].label = Some(l);
            if self.legal(i, st, &[]) {
                return;
            }
        }
        self.parts[i].label = None;
    }

    fn other_ends(&self, i: usize, n: usize, est: &[P]) -> Option<P> {
        let mut s = [0.0, 0.0];
        let mut c = 0.0;
        for &(j, k) in &self.nets[n].pins {
            if j == i {
                continue;
            }
            let p = if self.parts[j].placed { self.pad_pos(j, k) } else { est[j] };
            s[0] += p[0];
            s[1] += p[1];
            c += 1.0;
        }
        (c > 0.0).then(|| [s[0] / c, s[1] / c])
    }

    fn best_rotation(&self, i: usize, at: P, bottom: bool, est: &[P]) -> f64 {
        let nets = self.signal_nets(i);
        let ends: Vec<(usize, P, f64)> = nets
            .iter()
            .filter_map(|n| self.other_ends(i, *n, est).map(|p| (*n, p, self.nets[*n].weight)))
            .collect();
        let mut best = (f64::MAX, 0.0);
        for r in [0.0, 90.0, 180.0, 270.0] {
            let t = Transform { at, rotation: r, mirror: bottom };
            let mut c = 0.0;
            for (n, p, w) in &ends {
                for (k, q) in self.parts[i].pads.iter().enumerate() {
                    if q.net == Some(*n) {
                        let _ = k;
                        c += w * geom::dist(t.apply(q.c), *p);
                    }
                }
            }
            if c < best.0 - 1e-9 {
                best = (c, r);
            }
        }
        best.1
    }

    fn try_place(&mut self, i: usize, target: P, rot: f64, prefer_bottom: bool) -> bool {
        let sides: Vec<bool> = match self.sides {
            Sides::Top => vec![false],
            Sides::Bottom => vec![true],
            Sides::Both if self.parts[i].through || self.parts[i].large => vec![false],
            Sides::Both if prefer_bottom => vec![true, false],
            Sides::Both => vec![false, true],
        };
        let mut best: Option<(f64, St)> = None;
        for (x, &bottom) in sides.iter().enumerate() {
            let flex = |st: St| -> f64 { self.flex_penalty(i, st) };
            if let Some(st) = self.nearest(i, target, rot, bottom, 200.0, &flex) {
                let d = geom::dist(st.at, target) + if x > 0 { 1.0 } else { 0.0 };
                if best.as_ref().is_none_or(|b| d < b.0) {
                    best = Some((d, st));
                }
                if d < 2.5 {
                    break;
                }
            }
        }
        match best {
            Some((_, st)) => {
                self.parts[i].st = st;
                self.insert(i);
                true
            }
            None if self.parts[i].label.is_some() && !self.soft_labels => {
                self.parts[i].label = None;
                self.try_place(i, target, rot, prefer_bottom)
            }
            None if !self.parts[i].over_silk => {
                self.parts[i].over_silk = true;
                self.try_place(i, target, rot, prefer_bottom)
            }
            None if self.parts[i].spread_hot => {
                self.parts[i].spread_hot = false;
                self.try_place(i, target, rot, prefer_bottom)
            }
            None => false,
        }
    }

    fn flex_penalty(&self, i: usize, st: St) -> f64 {
        let Some(len) = self.parts[i].mlcc_len else { return 0.0 };
        match self.flex_hit(i, st) {
            None => 0.0,
            Some(_) if len >= LARGE_MLCC => FLEX_LARGE,
            Some(true) => FLEX_POINTING,
            Some(false) => 0.0,
        }
    }

    fn passive_rotation(
        &self,
        i: usize,
        at: P,
        bottom: bool,
        toward: Option<(usize, P)>,
        axis: Option<bool>,
        est: &[P],
    ) -> f64 {
        let p = &self.parts[i];
        let mut best = (f64::MAX, 0.0);
        for r in [0.0, 90.0, 180.0, 270.0] {
            let t = Transform { at, rotation: r, mirror: bottom };
            if let (Some(horizontal), Some((ka, kb))) = (axis, p.ends) {
                let (a, b) = (t.apply(p.pads[ka].c), t.apply(p.pads[kb].c));
                let h = (b[0] - a[0]).abs() >= (b[1] - a[1]).abs();
                if h != horizontal {
                    continue;
                }
            }
            let mut c = 0.0;
            if let Some((k, q)) = toward {
                c += geom::dist(t.apply(p.pads[k].c), q) * 3.0;
            }
            for n in self.signal_nets(i) {
                if let Some(o) = self.other_ends(i, n, est) {
                    for q in p.pads.iter().filter(|q| q.net == Some(n)) {
                        c += geom::dist(t.apply(q.c), o);
                    }
                }
            }
            if c < best.0 - 1e-9 {
                best = (c, r);
            }
        }
        best.1
    }

    fn legalise(&mut self, pos: &[P], failed: &mut Vec<String>) {
        let n = self.parts.len();
        let est: Vec<P> =
            (0..n).map(|i| if self.parts[i].placed { self.centre(i) } else { pos[i] }).collect();
        let mut est = est;
        let mut anchors: Vec<usize> = (0..n)
            .filter(|i| {
                let p = &self.parts[*i];
                let follower = matches!(p.role, Role::Passive | Role::Crystal | Role::TestPoint);
                let anchor = self.cluster_of[*i].is_some_and(|c| self.clusters[c].anchor == *i);
                p.active && !p.placed && (!follower || anchor)
            })
            .collect();
        anchors.sort_by(|a, b| {
            let (pa, pb) = (&self.parts[*a], &self.parts[*b]);
            pb.large.cmp(&pa.large).then(pb.area.total_cmp(&pa.area)).then(a.cmp(b))
        });
        let mut todo_members: Vec<usize> = Vec::new();
        for &a in &anchors {
            let own = self.cluster_of[a].filter(|c| self.clusters[*c].anchor == a);
            if !self.parts[a].placed {
                let c = self.b.centre;
                let target = if self.parts[a].large {
                    [
                        pos[a][0] + (c[0] - pos[a][0]) * CENTRE_BLEND,
                        pos[a][1] + (c[1] - pos[a][1]) * CENTRE_BLEND,
                    ]
                } else {
                    pos[a]
                };
                let Some((at, rot)) = self.cluster_spot(a, own, target, &est) else {
                    failed.push(self.parts[a].reference.clone());
                    continue;
                };
                if !self.try_place(a, at, rot, false) {
                    failed.push(self.parts[a].reference.clone());
                    continue;
                }
                est[a] = self.centre(a);
            }
            if let Some(c) = own {
                todo_members.push(c);
                self.place_members(c, &mut est, failed);
            }
        }
        for c in 0..self.clusters.len() {
            let a = self.clusters[c].anchor;
            if self.parts[a].placed && !todo_members.contains(&c) {
                self.place_members(c, &mut est, failed);
            }
        }
        let mut rest: Vec<usize> =
            (0..n).filter(|i| self.parts[*i].active && !self.parts[*i].placed).collect();
        while !rest.is_empty() {
            let score = |i: usize, s: &Self| -> usize {
                s.signal_nets(i)
                    .iter()
                    .flat_map(|x| s.nets[*x].pins.iter())
                    .filter(|(j, _)| s.parts[*j].placed)
                    .count()
            };
            let (x, &i) = rest
                .iter()
                .enumerate()
                .max_by(|a, b| score(*a.1, self).cmp(&score(*b.1, self)).then(b.1.cmp(a.1)))
                .unwrap();
            rest.remove(x);
            let mut s = [0.0, 0.0];
            let mut c = 0.0;
            for net in self.signal_nets(i) {
                for &(j, k) in &self.nets[net].pins {
                    if j != i && self.parts[j].placed {
                        let p = self.pad_pos(j, k);
                        s[0] += p[0];
                        s[1] += p[1];
                        c += 1.0;
                    }
                }
            }
            let target = if c > 0.0 { [s[0] / c, s[1] / c] } else { pos[i] };
            let bottom = self.sides == Sides::Bottom;
            let rot = if self.parts[i].role == Role::Passive {
                self.passive_rotation(i, target, bottom, None, None, &est)
            } else {
                self.best_rotation(i, target, bottom, &est)
            };
            if self.try_place(i, target, rot, false) {
                est[i] = self.centre(i);
            } else {
                failed.push(self.parts[i].reference.clone());
            }
        }
    }

    fn est_cost(&self, set: &[usize]) -> f64 {
        let mut nets: Vec<usize> =
            set.iter().flat_map(|i| self.part_nets[*i].iter().copied()).collect();
        nets.sort_unstable();
        nets.dedup();
        let mut cost = 0.0;
        for n in nets {
            let net = &self.nets[n];
            if net.power || net.weight == 0.0 {
                continue;
            }
            let mut b = Bounds::EMPTY;
            let mut count = 0;
            for &(i, k) in &net.pins {
                if !self.parts[i].placed && !self.parts[i].active {
                    continue;
                }
                let p = self.pad_pos(i, k);
                b.add(p);
                count += 1;
            }
            if count >= 2 {
                cost += net.weight * (b.size()[0] + b.size()[1]);
            }
            if self.is_two_pin[n] {
                cost += self.cross_w * self.est_crossings(n) as f64;
            }
        }
        let mut links: Vec<usize> =
            set.iter().flat_map(|i| self.part_links[*i].iter().copied()).collect();
        links.sort_unstable();
        links.dedup();
        for l in links {
            cost += self.link_cost(&self.links[l]);
        }
        cost + set.iter().map(|i| self.unary(*i)).sum::<f64>()
    }

    fn est_segment(&self, n: usize) -> Option<(P, P)> {
        let pins = &self.nets[n].pins;
        let end = |(i, k): (usize, usize)| {
            (self.parts[i].placed || self.parts[i].active).then(|| self.pad_pos(i, k))
        };
        Some((end(pins[0])?, end(pins[1])?))
    }

    fn est_crossings(&self, n: usize) -> usize {
        let Some((a, b)) = self.est_segment(n) else { return 0 };
        self.two_pin
            .iter()
            .filter(|m| **m != n)
            .filter_map(|m| self.est_segment(*m))
            .filter(|(c, d)| geom::segments_intersect(a, b, *c, *d))
            .count()
    }

    fn cluster_spot(
        &mut self,
        a: usize,
        own: Option<usize>,
        target: P,
        est: &[P],
    ) -> Option<(P, f64)> {
        let bottom = self.sides == Sides::Bottom;
        let lc = |r: f64, s: &Self| {
            Transform { at: [0.0, 0.0], rotation: r, mirror: bottom }
                .apply(s.parts[a].local.center())
        };
        let first = {
            let l = lc(0.0, self);
            self.best_rotation(a, [target[0] - l[0], target[1] - l[1]], bottom, est)
        };
        let Some(c) = own else {
            let l = lc(first, self);
            return Some(([target[0] - l[0], target[1] - l[1]], first));
        };
        let members: Vec<usize> = self.clusters[c]
            .members
            .iter()
            .copied()
            .filter(|m| self.parts[*m].active && !self.parts[*m].placed)
            .collect();
        let mut set = vec![a];
        set.extend(members.iter().copied());
        let area: f64 = set.iter().map(|i| self.parts[*i].area).sum::<f64>() * MACRO_ROOM;
        let step = area.sqrt() * CLUSTER_NUDGE;
        let turns: Vec<f64> = if self.parts[a].role == Role::Chip {
            (0..4).map(|k| (first + 90.0 * k as f64).rem_euclid(360.0)).collect()
        } else {
            vec![first]
        };
        let offs = [[0.0, 0.0], [step, 0.0], [-step, 0.0], [0.0, step], [0.0, -step]];
        let mut best: Option<(f64, P, f64)> = None;
        for &rot in &turns {
            for off in offs {
                let l = lc(rot, self);
                let at = [target[0] + off[0] - l[0], target[1] + off[1] - l[1]];
                let mut lost = Vec::new();
                let mut trial = est.to_vec();
                if self.try_place(a, at, rot, false) {
                    trial[a] = self.centre(a);
                    self.place_members(c, &mut trial, &mut lost);
                } else {
                    continue;
                }
                let cost = self.est_cost(&set) + 1e4 * lost.len() as f64;
                for &i in &set {
                    if self.parts[i].placed {
                        self.remove(i);
                    }
                }
                if best.as_ref().is_none_or(|b| cost < b.0 - 1e-9) {
                    best = Some((cost, at, rot));
                }
            }
        }
        best.map(|b| (b.1, b.2))
    }

    fn place_members(&mut self, c: usize, est: &mut [P], failed: &mut Vec<String>) {
        let a = self.clusters[c].anchor;
        let mut members: Vec<usize> = self.clusters[c]
            .members
            .iter()
            .copied()
            .filter(|m| self.parts[*m].active && !self.parts[*m].placed)
            .collect();
        let rank = |m: usize, s: &Self| -> (u8, i64) {
            let p = &s.parts[m];
            let served = s.clusters[c].served.contains_key(&m);
            let f = cap_farads(&p.value).unwrap_or(1.0);
            let class = if p.switcher {
                0
            } else if p.role == Role::Crystal {
                1
            } else if served {
                2
            } else {
                3
            };
            (class, (f * 1e12) as i64)
        };
        members.sort_by(|x, y| rank(*x, self).cmp(&rank(*y, self)).then(x.cmp(y)));
        let ac = self.centre(a);
        let mut hv = 0.0;
        let mut targets: Vec<Target> = Vec::new();
        for &m in &members {
            let (target, toward) = if let Some(&(k, pin)) = self.clusters[c].served.get(&m) {
                let q = self.pad_pos(a, pin);
                (q, Some((k, q)))
            } else {
                let mut s = [0.0, 0.0];
                let mut cnt = 0.0;
                for net in self.signal_nets(m) {
                    for &(j, k) in &self.nets[net].pins {
                        if j != m && (self.parts[j].placed || j == a) {
                            let p = self.pad_pos(j, k);
                            s[0] += p[0];
                            s[1] += p[1];
                            cnt += 1.0;
                        }
                    }
                }
                if cnt > 0.0 { ([s[0] / cnt, s[1] / cnt], None) } else { (ac, None) }
            };
            let d = [target[0] - ac[0], target[1] - ac[1]];
            hv += d[0].abs() - d[1].abs();
            targets.push((m, target, toward));
        }
        let horizontal = hv >= 0.0;
        for (m, target, toward) in targets {
            let is_passive = self.parts[m].role == Role::Passive;
            let under = self.sides == Sides::Both && self.parts[a].large && toward.is_some();
            let bottom = self.sides == Sides::Bottom || under;
            let rot = if is_passive {
                self.passive_rotation(m, target, bottom, toward, Some(horizontal), est)
            } else {
                self.best_rotation(m, target, bottom, est)
            };
            if self.try_place(m, target, rot, under) {
                est[m] = self.centre(m);
                if is_passive && self.parts[m].st.bottom != bottom {
                    let st = self.parts[m].st;
                    let r2 =
                        self.passive_rotation(m, st.at, st.bottom, toward, Some(horizontal), est);
                    let st2 = St { rot: r2, ..st };
                    self.remove(m);
                    if self.legal(m, st2, &[]) {
                        self.parts[m].st = st2;
                    }
                    self.insert(m);
                }
            } else {
                failed.push(self.parts[m].reference.clone());
            }
        }
    }

    fn cluster_set(&self, c: usize) -> Vec<usize> {
        let a = self.clusters[c].anchor;
        let mut set = vec![a];
        set.extend(self.clusters[c].members.iter().copied().filter(|m| self.movable(*m)));
        set
    }

    fn try_clusters(&mut self, jobs: &[(usize, P, Option<f64>)], reach: f64) -> Option<Trial> {
        let sets: Vec<Vec<usize>> = jobs.iter().map(|j| self.cluster_set(j.0)).collect();
        let all: Vec<usize> = sets.iter().flatten().copied().collect();
        let old: Vec<St> = all.iter().map(|i| self.parts[*i].st).collect();
        for &i in &all {
            self.remove(i);
        }
        let mut est: Vec<P> = (0..self.parts.len()).map(|i| self.centre(i)).collect();
        let mut ok = true;
        for &(c, centre, rot) in jobs {
            let a = self.clusters[c].anchor;
            let bottom = self.parts[a].st.bottom;
            let rot = rot.unwrap_or_else(|| self.best_rotation(a, centre, bottom, &est));
            let lc = Transform { at: [0.0, 0.0], rotation: rot, mirror: bottom }
                .apply(self.parts[a].local.center());
            let at = snap_p([centre[0] - lc[0], centre[1] - lc[1]]);
            match self.nearest(a, at, rot, bottom, reach, &|_| 0.0) {
                Some(st) => {
                    self.parts[a].st = st;
                    self.insert(a);
                    est[a] = self.centre(a);
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            for &(c, _, _) in jobs {
                let mut lost = Vec::new();
                self.place_members(c, &mut est, &mut lost);
                ok &= lost.is_empty();
            }
        }
        let out = (ok && all.iter().all(|i| self.parts[*i].placed)).then(|| {
            let cost = self.local_cost(&all);
            (cost, all.iter().map(|i| (*i, self.parts[*i].st)).collect())
        });
        for &i in &all {
            if self.parts[i].placed {
                self.remove(i);
            }
        }
        for (x, &i) in all.iter().enumerate() {
            self.parts[i].st = old[x];
            self.insert(i);
        }
        out
    }

    fn adopt(&mut self, states: &[(usize, St)]) {
        for (i, _) in states {
            self.remove(*i);
        }
        for (i, st) in states {
            self.parts[*i].st = *st;
            self.insert(*i);
        }
    }

    fn cluster_area(&self, c: usize) -> f64 {
        self.cluster_set(c).iter().map(|i| self.parts[*i].area).sum::<f64>() * MACRO_ROOM
    }

    fn turn_clusters(&mut self, order: &[usize]) {
        for &c in order {
            let a = self.clusters[c].anchor;
            let set = self.cluster_set(c);
            let before = self.local_cost(&set);
            let centre = self.centre(a);
            let rot0 = self.parts[a].st.rot;
            let step = self.cluster_area(c).sqrt() * CLUSTER_NUDGE;
            let mut best: Option<Trial> = None;
            for turn in [0.0, 90.0, 180.0, 270.0] {
                for off in [[0.0, 0.0], [step, 0.0], [-step, 0.0], [0.0, step], [0.0, -step]] {
                    if turn == 0.0 && off == [0.0, 0.0] {
                        continue;
                    }
                    let at = [centre[0] + off[0], centre[1] + off[1]];
                    let rot = Some((rot0 + turn).rem_euclid(360.0));
                    if let Some(t) = self.try_clusters(&[(c, at, rot)], 2.0 * step + 1.0)
                        && t.0 < before - 1e-6
                        && best.as_ref().is_none_or(|b| t.0 < b.0)
                    {
                        best = Some(t);
                    }
                }
            }
            if let Some((_, states)) = best {
                self.adopt(&states);
            }
        }
    }

    fn swap_clusters(&mut self, order: &[usize]) -> Vec<usize> {
        let mut swapped = Vec::new();
        for (x, &c) in order.iter().enumerate() {
            for &d in &order[x + 1..] {
                let (ac, ad) = (self.cluster_area(c), self.cluster_area(d));
                if ac > 2.0 * ad || ad > 2.0 * ac {
                    continue;
                }
                let (a, b) = (self.clusters[c].anchor, self.clusters[d].anchor);
                let mut set = self.cluster_set(c);
                set.extend(self.cluster_set(d));
                let before = self.local_cost(&set);
                let (pa, pb) = (self.centre(a), self.centre(b));
                let reach = (ac.max(ad)).sqrt();
                if let Some(t) = self.try_clusters(&[(c, pb, None), (d, pa, None)], reach)
                    && t.0 < before - 1e-6
                {
                    self.adopt(&t.1);
                    swapped.extend([c, d]);
                }
            }
        }
        swapped
    }

    fn rearrange_clusters(&mut self) {
        let mut order: Vec<usize> = (0..self.clusters.len())
            .filter(|c| {
                let a = self.clusters[*c].anchor;
                self.movable(a) && self.parts[a].role == Role::Chip
            })
            .collect();
        order.sort_by(|x, y| {
            let (a, b) = (self.clusters[*x].anchor, self.clusters[*y].anchor);
            self.parts[b].area.total_cmp(&self.parts[a].area).then(x.cmp(y))
        });
        self.turn_clusters(&order);
        let swapped = self.swap_clusters(&order);
        let order: Vec<usize> = order.iter().copied().filter(|c| swapped.contains(c)).collect();
        self.turn_clusters(&order);
    }

    fn clear_flex_zone(&mut self) {
        for i in 0..self.parts.len() {
            if !self.movable(i) || self.flex_penalty(i, self.parts[i].st) == 0.0 {
                continue;
            }
            let st0 = self.parts[i].st;
            self.remove(i);
            let turns = [0.0, 90.0, 270.0, 180.0].map(|t| (st0.rot + t).rem_euclid(360.0));
            let mut found = None;
            for k in 0..self.offsets.len() {
                let off = self.offsets[k];
                if off[0].hypot(off[1]) > FLEX_REACH {
                    break;
                }
                let at = snap_p([st0.at[0] + off[0], st0.at[1] + off[1]]);
                found = turns
                    .iter()
                    .map(|rot| St { at, rot: *rot, ..st0 })
                    .find(|st| self.flex_penalty(i, *st) == 0.0 && self.legal(i, *st, &[]));
                if found.is_some() {
                    break;
                }
            }
            self.parts[i].st = found.unwrap_or(st0);
            self.insert(i);
        }
    }

    fn movable(&self, i: usize) -> bool {
        let p = &self.parts[i];
        p.active && p.placed && !matches!(p.role, Role::Connector | Role::Hole | Role::Fiducial)
    }

    fn refine(&mut self, rng: &mut Rng, per_part: f64, heat: f64) -> usize {
        let movable: Vec<usize> = (0..self.parts.len()).filter(|i| self.movable(*i)).collect();
        if movable.is_empty() {
            return 0;
        }
        let total = (movable.len() as f64 * per_part) as usize;
        let size = self.b.bounds.size();
        let r0 = (size[0].min(size[1]) / 4.0).clamp(1.0, 8.0);
        let mut samples = Vec::new();
        let mut accepted = 0;
        let mut temp = 1.0;
        for step in 0..total + 300 {
            let warm = step < 300;
            let t = if warm { 0.0 } else { (step - 300) as f64 / total as f64 };
            if step == 300 {
                let mean = if samples.is_empty() {
                    1.0
                } else {
                    samples.iter().sum::<f64>() / samples.len() as f64
                };
                temp = (mean * heat).max(1e-3);
            }
            let cur_t = temp * (0.002f64).powf(t);
            let range = (r0 * (1.0 - t)).max(0.25);
            let i = movable[rng.below(movable.len())];
            let pick = rng.unit();
            let cluster_anchor = self.cluster_of[i].filter(|c| self.clusters[*c].anchor == i);
            let mut set: Vec<usize> = vec![i];
            let mut new: Vec<St> = Vec::new();
            let off = snap_p([(rng.unit() * 2.0 - 1.0) * range, (rng.unit() * 2.0 - 1.0) * range]);
            let st = self.parts[i].st;
            if let Some(c) = cluster_anchor.filter(|_| pick < 0.12) {
                set.extend(self.clusters[c].members.iter().copied().filter(|m| self.movable(*m)));
                for &m in &set {
                    let s = self.parts[m].st;
                    new.push(St { at: [s.at[0] + off[0], s.at[1] + off[1]], ..s });
                }
            } else if pick < 0.3 {
                let shared = self.parts[i].role == Role::Passive && self.cluster_of[i].is_some();
                let turn = if shared { 180.0 } else { [90.0, 180.0, 270.0][rng.below(3)] };
                let flip = self.sides == Sides::Both
                    && self.parts[i].role == Role::Passive
                    && !self.parts[i].through
                    && rng.unit() < 0.3;
                let rot = (st.rot + if flip { 0.0 } else { turn }).rem_euclid(360.0);
                new.push(St { rot, bottom: st.bottom != flip, ..st });
            } else if pick < 0.45 {
                let j = movable[rng.below(movable.len())];
                let (sa, sb) = (self.parts[i].local.size(), self.parts[j].local.size());
                if j == i || (sa[0] - sb[0]).abs() > 0.3 || (sa[1] - sb[1]).abs() > 0.3 {
                    continue;
                }
                set.push(j);
                let sj = self.parts[j].st;
                new.push(St { at: sj.at, ..st });
                new.push(St { at: st.at, ..sj });
            } else {
                new.push(St { at: [st.at[0] + off[0], st.at[1] + off[1]], ..st });
            }
            let old: Vec<St> = set.iter().map(|m| self.parts[*m].st).collect();
            let before = self.local_cost(&set);
            for &m in &set {
                self.remove(m);
            }
            let mut ok = true;
            for (x, &m) in set.iter().enumerate() {
                if !self.legal_shape(m, new[x], &set) {
                    ok = false;
                    break;
                }
            }
            if ok && set.len() == 2 && new.len() == 2 {
                let (a, b) = (self.shapes(set[0], new[0]), self.shapes(set[1], new[1]));
                ok = !a.iter().any(|s| {
                    b.iter().any(|t| {
                        s.side & t.side != 0
                            && strict_overlap(&s.b, &t.b)
                            && ((s.rect && t.rect) || polys_overlap(&s.poly, &t.poly))
                    })
                });
            }
            if !ok {
                for (x, &m) in set.iter().enumerate() {
                    self.parts[m].st = old[x];
                    self.insert(m);
                }
                continue;
            }
            for (x, &m) in set.iter().enumerate() {
                self.parts[m].st = new[x];
                self.insert(m);
            }
            let after = self.local_cost(&set);
            let delta = after - before;
            if warm {
                samples.push(delta.abs());
                for &m in &set {
                    self.remove(m);
                }
                for (x, &m) in set.iter().enumerate() {
                    self.parts[m].st = old[x];
                    self.insert(m);
                }
                continue;
            }
            let take = delta <= 0.0 || rng.unit() < (-delta / cur_t).exp();
            let copper = take
                && set.iter().enumerate().all(|(x, &m)| {
                    let sh = self.shapes(m, new[x]);
                    self.copper_ok(m, new[x], &sh, &set)
                });
            if copper {
                accepted += 1;
            } else {
                for &m in &set {
                    self.remove(m);
                }
                for (x, &m) in set.iter().enumerate() {
                    self.parts[m].st = old[x];
                    self.insert(m);
                }
            }
        }
        accepted
    }

    fn share_rotation(&mut self) {
        for c in 0..self.clusters.len() {
            let members: Vec<usize> = self.clusters[c]
                .members
                .iter()
                .copied()
                .filter(|m| self.movable(*m) && self.parts[*m].role == Role::Passive)
                .collect();
            let mut votes = 0i32;
            for &m in &members {
                if let Some(ax) = self.axis(m) {
                    votes += if ax[0].abs() >= ax[1].abs() { 1 } else { -1 };
                }
            }
            let horizontal = votes >= 0;
            for &m in &members {
                let Some(ax) = self.axis(m) else { continue };
                if (ax[0].abs() >= ax[1].abs()) == horizontal {
                    continue;
                }
                let st = self.parts[m].st;
                let before = self.local_cost(&[m]);
                self.remove(m);
                let mut best: Option<(f64, St)> = None;
                for turn in [90.0, 270.0] {
                    let cand = St { rot: (st.rot + turn).rem_euclid(360.0), ..st };
                    if self.legal(m, cand, &[]) {
                        self.parts[m].st = cand;
                        self.insert(m);
                        let c2 = self.local_cost(&[m]);
                        self.remove(m);
                        if best.as_ref().is_none_or(|b| c2 < b.0) {
                            best = Some((c2, cand));
                        }
                    }
                }
                self.parts[m].st = match best {
                    Some((c2, cand)) if c2 <= before + 2.0 => cand,
                    _ => st,
                };
                self.insert(m);
            }
        }
    }
}
