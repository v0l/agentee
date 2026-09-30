use crate::board::Board;
use crate::footprint::{Footprint, PadKind};
use crate::geom::{self, P, Transform};
use crate::graphic::{Bounds, Shape};
use crate::layout::{BoardSide, PlacementFile, glob};
use crate::schematic::{PinRef, Schematic};
use crate::units::{Length, Point};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const GRID: f64 = 0.05;
pub const FIDUCIAL_TO_EDGE: f64 = 3.0;
const HOT_GAP: f64 = 10.0;
const QUIET_GAP: f64 = 8.0;
const HOT_WATTS: f64 = 0.25;
const EPS: f64 = 1e-6;
const STARTS: u64 = 4;
const CENTRE_PULL: f64 = 2.0;
const CENTRE_BLEND: f64 = 0.5;
const MOVES_PER_PART: f64 = 600.0;

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
}

impl Default for PlaceOptions {
    fn default() -> Self {
        PlaceOptions { parts: Vec::new(), keep_placed: false, sides: Sides::Top, seed: 1 }
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
}

#[derive(Clone, Debug, Serialize)]
pub struct Placement {
    pub reference: String,
    pub at: P,
    pub rotation: f64,
    pub bottom: bool,
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

pub fn chip_length(fp_name: &str) -> Option<f64> {
    fp_name.split(|c: char| !c.is_ascii_alphanumeric()).find_map(|t| {
        let m = t.to_ascii_lowercase();
        let digits = m.strip_suffix("metric")?;
        if digits.len() == 4 && digits.chars().all(|c| c.is_ascii_digit()) {
            digits[..2].parse::<f64>().ok().map(|x| x / 10.0)
        } else {
            None
        }
    })
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

fn polys_overlap(a: &[P], b: &[P]) -> bool {
    let n = a.len();
    let m = b.len();
    for i in 0..n {
        for j in 0..m {
            if geom::segments_intersect(a[i], a[(i + 1) % n], b[j], b[(j + 1) % m]) {
                return true;
            }
        }
    }
    let inner = |p: P, q: &[P]| {
        let c = poly_bounds(q).center();
        let s = [p[0] + (c[0] - p[0]) * 1e-4, p[1] + (c[1] - p[1]) * 1e-4];
        geom::point_in_polygon(s, q)
    };
    a.iter().any(|p| inner(*p, b)) || b.iter().any(|p| inner(*p, a))
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

type Start<'a> = (f64, Placer<'a>, Vec<String>, BTreeMap<String, Edge>, usize);

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
    Pull { w: f64, thr: f64, extra: f64 },
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
    grid: HashMap<(i32, i32), Vec<usize>>,
    cache: Vec<Vec<WShape>>,
    offsets: Vec<P>,
    net_stamp: Vec<u32>,
    link_stamp: Vec<u32>,
    stamp: u32,
    holes: Vec<P>,
}

const CELL: f64 = 2.0;

impl<'a> Placer<'a> {
    fn shapes(&self, i: usize, st: St) -> Vec<WShape> {
        let p = &self.parts[i];
        let t = st.transform();
        p.loops
            .iter()
            .map(|(back, poly)| {
                let world: Vec<P> = poly.iter().map(|q| t.apply(*q)).collect();
                let side = if p.through {
                    3
                } else if *back != st.bottom {
                    2
                } else {
                    1
                };
                WShape { side, b: poly_bounds(&world), rect: is_axis_rect(&world), poly: world }
            })
            .collect()
    }

    fn cells(b: &Bounds) -> impl Iterator<Item = (i32, i32)> {
        let (x0, x1) = ((b.min[0] / CELL).floor() as i32, (b.max[0] / CELL).floor() as i32);
        let (y0, y1) = ((b.min[1] / CELL).floor() as i32, (b.max[1] / CELL).floor() as i32);
        (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
    }

    fn insert(&mut self, i: usize) {
        let sh = self.shapes(i, self.parts[i].st);
        for s in &sh {
            for c in Self::cells(&s.b) {
                let v = self.grid.entry(c).or_default();
                if !v.contains(&i) {
                    v.push(i);
                }
            }
        }
        self.cache[i] = sh;
        self.parts[i].placed = true;
    }

    fn remove(&mut self, i: usize) {
        for s in std::mem::take(&mut self.cache[i]) {
            for c in Self::cells(&s.b) {
                if let Some(v) = self.grid.get_mut(&c) {
                    v.retain(|j| *j != i);
                }
            }
        }
        self.parts[i].placed = false;
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
                    w.iter().all(|v| edge.contains(*v) && self.edge_gap(*v) >= min - 1e-6)
                        && self.clear_of_cutouts(&w, min)
                })
            })
        };
        let body_in = |min: f64| {
            sh.iter().all(|s| {
                s.poly.iter().all(|v| edge.contains(*v) && self.edge_gap(*v) >= min - 1e-6)
                    && !o.iter().any(|v| geom::point_in_polygon(*v, &s.poly))
                    && self.clear_of_cutouts(&s.poly, min)
            })
        };
        let ok = match part.role {
            Role::Connector if edge_mount(part.fp) => {
                part.pads.iter().filter(|q| !q.edge).all(|q| edge.contains(t.apply(q.c)))
            }
            Role::Hole => body_in(0.0) && pads_in(self.b.copper_edge, false),
            Role::Fiducial => body_in(0.0) && pads_in(FIDUCIAL_TO_EDGE, false),
            _ => {
                body_in(self.b.body_edge)
                    && ((part.pads_in_court && self.b.body_edge >= self.b.part_edge)
                        || pads_in(self.b.part_edge, true))
            }
        };
        ok && !sh.iter().any(|s| {
            self.b
                .keepouts
                .iter()
                .any(|k| strict_overlap(&s.b, &poly_bounds(k)) && polys_overlap(&s.poly, k))
        })
    }

    fn clashes(&self, i: usize, sh: &[WShape], skip: &[usize]) -> bool {
        for s in sh {
            for c in Self::cells(&s.b) {
                let Some(v) = self.grid.get(&c) else { continue };
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

    fn legal(&self, i: usize, st: St, skip: &[usize]) -> bool {
        let sh = self.shapes(i, st);
        self.inside(i, st, &sh) && !self.clashes(i, &sh, skip)
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
        rel.iter().for_each(|s| hull.union(&s.b));
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
            let moved: Vec<WShape> = rel
                .iter()
                .map(|s| {
                    let mut b = s.b;
                    b.min = [b.min[0] + at[0], b.min[1] + at[1]];
                    b.max = [b.max[0] + at[0], b.max[1] + at[1]];
                    WShape { side: s.side, b, rect: s.rect, poly: Vec::new() }
                })
                .collect();
            if self.clashes_boxes(i, &moved) {
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

    fn clashes_boxes(&self, i: usize, sh: &[WShape]) -> bool {
        for s in sh.iter().filter(|s| s.rect) {
            for c in Self::cells(&s.b) {
                let Some(v) = self.grid.get(&c) else { continue };
                for &j in v {
                    if j != i
                        && self.cache[j]
                            .iter()
                            .any(|t| s.side & t.side != 0 && t.rect && strict_overlap(&s.b, &t.b))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn pad_pos(&self, i: usize, k: usize) -> P {
        self.parts[i].st.transform().apply(self.parts[i].pads[k].c)
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

    fn segment(&self, n: usize) -> Option<(P, P)> {
        let pins = &self.nets[n].pins;
        let (a, b) = (pins[0], pins[1]);
        if !self.parts[a.0].placed || !self.parts[b.0].placed {
            return None;
        }
        Some((self.pad_pos(a.0, a.1), self.pad_pos(b.0, b.1)))
    }

    fn crossings_of(&self, n: usize) -> usize {
        let Some((a, b)) = self.segment(n) else { return 0 };
        let mut bb = Bounds::EMPTY;
        bb.add(a);
        bb.add(b);
        let mut count = 0;
        for &m in &self.two_pin {
            if m == n {
                continue;
            }
            let Some((c, d)) = self.segment(m) else { continue };
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
            LinkKind::Pull { w, thr, extra } => w * d + extra * (d - thr).max(0.0),
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
        if let Some(len) = p.mlcc_len {
            let edge = self.edge_gap(c);
            let hole = self.holes.iter().map(|h| geom::dist(*h, c)).fold(f64::MAX, f64::min);
            if edge.min(hole) < self.b.flex {
                if len >= 1.8 {
                    cost += 20.0;
                } else if let Some(axis) = self.axis(i) {
                    let n = self.edge_normal(c);
                    if dot(axis, n).abs() > 0.7 {
                        cost += 2.0;
                    }
                }
            }
        }
        cost
    }

    fn axis(&self, i: usize) -> Option<P> {
        let (ka, kb) = self.parts[i].ends?;
        let (a, b) = (self.pad_pos(i, ka), self.pad_pos(i, kb));
        let d = geom::dist(a, b);
        (d > 1e-6).then(|| [(b[0] - a[0]) / d, (b[1] - a[1]) / d])
    }

    fn edge_normal(&self, c: P) -> P {
        let bb = &self.b.bounds;
        let gaps = [
            (c[0] - bb.min[0], [1.0, 0.0]),
            (bb.max[0] - c[0], [1.0, 0.0]),
            (c[1] - bb.min[1], [0.0, 1.0]),
            (bb.max[1] - c[1], [0.0, 1.0]),
        ];
        gaps.iter().min_by(|a, b| a.0.total_cmp(&b.0)).map(|g| g.1).unwrap_or([1.0, 0.0])
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
                    cost += self.crossings_of(n) as f64;
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
                let hit = self.cache[i].iter().any(|s| {
                    self.cache[j].iter().any(|t| {
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

pub fn place(input: &PlaceInput, opts: &PlaceOptions) -> Result<PlaceResult, String> {
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
            loops.push((false, l));
        }
        for l in courtyard_loops(fp, "B.CrtYd") {
            loops.push((true, l));
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
            let (lo, hi) =
                ([bb.min[0] - 0.25, bb.min[1] - 0.25], [bb.max[0] + 0.25, bb.max[1] + 0.25]);
            loops.push((false, vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]]));
        }
        let mut local = Bounds::EMPTY;
        loops.iter().flat_map(|l| l.1.iter()).for_each(|q| local.add(*q));
        let through = fp.pads.iter().any(|q| q.kind == PadKind::Tht || q.kind == PadKind::Npth);
        let mut pad_box = Bounds::EMPTY;
        pads.iter().flat_map(|q| q.outline.iter().flatten()).for_each(|q| pad_box.add(*q));
        let pads_in_court = pad_box.is_empty() || local.contains(&pad_box);
        let role = role_of(r, &fp_name, fp);
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
            chip_length(&fp_name).or(Some(1.0))
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

    let n_parts = parts.len();
    let reach = ob.size()[0].hypot(ob.size()[1]) + 10.0;
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
        grid: HashMap::new(),
        offsets: offsets(reach),
        link_stamp: Vec::new(),
        stamp: 0,
        holes: Vec::new(),
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
        .map(|i| pl.centre(i))
        .collect();

    let mut best: Option<Start> = None;
    for start in 0..STARTS {
        let mut cand = pl.clone();
        let mut rng = Rng(opts.seed.wrapping_mul(0x9e37_79b9).wrapping_add(start));
        let mut failed = Vec::new();
        cand.place_corners(&mut failed);
        let pos = cand.global(&mut rng);
        let edges = cand.place_connectors(&pos, &mut failed);
        cand.legalise(&pos, &mut failed);
        let moves = cand.refine(&mut rng);
        cand.share_rotation();
        let all: Vec<usize> = (0..n_parts).collect();
        let score = cand.local_cost(&all) + 1e4 * failed.len() as f64;
        if best.as_ref().is_none_or(|b| score < b.0) {
            best = Some((score, cand, failed, edges, moves));
        }
    }
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
    let placements = pl
        .parts
        .iter()
        .filter(|p| p.active)
        .map(|p| Placement {
            reference: p.reference.clone(),
            at: [(p.st.at[0] * 1e4).round() / 1e4, (p.st.at[1] * 1e4).round() / 1e4],
            rotation: p.st.rot,
            bottom: p.st.bottom,
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
    Ok(PlaceResult { placements, kept, failed, edges, clusters, before, after, moves })
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
                        LinkKind::Pull { w, thr, extra: 5.0 },
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
                            LinkKind::Pull { w: 3.0, thr: self.b.crystal, extra: 5.0 },
                        ));
                    }
                } else {
                    let w = if p.switcher { 4.0 } else { 0.0 };
                    links.push((
                        End::Centre(m),
                        End::Centre(c.anchor),
                        LinkKind::Pull { w, thr: self.b.spread / 2.0, extra: 1.0 },
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
                match self.nearest(i, target, 0.0, fid_bottom, 40.0, &|_| 0.0) {
                    Some(st) => {
                        self.parts[i].st = st;
                        self.insert(i);
                        if role == Role::Hole {
                            let at = self.centre(i);
                            self.holes.push(at);
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

    fn global(&mut self, rng: &mut Rng) -> Vec<P> {
        let n = self.parts.len();
        let (macros, of) = self.macro_of();
        let m = macros.len();
        let bb = self.b.bounds;
        let centre = self.b.centre;
        let mut pos: Vec<P> = (0..m)
            .map(|_| {
                [
                    centre[0] + (rng.unit() - 0.5) * bb.size()[0] * 0.5,
                    centre[1] + (rng.unit() - 0.5) * bb.size()[1] * 0.5,
                ]
            })
            .collect();
        let radius: Vec<f64> = macros
            .iter()
            .map(|v| {
                let a: f64 = v.iter().map(|i| self.parts[*i].area.max(0.5)).sum();
                (a * 1.3 / std::f64::consts::PI).sqrt()
            })
            .collect();
        let large: Vec<bool> = macros.iter().map(|v| self.parts[v[0]].large).collect();
        let conn: Vec<bool> =
            macros.iter().map(|v| self.parts[v[0]].role == Role::Connector).collect();
        let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); m];
        let mut fixed_pull: Vec<(f64, P)> = vec![(0.0, [0.0, 0.0]); m];
        for net in &self.nets {
            if net.power || net.weight == 0.0 {
                continue;
            }
            let mut ms: Vec<usize> = Vec::new();
            let mut fixed_pts: Vec<P> = Vec::new();
            for &(i, k) in &net.pins {
                if let Some(x) = of[i] {
                    if !ms.contains(&x) {
                        ms.push(x);
                    }
                } else if self.parts[i].placed {
                    fixed_pts.push(self.pad_pos(i, k));
                }
            }
            let k = ms.len() + fixed_pts.len();
            if k < 2 {
                continue;
            }
            let w = net.weight / (k - 1) as f64;
            for (x, &a) in ms.iter().enumerate() {
                for &b in &ms[x + 1..] {
                    adj[a].push((b, w));
                    adj[b].push((a, w));
                }
                for p in &fixed_pts {
                    fixed_pull[a].0 += w;
                    fixed_pull[a].1[0] += w * p[0];
                    fixed_pull[a].1[1] += w * p[1];
                }
            }
        }
        let mut repel: Vec<(usize, usize, f64)> = Vec::new();
        for l in &self.links {
            if let LinkKind::Repel { thr, .. } = l.kind {
                let (a, b) = (Self::end_part(l.a), Self::end_part(l.b));
                if let (Some(x), Some(y)) = (of[a], of[b])
                    && x != y
                {
                    repel.push((x, y, thr));
                }
            }
        }
        let mut obstacles: Vec<(P, f64)> = Vec::new();
        for i in 0..n {
            if self.parts[i].placed {
                let s = self.parts[i].local.size();
                obstacles.push((self.centre(i), (s[0].hypot(s[1])) / 2.0));
            }
        }
        let iterations = 300;
        for it in 0..iterations {
            let spread = (it as f64 / iterations as f64).min(1.0);
            let mut next = pos.clone();
            for a in 0..m {
                let (mut sw, mut sx, mut sy) =
                    (fixed_pull[a].0, fixed_pull[a].1[0], fixed_pull[a].1[1]);
                for &(b, w) in &adj[a] {
                    sw += w;
                    sx += w * pos[b][0];
                    sy += w * pos[b][1];
                }
                let g = if large[a] { CENTRE_PULL * sw.max(1.0) } else { 0.05 * sw.max(0.2) };
                let g = if conn[a] { 0.0 } else { g };
                sw += g;
                sx += g * centre[0];
                sy += g * centre[1];
                if sw > 0.0 {
                    next[a] = [0.5 * pos[a][0] + 0.5 * sx / sw, 0.5 * pos[a][1] + 0.5 * sy / sw];
                }
            }
            for a in 0..m {
                for b in a + 1..m {
                    let mut need = (radius[a] + radius[b]) * spread;
                    if conn[a] && conn[b] {
                        need = need.max(bb.size()[0].min(bb.size()[1]) * 0.5 * spread);
                    }
                    let (dx, dy) = (next[b][0] - next[a][0], next[b][1] - next[a][1]);
                    let d = dx.hypot(dy);
                    if d < need {
                        let (ux, uy) = if d < 1e-9 {
                            let t = (a * 7 + b * 13) as f64;
                            (t.cos(), t.sin())
                        } else {
                            (dx / d, dy / d)
                        };
                        let push = (need - d) / 2.0 * 0.5;
                        next[a][0] -= ux * push;
                        next[a][1] -= uy * push;
                        next[b][0] += ux * push;
                        next[b][1] += uy * push;
                    }
                }
                for (c, r) in &obstacles {
                    let need = (radius[a] + r) * spread;
                    let (dx, dy) = (next[a][0] - c[0], next[a][1] - c[1]);
                    let d = dx.hypot(dy);
                    if d < need && d > 1e-9 {
                        next[a][0] += dx / d * (need - d) * 0.5;
                        next[a][1] += dy / d * (need - d) * 0.5;
                    }
                }
            }
            for &(a, b, thr) in &repel {
                let (dx, dy) = (next[b][0] - next[a][0], next[b][1] - next[a][1]);
                let d = dx.hypot(dy);
                if d < thr * spread && d > 1e-9 {
                    let push = (thr * spread - d) / 4.0;
                    next[a][0] -= dx / d * push;
                    next[a][1] -= dy / d * push;
                    next[b][0] += dx / d * push;
                    next[b][1] += dy / d * push;
                }
            }
            for a in 0..m {
                let r =
                    if conn[a] { 0.0 } else { radius[a].min(bb.size()[0].min(bb.size()[1]) / 2.0) };
                next[a][0] = next[a][0].clamp(bb.min[0] + r, bb.max[0] - r);
                next[a][1] = next[a][1].clamp(bb.min[1] + r, bb.max[1] - r);
            }
            pos = next;
        }
        let mut out = vec![centre; n];
        for (x, v) in macros.iter().enumerate() {
            for &i in v {
                out[i] = pos[x];
            }
        }
        out
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
                let depth = level - geo[x].e_loc;
                let nrm = e.normal();
                let base = |t: f64| -> P {
                    let t = snap(t);
                    [tg[0] * t + nrm[0] * depth, tg[1] * t + nrm[1] * depth]
                };
                let mut done = false;
                let mut steps = 0i64;
                while !done && (steps as f64) * GRID < (hi - lo) {
                    for sgn in [1.0, -1.0] {
                        let at = base(coords[y] + sgn * steps as f64 * GRID);
                        let st = St { at, rot, bottom };
                        if self.legal(i, st, &[]) {
                            self.parts[i].st = st;
                            self.insert(i);
                            done = true;
                            break;
                        }
                        if steps == 0 {
                            break;
                        }
                    }
                    steps += 1;
                }
                if !done {
                    let target = base(coords[y]);
                    match self.nearest(i, target, rot, bottom, 60.0, &|_| 0.0) {
                        Some(st) => {
                            self.parts[i].st = st;
                            self.insert(i);
                        }
                        None => failed.push(self.parts[i].reference.clone()),
                    }
                }
                out.insert(self.parts[i].reference.clone(), e);
            }
        }
        out
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
            None => false,
        }
    }

    fn flex_penalty(&self, i: usize, st: St) -> f64 {
        let p = &self.parts[i];
        let Some(len) = p.mlcc_len else { return 0.0 };
        let c = st.transform().apply(p.local.center());
        let edge = self.edge_gap(c);
        let hole = self.holes.iter().map(|h| geom::dist(*h, c)).fold(f64::MAX, f64::min);
        if edge.min(hole) >= self.b.flex {
            return 0.0;
        }
        if len >= 1.8 { 20.0 } else { 0.0 }
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
            if !self.parts[a].placed {
                let bottom = self.sides == Sides::Bottom;
                let c = self.b.centre;
                let target = if self.parts[a].large {
                    [
                        pos[a][0] + (c[0] - pos[a][0]) * CENTRE_BLEND,
                        pos[a][1] + (c[1] - pos[a][1]) * CENTRE_BLEND,
                    ]
                } else {
                    pos[a]
                };
                let lc = Transform { at: [0.0, 0.0], rotation: 0.0, mirror: bottom }
                    .apply(self.parts[a].local.center());
                let target = [target[0] - lc[0], target[1] - lc[1]];
                let rot = self.best_rotation(a, target, bottom, &est);
                if !self.try_place(a, target, rot, false) {
                    failed.push(self.parts[a].reference.clone());
                    continue;
                }
                est[a] = self.centre(a);
            }
            if let Some(c) = self.cluster_of[a]
                && self.clusters[c].anchor == a
            {
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

    fn movable(&self, i: usize) -> bool {
        let p = &self.parts[i];
        p.active && p.placed && !matches!(p.role, Role::Connector | Role::Hole | Role::Fiducial)
    }

    fn refine(&mut self, rng: &mut Rng) -> usize {
        let movable: Vec<usize> = (0..self.parts.len()).filter(|i| self.movable(*i)).collect();
        if movable.is_empty() {
            return 0;
        }
        let total = (movable.len() as f64 * MOVES_PER_PART) as usize;
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
                temp = (mean / 3.0).max(1e-3);
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
                if !self.legal(m, new[x], &set) {
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
            if delta <= 0.0 || rng.unit() < (-delta / cur_t).exp() {
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
