use crate::sexpr::Node;
use agentee_core::graphic::{Anchor, Fill, GraphicFile, GraphicKind};
use agentee_core::symbol::{PinFile, PinNames, PinShape, PinType, Side, SymbolFile};
use agentee_core::units::{Length, Point};

fn pt(v: [f64; 2]) -> Point {
    Point::mm(v[0], -v[1])
}

fn pin_type(s: &str) -> PinType {
    match s {
        "input" => PinType::Input,
        "output" => PinType::Output,
        "bidirectional" => PinType::Bidirectional,
        "tri_state" => PinType::TriState,
        "free" => PinType::Free,
        "unspecified" => PinType::Unspecified,
        "power_in" => PinType::PowerIn,
        "power_out" => PinType::PowerOut,
        "open_collector" => PinType::OpenCollector,
        "open_emitter" => PinType::OpenEmitter,
        "no_connect" => PinType::NoConnect,
        _ => PinType::Passive,
    }
}

fn pin_shape(s: &str) -> PinShape {
    match s {
        "inverted" => PinShape::Inverted,
        "clock" => PinShape::Clock,
        "inverted_clock" => PinShape::InvertedClock,
        "input_low" => PinShape::InputLow,
        "clock_low" => PinShape::ClockLow,
        "output_low" => PinShape::OutputLow,
        "edge_clock_high" => PinShape::EdgeClockHigh,
        "non_logic" => PinShape::NonLogic,
        _ => PinShape::Line,
    }
}

fn side(angle: f64) -> Side {
    match angle.rem_euclid(360.0).round() as i64 {
        180 => Side::Right,
        90 => Side::Bottom,
        270 => Side::Top,
        _ => Side::Left,
    }
}

fn fill(n: &Node) -> Option<Fill> {
    match n.find("fill")?.find("type")?.arg(0)? {
        "background" => Some(Fill::Background),
        "outline" | "color" => Some(Fill::Solid),
        _ => None,
    }
}

fn width(n: &Node) -> Option<Length> {
    let w = n.find("stroke")?.find("width")?.num(0)?;
    (w > 0.0).then(|| Length::mm(w))
}

fn graphic(n: &Node, unit: u32) -> Option<GraphicFile> {
    let mut g = match n.head()? {
        "rectangle" => GraphicFile {
            start: Some(pt(n.xy("start")?)),
            end: Some(pt(n.xy("end")?)),
            ..GraphicFile::new(GraphicKind::Rect)
        },
        "polyline" | "bezier" => {
            let pts: Vec<Point> = n.pts().into_iter().map(pt).collect();
            if pts.len() < 2 {
                return None;
            }
            let closed = pts.len() > 2 && pts.first() == pts.last();
            let points = if closed { pts[..pts.len() - 1].to_vec() } else { pts };
            GraphicFile {
                points: Some(points),
                ..GraphicFile::new(if closed {
                    GraphicKind::Polygon
                } else {
                    GraphicKind::Polyline
                })
            }
        }
        "circle" => GraphicFile {
            center: Some(pt(n.xy("center")?)),
            radius: Some(Length::mm(n.find("radius")?.num(0)?)),
            ..GraphicFile::new(GraphicKind::Circle)
        },
        "arc" => GraphicFile {
            start: Some(pt(n.xy("start")?)),
            mid: Some(pt(n.xy("mid")?)),
            end: Some(pt(n.xy("end")?)),
            ..GraphicFile::new(GraphicKind::Arc)
        },
        "text" => {
            let at = n.find("at")?;
            if n.arg(0)?.trim().is_empty() {
                return None;
            }
            let size = n
                .find("effects")
                .and_then(|e| e.find("font"))
                .and_then(|f| f.find("size"))
                .and_then(|s| s.num(0));
            GraphicFile {
                text: Some(n.arg(0)?.to_string()),
                at: Some(pt([at.num(0)?, at.num(1)?])),
                rotation: at
                    .num(2)
                    .map(|r| if r.abs() > 360.0 { r / 10.0 } else { r })
                    .filter(|r| *r != 0.0),
                size: size.map(Length::mm),
                anchor: Some(Anchor::Center),
                ..GraphicFile::new(GraphicKind::Text)
            }
        }
        _ => return None,
    };
    g.unit = (unit > 0).then_some(unit);
    g.width = width(n);
    g.fill = fill(n);
    Some(g)
}

fn pin(n: &Node, unit: u32) -> Option<PinFile> {
    let at = n.find("at")?;
    let name = n.find("name").and_then(|x| x.arg(0)).unwrap_or("");
    Some(PinFile {
        number: n.find("number")?.arg(0)?.to_string(),
        name: if name == "~" { String::new() } else { name.to_string() },
        kind: Some(pin_type(n.arg(0).unwrap_or("passive"))),
        at: pt([at.num(0)?, at.num(1)?]),
        side: side(at.num(2).unwrap_or(0.0)),
        length: n.find("length").and_then(|l| l.num(0)).map(Length::mm),
        unit: (unit > 0).then_some(unit),
        shape: Some(pin_shape(n.arg(1).unwrap_or("line"))).filter(|s| *s != PinShape::Line),
        hidden: n.flag("hide"),
        levels: None,
    })
}

fn unit_style(sub: &str, parent: &str) -> Option<(u32, u32)> {
    let rest = sub.strip_prefix(parent)?.strip_prefix('_')?;
    let (u, s) = rest.split_once('_')?;
    Some((u.parse().ok()?, s.parse().ok()?))
}

pub fn find<'a>(lib: &'a Node, name: &str) -> Option<&'a Node> {
    lib.all("symbol").find(|s| s.arg(0) == Some(name))
}

pub fn names(lib: &Node) -> Vec<String> {
    lib.all("symbol").filter_map(|s| s.arg(0)).map(str::to_string).collect()
}

pub fn convert(lib: &Node, name: &str) -> Result<SymbolFile, String> {
    let sym = find(lib, name).ok_or_else(|| format!("symbol `{name}` is not in this library"))?;
    let mut body = sym;
    for _ in 0..8 {
        let Some(parent) = body.find("extends").and_then(|e| e.arg(0)) else { break };
        body = find(lib, parent)
            .ok_or_else(|| format!("`{name}` extends `{parent}`, which is missing"))?;
    }
    let body_name = body.arg(0).unwrap_or(name);
    let prop = |key: &str| {
        sym.property_text(key)
            .or_else(|| body.property_text(key))
            .filter(|v| !v.is_empty() && v != "~")
    };

    let mut graphics = Vec::new();
    let mut pins = Vec::new();
    for sub in body.all("symbol") {
        let Some((unit, style)) = sub.arg(0).and_then(|s| unit_style(s, body_name)) else {
            continue;
        };
        if style > 1 {
            continue;
        }
        for item in sub.items().iter().skip(2) {
            if item.head() == Some("pin") {
                pins.extend(pin(item, unit));
            } else {
                graphics.extend(graphic(item, unit));
            }
        }
    }

    let pin_names_node = body.find("pin_names");
    let pin_names = match pin_names_node {
        Some(n) if n.flag("hide") => Some(PinNames::Hidden),
        Some(n) if n.find("offset").and_then(|o| o.num(0)) == Some(0.0) => Some(PinNames::Outside),
        _ => None,
    };
    let offset =
        pin_names_node.and_then(|n| n.find("offset")).and_then(|o| o.num(0)).filter(|o| *o > 0.0);

    Ok(SymbolFile {
        name: name.to_string(),
        reference: prop("Reference")
            .map(|r| r.trim_end_matches(|c: char| c == '?' || c.is_ascii_digit()).to_string()),
        value: prop("Value").filter(|v| v != name),
        description: prop("Description").or_else(|| prop("ki_description")).unwrap_or_default(),
        datasheet: prop("Datasheet").unwrap_or_default(),
        keywords: prop("ki_keywords")
            .map(|k| k.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
        footprint: prop("Footprint"),
        footprint_filters: prop("ki_fp_filters")
            .map(|k| k.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
        power: body.find("power").is_some(),
        pin_names,
        pin_name_offset: offset.map(Length::mm),
        hide_pin_numbers: body.find("pin_numbers").is_some_and(|n| n.flag("hide")),
        bodies: Vec::new(),
        graphics,
        pins,
        levels: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sexpr::parse;

    const LIB: &str = r#"(kicad_symbol_lib (version 20241209)
  (symbol "LM2904" (pin_names (offset 0.127))
    (property "Reference" "U" (at 0 5.08 0))
    (property "Value" "LM2904" (at 0 -5.08 0))
    (property "Footprint" "" (at 0 0 0))
    (property "ki_keywords" "dual opamp" (at 0 0 0))
    (symbol "LM2904_1_1"
      (polyline (pts (xy -5.08 5.08) (xy 5.08 0) (xy -5.08 -5.08) (xy -5.08 5.08))
        (stroke (width 0.254) (type default)) (fill (type background)))
      (pin input line (at -7.62 2.54 0) (length 2.54) (name "+") (number "3"))
      (pin output line (at 7.62 0 180) (length 2.54) (name "~") (number "1")))
    (symbol "LM2904_3_1"
      (pin power_in line (at -2.54 7.62 270) (length 3.81) (name "V+") (number "8"))))
  (symbol "LM358" (extends "LM2904")
    (property "Reference" "U" (at 0 5.08 0))
    (property "Value" "LM358" (at 0 -5.08 0))))"#;

    #[test]
    fn extends_and_flips() {
        let lib = parse(LIB).unwrap();
        let s = convert(&lib, "LM358").unwrap();
        assert_eq!(s.name, "LM358");
        assert_eq!(s.pins.len(), 3);
        let plus = s.pins.iter().find(|p| p.number == "3").unwrap();
        assert_eq!(plus.at, Point::mm(-7.62, -2.54));
        assert_eq!(plus.side, Side::Left);
        let vp = s.pins.iter().find(|p| p.number == "8").unwrap();
        assert_eq!(vp.side, Side::Top);
        assert_eq!(vp.at, Point::mm(-2.54, -7.62));
        assert_eq!(s.graphics[0].kind, GraphicKind::Polygon);
        assert_eq!(s.keywords, ["dual", "opamp"]);
        assert!(s.footprint.is_none());
    }
}
