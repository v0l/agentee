use crate::paint::{self, Ink, Layers, Xf, fill_polygon, layer_color, text};
use agentee_core::footprint::PadKind;
use agentee_core::geom::{self, P};
use agentee_core::graphic::{Graphic, Shape};
use agentee_core::layout::Layout;
use egui::epaint::PathShape;
use egui::{
    Align2, Color32, ColorImage, FontFamily, Painter, Pos2, Stroke, TextureHandle, TextureOptions,
};
use egui_bench::theme::{self, ETCH, TRACE, VALUE, WELL};

pub const IN1: Color32 = Color32::from_rgb(0x9E, 0xC2, 0x5A);
pub const IN2: Color32 = Color32::from_rgb(0xC0, 0x78, 0xB8);
pub const SUBSTRATE: Color32 = Color32::from_rgb(0x1E, 0x24, 0x22);

pub fn copper_color(layer: &str) -> Color32 {
    match layer {
        "F.Cu" => paint::F_CU,
        "B.Cu" => paint::B_CU,
        _ => match inner_index(layer) {
            Some(i) if i % 2 == 1 => IN1,
            Some(_) => IN2,
            None => paint::PTH,
        },
    }
}

fn arc(c: Pos2, r: f32, from: f32) -> Vec<Pos2> {
    (0..=12)
        .map(|k| {
            let a = from + std::f32::consts::PI * k as f32 / 12.0;
            Pos2::new(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}

fn inner_index(layer: &str) -> Option<u32> {
    layer.strip_prefix("In")?.strip_suffix(".Cu")?.parse().ok()
}

pub fn default_layers() -> Layers {
    let mut hidden: Vec<String> = [
        "F.Fab", "B.Fab", "F.CrtYd", "B.CrtYd", "F.Mask", "B.Mask", "F.Paste", "B.Paste", "B.Cu",
        "B.SilkS",
    ]
    .map(String::from)
    .to_vec();
    hidden.extend((1..=30).map(|i| format!("In{i}.Cu")));
    Layers { hidden }
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
    let board_cutouts: Vec<Vec<Pos2>> = l
        .board_cutouts
        .iter()
        .filter(|c| c.len() >= 3)
        .map(|c| c.iter().map(|q| xf.world(*q)).collect())
        .collect();
    if outline.len() >= 3 {
        fill_polygon(p, outline.clone(), SUBSTRATE, Stroke::NONE);
        for c in &board_cutouts {
            fill_polygon(p, c.clone(), WELL, Stroke::NONE);
        }
    }

    for layer in l.copper.iter().rev() {
        if !layers.shows(layer) {
            continue;
        }
        let color = copper_color(layer);
        let _ = zones;
        for z in l.zones.iter().filter(|z| &z.layer == layer) {
            let fill = copper_color(&z.layer).gamma_multiply(0.55);
            let mut mesh = egui::Mesh::default();
            for t in &z.triangles {
                let base = mesh.vertices.len() as u32;
                for q in t {
                    mesh.colored_vertex(xf.world(*q), fill);
                }
                mesh.add_triangle(base, base + 1, base + 2);
            }
            p.add(mesh);
            let edge = Stroke::new(1.0, fill);
            for r in &z.rings {
                p.add(PathShape::closed_line(r.iter().map(|q| xf.world(*q)).collect(), edge));
            }
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
                let c = if pad.copper.len() > 1 {
                    paint::PTH
                } else {
                    color.lerp_to_gamma(Color32::WHITE, 0.22)
                };
                let lit = hit.net.is_some() && pad.net == hit.net || hit.pad == Some((pi, k));
                let c = if lit { c.lerp_to_gamma(Color32::WHITE, 0.35) } else { c };
                for o in &pad.outlines {
                    fill_polygon(p, o.iter().map(|q| xf.world(*q)).collect(), c, Stroke::NONE);
                }
            }
            let placed = xf.placed(part.transform());
            let c = color.lerp_to_gamma(Color32::WHITE, 0.22);
            for g in part.footprint.graphics.iter().filter(|g| part.flip_layer(&g.layer) == *layer)
            {
                if !matches!(g.shape, Shape::Text { .. }) {
                    paint::graphic(p, &placed, g, c, c);
                }
            }
        }
    }

    for v in l.vias.iter().filter(|v| v.layers.iter().any(|x| layers.shows(x))) {
        let c = xf.world(v.at);
        let lit = hit.net == Some(v.net);
        let ring = paint::via_color(v.kind);
        p.circle_filled(
            c,
            xf.len(v.diameter / 2.0),
            if lit { ring.lerp_to_gamma(Color32::WHITE, 0.35) } else { ring },
        );
        p.circle_filled(c, xf.len(v.drill / 2.0), WELL);
        if v.kind != agentee_core::board::ViaKind::Through {
            let edge = [v.layers.first(), v.layers.last()]
                .map(|x| x.map(|n| copper_color(n)).unwrap_or(ring));
            let r = xf.len(v.diameter / 2.0);
            let w = (r * 0.25).max(1.0);
            p.add(PathShape::line(
                arc(c, r - w / 2.0, -std::f32::consts::PI),
                Stroke::new(w, edge[0]),
            ));
            p.add(PathShape::line(arc(c, r - w / 2.0, 0.0), Stroke::new(w, edge[1])));
        }
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

    board_art(p, xf, l, layers);
    for (pi, part) in l.parts.iter().enumerate() {
        let placed = xf.placed(part.transform());
        for g in &part.footprint.graphics {
            let layer = part.flip_layer(&g.layer);
            if layer.ends_with(".Cu")
                || !layers.shows(&layer)
                || layer.ends_with("Mask")
                || layer.ends_with("Paste")
                || ((layer.ends_with(".SilkS") || layer.ends_with(".Fab"))
                    && matches!(g.shape, Shape::Text { .. }))
            {
                continue;
            }
            let g = substitute(g, &part.reference, &part.value);
            let col = layer_color(&layer);
            paint::graphic(p, &placed, &g, col, col.gamma_multiply(0.25));
        }
        for side in ["F", "B"] {
            let fab = format!("{side}.Fab");
            if !layers.shows(&fab) {
                continue;
            }
            let mut body = agentee_core::graphic::Bounds::EMPTY;
            for g in part.footprint.graphics.iter().filter(|g| part.flip_layer(&g.layer) == fab) {
                if !matches!(g.shape, Shape::Text { .. }) {
                    body.union(&g.bounds());
                }
            }
            if body.is_empty() {
                continue;
            }
            let t = part.transform();
            let mut world = agentee_core::graphic::Bounds::EMPTY;
            for c in [body.min, body.max] {
                world.add(t.apply(c));
            }
            let [w, h] = world.size();
            let (long, short, angle) =
                if h > w { (h, w, -std::f32::consts::FRAC_PI_2) } else { (w, h, 0.0) };
            let chars = part.reference.chars().count().max(1) as f64;
            let size = (long * 0.8 / (0.62 * chars)).min(short * 0.6).min(1.0);
            if xf.len(size) < 3.0 {
                continue;
            }
            text(
                p,
                xf.world(world.center()),
                &part.reference,
                Ink {
                    px: xf.len(size) * 1.25,
                    color: layer_color(&fab),
                    angle,
                    anchor: Align2::CENTER_CENTER,
                    font: FontFamily::Proportional,
                },
            );
        }
        for side in ["F.Mask", "B.Mask"] {
            if !layers.shows(side) {
                continue;
            }
            for pad in part.pads.iter().filter(|q| q.mask.iter().any(|m| m == side)) {
                for o in &pad.outlines {
                    let pts: Vec<Pos2> = o.iter().map(|q| xf.world(*q)).collect();
                    p.add(PathShape::closed_line(pts, Stroke::new(1.2, layer_color(side))));
                }
            }
        }
        for t in part.silk_texts(pi) {
            if layers.shows(&t.layer) {
                silk_text(p, xf, &t);
            }
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
        for c in board_cutouts {
            p.add(PathShape::closed_line(c, Stroke::new(1.5, paint::EDGE)));
        }
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

fn board_art(p: &Painter, xf: &Xf, l: &Layout, layers: &Layers) {
    for a in l.artwork.iter().filter(|a| layers.shows(&a.layer)) {
        let col = layer_color(&a.layer);
        for r in &a.polygons {
            fill_polygon(p, r.iter().map(|q| xf.world(*q)).collect(), col, Stroke::NONE);
        }
    }
    let base =
        xf.placed(agentee_core::geom::Transform { at: [0.0, 0.0], rotation: 0.0, mirror: false });
    for g in l.graphics.iter().filter(|g| layers.shows(&g.layer)) {
        let col = layer_color(&g.layer);
        if matches!(g.shape, Shape::Text { .. }) && g.layer.ends_with(".SilkS") {
            continue;
        }
        paint::graphic(p, &base, g, col, col.gamma_multiply(0.25));
    }
    for t in l.board_texts() {
        if layers.shows(&t.layer) {
            silk_text(p, xf, &t);
        }
    }
}

fn silk_text(p: &Painter, xf: &Xf, t: &agentee_core::layout::SilkText) {
    let pen = agentee_core::font::default_thickness(t.size);
    let stroke = xf.stroke(pen, layer_color(&t.layer));
    let bottom = t.layer.starts_with("B.");
    for st in agentee_core::font::strokes(&t.text, t.at, t.size, t.rotation, t.anchor, bottom) {
        let pts: Vec<Pos2> = st.iter().map(|q| xf.world(*q)).collect();
        paint::round_joints(p, &pts, stroke);
        p.add(PathShape::line(pts, stroke));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layers_hide_every_inner_layer() {
        let layers = default_layers();
        for l in ["In1.Cu", "In2.Cu", "In3.Cu", "In6.Cu", "In30.Cu", "B.Cu"] {
            assert!(!layers.shows(l), "{l}");
        }
        assert!(layers.shows("F.Cu"));
    }

    #[test]
    fn inner_layers_past_in2_get_inner_colors() {
        assert_eq!(copper_color("In3.Cu"), IN1);
        assert_eq!(copper_color("In6.Cu"), IN2);
        assert_eq!(copper_color("B.Cu"), paint::B_CU);
    }
}
