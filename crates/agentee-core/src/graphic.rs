use crate::diag::Diags;
use crate::units::{Length, Point};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicKind {
    Line,
    Polyline,
    Polygon,
    Rect,
    Circle,
    Arc,
    Text,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fill {
    #[default]
    None,
    Solid,
    Background,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphicFile {
    pub kind: GraphicKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mid: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<Point>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
}

impl GraphicFile {
    pub fn new(kind: GraphicKind) -> Self {
        GraphicFile {
            kind,
            layer: None,
            unit: None,
            start: None,
            mid: None,
            end: None,
            points: None,
            center: None,
            radius: None,
            at: None,
            text: None,
            size: None,
            rotation: None,
            anchor: None,
            width: None,
            fill: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    Line { start: Point, end: Point },
    Polyline { points: Vec<Point>, closed: bool },
    Rect { start: Point, end: Point },
    Circle { center: Point, radius: Length },
    Arc { start: Point, mid: Point, end: Point },
    Text { at: Point, text: String, size: Length, rotation: f64, anchor: Anchor },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Graphic {
    #[serde(flatten)]
    pub shape: Shape,
    pub width: Length,
    pub fill: Fill,
    pub layer: String,
    pub unit: u32,
}

pub struct GraphicDefaults<'a> {
    pub width: Length,
    pub text_size: Length,
    pub layer: Option<&'a str>,
}

pub fn resolve(g: &GraphicFile, at: &str, d: &mut Diags, def: &GraphicDefaults) -> Option<Graphic> {
    let need = |v: Option<Point>, name: &str, d: &mut Diags| {
        if v.is_none() {
            d.error(at, format!("{:?} needs `{name}`", g.kind).to_lowercase());
        }
        v
    };
    let shape = match g.kind {
        GraphicKind::Line => {
            Shape::Line { start: need(g.start, "start", d)?, end: need(g.end, "end", d)? }
        }
        GraphicKind::Rect => {
            Shape::Rect { start: need(g.start, "start", d)?, end: need(g.end, "end", d)? }
        }
        GraphicKind::Polyline | GraphicKind::Polygon => {
            let points = g.points.clone().unwrap_or_default();
            if points.len() < 2 {
                d.error(at, "`points` needs at least two points");
                return None;
            }
            Shape::Polyline { points, closed: g.kind == GraphicKind::Polygon }
        }
        GraphicKind::Circle => {
            let center = need(g.center, "center", d)?;
            let Some(radius) = g.radius.filter(|r| r.is_positive()) else {
                d.error(at, "circle needs a positive `radius`");
                return None;
            };
            Shape::Circle { center, radius }
        }
        GraphicKind::Arc => {
            let (start, mid, end) =
                (need(g.start, "start", d)?, need(g.mid, "mid", d)?, need(g.end, "end", d)?);
            if arc_center(start, mid, end).is_none() {
                d.error(at, "arc points are collinear, `mid` must sit on the arc between them");
                return None;
            }
            Shape::Arc { start, mid, end }
        }
        GraphicKind::Text => {
            let Some(text) = g.text.clone().filter(|t| !t.is_empty()) else {
                d.error(at, "text needs `text`");
                return None;
            };
            Shape::Text {
                at: need(g.at, "at", d)?,
                text,
                size: g.size.unwrap_or(def.text_size),
                rotation: g.rotation.unwrap_or(0.0),
                anchor: g.anchor.unwrap_or_default(),
            }
        }
    };
    let layer = match (&g.layer, def.layer) {
        (Some(l), _) => l.clone(),
        (None, Some(l)) => l.to_string(),
        (None, None) => String::new(),
    };
    Some(Graphic {
        shape,
        width: g.width.unwrap_or(def.width),
        fill: g.fill.unwrap_or_default(),
        layer,
        unit: g.unit.unwrap_or(0),
    })
}

pub fn arc_center(a: Point, b: Point, c: Point) -> Option<[f64; 2]> {
    let ([ax, ay], [bx, by], [cx, cy]) = (a.to_mm(), b.to_mm(), c.to_mm());
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    if d.abs() < 1e-12 {
        return None;
    }
    let (a2, b2, c2) = (ax * ax + ay * ay, bx * bx + by * by, cx * cx + cy * cy);
    Some([
        (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d,
        (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d,
    ])
}

pub fn arc_points(start: Point, mid: Point, end: Point, segments: usize) -> Vec<[f64; 2]> {
    let Some([cx, cy]) = arc_center(start, mid, end) else {
        return vec![start.to_mm(), mid.to_mm(), end.to_mm()];
    };
    let ang = |p: Point| {
        let [x, y] = p.to_mm();
        (y - cy).atan2(x - cx)
    };
    let (a0, am, a1) = (ang(start), ang(mid), ang(end));
    let tau = std::f64::consts::TAU;
    let norm = |a: f64| a.rem_euclid(tau);
    let ccw_span = norm(a1 - a0);
    let sweep = if norm(am - a0) <= ccw_span { ccw_span } else { ccw_span - tau };
    let [sx, sy] = start.to_mm();
    let r = ((sx - cx).powi(2) + (sy - cy).powi(2)).sqrt();
    (0..=segments)
        .map(|i| {
            let a = a0 + sweep * i as f64 / segments as f64;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl Bounds {
    pub const EMPTY: Bounds = Bounds { min: [f64::MAX; 2], max: [f64::MIN; 2] };

    pub fn is_empty(&self) -> bool {
        self.min[0] > self.max[0]
    }

    pub fn add(&mut self, p: [f64; 2]) {
        self.min = [self.min[0].min(p[0]), self.min[1].min(p[1])];
        self.max = [self.max[0].max(p[0]), self.max[1].max(p[1])];
    }

    pub fn add_circle(&mut self, c: [f64; 2], r: f64) {
        self.add([c[0] - r, c[1] - r]);
        self.add([c[0] + r, c[1] + r]);
    }

    pub fn union(&mut self, o: &Bounds) {
        if !o.is_empty() {
            self.add(o.min);
            self.add(o.max);
        }
    }

    pub fn center(&self) -> [f64; 2] {
        [(self.min[0] + self.max[0]) / 2.0, (self.min[1] + self.max[1]) / 2.0]
    }

    pub fn size(&self) -> [f64; 2] {
        [self.max[0] - self.min[0], self.max[1] - self.min[1]]
    }

    pub fn contains(&self, o: &Bounds) -> bool {
        o.min[0] >= self.min[0] - 1e-9
            && o.min[1] >= self.min[1] - 1e-9
            && o.max[0] <= self.max[0] + 1e-9
            && o.max[1] <= self.max[1] + 1e-9
    }

    pub fn overlaps(&self, o: &Bounds) -> bool {
        self.min[0] < o.max[0]
            && o.min[0] < self.max[0]
            && self.min[1] < o.max[1]
            && o.min[1] < self.max[1]
    }
}

impl Graphic {
    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        match &self.shape {
            Shape::Line { start, end } | Shape::Rect { start, end } => {
                b.add(start.to_mm());
                b.add(end.to_mm());
            }
            Shape::Polyline { points, .. } => points.iter().for_each(|p| b.add(p.to_mm())),
            Shape::Circle { center, radius } => b.add_circle(center.to_mm(), radius.to_mm()),
            Shape::Arc { start, mid, end } => {
                arc_points(*start, *mid, *end, 32).into_iter().for_each(|p| b.add(p))
            }
            Shape::Text { at, text, size, .. } => {
                let h = size.to_mm();
                let w = h * 0.6 * text.chars().count() as f64;
                let [x, y] = at.to_mm();
                b.add([x - w / 2.0, y - h / 2.0]);
                b.add([x + w / 2.0, y + h / 2.0]);
            }
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_sweeps_through_mid() {
        let pts = arc_points(Point::mm(1.0, 0.0), Point::mm(0.0, 1.0), Point::mm(-1.0, 0.0), 4);
        assert!((pts[2][1] - 1.0).abs() < 1e-9);
        let pts = arc_points(Point::mm(1.0, 0.0), Point::mm(0.0, -1.0), Point::mm(-1.0, 0.0), 4);
        assert!((pts[2][1] + 1.0).abs() < 1e-9);
    }
}
