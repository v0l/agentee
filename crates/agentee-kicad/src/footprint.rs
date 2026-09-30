use crate::sexpr::Node;
use agentee_core::footprint::{Drill, FootprintFile, Mount, PadFile, PadKind, PadShape};
use agentee_core::geom;
use agentee_core::graphic::{Anchor, Fill, GraphicFile, GraphicKind};
use agentee_core::layout::PadConnection;
use agentee_core::units::{Length, Point};

fn pt(v: [f64; 2]) -> Point {
    Point::mm(v[0], v[1])
}

fn width(n: &Node) -> Option<Length> {
    let w = n.find("stroke").and_then(|s| s.find("width")).or_else(|| n.find("width"))?.num(0)?;
    Some(Length::mm(w))
}

fn filled(n: &Node) -> bool {
    match n.find("fill") {
        Some(f) => {
            matches!(f.arg(0), Some("yes" | "solid"))
                || f.find("type").and_then(|t| t.arg(0)) == Some("solid")
        }
        None => false,
    }
}

fn layer_name(l: &str) -> String {
    match l {
        "F&B.Cu" => "*.Cu".into(),
        l => l.to_string(),
    }
}

fn layer(n: &Node) -> Option<String> {
    n.find("layer").and_then(|l| l.arg(0)).map(layer_name)
}

pub(crate) fn text(n: &Node, content: &str) -> Option<GraphicFile> {
    if n.flag("hide") || n.find("effects").is_some_and(|e| e.flag("hide")) {
        return None;
    }
    let at = n.find("at")?;
    let size = n
        .find("effects")
        .and_then(|e| e.find("font"))
        .and_then(|f| f.find("size"))
        .and_then(|s| s.num(0));
    let anchor = match n.find("effects").and_then(|e| e.find("justify")) {
        Some(j) if j.has_atom("left") => Anchor::Left,
        Some(j) if j.has_atom("right") => Anchor::Right,
        _ => Anchor::Center,
    };
    Some(GraphicFile {
        layer: layer(n),
        text: Some(content.to_string()),
        at: Some(pt([at.num(0)?, at.num(1)?])),
        rotation: at.num(2).filter(|r| *r != 0.0),
        size: size.map(Length::mm),
        anchor: Some(anchor).filter(|a| *a != Anchor::Center),
        ..GraphicFile::new(GraphicKind::Text)
    })
}

pub(crate) fn graphic(n: &Node) -> Option<GraphicFile> {
    let head = n.head()?.replacen("gr_", "fp_", 1);
    let mut g = match head.as_str() {
        "fp_line" => GraphicFile {
            start: Some(pt(n.xy("start")?)),
            end: Some(pt(n.xy("end")?)),
            ..GraphicFile::new(GraphicKind::Line)
        },
        "fp_rect" => GraphicFile {
            start: Some(pt(n.xy("start")?)),
            end: Some(pt(n.xy("end")?)),
            ..GraphicFile::new(GraphicKind::Rect)
        },
        "fp_circle" => {
            let c = n.xy("center")?;
            let e = n.xy("end")?;
            GraphicFile {
                center: Some(pt(c)),
                radius: Some(Length::mm(((e[0] - c[0]).powi(2) + (e[1] - c[1]).powi(2)).sqrt())),
                ..GraphicFile::new(GraphicKind::Circle)
            }
        }
        "fp_arc" => {
            let (start, mid, end) = match n.xy("mid") {
                Some(mid) => (n.xy("start")?, mid, n.xy("end")?),
                None => {
                    let c = n.xy("start")?;
                    let s = n.xy("end")?;
                    let a = n.find("angle")?.num(0)?;
                    let at = |deg: f64| {
                        let [x, y] = geom::rotate([s[0] - c[0], s[1] - c[1]], -deg);
                        [c[0] + x, c[1] + y]
                    };
                    (s, at(a / 2.0), at(a))
                }
            };
            if agentee_core::graphic::arc_center(pt(start), pt(mid), pt(end)).is_none() {
                GraphicFile {
                    start: Some(pt(start)),
                    end: Some(pt(end)),
                    ..GraphicFile::new(GraphicKind::Line)
                }
            } else {
                GraphicFile {
                    start: Some(pt(start)),
                    mid: Some(pt(mid)),
                    end: Some(pt(end)),
                    ..GraphicFile::new(GraphicKind::Arc)
                }
            }
        }
        "fp_poly" => GraphicFile {
            points: Some(n.pts().into_iter().map(pt).collect()),
            ..GraphicFile::new(GraphicKind::Polygon)
        },
        "fp_text" => {
            let content = match n.arg(0)? {
                "reference" => "${REFERENCE}".to_string(),
                "value" => "${VALUE}".to_string(),
                _ => n.arg(1)?.to_string(),
            };
            return text(n, &content);
        }
        "property" => {
            let content = match n.arg(0)? {
                "Reference" => "${REFERENCE}",
                "Value" => "${VALUE}",
                _ => return None,
            };
            return text(n, content);
        }
        _ => return None,
    };
    g.layer = layer(n);
    g.width = width(n).filter(|w| w.is_positive());
    if filled(n) {
        g.fill = Some(Fill::Solid);
    }
    Some(g)
}

fn zone_connect(n: &Node, kind: PadKind) -> Option<PadConnection> {
    match n.find("zone_connect")?.num(0)? as i64 {
        0 => Some(PadConnection::None),
        1 => Some(PadConnection::Relief),
        2 => Some(PadConnection::Solid),
        3 if kind == PadKind::Tht => Some(PadConnection::Relief),
        3 => Some(PadConnection::Solid),
        _ => None,
    }
}

fn pad(n: &Node, footprint: &Node) -> Option<PadFile> {
    let kind = match n.arg(1)? {
        "smd" => PadKind::Smd,
        "thru_hole" => PadKind::Tht,
        "np_thru_hole" => PadKind::Npth,
        "connect" => PadKind::Connect,
        _ => return None,
    };
    let shape = match n.arg(2)? {
        "rect" | "trapezoid" => PadShape::Rect,
        "roundrect" => PadShape::Roundrect,
        "circle" => PadShape::Circle,
        "oval" => PadShape::Oval,
        "custom" => PadShape::Custom,
        _ => PadShape::Rect,
    };
    let at = n.find("at")?;
    let size = n.find("size").and_then(|s| Some(Point::mm(s.num(0)?, s.num(1)?)));
    let drill = n.find("drill").and_then(|d| {
        if d.has_atom("oval") {
            Some(Drill::Slot(Point::mm(d.num(1)?, d.num(2).or(d.num(1))?)))
        } else {
            d.num(0).filter(|v| *v > 0.0).map(|v| Drill::Round(Length::mm(v)))
        }
    });
    let layers = n
        .find("layers")
        .map(|l| l.items().iter().skip(1).filter_map(Node::text).map(layer_name).collect());
    let rotation = at.num(2).filter(|r| *r != 0.0);
    let offset = n
        .find("drill")
        .and_then(|d| d.find("offset"))
        .and_then(|o| Some([o.num(0)?, o.num(1)?]))
        .filter(|o| o[0] != 0.0 || o[1] != 0.0);
    let shape_at = match offset {
        Some(o) => {
            let r = geom::rotate(o, rotation.unwrap_or(0.0));
            [at.num(0)? + r[0], at.num(1)? + r[1]]
        }
        None => [at.num(0)?, at.num(1)?],
    };
    let points = (shape == PadShape::Custom)
        .then(|| custom_outline(n, size.map(|s| s.to_mm()).unwrap_or([0.0, 0.0])))
        .flatten()
        .map(|pts| pts.into_iter().map(pt).collect::<Vec<_>>());
    let circle_anchor =
        n.find("options").and_then(|o| o.find("anchor")).and_then(|a| a.arg(0)) == Some("circle");
    let size = if shape == PadShape::Custom && points.is_some() && circle_anchor {
        Some(Point::mm(0.0, 0.0))
    } else {
        size
    };
    let shape = if shape == PadShape::Custom && points.is_none() {
        match n.find("options").and_then(|o| o.find("anchor")).and_then(|a| a.arg(0)) {
            Some("circle") => PadShape::Circle,
            _ => PadShape::Rect,
        }
    } else {
        shape
    };
    Some(PadFile {
        number: n.arg(0).unwrap_or("").to_string(),
        kind,
        shape,
        at: Point::mm(shape_at[0], shape_at[1]),
        size,
        rotation,
        roundrect_ratio: n.find("roundrect_rratio").and_then(|r| r.num(0)),
        drill,
        drill_offset: offset.map(|o| Point::mm(-o[0], -o[1])),
        layers,
        points,
        count: None,
        pitch: None,
        number_step: None,
        edge: false,
        zone_connect: zone_connect(n, kind).or_else(|| zone_connect(footprint, kind)),
    })
}

fn disc(c: [f64; 2], r: f64) -> Vec<[f64; 2]> {
    let n = 48;
    (0..n)
        .map(|k| {
            let t = std::f64::consts::TAU * k as f64 / n as f64;
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
        .collect()
}

fn stroke(a: [f64; 2], b: [f64; 2], w: f64) -> Vec<Vec<[f64; 2]>> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt();
    let mut out = vec![disc(a, w / 2.0), disc(b, w / 2.0)];
    if len > 1e-9 {
        let (nx, ny) = (-dy / len * w / 2.0, dx / len * w / 2.0);
        out.push(vec![
            [a[0] + nx, a[1] + ny],
            [b[0] + nx, b[1] + ny],
            [b[0] - nx, b[1] - ny],
            [a[0] - nx, a[1] - ny],
        ]);
    }
    out
}

fn custom_outline(n: &Node, size: [f64; 2]) -> Option<Vec<[f64; 2]>> {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;
    let [w, _] = size;
    let mut parts: Vec<Vec<[f64; 2]>> = Vec::new();
    if w > 0.0
        && n.find("options").and_then(|o| o.find("anchor")).and_then(|a| a.arg(0)) == Some("circle")
    {
        parts.push(disc([0.0, 0.0], w / 2.0));
    }
    let prims = n.find("primitives")?;
    for g in prims.items().iter().skip(1) {
        let width = g.find("width").and_then(|v| v.num(0)).unwrap_or(0.0);
        let filled = g.find("fill").is_none_or(|f| f.arg(0) != Some("no"));
        match g.head() {
            Some("gr_poly") => {
                let pts = g.pts();
                if pts.len() >= 3 {
                    if width > 0.0 {
                        for i in 0..pts.len() {
                            parts.extend(stroke(pts[i], pts[(i + 1) % pts.len()], width));
                        }
                    }
                    parts.push(pts);
                }
            }
            Some("gr_circle") => {
                let (Some(c), Some(e)) = (g.xy("center"), g.xy("end")) else { continue };
                let r = ((e[0] - c[0]).powi(2) + (e[1] - c[1]).powi(2)).sqrt();
                if filled {
                    parts.push(disc(c, r + width / 2.0));
                } else if width > 0.0 {
                    let ring = disc(c, r);
                    for i in 0..ring.len() {
                        parts.extend(stroke(ring[i], ring[(i + 1) % ring.len()], width));
                    }
                }
            }
            Some("gr_rect") => {
                let (Some(a), Some(b)) = (g.xy("start"), g.xy("end")) else { continue };
                let rect = vec![a, [b[0], a[1]], b, [a[0], b[1]]];
                if width > 0.0 {
                    for i in 0..4 {
                        parts.extend(stroke(rect[i], rect[(i + 1) % 4], width));
                    }
                }
                if filled {
                    parts.push(rect);
                }
            }
            Some("gr_line") => {
                let (Some(a), Some(b)) = (g.xy("start"), g.xy("end")) else { continue };
                parts.extend(stroke(a, b, width));
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        return None;
    }
    let signed = |r: &[[f64; 2]]| {
        (0..r.len())
            .map(|i| {
                let (a, b) = (r[i], r[(i + 1) % r.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
    };
    let area = |r: &[[f64; 2]]| signed(r).abs();
    for p in parts.iter_mut() {
        if signed(p) < 0.0 {
            p.reverse();
        }
    }
    let merged =
        parts.overlay(&Vec::<Vec<[f64; 2]>>::new(), OverlayRule::Subject, FillRule::NonZero);
    merged
        .into_iter()
        .filter_map(|shape| shape.into_iter().next())
        .max_by(|a, b| area(a).total_cmp(&area(b)))
}

fn same_template(a: &PadFile, b: &PadFile) -> bool {
    a.kind == b.kind
        && a.shape == b.shape
        && a.size == b.size
        && a.rotation == b.rotation
        && a.roundrect_ratio == b.roundrect_ratio
        && a.drill == b.drill
        && a.layers == b.layers
        && a.zone_connect == b.zone_connect
        && a.points.is_none()
        && b.points.is_none()
}

fn next_number(n: &str, step: i64) -> Option<String> {
    let digits = n.len() - n.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let (p, d) = n.split_at(n.len() - digits);
    Some(format!("{p}{}", d.parse::<i64>().ok()? + step))
}

pub fn collapse(pads: Vec<PadFile>) -> Vec<PadFile> {
    let mut out: Vec<PadFile> = Vec::new();
    let mut i = 0;
    while i < pads.len() {
        let first = pads[i].clone();
        let mut run = 1;
        if i + 1 < pads.len() && same_template(&first, &pads[i + 1]) {
            let pitch = pads[i + 1].at - first.at;
            let step_ok = |k: usize| {
                let prev = &pads[i + k - 1];
                let cur = &pads[i + k];
                same_template(&first, cur)
                    && cur.at - prev.at == pitch
                    && next_number(&prev.number, 1).as_deref() == Some(cur.number.as_str())
            };
            while i + run < pads.len() && step_ok(run) {
                run += 1;
            }
            if run >= 3 {
                out.push(PadFile { count: Some(run as u32), pitch: Some(pitch), ..first });
                i += run;
                continue;
            }
        }
        out.push(first);
        i += 1;
    }
    out
}

pub fn convert(root: &Node) -> Result<FootprintFile, String> {
    if !matches!(root.head(), Some("footprint" | "module")) {
        return Err("not a KiCad footprint".into());
    }
    let name = root.arg(0).ok_or("footprint has no name")?;
    let mount = match root.find("attr").and_then(|a| a.arg(0)) {
        Some("smd") => Some(Mount::Smd),
        Some("through_hole") => Some(Mount::Tht),
        _ => None,
    };
    let mut graphics = Vec::new();
    let mut pads = Vec::new();
    for item in root.items().iter().skip(2) {
        if item.head() == Some("pad") {
            pads.extend(pad(item, root));
        } else {
            graphics.extend(graphic(item));
        }
    }
    Ok(FootprintFile {
        name: name.to_string(),
        description: root.find("descr").and_then(|d| d.arg(0)).unwrap_or("").to_string(),
        tags: root
            .find("tags")
            .and_then(|t| t.arg(0))
            .map(|t| t.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
        mount,
        model: root.find("model").and_then(|m| m.arg(0)).map(str::to_string),
        model_offset: model_xyz(root, "offset", 0.0).map(|v| v.map(Length::mm)),
        model_rotate: model_xyz(root, "rotate", 0.0),
        model_scale: model_xyz(root, "scale", 1.0),
        height: None,
        mask_web: None,
        clearance: root
            .find("clearance")
            .and_then(|c| c.num(0))
            .filter(|c| *c > 0.0)
            .map(Length::mm),
        overhang: false,
        mlcc: None,
        net_tie_pad_groups: root
            .find("net_tie_pad_groups")
            .map(|g| {
                (0..)
                    .map_while(|i| g.arg(i))
                    .map(|group| group.split(',').map(|n| n.trim().to_string()).collect())
                    .collect()
            })
            .unwrap_or_default(),
        pads: collapse(pads),
        graphics,
    })
}

fn model_xyz(root: &Node, key: &str, default: f64) -> Option<[f64; 3]> {
    let xyz = root.find("model")?.find(key)?.find("xyz")?;
    let v = [xyz.num(0)?, xyz.num(1)?, xyz.num(2)?];
    v.iter().any(|c| (c - default).abs() > 1e-9).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sexpr::parse;

    #[test]
    fn soic_pads_collapse_into_two_rows() {
        let mut src = String::from(
            "(footprint \"SOIC-8\" (attr smd) (fp_line (start 0 -2.56) (end 1.95 -2.56) (stroke (width 0.12) (type solid)) (layer \"F.SilkS\"))",
        );
        for (i, y) in [-1.905, -0.635, 0.635, 1.905].iter().enumerate() {
            src += &format!(
                "(pad \"{}\" smd roundrect (at -2.475 {y}) (size 1.95 0.6) (layers \"F.Cu\" \"F.Mask\" \"F.Paste\") (roundrect_rratio 0.25))",
                i + 1
            );
        }
        for (i, y) in [1.905, 0.635, -0.635, -1.905].iter().enumerate() {
            src += &format!(
                "(pad \"{}\" smd roundrect (at 2.475 {y}) (size 1.95 0.6) (layers \"F.Cu\" \"F.Mask\" \"F.Paste\") (roundrect_rratio 0.25))",
                i + 5
            );
        }
        src += ")";
        let fp = convert(&parse(&src).unwrap()).unwrap();
        assert_eq!(fp.pads.len(), 2);
        assert_eq!(fp.pads[1].count, Some(4));
        assert_eq!(fp.pads[1].pitch, Some(Point::mm(0.0, -1.27)));
        assert_eq!(fp.graphics.len(), 1);
    }

    #[test]
    fn footprint_clearance_is_kept_and_pad_options_are_not_it() {
        let src = "(footprint \"JP\" (clearance 0.2) (pad \"1\" smd custom (at 0 0) (size 0.3 0.3) (layers \"F.Cu\") (options (clearance outline))))";
        let fp = convert(&parse(src).unwrap()).unwrap();
        assert_eq!(fp.clearance, Some(Length::mm(0.2)));
        let src = "(footprint \"R\" (pad \"1\" smd rect (at 0 0) (size 0.3 0.3) (layers \"F.Cu\") (clearance 0.3)))";
        assert_eq!(convert(&parse(src).unwrap()).unwrap().clearance, None);
    }

    #[test]
    fn net_tie_pad_groups_split_on_commas() {
        let src = "(footprint \"NT\" (net_tie_pad_groups \"1, 2\" \"3,4,5\") (pad \"1\" smd rect (at 0 0) (size 1 1) (layers \"F.Cu\")))";
        let fp = convert(&parse(src).unwrap()).unwrap();
        assert_eq!(fp.net_tie_pad_groups, [vec!["1", "2"], vec!["3", "4", "5"]]);
    }

    #[test]
    fn pad_zone_connect_overrides_the_footprint_setting() {
        let src = "(footprint \"J\" (zone_connect 1) (pad \"1\" smd rect (at 0 0) (size 1 1) (layers \"F.Cu\") (zone_connect 2)) (pad \"2\" smd rect (at 2 0) (size 1 1) (layers \"F.Cu\")) (pad \"3\" smd rect (at 4 0) (size 1 1) (layers \"F.Cu\") (zone_connect 0)) (pad \"4\" thru_hole circle (at 6 0) (size 1 1) (drill 0.5) (layers \"*.Cu\") (zone_connect 3)))";
        let fp = convert(&parse(src).unwrap()).unwrap();
        let modes: Vec<Option<PadConnection>> = fp.pads.iter().map(|p| p.zone_connect).collect();
        assert_eq!(
            modes,
            [
                Some(PadConnection::Solid),
                Some(PadConnection::Relief),
                Some(PadConnection::None),
                Some(PadConnection::Relief)
            ]
        );
    }
}
