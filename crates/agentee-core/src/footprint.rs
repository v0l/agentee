use crate::board::Rules;
use crate::diag::Diags;
use crate::geom::{self, P};
use crate::graphic::{self, Bounds, Graphic, GraphicDefaults, GraphicFile, Shape};
use crate::units::{Length, Point};
use serde::{Deserialize, Serialize};

pub const KNOWN_LAYERS: &[&str] = &[
    "F.Cu",
    "B.Cu",
    "F.SilkS",
    "B.SilkS",
    "F.Mask",
    "B.Mask",
    "F.Paste",
    "B.Paste",
    "F.Fab",
    "B.Fab",
    "F.CrtYd",
    "B.CrtYd",
    "Edge.Cuts",
    "Dwgs.User",
    "Cmts.User",
    "F.Adhes",
    "B.Adhes",
];

pub fn is_known_layer(l: &str) -> bool {
    KNOWN_LAYERS.contains(&l)
        || l.starts_with("User.")
        || l.starts_with("*.")
        || (l.starts_with("In") && l.ends_with(".Cu"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadKind {
    Smd,
    Tht,
    Npth,
    Connect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadShape {
    Rect,
    Roundrect,
    Circle,
    Oval,
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mount {
    Smd,
    Tht,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Drill {
    Round(Length),
    Slot(Point),
}

impl Drill {
    pub fn size(&self) -> Point {
        match *self {
            Drill::Round(d) => Point(d, d),
            Drill::Slot(p) => p,
        }
    }

    pub fn min(&self) -> Length {
        let s = self.size();
        s.0.min(s.1)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PadFile {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub number: String,
    pub kind: PadKind,
    pub shape: PadShape,
    pub at: Point,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roundrect_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drill: Option<Drill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layers: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<Point>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_step: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FootprintFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount: Option<Mount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pads: Vec<PadFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graphics: Vec<GraphicFile>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Pad {
    pub number: String,
    pub kind: PadKind,
    pub shape: PadShape,
    pub at: Point,
    pub size: Point,
    pub rotation: f64,
    pub roundrect_ratio: f64,
    pub drill: Option<Drill>,
    pub layers: Vec<String>,
    pub points: Vec<Point>,
}

impl Pad {
    pub fn on_layer(&self, layer: &str) -> bool {
        self.layers.iter().any(|l| {
            l == layer
                || l.strip_prefix("*.").is_some_and(|suffix| layer.ends_with(&format!(".{suffix}")))
        })
    }

    pub fn is_copper(&self) -> bool {
        self.layers.iter().any(|l| l.ends_with(".Cu"))
    }

    pub fn outline(&self) -> Vec<P> {
        let [w, h] = self.size.to_mm();
        let local: Vec<P> = match self.shape {
            PadShape::Rect => geom::rounded_rect(w, h, 0.0, 0),
            PadShape::Roundrect => geom::rounded_rect(w, h, w.min(h) * self.roundrect_ratio, 4),
            PadShape::Circle => geom::circle([0.0, 0.0], w / 2.0, 32),
            PadShape::Oval => geom::rounded_rect(w, h, w.min(h) / 2.0, 8),
            PadShape::Custom => {
                if self.points.len() >= 3 {
                    self.points.iter().map(|p| p.to_mm()).collect()
                } else {
                    geom::rounded_rect(w, h, 0.0, 0)
                }
            }
        };
        let [ax, ay] = self.at.to_mm();
        local
            .into_iter()
            .map(|p| {
                let [x, y] = geom::rotate(p, self.rotation);
                [x + ax, y + ay]
            })
            .collect()
    }

    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        self.outline().into_iter().for_each(|p| b.add(p));
        b
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Footprint {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub mount: Mount,
    pub model: Option<String>,
    pub pads: Vec<Pad>,
    pub graphics: Vec<Graphic>,
}

fn step_number(base: &str, i: i64) -> Option<String> {
    let digits = base.len() - base.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let (prefix, n) = base.split_at(base.len() - digits);
    Some(format!("{prefix}{}", n.parse::<i64>().ok()? + i))
}

fn default_layers(kind: PadKind) -> Vec<String> {
    let v: &[&str] = match kind {
        PadKind::Smd => &["F.Cu", "F.Paste", "F.Mask"],
        PadKind::Tht => &["*.Cu", "*.Mask"],
        PadKind::Npth => &["*.Mask"],
        PadKind::Connect => &["F.Cu", "F.Mask"],
    };
    v.iter().map(|s| s.to_string()).collect()
}

impl FootprintFile {
    pub fn resolve(&self, d: &mut Diags) -> Footprint {
        let mut pads = Vec::new();
        for (i, p) in self.pads.iter().enumerate() {
            let at = format!("pads[{i}]");
            let count = p.count.unwrap_or(1);
            if count > 1 && p.pitch.is_none() {
                d.error(&at, "`count` needs a `pitch` to step each pad by");
            }
            let size = match (p.size, p.shape, p.drill) {
                (Some(s), _, _) => s,
                (None, PadShape::Custom, _) => Point::mm(0.0, 0.0),
                (None, PadShape::Circle, Some(dr)) if p.kind == PadKind::Npth => dr.size(),
                _ => {
                    d.error(&at, "pad needs `size = [w, h]`");
                    Point::ZERO
                }
            };
            let step = p.number_step.unwrap_or(1);
            for k in 0..count {
                let number = if k == 0 {
                    p.number.clone()
                } else {
                    step_number(&p.number, step * k as i64).unwrap_or_else(|| {
                        if k == 1 {
                            d.error(&at, format!("cannot count up from pad number `{}`", p.number));
                        }
                        p.number.clone()
                    })
                };
                let pitch = p.pitch.unwrap_or(Point::ZERO);
                pads.push(Pad {
                    number,
                    kind: p.kind,
                    shape: p.shape,
                    at: p.at + Point(pitch.0 * k as f64, pitch.1 * k as f64),
                    size,
                    rotation: p.rotation.unwrap_or(0.0),
                    roundrect_ratio: p.roundrect_ratio.unwrap_or(0.25),
                    drill: p.drill,
                    layers: p.layers.clone().unwrap_or_else(|| default_layers(p.kind)),
                    points: p.points.clone().unwrap_or_default(),
                });
            }
        }
        let defaults = GraphicDefaults {
            width: Length::mm(0.12),
            text_size: Length::mm(1.0),
            layer: Some("F.Fab"),
        };
        let graphics = self
            .graphics
            .iter()
            .enumerate()
            .filter_map(|(i, g)| {
                let at = format!("graphics[{i}]");
                if g.layer.is_none() {
                    d.warn(&at, "no `layer`, drawing on F.Fab");
                }
                if g.unit.is_some() {
                    d.warn(&at, "footprints have no units, `unit` is ignored");
                }
                let mut def = GraphicDefaults { ..defaults };
                if let Some(l) = g.layer.as_deref() {
                    def.width = match l {
                        "F.CrtYd" | "B.CrtYd" => Length::mm(0.05),
                        "F.Fab" | "B.Fab" => Length::mm(0.1),
                        _ => Length::mm(0.12),
                    };
                }
                graphic::resolve(g, &at, d, &def)
            })
            .collect();
        let mount = self.mount.unwrap_or_else(|| {
            if pads.iter().any(|p| p.kind == PadKind::Tht) {
                Mount::Tht
            } else if pads.iter().any(|p| p.kind == PadKind::Smd) {
                Mount::Smd
            } else {
                Mount::Other
            }
        });
        Footprint {
            name: self.name.clone(),
            description: self.description.clone(),
            tags: self.tags.clone(),
            mount,
            model: self.model.clone(),
            pads,
            graphics,
        }
    }
}

fn graphic_path(g: &Graphic) -> Vec<P> {
    match &g.shape {
        Shape::Line { start, end } => vec![start.to_mm(), end.to_mm()],
        Shape::Polyline { points, closed } => {
            let mut v: Vec<P> = points.iter().map(|p| p.to_mm()).collect();
            if *closed && let Some(f) = v.first().copied() {
                v.push(f);
            }
            v
        }
        Shape::Rect { start, end } => {
            let ([x0, y0], [x1, y1]) = (start.to_mm(), end.to_mm());
            vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]
        }
        Shape::Circle { center, radius } => {
            let mut v = geom::circle(center.to_mm(), radius.to_mm(), 48);
            v.push(v[0]);
            v
        }
        Shape::Arc { start, mid, end } => graphic::arc_points(*start, *mid, *end, 24),
        Shape::Text { .. } => Vec::new(),
    }
}

impl Footprint {
    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        for p in &self.pads {
            b.union(&p.bounds());
        }
        for g in &self.graphics {
            b.union(&g.bounds());
        }
        b
    }

    pub fn courtyard(&self, side: &str) -> Bounds {
        let mut b = Bounds::EMPTY;
        for g in self.graphics.iter().filter(|g| g.layer == format!("{side}.CrtYd")) {
            b.union(&g.bounds());
        }
        b
    }

    pub fn pad_numbers(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .pads
            .iter()
            .filter(|p| !p.number.is_empty() && p.is_copper())
            .map(|p| p.number.as_str())
            .collect();
        v.sort_by(|a, b| natural_cmp(a, b));
        v.dedup();
        v
    }

    pub fn check(&self, rules: &Rules, d: &mut Diags) {
        if self.name.trim().is_empty() {
            d.error("name", "footprint has no name");
        }
        if self.pads.is_empty() {
            d.warn("pads", "footprint has no pads");
        }
        let mut small_drills: Vec<(&str, Length)> = Vec::new();
        let mut unnumbered = 0;
        let mut tht_on_smd = 0;
        for (i, p) in self.pads.iter().enumerate() {
            let at = format!("pad {} (#{i})", if p.number.is_empty() { "-" } else { &p.number });
            let [w, h] = p.size.to_mm();
            if p.shape != PadShape::Custom && (w <= 0.0 || h <= 0.0) {
                d.error(&at, "pad size must be positive");
            }
            if p.shape == PadShape::Circle && (w - h).abs() > 1e-6 {
                d.warn(&at, "circle pad with unequal size, the width is used as the diameter");
            }
            if p.shape == PadShape::Custom && p.points.len() < 3 {
                d.error(&at, "custom pad needs `points` (at least three, relative to `at`)");
            }
            if !(0.0..=0.5).contains(&p.roundrect_ratio) {
                d.error(&at, "`roundrect_ratio` must be between 0 and 0.5");
            }
            for l in &p.layers {
                if !is_known_layer(l) {
                    d.warn(&at, format!("unknown layer `{l}`"));
                }
            }
            match (p.kind, p.drill) {
                (PadKind::Tht | PadKind::Npth, None) => {
                    d.error(&at, "through-hole pad needs a `drill`")
                }
                (PadKind::Smd | PadKind::Connect, Some(_)) => {
                    d.error(&at, "surface pad cannot have a `drill`")
                }
                (PadKind::Tht, Some(dr)) => {
                    if dr.min() < rules.min_drill {
                        small_drills.push((&p.number, dr.min()));
                    }
                    let ring = ((p.size.0 - dr.size().0) / 2.0).min((p.size.1 - dr.size().1) / 2.0);
                    if ring < rules.min_annular_ring {
                        d.error(
                            &at,
                            format!(
                                "annular ring {ring} is under the fab minimum {}",
                                rules.min_annular_ring
                            ),
                        );
                    }
                }
                _ => {}
            }
            if p.kind == PadKind::Tht && self.mount == Mount::Smd {
                tht_on_smd += 1;
            }
            if p.number.is_empty() && p.kind != PadKind::Npth && p.is_copper() {
                unnumbered += 1;
            }
        }
        if let Some(smallest) = small_drills.iter().map(|x| x.1).min() {
            let pads = list(small_drills.iter().map(|x| x.0));
            d.error(
                "pads",
                format!(
                    "{} holes are under the fab minimum drill {} (smallest {smallest}, pads {pads})",
                    small_drills.len(),
                    rules.min_drill
                ),
            );
        }
        if tht_on_smd > 0 {
            d.warn(
                "mount",
                format!("{tht_on_smd} through-hole pads on a footprint marked `mount = \"smd\"`"),
            );
        }
        if unnumbered > 0 {
            d.info(
                "pads",
                format!("{unnumbered} copper pads have no number, they cannot connect to a pin"),
            );
        }

        let outlines: Vec<Vec<P>> = self.pads.iter().map(|p| p.outline()).collect();
        let min_clear = rules.min_clearance.to_mm();
        let mut overlaps = Vec::new();
        let mut close: Vec<(f64, String)> = Vec::new();
        for i in 0..self.pads.len() {
            for j in i + 1..self.pads.len() {
                let (a, b) = (&self.pads[i], &self.pads[j]);
                let shared = ["F.Cu", "B.Cu"].iter().any(|l| a.on_layer(l) && b.on_layer(l));
                if !shared || a.number == b.number || a.number.is_empty() || b.number.is_empty() {
                    continue;
                }
                let (ba, bb) = (a.bounds(), b.bounds());
                let grown = Bounds {
                    min: [ba.min[0] - min_clear, ba.min[1] - min_clear],
                    max: [ba.max[0] + min_clear, ba.max[1] + min_clear],
                };
                if !grown.overlaps(&bb) {
                    continue;
                }
                let gap = geom::polygon_distance(&outlines[i], &outlines[j]);
                let pair = format!("{}/{}", or_dash(&a.number), or_dash(&b.number));
                if gap == 0.0 {
                    overlaps.push(pair);
                } else if gap + 1e-6 < min_clear {
                    close.push((gap, pair));
                }
            }
        }
        if !overlaps.is_empty() {
            d.error(
                "pads",
                format!(
                    "{} pad pairs overlap: {}",
                    overlaps.len(),
                    list(overlaps.iter().map(String::as_str))
                ),
            );
        }
        if let Some((gap, pair)) = close.iter().min_by(|a, b| a.0.total_cmp(&b.0)) {
            d.warn(
                "pads",
                format!(
                    "{} pad pairs are closer than the fab clearance {}, closest {} ({pair})",
                    close.len(),
                    rules.min_clearance,
                    Length::mm(*gap)
                ),
            );
        }

        for side in ["F", "B"] {
            let cu = format!("{side}.Cu");
            let mut pads = Bounds::EMPTY;
            for p in self
                .pads
                .iter()
                .filter(|p| p.layers.contains(&cu) || (side == "F" && p.on_layer(&cu)))
            {
                pads.union(&p.bounds());
            }
            if pads.is_empty() {
                continue;
            }
            let cy = self.courtyard(side);
            if cy.is_empty() {
                d.warn(format!("{side}.CrtYd"), "no courtyard, placement cannot check spacing");
            } else if !cy.contains(&pads) {
                d.warn(format!("{side}.CrtYd"), "courtyard does not enclose the pads");
            }
        }

        let mut thin_silk: Vec<Length> = Vec::new();
        let mut small_text: Vec<Length> = Vec::new();
        let mut over_pads: Vec<&str> = Vec::new();
        for (i, g) in self.graphics.iter().enumerate() {
            let at = format!("graphics[{i}]");
            if !is_known_layer(&g.layer) {
                d.warn(&at, format!("unknown layer `{}`", g.layer));
            }
            let Some(side) = g.layer.strip_suffix(".SilkS") else { continue };
            match &g.shape {
                Shape::Text { size, .. } if *size < rules.min_silk_text_height => {
                    small_text.push(*size)
                }
                Shape::Text { .. } => {}
                _ if g.width < rules.min_silk_width => thin_silk.push(g.width),
                _ => {}
            }
            let path = graphic_path(g);
            if path.len() < 2 {
                continue;
            }
            let half = g.width.to_mm() / 2.0;
            for (p, o) in self.pads.iter().zip(&outlines) {
                let exposed =
                    p.on_layer(&format!("{side}.Mask")) || p.on_layer(&format!("{side}.Cu"));
                if exposed
                    && geom::polyline_polygon_distance(&path, o) < half
                    && !over_pads.contains(&p.number.as_str())
                {
                    over_pads.push(&p.number);
                }
            }
        }
        if let Some(w) = thin_silk.iter().min() {
            d.warn(
                "silk",
                format!(
                    "silk lines under the fab minimum width {}: {}, thinnest {w}",
                    rules.min_silk_width,
                    thin_silk.len()
                ),
            );
        }
        if let Some(h) = small_text.iter().min() {
            d.warn(
                "silk",
                format!(
                    "silk texts under the fab minimum height {}: {}, smallest {h}",
                    rules.min_silk_text_height,
                    small_text.len()
                ),
            );
        }
        if !over_pads.is_empty() {
            d.warn(
                "silk",
                format!(
                    "silk runs over pads {}, it will be clipped",
                    list(over_pads.iter().copied())
                ),
            );
        }
    }
}

fn or_dash(s: &str) -> &str {
    if s.is_empty() { "-" } else { s }
}

fn list<'a>(it: impl Iterator<Item = &'a str>) -> String {
    let v: Vec<&str> = it.collect();
    let mut s = v.iter().take(6).map(|x| or_dash(x)).collect::<Vec<_>>().join(", ");
    if v.len() > 6 {
        s += &format!(" and {} more", v.len() - 6);
    }
    s
}

pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<i64>(), b.parse::<i64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        _ => a.cmp(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fab_rules;

    #[test]
    fn pad_rows_count_up() {
        let f: FootprintFile = toml::from_str(
            r#"
name = "SOIC-8"
[[pads]]
number = "1"
kind = "smd"
shape = "roundrect"
at = [-2.475, -1.905]
size = [1.95, 0.6]
count = 4
pitch = [0, 1.27]
[[pads]]
number = "5"
kind = "smd"
shape = "roundrect"
at = [2.475, 1.905]
size = [1.95, 0.6]
count = 4
pitch = [0, -1.27]
[[graphics]]
kind = "rect"
layer = "F.CrtYd"
start = [-3.7, -2.7]
end = [3.7, 2.7]
"#,
        )
        .unwrap();
        let mut d = Diags::new("SOIC-8");
        let fp = f.resolve(&mut d);
        fp.check(&fab_rules("jlcpcb").unwrap(), &mut d);
        assert_eq!(fp.pad_numbers(), ["1", "2", "3", "4", "5", "6", "7", "8"]);
        assert_eq!(fp.pads[7].at, Point::mm(2.475, -1.905));
        assert!(!d.has_errors(), "{:?}", d.list);
    }

    #[test]
    fn overlapping_pads_are_errors() {
        let f: FootprintFile = toml::from_str(
            r#"
name = "bad"
[[pads]]
number = "1"
kind = "smd"
shape = "rect"
at = [0, 0]
size = [1, 1]
count = 2
pitch = [0.8, 0]
"#,
        )
        .unwrap();
        let mut d = Diags::new("bad");
        f.resolve(&mut d).check(&fab_rules("generic").unwrap(), &mut d);
        assert!(d.list.iter().any(|x| x.message.contains("overlap")), "{:?}", d.list);
    }

    #[test]
    fn stepping_keeps_prefixes() {
        assert_eq!(step_number("A1", 2).as_deref(), Some("A3"));
        assert_eq!(step_number("EP", 1), None);
    }
}
