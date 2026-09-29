use agentee_core::footprint::{Drill, Footprint, Pad, PadKind};
use agentee_core::geom;
use agentee_core::graphic::{Anchor, Bounds, Fill, Graphic, Shape, arc_points};
use agentee_core::symbol::{PinNames, PinShape, Side, Symbol};
use egui::epaint::{PathShape, TextShape};
use egui::{Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect, Stroke, Vec2};
use egui_bench::theme::{self, ETCH, LEGEND, PANEL, READOUT, TRACE, VALUE, WELL};

pub const F_CU: Color32 = Color32::from_rgb(0xD0, 0x6A, 0x50);
pub const B_CU: Color32 = Color32::from_rgb(0x4F, 0x8C, 0xD6);
pub const PTH: Color32 = Color32::from_rgb(0xC9, 0xA2, 0x4B);
pub const SILK: Color32 = Color32::from_rgb(0xE6, 0xE9, 0xEE);
pub const B_SILK: Color32 = Color32::from_rgb(0x9A, 0x8F, 0xC8);
pub const FAB: Color32 = Color32::from_rgb(0x8B, 0x92, 0x9C);
pub const CRTYD: Color32 = Color32::from_rgb(0xC0, 0x6C, 0xD0);
pub const PASTE: Color32 = Color32::from_rgb(0x7A, 0x9C, 0xA8);
pub const MASK: Color32 = Color32::from_rgb(0x6A, 0x5A, 0x9A);
pub const EDGE: Color32 = Color32::from_rgb(0xE8, 0xD0, 0x5E);

pub fn layer_color(layer: &str) -> Color32 {
    match layer {
        "F.Cu" => F_CU,
        "B.Cu" => B_CU,
        "F.SilkS" => SILK,
        "B.SilkS" => B_SILK,
        "F.Fab" | "B.Fab" => FAB,
        "F.CrtYd" | "B.CrtYd" => CRTYD,
        "F.Paste" | "B.Paste" => PASTE,
        "F.Mask" | "B.Mask" => MASK,
        "Edge.Cuts" => EDGE,
        l if l.ends_with(".Cu") => PTH,
        _ => LEGEND,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Xf {
    pub rect: Rect,
    pub center: [f64; 2],
    pub scale: f32,
    pub max_stroke: f32,
}

impl Xf {
    pub fn pos(&self, p: [f64; 2]) -> Pos2 {
        self.rect.center()
            + Vec2::new(
                ((p[0] - self.center[0]) as f32) * self.scale,
                ((p[1] - self.center[1]) as f32) * self.scale,
            )
    }

    pub fn mm(&self, s: Pos2) -> [f64; 2] {
        let d = s - self.rect.center();
        [self.center[0] + (d.x / self.scale) as f64, self.center[1] + (d.y / self.scale) as f64]
    }

    pub fn len(&self, mm: f64) -> f32 {
        mm as f32 * self.scale
    }

    pub fn stroke(&self, mm: f64, color: Color32) -> Stroke {
        Stroke::new(self.len(mm).clamp(1.0, self.max_stroke), color)
    }
}

pub struct Ink {
    pub px: f32,
    pub color: Color32,
    pub angle: f32,
    pub anchor: Align2,
    pub font: FontFamily,
}

pub fn text(p: &Painter, at: Pos2, s: &str, ink: Ink) {
    let Ink { px, color, angle, anchor, font } = ink;
    if px < 3.5 || s.is_empty() {
        return;
    }
    let (clean, overbar) = strip_overbar(s);
    let galley = p.layout_no_wrap(clean, FontId::new(px, font), color);
    let size = galley.size();
    let pos = at - anchor.pos_in_rect(&Rect::from_min_size(Pos2::ZERO, size)).to_vec2();
    if overbar {
        let a = egui::emath::Rot2::from_angle(angle);
        let pivot = anchor.pos_in_rect(&Rect::from_min_size(Pos2::ZERO, size)).to_vec2();
        let y = size.y * 0.12;
        let l = at + a * (Vec2::new(0.0, y) - pivot);
        let r = at + a * (Vec2::new(size.x, y) - pivot);
        p.line_segment([l, r], Stroke::new((px * 0.08).max(1.0), color));
    }
    p.add(TextShape::new(pos, galley, color).with_angle_and_anchor(angle, anchor));
}

fn strip_overbar(s: &str) -> (String, bool) {
    if s.contains("~{") {
        (s.replace("~{", "").replace('}', ""), true)
    } else {
        (s.to_string(), false)
    }
}

fn anchor_of(a: Anchor) -> Align2 {
    match a {
        Anchor::Left => Align2::LEFT_CENTER,
        Anchor::Center => Align2::CENTER_CENTER,
        Anchor::Right => Align2::RIGHT_CENTER,
    }
}

pub fn grid(p: &Painter, xf: &Xf, pitch_mm: f64) {
    let mut pitch = pitch_mm;
    while xf.len(pitch) < 9.0 {
        pitch *= 5.0;
    }
    let r = xf.rect;
    let a = xf.mm(r.left_top());
    let b = xf.mm(r.right_bottom());
    let (x0, x1) = ((a[0] / pitch).floor() as i64, (b[0] / pitch).ceil() as i64);
    let (y0, y1) = ((a[1] / pitch).floor() as i64, (b[1] / pitch).ceil() as i64);
    if (x1 - x0) * (y1 - y0) > 40_000 {
        return;
    }
    let dot = ETCH.gamma_multiply(0.9);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let s = xf.pos([x as f64 * pitch, y as f64 * pitch]);
            p.rect_filled(Rect::from_center_size(s, Vec2::splat(1.0)), 0.0, dot);
        }
    }
    let o = xf.pos([0.0, 0.0]);
    let c = LEGEND.gamma_multiply(0.5);
    p.line_segment([o - Vec2::X * 6.0, o + Vec2::X * 6.0], Stroke::new(1.0, c));
    p.line_segment([o - Vec2::Y * 6.0, o + Vec2::Y * 6.0], Stroke::new(1.0, c));
}

fn fill_polygon(p: &Painter, pts: Vec<Pos2>, fill: Color32, stroke: Stroke) {
    if pts.len() < 3 {
        return;
    }
    p.add(PathShape::convex_polygon(pts, fill, stroke));
}

pub fn graphic(p: &Painter, xf: &Xf, g: &Graphic, color: Color32, body_fill: Color32) {
    let stroke = xf.stroke(g.width.to_mm(), color);
    let fill = match g.fill {
        Fill::None => Color32::TRANSPARENT,
        Fill::Solid => color,
        Fill::Background => body_fill,
    };
    match &g.shape {
        Shape::Line { start, end } => {
            p.line_segment([xf.pos(start.to_mm()), xf.pos(end.to_mm())], stroke);
        }
        Shape::Rect { start, end } => {
            let r = Rect::from_two_pos(xf.pos(start.to_mm()), xf.pos(end.to_mm()));
            p.rect_filled(r, 0.0, fill);
            p.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Middle);
        }
        Shape::Polyline { points, closed } => {
            let pts: Vec<Pos2> = points.iter().map(|q| xf.pos(q.to_mm())).collect();
            if *closed {
                if fill != Color32::TRANSPARENT {
                    fill_polygon(p, pts.clone(), fill, Stroke::NONE);
                }
                p.add(PathShape::closed_line(pts, stroke));
            } else {
                p.add(PathShape::line(pts, stroke));
            }
        }
        Shape::Circle { center, radius } => {
            let c = xf.pos(center.to_mm());
            let r = xf.len(radius.to_mm());
            p.circle(c, r, fill, stroke);
        }
        Shape::Arc { start, mid, end } => {
            let pts = arc_points(*start, *mid, *end, 32).into_iter().map(|q| xf.pos(q)).collect();
            p.add(PathShape::line(pts, stroke));
        }
        Shape::Text { at, text: t, size, rotation, anchor } => {
            text(
                p,
                xf.pos(at.to_mm()),
                t,
                Ink {
                    px: xf.len(size.to_mm()) * 1.25,
                    color,
                    angle: -(rotation.to_radians() as f32),
                    anchor: anchor_of(*anchor),
                    font: FontFamily::Proportional,
                },
            );
        }
    }
}

pub struct SymbolStyle {
    pub show_hidden: bool,
    pub reference: String,
}

pub fn symbol(
    p: &Painter,
    xf: &Xf,
    s: &Symbol,
    unit: u32,
    style: &SymbolStyle,
    hover: Option<Pos2>,
) -> Option<usize> {
    let xf = &Xf { max_stroke: 2.5, ..*xf };
    let body = VALUE;
    for g in s.graphics.iter().filter(|g| g.unit == 0 || g.unit == unit) {
        graphic(p, xf, g, body, PANEL);
    }
    let pin_stroke = xf.stroke(0.1524, LEGEND);
    let name_px = xf.len(1.27) * 1.2;
    let num_px = xf.len(1.0) * 1.2;
    let mut hovered = None;
    let mut best = 7.0f32;
    for (i, pin) in s.pins.iter().enumerate() {
        if !pin.in_unit(unit) || (pin.hidden && !style.show_hidden) {
            continue;
        }
        let a = xf.pos(pin.at.to_mm());
        let b = xf.pos(pin.body_end().to_mm());
        if let Some(h) = hover {
            let d = geom::point_segment_distance(
                [h.x as f64, h.y as f64],
                [a.x as f64, a.y as f64],
                [b.x as f64, b.y as f64],
            ) as f32;
            if d < best {
                best = d;
                hovered = Some(i);
            }
        }
        let dim = if pin.hidden { 0.4 } else { 1.0 };
        let stroke = Stroke::new(pin_stroke.width, pin_stroke.color.gamma_multiply(dim));
        let dir = Vec2::new(pin.side.outward()[0] as f32, pin.side.outward()[1] as f32);
        let bubble = xf.len(0.4);
        let line_end = if matches!(pin.shape, PinShape::Inverted | PinShape::InvertedClock) {
            b + dir * bubble * 2.0
        } else {
            b
        };
        p.line_segment([a, line_end], stroke);
        if matches!(pin.shape, PinShape::Inverted | PinShape::InvertedClock) {
            p.circle_stroke(b + dir * bubble, bubble, stroke);
        }
        if matches!(
            pin.shape,
            PinShape::Clock
                | PinShape::InvertedClock
                | PinShape::ClockLow
                | PinShape::EdgeClockHigh
        ) {
            let n = Vec2::new(-dir.y, dir.x) * xf.len(0.5);
            let tip = b - dir * xf.len(0.6);
            p.add(PathShape::line(vec![b + n, tip, b - n], stroke));
        }
        if matches!(pin.shape, PinShape::InputLow | PinShape::OutputLow | PinShape::ClockLow) {
            let n = Vec2::new(-dir.y, dir.x);
            let up = if n.y > 0.0 || n.x < 0.0 { -n } else { n };
            p.add(PathShape::line(
                vec![b, b + dir * xf.len(0.8) + up * xf.len(0.8), b + dir * xf.len(0.8)],
                stroke,
            ));
        }
        p.circle_stroke(
            a,
            xf.len(0.25).max(1.5),
            Stroke::new(1.0, READOUT.gamma_multiply(0.8 * dim)),
        );

        let vertical = matches!(pin.side, Side::Top | Side::Bottom);
        let angle = if vertical { -std::f32::consts::FRAC_PI_2 } else { 0.0 };
        let mid = a + (b - a) * 0.5;
        let across = if vertical { Vec2::new(-1.0, 0.0) } else { Vec2::new(0.0, -1.0) };
        let name_col = VALUE.gamma_multiply(dim);
        let num_col = LEGEND.gamma_multiply(dim);
        let gap = xf.len(0.25);
        if style_names(s) == PinNames::Inside && !pin.name.is_empty() {
            let off = xf.len(s.pin_name_offset.to_mm());
            let inward = -dir;
            let at = b + inward * off;
            let anchor = match pin.side {
                Side::Left => Align2::LEFT_CENTER,
                Side::Right => Align2::RIGHT_CENTER,
                Side::Top => Align2::RIGHT_CENTER,
                Side::Bottom => Align2::LEFT_CENTER,
            };
            text(
                p,
                at,
                &pin.name,
                Ink { px: name_px, color: name_col, angle, anchor, font: FontFamily::Proportional },
            );
        } else if style_names(s) == PinNames::Outside && !pin.name.is_empty() {
            text(
                p,
                mid + across * gap,
                &pin.name,
                Ink {
                    px: name_px,
                    color: name_col,
                    angle,
                    anchor: Align2::CENTER_BOTTOM,
                    font: FontFamily::Proportional,
                },
            );
        }
        if s.show_pin_numbers && !s.power {
            let (at, anchor) = if style_names(s) == PinNames::Outside {
                (mid - across * gap, Align2::CENTER_TOP)
            } else {
                (mid + across * gap, Align2::CENTER_BOTTOM)
            };
            text(
                p,
                at,
                &pin.number,
                Ink {
                    px: num_px,
                    color: num_col,
                    angle,
                    anchor,
                    font: FontFamily::Name(theme::FIGURE_FONT.into()),
                },
            );
        }
    }
    let b = s.bounds(unit);
    if !b.is_empty() {
        let label = if s.units > 1 {
            format!("{}?{}", style.reference, s.unit_label(unit))
        } else {
            format!("{}?", style.reference)
        };
        text(
            p,
            xf.pos([b.min[0], b.min[1]]) - Vec2::new(0.0, xf.len(0.6)),
            &label,
            Ink {
                px: xf.len(1.27) * 1.25,
                color: READOUT,
                angle: 0.0,
                anchor: Align2::LEFT_BOTTOM,
                font: FontFamily::Proportional,
            },
        );
        text(
            p,
            xf.pos([b.min[0], b.max[1]]) + Vec2::new(0.0, xf.len(0.6)),
            &s.value,
            Ink {
                px: xf.len(1.27) * 1.25,
                color: VALUE.gamma_multiply(0.8),
                angle: 0.0,
                anchor: Align2::LEFT_TOP,
                font: FontFamily::Proportional,
            },
        );
    }
    hovered
}

fn style_names(s: &Symbol) -> PinNames {
    s.pin_names
}

#[derive(Clone, Debug)]
pub struct Layers {
    pub hidden: Vec<String>,
}

impl Default for Layers {
    fn default() -> Self {
        Layers { hidden: ["F.Mask", "B.Mask", "F.Paste", "B.Paste"].map(String::from).to_vec() }
    }
}

impl Layers {
    pub fn shows(&self, l: &str) -> bool {
        !self.hidden.iter().any(|h| h == l)
    }

    pub fn toggle(&mut self, l: &str) {
        if let Some(i) = self.hidden.iter().position(|h| h == l) {
            self.hidden.remove(i);
        } else {
            self.hidden.push(l.to_string());
        }
    }
}

pub const LAYER_ORDER: &[&str] = &[
    "B.CrtYd",
    "B.Fab",
    "B.Paste",
    "B.Mask",
    "B.Cu",
    "B.SilkS",
    "F.CrtYd",
    "F.Fab",
    "F.Mask",
    "F.Paste",
    "F.Cu",
    "F.SilkS",
    "Edge.Cuts",
];

fn pad_color(pad: &Pad) -> Color32 {
    let f = pad.on_layer("F.Cu");
    let b = pad.on_layer("B.Cu");
    match (f, b, pad.kind) {
        (_, _, PadKind::Npth) => WELL,
        (true, true, _) => PTH,
        (false, true, _) => B_CU,
        _ => F_CU,
    }
}

fn outline_px(xf: &Xf, pad: &Pad) -> Vec<Pos2> {
    pad.outline().into_iter().map(|q| xf.pos(q)).collect()
}

fn substitute(g: &Graphic) -> Option<Graphic> {
    let Shape::Text { text, .. } = &g.shape else { return None };
    if !text.contains("${") {
        return None;
    }
    let mut g = g.clone();
    if let Shape::Text { text, .. } = &mut g.shape {
        *text = text.replace("${REFERENCE}", "REF**").replace("${VALUE}", "VALUE");
    }
    Some(g)
}

pub fn footprint(
    p: &Painter,
    xf: &Xf,
    fp: &Footprint,
    layers: &Layers,
    hover: Option<Pos2>,
) -> Option<usize> {
    let mut hovered = None;
    let draw_layer = |l: &str| {
        for g in fp.graphics.iter().filter(|g| g.layer == l) {
            let shown = substitute(g);
            graphic(
                p,
                xf,
                shown.as_ref().unwrap_or(g),
                layer_color(l),
                layer_color(l).gamma_multiply(0.25),
            );
        }
    };
    for l in LAYER_ORDER {
        if !layers.shows(l) {
            continue;
        }
        match *l {
            "F.Cu" | "B.Cu" => {
                for (i, pad) in fp.pads.iter().enumerate() {
                    let on = pad.on_layer(l) || (pad.kind == PadKind::Npth && *l == "F.Cu");
                    if !on || (*l == "B.Cu" && pad.on_layer("F.Cu")) {
                        continue;
                    }
                    let pts = outline_px(xf, pad);
                    let col = pad_color(pad);
                    if pad.kind == PadKind::Npth {
                        fill_polygon(p, pts.clone(), WELL, Stroke::new(1.0, ETCH));
                    } else {
                        fill_polygon(p, pts.clone(), col, Stroke::NONE);
                    }
                    if let Some(h) = hover {
                        let poly: Vec<[f64; 2]> = pad.outline();
                        if geom::point_in_polygon(xf.mm(h), &poly) {
                            hovered = Some(i);
                        }
                    }
                }
                draw_layer(l);
            }
            "F.Mask" | "B.Mask" | "F.Paste" | "B.Paste" => {
                for pad in fp.pads.iter().filter(|pad| pad.on_layer(l)) {
                    let pts = outline_px(xf, pad);
                    p.add(PathShape::closed_line(pts, Stroke::new(1.0, layer_color(l))));
                }
                draw_layer(l);
            }
            _ => draw_layer(l),
        }
    }
    for pad in &fp.pads {
        if let Some(d) = pad.drill {
            drill(p, xf, pad, d);
        }
    }
    for pad in fp.pads.iter().filter(|p| !p.number.is_empty()) {
        let b = pad.bounds();
        let [w, h] = b.size();
        let px = xf.len(w.min(h)) * 0.55;
        let px = px.min(xf.len(w.max(h)) / (pad.number.len() as f32 * 0.7).max(1.0));
        let c = xf.pos([(b.min[0] + b.max[0]) / 2.0, (b.min[1] + b.max[1]) / 2.0]);
        let col = if pad.drill.is_some() { VALUE } else { WELL };
        text(
            p,
            c,
            &pad.number,
            Ink {
                px: px.min(28.0),
                color: col,
                angle: 0.0,
                anchor: Align2::CENTER_CENTER,
                font: FontFamily::Name(theme::FIGURE_FONT.into()),
            },
        );
    }
    if let Some(i) = hovered {
        let pts = outline_px(xf, &fp.pads[i]);
        p.add(PathShape::closed_line(pts, Stroke::new(2.0, TRACE)));
    }
    hovered
}

fn drill(p: &Painter, xf: &Xf, pad: &Pad, d: Drill) {
    let [w, h] = d.size().to_mm();
    let local = geom::rounded_rect(w, h, w.min(h) / 2.0, 8);
    let [ax, ay] = pad.at.to_mm();
    let pts: Vec<Pos2> = local
        .into_iter()
        .map(|q| {
            let [x, y] = geom::rotate(q, pad.rotation);
            xf.pos([x + ax, y + ay])
        })
        .collect();
    fill_polygon(p, pts, WELL, Stroke::new(1.0, ETCH));
}

pub fn outline_points(o: &agentee_core::board::Outline) -> Vec<[f64; 2]> {
    use agentee_core::board::Outline;
    match o {
        Outline::Rect { origin, size, corner_radius } => {
            let [w, h] = size.to_mm();
            let [x0, y0] = origin.to_mm();
            geom::rounded_rect(w, h, corner_radius.to_mm(), 8)
                .into_iter()
                .map(|q| [q[0] + x0 + w / 2.0, q[1] + y0 + h / 2.0])
                .collect()
        }
        Outline::Polygon { points } => points.iter().map(|q| q.to_mm()).collect(),
    }
}

pub fn outline_bounds(o: &agentee_core::board::Outline) -> Bounds {
    let mut b = Bounds::EMPTY;
    outline_points(o).into_iter().for_each(|q| b.add(q));
    b
}
