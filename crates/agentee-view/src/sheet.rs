use crate::paint::{self, Ink, SymbolStyle, Xf, text};
use agentee_core::geom::{self, P};
use agentee_core::schematic::{NetStyle, PinRef, Schematic};
use egui::epaint::PathShape;
use egui::{Align2, Color32, FontFamily, Painter, Pos2, Stroke, Vec2};
use egui_bench::theme::{LEGEND, TRACE};

pub const WIRE: Color32 = Color32::from_rgb(0x5C, 0xB0, 0x7A);
pub const LABEL: Color32 = Color32::from_rgb(0xE8, 0xB0, 0x3E);
pub const POWER: Color32 = Color32::from_rgb(0xE2, 0x6D, 0x5A);

pub fn is_ground(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    n.contains("GND") || n.starts_with("VSS") || n == "0V"
}

fn px(xf: &Xf, p: P) -> Pos2 {
    xf.world(p)
}

pub struct Hover {
    pub net: Option<usize>,
    pub pin: Option<PinRef>,
}

pub fn schematic(
    p: &Painter,
    xf: &Xf,
    s: &Schematic,
    hover: Option<Pos2>,
    show_hidden: bool,
) -> Hover {
    let mut hit = Hover { net: None, pin: None };
    let mut best = 6.0f32;
    let highlight = hover.and_then(|h| {
        let mut found = None;
        for (ni, n) in s.nets.iter().enumerate() {
            for w in &n.wires {
                for seg in w.windows(2) {
                    let (a, b) = (px(xf, seg[0]), px(xf, seg[1]));
                    let d = geom::point_segment_distance(
                        [h.x as f64, h.y as f64],
                        [a.x as f64, a.y as f64],
                        [b.x as f64, b.y as f64],
                    ) as f32;
                    if d < best {
                        best = d;
                        found = Some(ni);
                    }
                }
            }
        }
        found
    });

    for (pi, part) in s.parts.iter().enumerate() {
        let local = xf.placed(part.transform());
        let unit =
            if part.symbol.units > 1 { part.symbol.unit_label(part.unit) } else { String::new() };
        let style = SymbolStyle {
            show_hidden,
            reference: format!("{}{unit}", part.reference),
            value: if part.dnp { format!("{} DNP", part.value) } else { part.value.clone() },
            dim: part.dnp,
            tips: false,
        };
        if let Some(pin) = paint::symbol(p, &local, &part.symbol, part.unit, &style, hover) {
            let r = PinRef { part: pi, pin };
            let tip = px(xf, part.pin_at(pin));
            let d = hover.map(|h| (h - tip).length()).unwrap_or(f32::MAX);
            if d < 40.0 {
                hit.pin = Some(r);
            }
        }
    }
    hit.net = highlight.or_else(|| hit.pin.and_then(|r| s.net_of(r)));

    for (ni, n) in s.nets.iter().enumerate() {
        let lit = hit.net == Some(ni);
        let color = if lit { TRACE } else { WIRE };
        let stroke = Stroke::new(if lit { 2.5 } else { 1.6 }, color);
        match n.style {
            NetStyle::Wire => {
                for w in &n.wires {
                    let pts: Vec<Pos2> = w.iter().map(|q| px(xf, *q)).collect();
                    p.add(PathShape::line(pts, stroke));
                }
                for j in &n.junctions {
                    p.circle_filled(px(xf, *j), xf.len(0.45).max(3.0), color);
                }
                let longest = n
                    .wires
                    .iter()
                    .flat_map(|w| w.windows(2))
                    .filter(|w| (w[0][1] - w[1][1]).abs() < 1e-6)
                    .max_by(|a, b| geom::dist(a[0], a[1]).total_cmp(&geom::dist(b[0], b[1])));
                if let Some(w) = longest
                    && geom::dist(w[0], w[1]) > 5.0
                {
                    let mid = px(xf, [(w[0][0] + w[1][0]) / 2.0, w[0][1]]);
                    text(
                        p,
                        mid - Vec2::new(0.0, xf.len(0.3)),
                        &n.name,
                        Ink {
                            px: xf.len(1.0) * 1.2,
                            color: color.gamma_multiply(0.75),
                            angle: 0.0,
                            anchor: Align2::CENTER_BOTTOM,
                            font: FontFamily::Proportional,
                        },
                    );
                }
            }
            NetStyle::Label => {
                for r in &n.pins {
                    label(p, xf, s, *r, &n.name, if lit { TRACE } else { LABEL });
                }
            }
            NetStyle::Power => {
                for r in &n.pins {
                    power(p, xf, s, *r, &n.name, if lit { TRACE } else { POWER });
                }
            }
        }
    }
    for r in &s.no_connect {
        let c = px(xf, s.parts[r.part].pin_at(r.pin));
        let k = xf.len(0.6).max(4.0);
        let st = Stroke::new(1.6, LEGEND);
        p.line_segment([c - Vec2::splat(k), c + Vec2::splat(k)], st);
        p.line_segment([c + Vec2::new(-k, k), c + Vec2::new(k, -k)], st);
    }
    hit
}

fn outward(s: &Schematic, r: PinRef) -> Vec2 {
    let o = s.parts[r.part].pin_outward(r.pin);
    Vec2::new(o[0] as f32, o[1] as f32)
}

fn label(p: &Painter, xf: &Xf, s: &Schematic, r: PinRef, name: &str, color: Color32) {
    let tip = px(xf, s.parts[r.part].pin_at(r.pin));
    let dir = outward(s, r);
    let stub = tip + dir * xf.len(1.27);
    p.line_segment([tip, stub], Stroke::new(1.6, WIRE));
    let size = xf.len(1.27) * 1.2;
    let vertical = dir.y.abs() > dir.x.abs();
    let angle = if vertical { -std::f32::consts::FRAC_PI_2 } else { 0.0 };
    let anchor = match (vertical, dir.x > 0.0, dir.y > 0.0) {
        (false, true, _) => Align2::LEFT_BOTTOM,
        (false, false, _) => Align2::RIGHT_BOTTOM,
        (true, _, true) => Align2::RIGHT_BOTTOM,
        (true, _, false) => Align2::LEFT_BOTTOM,
    };
    let lift = if vertical { Vec2::new(-xf.len(0.2), 0.0) } else { Vec2::new(0.0, -xf.len(0.2)) };
    text(
        p,
        stub + lift - dir * xf.len(0.9),
        name,
        Ink { px: size, color, angle, anchor, font: FontFamily::Proportional },
    );
}

fn power(p: &Painter, xf: &Xf, s: &Schematic, r: PinRef, name: &str, color: Color32) {
    let tip = px(xf, s.parts[r.part].pin_at(r.pin));
    let ground = is_ground(name);
    let dir = if ground { Vec2::new(0.0, 1.0) } else { Vec2::new(0.0, -1.0) };
    let o = outward(s, r);
    let st = Stroke::new(1.6, color);
    let turn = if o.dot(dir) > 0.5 { tip } else { tip + o * xf.len(1.27) };
    p.line_segment([tip, turn], Stroke::new(1.6, WIRE));
    let stem = turn + dir * xf.len(1.27);
    p.line_segment([turn, stem], st);
    let w = xf.len(1.27);
    if ground {
        p.add(PathShape::convex_polygon(
            vec![stem + Vec2::new(-w, 0.0), stem + Vec2::new(w, 0.0), stem + Vec2::new(0.0, w)],
            Color32::TRANSPARENT,
            st,
        ));
    } else {
        p.line_segment([stem + Vec2::new(-w * 0.8, 0.0), stem + Vec2::new(w * 0.8, 0.0)], st);
        text(
            p,
            stem - Vec2::new(0.0, xf.len(0.3)),
            name,
            Ink {
                px: xf.len(1.27) * 1.2,
                color,
                angle: 0.0,
                anchor: Align2::CENTER_BOTTOM,
                font: FontFamily::Proportional,
            },
        );
    }
}

pub fn legend_for(s: &Schematic, h: &Hover) -> Option<(String, String)> {
    if let Some(r) = h.pin {
        let pin = &s.parts[r.part].symbol.pins[r.pin];
        let net =
            s.net_of(r).map(|n| s.nets[n].name.clone()).unwrap_or_else(|| "not connected".into());
        return Some((format!("{} {}", s.pin_label(r), pin.name), net));
    }
    h.net.map(|n| (s.nets[n].name.clone(), s.nets[n].class.clone()))
}
