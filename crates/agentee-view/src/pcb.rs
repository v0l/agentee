use crate::paint::{self, Ink, Layers, Xf, fill_polygon, layer_color, text};
use agentee_core::footprint::PadKind;
use agentee_core::geom::{self, P};
use agentee_core::graphic::{Graphic, Shape};
use agentee_core::layout::Layout;
use egui::epaint::PathShape;
use egui::{
    Align2, Color32, ColorImage, FontFamily, Painter, Pos2, Rect, Stroke, TextureHandle,
    TextureOptions,
};
use egui_bench::theme::{self, ETCH, TRACE, VALUE, WELL};

pub const IN1: Color32 = Color32::from_rgb(0x9E, 0xC2, 0x5A);
pub const IN2: Color32 = Color32::from_rgb(0xC0, 0x78, 0xB8);
pub const SUBSTRATE: Color32 = Color32::from_rgb(0x1E, 0x24, 0x22);

pub fn copper_color(layer: &str) -> Color32 {
    match layer {
        "F.Cu" => paint::F_CU,
        "B.Cu" => paint::B_CU,
        "In1.Cu" => IN1,
        "In2.Cu" => IN2,
        _ => paint::PTH,
    }
}

pub fn default_layers() -> Layers {
    Layers {
        hidden: [
            "F.Fab", "B.Fab", "F.CrtYd", "B.CrtYd", "F.Mask", "B.Mask", "F.Paste", "B.Paste",
            "In1.Cu", "In2.Cu", "B.Cu", "B.SilkS",
        ]
        .map(String::from)
        .to_vec(),
    }
}

pub fn zone_textures(ctx: &egui::Context, l: &Layout) -> Vec<TextureHandle> {
    l.zones
        .iter()
        .enumerate()
        .map(|(i, z)| {
            let c = copper_color(&z.layer).gamma_multiply(0.55);
            let pixels: Vec<Color32> =
                z.mask.iter().map(|m| if *m != 0 { c } else { Color32::TRANSPARENT }).collect();
            let img = ColorImage::new([z.width, z.height], pixels);
            ctx.load_texture(format!("zone-{}-{i}", l.name), img, TextureOptions::NEAREST)
        })
        .collect()
}

pub struct Hit {
    pub net: Option<usize>,
    pub pad: Option<(usize, usize)>,
}

fn round_track(p: &Painter, xf: &Xf, pts: &[P], width: f64, color: Color32) {
    let w = xf.len(width).max(1.0);
    let s: Vec<Pos2> = pts.iter().map(|q| xf.world(*q)).collect();
    for seg in s.windows(2) {
        p.line_segment([seg[0], seg[1]], Stroke::new(w, color));
    }
    for q in &s {
        p.circle_filled(*q, w / 2.0, color);
    }
}

fn substitute(g: &Graphic, reference: &str, value: &str) -> Graphic {
    let mut g = g.clone();
    if let Shape::Text { text, .. } = &mut g.shape {
        *text = text.replace("${REFERENCE}", reference).replace("${VALUE}", value);
    }
    g
}

pub fn layout(
    p: &Painter,
    xf: &Xf,
    l: &Layout,
    layers: &Layers,
    zones: &[TextureHandle],
    hover: Option<Pos2>,
    ratsnest: bool,
) -> Hit {
    let mut hit = Hit { net: None, pad: None };
    if let Some(h) = hover {
        let m = xf.mm(h);
        for (pi, part) in l.parts.iter().enumerate() {
            for (k, pad) in part.pads.iter().enumerate() {
                if pad.outlines.iter().any(|o| geom::point_in_polygon(m, o)) {
                    hit.pad = Some((pi, k));
                    hit.net = pad.net;
                }
            }
        }
        if hit.net.is_none() {
            hit.net = l
                .tracks
                .iter()
                .filter(|t| layers.shows(&t.layer))
                .find(|t| {
                    t.points
                        .windows(2)
                        .any(|w| geom::point_segment_distance(m, w[0], w[1]) < t.width / 2.0)
                })
                .map(|t| t.net)
                .or_else(|| {
                    l.vias.iter().find(|v| geom::dist(m, v.at) < v.diameter / 2.0).map(|v| v.net)
                });
        }
    }

    let outline: Vec<Pos2> = l.outline.iter().map(|q| xf.world(*q)).collect();
    if outline.len() >= 3 {
        fill_polygon(p, outline.clone(), SUBSTRATE, Stroke::NONE);
    }

    for layer in l.copper.iter().rev() {
        if !layers.shows(layer) {
            continue;
        }
        let color = copper_color(layer);
        for (z, tex) in l.zones.iter().zip(zones) {
            if &z.layer != layer {
                continue;
            }
            let min = xf.world(z.origin);
            let max = xf.world([
                z.origin[0] + z.width as f64 * z.cell,
                z.origin[1] + z.height as f64 * z.cell,
            ]);
            p.image(
                tex.id(),
                Rect::from_min_max(min, max),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        for t in l.tracks.iter().filter(|t| &t.layer == layer) {
            let c = if hit.net == Some(t.net) {
                color.lerp_to_gamma(Color32::WHITE, 0.35)
            } else {
                color
            };
            round_track(p, xf, &t.points, t.width, c);
        }
        for (pi, part) in l.parts.iter().enumerate() {
            for (k, pad) in part.pads.iter().enumerate() {
                if !pad.copper.iter().any(|c| c == layer) {
                    continue;
                }
                let c = if pad.copper.len() > 1 { paint::PTH } else { color };
                let lit = hit.net.is_some() && pad.net == hit.net || hit.pad == Some((pi, k));
                let c = if lit { c.lerp_to_gamma(Color32::WHITE, 0.35) } else { c };
                for o in &pad.outlines {
                    fill_polygon(p, o.iter().map(|q| xf.world(*q)).collect(), c, Stroke::NONE);
                }
            }
        }
    }

    for v in &l.vias {
        let c = xf.world(v.at);
        let lit = hit.net == Some(v.net);
        p.circle_filled(
            c,
            xf.len(v.diameter / 2.0),
            if lit { paint::PTH.lerp_to_gamma(Color32::WHITE, 0.35) } else { paint::PTH },
        );
        p.circle_filled(c, xf.len(v.drill / 2.0), WELL);
    }
    for part in &l.parts {
        for pad in &part.pads {
            if let Some((c, s, rot)) = pad.drill {
                let local = geom::rounded_rect(s[0], s[1], s[0].min(s[1]) / 2.0, 8);
                let pts = local
                    .into_iter()
                    .map(|q| {
                        let [x, y] = geom::rotate(q, rot);
                        xf.world([x + c[0], y + c[1]])
                    })
                    .collect();
                let stroke =
                    if pad.kind == PadKind::Npth { Stroke::new(1.0, ETCH) } else { Stroke::NONE };
                fill_polygon(p, pts, WELL, stroke);
            }
        }
    }

    for (pi, part) in l.parts.iter().enumerate() {
        let placed = xf.placed(part.transform());
        for g in &part.footprint.graphics {
            let layer = part.flip_layer(&g.layer);
            if layer.ends_with(".Cu")
                || !layers.shows(&layer)
                || layer.ends_with("Mask")
                || layer.ends_with("Paste")
                || (layer.ends_with(".SilkS") && matches!(g.shape, Shape::Text { .. }))
            {
                continue;
            }
            let g = substitute(g, &part.reference, &part.value);
            let col = layer_color(&layer);
            paint::graphic(p, &placed, &g, col, col.gamma_multiply(0.25));
        }
        for t in part.silk_texts(pi) {
            if !layers.shows(&t.layer) {
                continue;
            }
            let anchor = match t.anchor {
                agentee_core::graphic::Anchor::Left => Align2::LEFT_CENTER,
                agentee_core::graphic::Anchor::Center => Align2::CENTER_CENTER,
                agentee_core::graphic::Anchor::Right => Align2::RIGHT_CENTER,
            };
            text(
                p,
                xf.world(t.at),
                &t.text,
                Ink {
                    px: xf.len(t.size) * 1.25,
                    color: layer_color(&t.layer),
                    angle: -(t.rotation.to_radians() as f32),
                    anchor,
                    font: FontFamily::Proportional,
                },
            );
        }
        let shown = part.pads.iter().filter(|q| !q.number.is_empty());
        for pad in shown {
            let mut b = agentee_core::graphic::Bounds::EMPTY;
            pad.outlines.iter().flatten().for_each(|q| b.add(*q));
            let [w, h] = b.size();
            let size = xf.len(w.min(h)) * 0.5;
            if size < 7.0 {
                continue;
            }
            text(
                p,
                xf.world(b.center()),
                &pad.number,
                Ink {
                    px: size.min(22.0),
                    color: WELL,
                    angle: 0.0,
                    anchor: Align2::CENTER_CENTER,
                    font: FontFamily::Name(theme::FIGURE_FONT.into()),
                },
            );
        }
    }

    if outline.len() >= 3 && layers.shows("Edge.Cuts") {
        p.add(PathShape::closed_line(outline, Stroke::new(1.5, paint::EDGE)));
    }
    if layers.shows("Cutouts") {
        for (ls, pts) in &l.cutouts {
            if ls.iter().any(|x| layers.shows(x)) {
                let s: Vec<Pos2> = pts.iter().map(|q| xf.world(*q)).collect();
                p.add(PathShape::closed_line(s, Stroke::new(1.0, VALUE.gamma_multiply(0.5))));
            }
        }
    }
    if ratsnest {
        for (a, b, n) in &l.ratsnest {
            let lit = hit.net == Some(*n);
            p.line_segment(
                [xf.world(*a), xf.world(*b)],
                Stroke::new(if lit { 2.0 } else { 1.0 }, if lit { TRACE } else { VALUE }),
            );
        }
    }
    hit
}
