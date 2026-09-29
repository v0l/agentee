use crate::diag::Diags;
use crate::graphic::{self, Bounds, Fill, Graphic, GraphicDefaults, GraphicFile, Shape};
use crate::units::{Length, Point};
use serde::{Deserialize, Serialize};

pub const GRID: Length = Length(1_270_000);
pub const PIN_PITCH: Length = Length(2_540_000);
pub const PIN_LENGTH: Length = Length(2_540_000);
pub const TEXT_SIZE: Length = Length(1_270_000);
pub const BODY_WIDTH: Length = Length(254_000);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinType {
    Input,
    Output,
    Bidirectional,
    TriState,
    #[default]
    Passive,
    Free,
    Unspecified,
    PowerIn,
    PowerOut,
    OpenCollector,
    OpenEmitter,
    NoConnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub fn outward(self) -> [f64; 2] {
        match self {
            Side::Left => [-1.0, 0.0],
            Side::Right => [1.0, 0.0],
            Side::Top => [0.0, -1.0],
            Side::Bottom => [0.0, 1.0],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinShape {
    #[default]
    Line,
    Inverted,
    Clock,
    InvertedClock,
    InputLow,
    ClockLow,
    OutputLow,
    EdgeClockHigh,
    NonLogic,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinNames {
    #[default]
    Inside,
    Outside,
    Hidden,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinFile {
    pub number: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<PinType>,
    pub at: Point,
    pub side: Side,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<PinShape>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyPinFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<PinType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<PinShape>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_length: Option<Length>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<BodyPinFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub right: Vec<BodyPinFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top: Vec<BodyPinFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bottom: Vec<BodyPinFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub datasheet: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footprint_filters: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub power: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_names: Option<PinNames>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_name_offset: Option<Length>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hide_pin_numbers: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bodies: Vec<BodyFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graphics: Vec<GraphicFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<PinFile>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Pin {
    pub number: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: PinType,
    pub at: Point,
    pub side: Side,
    pub length: Length,
    pub unit: u32,
    pub shape: PinShape,
    pub hidden: bool,
}

impl Pin {
    pub fn body_end(&self) -> Point {
        let [dx, dy] = self.side.outward();
        self.at - Point(self.length * dx, self.length * dy)
    }

    pub fn in_unit(&self, unit: u32) -> bool {
        self.unit == 0 || self.unit == unit
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Symbol {
    pub name: String,
    pub reference: String,
    pub value: String,
    pub description: String,
    pub datasheet: String,
    pub keywords: Vec<String>,
    pub footprint: Option<String>,
    pub footprint_filters: Vec<String>,
    pub power: bool,
    pub pin_names: PinNames,
    pub pin_name_offset: Length,
    pub show_pin_numbers: bool,
    pub units: u32,
    pub graphics: Vec<Graphic>,
    pub pins: Vec<Pin>,
}

impl Symbol {
    pub fn bounds(&self, unit: u32) -> Bounds {
        let mut b = Bounds::EMPTY;
        for g in self.graphics.iter().filter(|g| g.unit == 0 || g.unit == unit) {
            b.union(&g.bounds());
        }
        for p in self.pins.iter().filter(|p| p.in_unit(unit)) {
            b.add(p.at.to_mm());
            b.add(p.body_end().to_mm());
        }
        b
    }

    pub fn unit_label(&self, unit: u32) -> String {
        if self.units <= 1 {
            return String::new();
        }
        let mut n = unit;
        let mut s = String::new();
        while n > 0 {
            let r = (n - 1) % 26;
            s.insert(0, (b'A' + r as u8) as char);
            n = (n - 1) / 26;
        }
        s
    }
}

fn estimate_text_mm(s: &str) -> f64 {
    s.chars().filter(|c| *c != '~' && *c != '{' && *c != '}').count() as f64
        * TEXT_SIZE.to_mm()
        * 0.72
}

fn snap_up(v: f64, grid: f64) -> f64 {
    (v / grid - 1e-9).ceil() * grid
}

fn expand_body(
    b: &BodyFile,
    d: &mut Diags,
    at: &str,
    pins: &mut Vec<PinFile>,
    graphics: &mut Vec<GraphicFile>,
) {
    let p = PIN_PITCH.to_mm();
    let len = b.pin_length.unwrap_or(PIN_LENGTH);
    let slots =
        |v: &[BodyPinFile]| v.iter().map(|x| x.gap.unwrap_or(1).max(1) as usize).sum::<usize>();
    let longest = |v: &[BodyPinFile]| {
        v.iter().filter_map(|x| x.name.as_deref()).map(estimate_text_mm).fold(0.0, f64::max)
    };
    let offset = 0.508;
    let (nl, nr, nt, nb) = (slots(&b.left), slots(&b.right), slots(&b.top), slots(&b.bottom));
    let rows = nl.max(nr).max(1);
    let cols = nt.max(nb);

    let side_names = longest(&b.left).max(longest(&b.right)) + offset;
    let columns = if cols > 0 {
        (cols - 1) as f64 * p / 2.0 + TEXT_SIZE.to_mm()
    } else if nl > 0 && nr > 0 {
        p / 2.0
    } else {
        0.0
    };
    let min_w = b.width.map(|w| w.to_mm()).unwrap_or(0.0).max((cols + 1) as f64 * p).max(2.0 * p);
    let half_w = snap_up((min_w / 2.0).max(side_names + columns), p);

    let name_h = longest(&b.top)
        + longest(&b.bottom)
        + 2.0 * offset
        + if nt > 0 && nb > 0 { p } else { 0.0 };
    let min_h = ((rows + 1) as f64 * p).max(name_h);
    let rows_total = snap_up(min_h, p) / p;
    let top = -(rows_total / 2.0).floor() * p;
    let bottom = top + rows_total * p;

    graphics.push(GraphicFile {
        unit: b.unit,
        start: Some(Point::mm(-half_w, top)),
        end: Some(Point::mm(half_w, bottom)),
        width: Some(BODY_WIDTH),
        fill: Some(Fill::Background),
        ..GraphicFile::new(crate::graphic::GraphicKind::Rect)
    });

    let mut place = |list: &[BodyPinFile], side: Side, d: &mut Diags| {
        let mut slot = 0usize;
        for (i, e) in list.iter().enumerate() {
            if let Some(gap) = e.gap {
                if e.number.is_some() {
                    d.error(
                        format!("{at}.{side:?}[{i}]").to_lowercase(),
                        "an entry is either a pin or a `gap`",
                    );
                }
                slot += gap.max(1) as usize;
                continue;
            }
            let Some(number) = e.number.clone() else {
                d.error(
                    format!("{at}.{side:?}[{i}]").to_lowercase(),
                    "pin needs a `number` (or use `gap = 1`)",
                );
                slot += 1;
                continue;
            };
            let along = (slot as f64 - (cols.max(1).saturating_sub(1) / 2) as f64) * p;
            let at = match side {
                Side::Left => Point::mm(-half_w - len.to_mm(), top + (slot + 1) as f64 * p),
                Side::Right => Point::mm(half_w + len.to_mm(), top + (slot + 1) as f64 * p),
                Side::Top => Point::mm(along, top - len.to_mm()),
                Side::Bottom => Point::mm(along, bottom + len.to_mm()),
            };
            pins.push(PinFile {
                number,
                name: e.name.clone().unwrap_or_default(),
                kind: e.kind,
                at,
                side,
                length: Some(len),
                unit: b.unit,
                shape: e.shape,
                hidden: e.hidden,
            });
            slot += 1;
        }
    };
    place(&b.left, Side::Left, d);
    place(&b.right, Side::Right, d);
    place(&b.top, Side::Top, d);
    place(&b.bottom, Side::Bottom, d);
}

impl SymbolFile {
    pub fn resolve(&self, d: &mut Diags) -> Symbol {
        let mut pins_raw = self.pins.clone();
        let mut graphics_raw = self.graphics.clone();
        for (i, b) in self.bodies.iter().enumerate() {
            expand_body(b, d, &format!("bodies[{i}]"), &mut pins_raw, &mut graphics_raw);
        }
        let defaults = GraphicDefaults { width: BODY_WIDTH, text_size: TEXT_SIZE, layer: None };
        let graphics: Vec<Graphic> = graphics_raw
            .iter()
            .enumerate()
            .filter_map(|(i, g)| {
                if g.layer.is_some() {
                    d.warn(format!("graphics[{i}]"), "symbols have no layers, `layer` is ignored");
                }
                graphic::resolve(g, &format!("graphics[{i}]"), d, &defaults)
            })
            .collect();
        let pins: Vec<Pin> = pins_raw
            .iter()
            .map(|p| Pin {
                number: p.number.clone(),
                name: p.name.clone(),
                kind: p.kind.unwrap_or_default(),
                at: p.at,
                side: p.side,
                length: p.length.unwrap_or(PIN_LENGTH),
                unit: p.unit.unwrap_or(0),
                shape: p.shape.unwrap_or_default(),
                hidden: p.hidden,
            })
            .collect();
        let units = pins
            .iter()
            .map(|p| p.unit)
            .chain(graphics.iter().map(|g| g.unit))
            .max()
            .unwrap_or(0)
            .max(1);
        Symbol {
            name: self.name.clone(),
            reference: self.reference.clone().unwrap_or_else(|| "U".into()),
            value: self.value.clone().unwrap_or_else(|| self.name.clone()),
            description: self.description.clone(),
            datasheet: self.datasheet.clone(),
            keywords: self.keywords.clone(),
            footprint: self.footprint.clone().filter(|f| !f.trim().is_empty()),
            footprint_filters: self.footprint_filters.clone(),
            power: self.power,
            pin_names: self.pin_names.unwrap_or_default(),
            pin_name_offset: self.pin_name_offset.unwrap_or(Length::mm(0.508)),
            show_pin_numbers: !self.hide_pin_numbers,
            units,
            graphics,
            pins,
        }
    }
}

impl Symbol {
    pub fn check(&self, d: &mut Diags) {
        if self.name.trim().is_empty() {
            d.error("name", "symbol has no name");
        }
        if !self.reference.chars().all(|c| c.is_ascii_alphabetic() || c == '#')
            || self.reference.is_empty()
        {
            d.warn(
                "reference",
                format!("reference prefix `{}` should be letters, like U, R, C", self.reference),
            );
        }
        if self.pins.is_empty() {
            d.warn("pins", "symbol has no pins");
        }
        for unit in 1..=self.units {
            if self.units > 1 && !self.pins.iter().any(|p| p.unit == unit) {
                d.warn("pins", format!("unit {unit} has no pins of its own"));
            }
        }
        let grid = GRID.0;
        for (i, p) in self.pins.iter().enumerate() {
            let at = format!("pins[{i}] ({})", p.number);
            if p.number.trim().is_empty() {
                d.error(&at, "pin has no number");
            }
            if !p.length.is_positive() && !self.power {
                d.warn(&at, "pin length is zero");
            }
            if p.at.0.0 % grid != 0 || p.at.1.0 % grid != 0 {
                d.warn(
                    &at,
                    format!(
                        "connection point {} is off the 1.27 mm grid, wires will not land on it",
                        p.at
                    ),
                );
            }
            for (j, q) in self.pins.iter().enumerate().skip(i + 1) {
                let shared_unit = p.unit == 0 || q.unit == 0 || p.unit == q.unit;
                if p.number == q.number && shared_unit {
                    if p.at == q.at {
                        d.info(&at, format!("pin {} is stacked with pins[{j}]", p.number));
                    } else {
                        d.error(&at, format!("pin number {} is used again by pins[{j}]", p.number));
                    }
                } else if p.number == q.number && p.name != q.name {
                    d.error(
                        &at,
                        format!(
                            "pin number {} is `{}` in unit {} but `{}` in unit {}",
                            p.number, p.name, p.unit, q.name, q.unit
                        ),
                    );
                } else if shared_unit && p.at == q.at && !p.hidden && !q.hidden {
                    d.error(
                        &at,
                        format!(
                            "pins {} and {} share the connection point {}",
                            p.number, q.number, p.at
                        ),
                    );
                }
            }
        }
        for (i, g) in self.graphics.iter().enumerate() {
            if g.unit > self.units {
                d.error(format!("graphics[{i}]"), format!("unit {} does not exist", g.unit));
            }
            if let Shape::Text { size, .. } = &g.shape
                && !size.is_positive()
            {
                d.error(format!("graphics[{i}]"), "text size must be positive");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_places_pins_on_the_grid() {
        let f: SymbolFile = toml::from_str(
            r#"
name = "MCU"
[[bodies]]
left = [{ number = "1", name = "VDD", type = "power_in" }, { gap = 1 }, { number = "2", name = "PA0" }]
right = [{ number = "3", name = "PB0_LONG_NAME" }]
top = [{ number = "4", name = "VBAT" }]
bottom = [{ number = "5", name = "GND", type = "power_in" }]
"#,
        )
        .unwrap();
        let mut d = Diags::new("MCU");
        let s = f.resolve(&mut d);
        s.check(&mut d);
        assert!(!d.has_errors(), "{:?}", d.list);
        assert!(d.list.iter().all(|x| !x.message.contains("grid")), "{:?}", d.list);
        assert_eq!(s.pins.len(), 5);
        let p1 = &s.pins[0];
        let p2 = &s.pins[1];
        assert_eq!(p2.at.1 - p1.at.1, PIN_PITCH * 2.0);
        assert_eq!(p1.side, Side::Left);
        let b = s.bounds(1);
        assert!(b.size()[0] > 10.0);
    }

    #[test]
    fn duplicate_pins_are_errors() {
        let f: SymbolFile = toml::from_str(
            r#"
name = "R"
reference = "R"
[[pins]]
number = "1"
at = [0, -3.81]
side = "top"
[[pins]]
number = "1"
at = [0, 3.81]
side = "bottom"
"#,
        )
        .unwrap();
        let mut d = Diags::new("R");
        f.resolve(&mut d).check(&mut d);
        assert!(d.has_errors());
    }
}
