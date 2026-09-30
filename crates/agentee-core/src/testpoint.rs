use crate::board::Board;
use crate::diag::Diags;
use crate::footprint::PadKind;
use crate::geom::{self, P};
use crate::layout::{LayoutNet, Pair, Placed, Via, glob};
use crate::units::Length;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestFile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through_holes: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vias: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_test_pad: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_test_pad_pitch: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_test_pad_to_body: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_test_pad_to_edge: Option<Length>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TestSpec {
    pub nets: Vec<String>,
    pub exclude: Vec<String>,
    pub side: String,
    pub through_holes: bool,
    pub vias: bool,
    pub min_test_pad: f64,
    pub min_test_pad_pitch: f64,
    pub min_test_pad_to_body: f64,
    pub min_test_pad_to_edge: f64,
}

pub const PAD_FOOTPRINT: &str = "TestPoint_Pad_D1.0mm";

pub const PAD_FOOTPRINT_TOML: &str = r#"name = "TestPoint_Pad_D1.0mm"
description = "SMD round test point pad, 1.0 mm diameter, no paste"
tags = ["test point", "pad"]
mount = "smd"

[[pads]]
number = "1"
kind = "smd"
shape = "circle"
at = [0, 0]
size = [1.0, 1.0]
layers = ["F.Cu", "F.Mask"]

[[graphics]]
kind = "circle"
layer = "F.SilkS"
center = [0, 0]
radius = 0.7
width = 0.15

[[graphics]]
kind = "circle"
layer = "F.CrtYd"
center = [0, 0]
radius = 1.0
width = 0.05

[[graphics]]
kind = "text"
layer = "F.SilkS"
at = [0, -1.45]
text = "${REFERENCE}"
size = 1.0

[[graphics]]
kind = "text"
layer = "F.Fab"
at = [0, 1.45]
text = "${VALUE}"
size = 1.0
"#;

pub const SYMBOL: &str = "TestPoint";

pub const SYMBOL_TOML: &str = r#"name = "TestPoint"
reference = "TP"
description = "Test point, one pad to probe"
keywords = ["test", "point", "tp"]
footprint = "TestPoint_Pad_D1.0mm"
footprint_filters = ["TestPoint*"]
pin_names = "hidden"
hide_pin_numbers = true

[[graphics]]
kind = "circle"
center = [0, -3.048]
radius = 0.762
width = 0.254

[[pins]]
number = "1"
name = "1"
type = "passive"
at = [0, 0]
side = "bottom"
length = 2.286
"#;

pub const DEFAULT_NETS: &[&str] = &[
    "GND", "*GND", "GND*", "*RST*", "*RESET*", "*EN*", "*PG*", "*CLK*", "*TX*", "*RX*", "*SCL*",
    "*SDA*", "*SWD*", "*TCK*", "*TMS*", "*TDI*", "*TDO*",
];

const RAIL_PREFIXES: &[&str] =
    &["VCC", "VDD", "VBUS", "VBAT", "VIN", "VSYS", "VCORE", "VIO", "VREF", "VPP", "PWR", "+"];

impl Default for TestSpec {
    fn default() -> TestSpec {
        TestSpec {
            nets: Vec::new(),
            exclude: Vec::new(),
            side: "B".into(),
            through_holes: true,
            vias: false,
            min_test_pad: 1.0,
            min_test_pad_pitch: 1.27,
            min_test_pad_to_body: 1.0,
            min_test_pad_to_edge: 3.0,
        }
    }
}

impl TestFile {
    pub fn resolve(&self, d: &mut Diags) -> TestSpec {
        let base = TestSpec::default();
        let side = match self.side.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None => base.side.clone(),
            Some("b" | "bottom" | "b.cu") => "B".into(),
            Some("f" | "top" | "f.cu") => "F".into(),
            Some(other) => {
                d.error("test", format!("side `{other}` is not F or B"));
                base.side.clone()
            }
        };
        let mm = |v: Option<Length>, dflt: f64| v.map(Length::to_mm).unwrap_or(dflt);
        TestSpec {
            nets: self.nets.clone(),
            exclude: self.exclude.clone(),
            side,
            through_holes: self.through_holes.unwrap_or(base.through_holes),
            vias: self.vias.unwrap_or(base.vias),
            min_test_pad: mm(self.min_test_pad, base.min_test_pad),
            min_test_pad_pitch: mm(self.min_test_pad_pitch, base.min_test_pad_pitch),
            min_test_pad_to_body: mm(self.min_test_pad_to_body, base.min_test_pad_to_body),
            min_test_pad_to_edge: mm(self.min_test_pad_to_edge, base.min_test_pad_to_edge),
        }
    }
}

impl TestSpec {
    pub fn copper(&self) -> String {
        format!("{}.Cu", self.side)
    }

    pub fn mask(&self) -> String {
        format!("{}.Mask", self.side)
    }

    pub fn bottom(&self) -> bool {
        self.side == "B"
    }
}

fn matches(pattern: &str, name: &str) -> bool {
    glob(&pattern.to_ascii_uppercase(), &name.to_ascii_uppercase())
}

fn rail_token(tok: &str) -> bool {
    let t = tok.strip_prefix('+').unwrap_or(tok);
    let t =
        t.strip_prefix('V').filter(|r| r.starts_with(|c: char| c.is_ascii_digit())).unwrap_or(t);
    let Some(i) = t.find('V') else { return false };
    let (lead, tail) = (&t[..i], &t[i + 1..]);
    lead.starts_with(|c: char| c.is_ascii_digit())
        && lead.chars().all(|c| c.is_ascii_digit() || c == '.' || c == 'P')
        && tail.chars().all(|c| c.is_ascii_digit())
}

pub fn is_rail(name: &str) -> bool {
    let up = name.to_ascii_uppercase();
    RAIL_PREFIXES.iter().any(|p| up.starts_with(p)) || up.split(['_', '-', '/']).any(rail_token)
}

pub fn is_power(board: &Board, net: &LayoutNet) -> bool {
    let class = board.netclasses.iter().find(|c| c.name == net.class);
    class.is_some_and(|c| c.current.is_some()) || is_rail(&net.name)
}

pub fn exempt(board: &Board, pairs: &[Pair], ni: usize, net: &LayoutNet) -> Option<&'static str> {
    let class = board.netclasses.iter().find(|c| c.name == net.class);
    if class.is_some_and(|c| c.impedance.is_some()) {
        return Some("impedance class");
    }
    if pairs.iter().any(|p| p.p == ni || p.n == ni) {
        return Some("differential pair");
    }
    None
}

pub fn wanted(spec: &TestSpec, board: &Board, net: &LayoutNet) -> bool {
    if spec.exclude.iter().any(|g| matches(g, &net.name)) {
        return false;
    }
    if !spec.nets.is_empty() {
        return spec.nets.iter().any(|g| matches(g, &net.name));
    }
    is_power(board, net) || DEFAULT_NETS.iter().any(|g| matches(g, &net.name))
}

pub fn is_test_point(p: &Placed) -> bool {
    let r = p.reference.as_str();
    let tp = r
        .strip_prefix("TP")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_digit()));
    tp || p.footprint_name.starts_with("TestPoint")
}

pub fn pad_center(q: &crate::layout::PlacedPad) -> P {
    q.drill.map(|d| d.0).unwrap_or_else(|| crate::drc::rings_bounds(&q.outlines).center())
}

pub fn pad_diameter(q: &crate::layout::PlacedPad) -> f64 {
    let b = crate::drc::rings_bounds(&q.outlines);
    if b.is_empty() {
        return 0.0;
    }
    let s = b.size();
    s[0].min(s[1])
}

pub fn has_access(spec: &TestSpec, parts: &[Placed], vias: &[Via], ni: usize) -> bool {
    let cu = spec.copper();
    let mask = spec.mask();
    parts.iter().any(|p| {
        p.pads.iter().any(|q| {
            q.net == Some(ni)
                && !q.copper.is_empty()
                && (is_test_point(p)
                    || (spec.through_holes
                        && q.drill.is_some()
                        && q.kind != PadKind::Npth
                        && q.mask.contains(&mask)))
        })
    }) || (spec.vias && vias.iter().any(|v| v.net == ni && v.layers.contains(&cu)))
}

#[derive(Clone, Debug, Serialize)]
pub struct Probe {
    pub reference: String,
    pub pad: String,
    pub net: String,
    pub at: P,
    pub side: String,
    pub diameter: f64,
}

pub fn probes(spec: &TestSpec, parts: &[Placed], nets: &[LayoutNet]) -> Vec<Probe> {
    let mut out = Vec::new();
    for p in parts.iter().filter(|p| is_test_point(p)) {
        for q in p.pads.iter().filter(|q| !q.copper.is_empty()) {
            let side = if q.drill.is_some() {
                spec.side.clone()
            } else if q.copper.iter().any(|l| l == "B.Cu") {
                "B".into()
            } else {
                "F".into()
            };
            out.push(Probe {
                reference: p.reference.clone(),
                pad: q.number.clone(),
                net: q.net.and_then(|n| nets.get(n)).map(|n| n.name.clone()).unwrap_or_default(),
                at: pad_center(q),
                side,
                diameter: pad_diameter(q),
            });
        }
    }
    out.sort_by(|a, b| crate::footprint::natural_cmp(&a.reference, &b.reference));
    out
}

pub fn tooling_holes(parts: &[Placed]) -> Vec<(P, f64)> {
    parts
        .iter()
        .flat_map(|p| {
            let hole = p.footprint_name.starts_with("MountingHole");
            p.pads.iter().filter(move |q| hole || q.kind == PadKind::Npth)
        })
        .filter_map(|q| q.drill.map(|(c, s, _)| (c, s[0].max(s[1]) / 2.0)))
        .collect()
}

fn corners(b: &crate::graphic::Bounds, p: &Placed) -> Vec<P> {
    let tf = p.transform();
    [b.min, [b.max[0], b.min[1]], b.max, [b.min[0], b.max[1]]]
        .into_iter()
        .map(|q| tf.apply(q))
        .collect()
}

fn footprint_side(p: &Placed, side: &str) -> &'static str {
    match (p.bottom, side) {
        (false, "B") | (true, "F") => "B",
        _ => "F",
    }
}

pub fn courtyard_rect(p: &Placed, side: &str) -> Option<Vec<P>> {
    let court = p.footprint.courtyard(footprint_side(p, side));
    (!court.is_empty()).then(|| corners(&court, p))
}

pub fn body_rect(p: &Placed, side: &str) -> Option<Vec<P>> {
    crate::layout::body_box(p, side).or_else(|| courtyard_rect(p, side))
}

pub fn gap_to_rect(c: P, r: f64, rect: &[P]) -> f64 {
    if geom::point_in_polygon(c, rect) {
        return -r;
    }
    geom::polyline_polygon_distance(&[c, c], rect) - r
}

const OWN_ROOM: f64 = 0.3;

struct Added {
    net: usize,
    a: P,
    b: P,
    r: f64,
    via: Option<f64>,
}

impl Added {
    fn gap(&self, c: P, r: f64) -> f64 {
        geom::point_segment_distance(c, self.a, self.b) - self.r - r
    }

    fn segment_gap(&self, a: P, b: P, r: f64) -> f64 {
        geom::segment_segment_distance(self.a, self.b, a, b) - self.r - r
    }
}

pub struct Placement {
    pub net: usize,
    pub at: Option<P>,
    pub via: Option<P>,
}

fn class_via<'a>(board: &'a Board, net: &LayoutNet) -> Option<&'a crate::board::Via> {
    let class = board.netclasses.iter().find(|c| c.name == net.class);
    class
        .and_then(|c| c.via.as_ref())
        .and_then(|n| board.vias.iter().find(|v| &v.name == n))
        .or(board.vias.first())
}

pub fn place(
    layout: &crate::layout::Layout,
    board: &Board,
    spec: &TestSpec,
    nets: &[usize],
    pitch: f64,
) -> Vec<Placement> {
    let r = spec.min_test_pad.max(1.0) / 2.0;
    let court = r + 0.5;
    let spacing = pitch.max(spec.min_test_pad_pitch);
    let cu = spec.copper();
    let silk = format!("{}.SilkS", spec.side);
    let cx = crate::drc::Ctx::of_layout(board, layout);
    let items = cx.copper_items();
    let holes = cx.holes();
    let hole_gap =
        board.rules.min_hole_to_smd_pad.to_mm().max(board.rules.min_via_hole_to_copper.to_mm());
    let tooling = tooling_holes(&layout.parts);
    let bodies: Vec<Vec<P>> = layout
        .parts
        .iter()
        .filter(|p| !is_test_point(p))
        .filter_map(|p| body_rect(p, &spec.side))
        .collect();
    let courts: Vec<Vec<P>> =
        layout.parts.iter().filter_map(|p| courtyard_rect(p, &spec.side)).collect();
    let texts: Vec<&Vec<P>> =
        layout.silk.iter().filter(|b| b.layer == silk).map(|b| &b.outline).collect();
    let mut taken: Vec<P> =
        probes(spec, &layout.parts, &layout.nets).iter().map(|p| p.at).collect();
    let mut ob = crate::graphic::Bounds::EMPTY;
    layout.outline.iter().for_each(|p| ob.add(*p));
    let mut added: Vec<Added> = Vec::new();
    let mut out = Vec::new();
    for &ni in nets {
        let clearance = layout.nets[ni].clearance;
        let apart = |n: usize| layout.nets[n].clearance.max(clearance);
        let mut anchors: Vec<P> = Vec::new();
        for p in &layout.parts {
            for q in p.pads.iter().filter(|q| q.net == Some(ni) && !q.copper.is_empty()) {
                anchors.push(pad_center(q));
            }
        }
        anchors.extend(layout.vias.iter().filter(|v| v.net == ni).map(|v| v.at));
        anchors.extend(
            layout.tracks.iter().filter(|t| t.net == ni).flat_map(|t| t.points.iter().copied()),
        );
        let mut ab = crate::graphic::Bounds::EMPTY;
        anchors.iter().for_each(|a| ab.add(*a));
        if ab.is_empty() || ob.is_empty() {
            out.push(Placement { net: ni, at: None, via: None });
            continue;
        }
        let reach = 12.0;
        let snap = |v: f64, o: f64| o + ((v - o) / spacing).round() * spacing;
        let (x0, x1) = (snap(ab.min[0] - reach, ob.min[0]), snap(ab.max[0] + reach, ob.min[0]));
        let (y0, y1) = (snap(ab.min[1] - reach, ob.min[1]), snap(ab.max[1] + reach, ob.min[1]));
        let mut candidates: Vec<(f64, P)> = Vec::new();
        let mut y = y0;
        while y <= y1 + 1e-9 {
            let mut x = x0;
            while x <= x1 + 1e-9 {
                let c = [(x * 1e4).round() / 1e4, (y * 1e4).round() / 1e4];
                let d = anchors.iter().map(|a| geom::dist(*a, c)).fold(f64::MAX, f64::min);
                if d <= reach {
                    candidates.push((d, c));
                }
                x += spacing;
            }
            y += spacing;
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
        let clear = |c: P, taken: &[P]| -> bool {
            let board = layout.edge();
            if !board.is_closed()
                || !board.contains(c)
                || board.distance(c) - r < spec.min_test_pad_to_edge
            {
                return false;
            }
            if tooling.iter().any(|(h, hr)| geom::dist(c, *h) - hr - r < spec.min_test_pad_to_edge)
            {
                return false;
            }
            if taken.iter().any(|t| geom::dist(*t, c) < spacing - 1e-6) {
                return false;
            }
            if bodies.iter().any(|b| gap_to_rect(c, r, b) < spec.min_test_pad_to_body) {
                return false;
            }
            if courts.iter().any(|b| gap_to_rect(c, court * std::f64::consts::SQRT_2, b) < 0.0) {
                return false;
            }
            if texts.iter().any(|b| gap_to_rect(c, 0.8, b) < 0.2) {
                return false;
            }
            if added.iter().any(|o| {
                o.net != ni && o.gap(c, r) < apart(o.net)
                    || o.via.is_some_and(|h| geom::dist(o.a, c) - h - r < hole_gap)
            }) {
                return false;
            }
            let mut bb = crate::graphic::Bounds::EMPTY;
            bb.add_circle(c, r);
            let reach = r + clearance + 1.0;
            for k in cx.items_near(&bb, reach) {
                let it = &items[k];
                if !it.layers.contains(&cu) {
                    continue;
                }
                let gap = it
                    .net
                    .map(|n| layout.nets[n].clearance)
                    .unwrap_or(clearance)
                    .max(clearance)
                    .max(if it.net == Some(ni) { OWN_ROOM } else { 0.0 });
                if it.shape.circle_gap(c, r) < gap {
                    return false;
                }
            }
            !holes.iter().any(|h| geom::dist(h.a, c).min(geom::dist(h.b, c)) - h.r - r < hole_gap)
        };
        let via = class_via(board, &layout.nets[ni]);
        let mut found: Option<(P, P, f64, f64, f64)> = None;
        for (_, c) in candidates {
            if !clear(c, &taken) {
                continue;
            }
            let Some(v) = via else { break };
            let (vr, dr) = (v.diameter.to_mm() / 2.0, v.drill.to_mm() / 2.0);
            let len = (r + vr + clearance).max(r + dr + hole_gap);
            let len = (len / 0.05).ceil() * 0.05;
            let near = anchors
                .iter()
                .copied()
                .min_by(|a, b| geom::dist(*a, c).total_cmp(&geom::dist(*b, c)))
                .unwrap_or(c);
            let toward = (near[1] - c[1]).atan2(near[0] - c[0]);
            let mut dirs: Vec<f64> =
                (0..8).map(|k| k as f64 * std::f64::consts::FRAC_PI_4).collect();
            dirs.sort_by(|a, b| {
                let off = |d: f64| {
                    (d - toward).sin().abs() + if (d - toward).cos() < 0.0 { 2.0 } else { 0.0 }
                };
                off(*a).total_cmp(&off(*b))
            });
            let width = layout.nets[ni].width;
            let spot = dirs
                .into_iter()
                .map(|d| {
                    let q = [c[0] + len * d.cos(), c[1] + len * d.sin()];
                    [(q[0] * 1e4).round() / 1e4, (q[1] * 1e4).round() / 1e4]
                })
                .find(|q| {
                    via_clear(&cx, layout, board, ni, *q, vr, dr, clearance)
                        && stub_clear(&cx, layout, ni, c, *q, width / 2.0, &cu)
                        && added.iter().all(|o| {
                            let hh = board.rules.min_hole_to_hole.to_mm();
                            let hole = match o.via {
                                Some(h) => geom::dist(o.a, *q) - h - dr >= hh,
                                None => o.net == ni || o.gap(*q, dr) >= hole_gap,
                            };
                            hole && (o.net == ni
                                || (o.gap(*q, vr) >= apart(o.net)
                                    && o.segment_gap(c, *q, width / 2.0) >= apart(o.net)))
                        })
                });
            if let Some(q) = spot {
                found = Some((c, q, vr, dr, width / 2.0));
                break;
            }
        }
        if let Some((c, q, vr, dr, half)) = found {
            taken.push(c);
            added.push(Added { net: ni, a: c, b: c, r, via: None });
            added.push(Added { net: ni, a: c, b: q, r: half, via: None });
            added.push(Added { net: ni, a: q, b: q, r: vr, via: Some(dr) });
        }
        out.push(Placement { net: ni, at: found.map(|f| f.0), via: found.map(|f| f.1) });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn via_clear(
    cx: &crate::drc::Ctx,
    layout: &crate::layout::Layout,
    board: &Board,
    net: usize,
    at: P,
    vr: f64,
    dr: f64,
    clearance: f64,
) -> bool {
    let edge = layout.edge();
    if !edge.is_closed()
        || !edge.contains(at)
        || edge.distance(at) - vr < board.rules.min_copper_to_edge.to_mm()
    {
        return false;
    }
    let items = cx.copper_items();
    let mut bb = crate::graphic::Bounds::EMPTY;
    bb.add_circle(at, vr);
    let hole_cu = board.rules.min_via_hole_to_copper.to_mm();
    let hole_smd = board.rules.min_hole_to_smd_pad.to_mm();
    for k in cx.items_near(&bb, vr + clearance + 1.0) {
        let it = &items[k];
        let pad = matches!(it.owner, crate::drc::Owner::Pad(..));
        let gap = if it.net == Some(net) {
            if !pad {
                let g = it.shape.circle_gap(at, vr);
                if g > -1e-3 && g < OWN_ROOM {
                    return false;
                }
                continue;
            }
            dr + hole_smd - vr
        } else {
            it.net
                .map(|n| layout.nets[n].clearance)
                .unwrap_or(clearance)
                .max(clearance)
                .max(dr + hole_cu - vr)
        };
        if it.shape.circle_gap(at, vr) < gap.max(0.0) + if pad { 1e-3 } else { 0.0 } {
            return false;
        }
    }
    if layout.silk.iter().any(|b| {
        geom::point_in_polygon(at, &b.outline)
            || geom::polyline_polygon_distance(&[at, at], &b.outline) < vr
    }) {
        return false;
    }
    let hh = board.rules.min_hole_to_hole.to_mm();
    !cx.holes().iter().any(|h| geom::point_segment_distance(at, h.a, h.b) - h.r - dr < hh)
}

fn stub_clear(
    cx: &crate::drc::Ctx,
    layout: &crate::layout::Layout,
    net: usize,
    a: P,
    b: P,
    half: f64,
    cu: &str,
) -> bool {
    let items = cx.copper_items();
    let mut bb = crate::graphic::Bounds::EMPTY;
    bb.add_circle(a, half);
    bb.add_circle(b, half);
    let near = cx.items_near(&bb, 2.0);
    let steps = ((geom::dist(a, b) / 0.05).ceil() as usize).max(1);
    near.into_iter().all(|k| {
        let it = &items[k];
        if it.net == Some(net) || !it.layers.iter().any(|l| l == cu) {
            return true;
        }
        let gap =
            it.net.map(|n| layout.nets[n].clearance).unwrap_or(0.0).max(layout.nets[net].clearance);
        (0..=steps).all(|i| {
            let t = i as f64 / steps as f64;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            it.shape.circle_gap(p, half) >= gap
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rails_by_name() {
        for n in ["3V3", "+5V", "VCC_IO", "V1V8", "1.8V", "VBUS", "12V", "AVDD_1V0", "3P3V"] {
            assert!(is_rail(n), "{n}");
        }
        for n in ["RF_OUT", "LED_DV2", "USB_DP", "SPI_MOSI", "V_SENSE"] {
            assert!(!is_rail(n), "{n}");
        }
    }
}
