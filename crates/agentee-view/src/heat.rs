use crate::canvas::cursor_readout;
use crate::pages::PageState;
use crate::paint::{self, Xf};
use agentee_core::layout::Layout;
use agentee_core::project::Project;
use agentee_core::sim::{LayerMap, MapResult};
use egui::epaint::PathShape;
use egui::{Align2, Color32, ColorImage, Pos2, Rect, Stroke, TextureOptions, Ui, Vec2};
use egui_bench::prelude::*;

const STOPS: [(f32, [u8; 3]); 6] = [
    (0.0, [0x10, 0x0c, 0x2c]),
    (0.2, [0x42, 0x0a, 0x68]),
    (0.4, [0x93, 0x26, 0x67]),
    (0.6, [0xdd, 0x51, 0x3a]),
    (0.8, [0xfc, 0xa5, 0x0a]),
    (1.0, [0xfc, 0xff, 0xa4]),
];

pub fn colour(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    for w in STOPS.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if t <= b {
            let f = (t - a) / (b - a);
            let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f) as u8;
            return Color32::from_rgb(mix(ca[0], cb[0]), mix(ca[1], cb[1]), mix(ca[2], cb[2]));
        }
    }
    let c = STOPS[5].1;
    Color32::from_rgb(c[0], c[1], c[2])
}

fn image(m: &LayerMap, values: &[f32]) -> ColorImage {
    let span = (m.max - m.min).max(1e-30);
    let pixels = values
        .iter()
        .map(|v| {
            if v.is_finite() {
                colour((v - m.min) / span).gamma_multiply(0.92)
            } else {
                Color32::TRANSPARENT
            }
        })
        .collect();
    ColorImage::new([m.width, m.height], pixels)
}

fn fmt(v: f32, unit: &str) -> String {
    let a = v.abs();
    let s = if unit.starts_with("dB") || a >= 100.0 {
        format!("{v:.1}")
    } else if a >= 1.0 {
        format!("{v:.3}")
    } else {
        format!("{v:.4}")
    };
    format!("{s} {unit}")
}

pub fn canvas(
    ui: &mut Ui,
    project: &Project,
    s: &agentee_core::sim::Sim,
    index: usize,
    maps: &[LayerMap],
    st: &mut PageState,
) {
    if maps.is_empty() {
        return;
    }
    let layout = project.layouts.iter().find(|l| l.name == s.layout).map(|l| &l.item);
    let unique = |f: &dyn Fn(&LayerMap) -> String| {
        let mut v: Vec<String> = Vec::new();
        for m in maps {
            let x = f(m);
            if !v.contains(&x) {
                v.push(x);
            }
        }
        v
    };
    let quantities = unique(&|m| m.quantity.clone());
    let layers = unique(&|m| m.layer.clone());
    let current = maps.get(st.map_index).cloned().unwrap_or_else(|| maps[0].clone());
    ui.horizontal(|ui| {
        for q in &quantities {
            if toggle(ui, q, *q == current.quantity).clicked()
                && let Some(k) =
                    maps.iter().position(|m| &m.quantity == q && m.layer == current.layer)
            {
                st.map_index = k;
            }
        }
        ui.add_space(12.0);
        for l in &layers {
            if toggle(ui, l, *l == current.layer).clicked()
                && let Some(k) =
                    maps.iter().position(|m| &m.layer == l && m.quantity == current.quantity)
            {
                st.map_index = k;
            }
        }
    });
    ui.add_space(6.0);
    let key = (project.generation, index, st.map_index);
    if st.map_key != Some(key) {
        let values = current.decode();
        st.map_tex = Some(ui.ctx().load_texture(
            format!("map-{index}-{}", st.map_index),
            image(&current, &values),
            TextureOptions::LINEAR,
        ));
        st.map_values = values;
        st.map_key = Some(key);
    }
    let mut b = agentee_core::graphic::Bounds::EMPTY;
    b.add(current.origin);
    b.add([
        current.origin[0] + (current.width - 1) as f64 * current.cell,
        current.origin[1] + (current.height - 1) as f64 * current.cell,
    ]);
    st.view.max_fit = 2000.0;
    let (resp, xf) = st.view.show(ui, &st.region.unwrap_or(b), 30.0);
    let p = ui.painter_at(xf.rect);
    if let Some(tex) = &st.map_tex {
        let half = current.cell / 2.0;
        let min = xf.world([current.origin[0] - half, current.origin[1] - half]);
        let max = xf.world([
            current.origin[0] + (current.width as f64 - 0.5) * current.cell,
            current.origin[1] + (current.height as f64 - 0.5) * current.cell,
        ]);
        p.image(
            tex.id(),
            Rect::from_min_max(min, max),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    if let Some(l) = layout {
        overlay(&p, &xf, l, &current.layer);
    }
    bar(&p, &xf, &current);
    let hover = if st.interactive { resp.hover_pos() } else { None };
    cursor_readout(ui, &xf, hover);
    if let Some(h) = hover {
        let [x, y] = xf.mm(h);
        let i = ((x - current.origin[0]) / current.cell).round();
        let j = ((y - current.origin[1]) / current.cell).round();
        if i >= 0.0 && j >= 0.0 && (i as usize) < current.width && (j as usize) < current.height {
            let v = st
                .map_values
                .get(j as usize * current.width + i as usize)
                .copied()
                .unwrap_or(f32::NAN);
            if v.is_finite() {
                resp.on_hover_text(format!("{} {}", current.quantity, fmt(v, &current.unit)));
            }
        }
    }
}

fn overlay(p: &egui::Painter, xf: &Xf, l: &Layout, layer: &str) {
    let edge = Stroke::new(1.2, paint::EDGE.gamma_multiply(0.8));
    let pts: Vec<Pos2> = l.outline.iter().map(|q| xf.world(*q)).collect();
    if pts.len() >= 3 {
        p.add(PathShape::closed_line(pts, edge));
    }
    let thin = Stroke::new(1.0, Color32::from_white_alpha(70));
    for part in &l.parts {
        for pad in part.pads.iter().filter(|q| q.copper.iter().any(|c| c == layer)) {
            for o in &pad.outlines {
                p.add(PathShape::closed_line(o.iter().map(|q| xf.world(*q)).collect(), thin));
            }
        }
    }
    for t in l.tracks.iter().filter(|t| t.layer == layer) {
        let s: Vec<Pos2> = t.points.iter().map(|q| xf.world(*q)).collect();
        p.add(PathShape::line(s, Stroke::new(1.0, Color32::from_white_alpha(40))));
    }
}

fn bar(p: &egui::Painter, xf: &Xf, m: &LayerMap) {
    let r = xf.rect;
    let w = 16.0;
    let h = (r.height() * 0.5).min(260.0);
    let area =
        Rect::from_min_size(Pos2::new(r.right() - w - 70.0, r.top() + 16.0), Vec2::new(w, h));
    let n = 48;
    for k in 0..n {
        let t = k as f32 / (n - 1) as f32;
        let y0 = area.bottom() - (k as f32 + 1.0) / n as f32 * h;
        p.rect_filled(
            Rect::from_min_size(Pos2::new(area.left(), y0), Vec2::new(w, h / n as f32 + 0.5)),
            0.0,
            colour(t),
        );
    }
    p.rect_stroke(area, 0.0, Stroke::new(1.0, ETCH), egui::StrokeKind::Outside);
    let label = |v: f32, y: f32| {
        p.text(
            Pos2::new(area.right() + 6.0, y),
            Align2::LEFT_CENTER,
            fmt(v, &m.unit),
            figure(10.5),
            VALUE,
        );
    };
    label(m.max, area.top());
    label(0.5 * (m.min + m.max), area.center().y);
    label(m.min, area.bottom());
    p.text(
        Pos2::new(area.left(), area.bottom() + 8.0),
        Align2::LEFT_TOP,
        m.quantity.to_uppercase(),
        legend_font(10.0),
        LEGEND,
    );
}

pub fn props(ui: &mut Ui, s: &agentee_core::sim::Sim, r: &MapResult, rail: Color32) {
    card(
        ui,
        Some(rail),
        |ui| {
            Line::new().legend(&format!("{:?}", r.kind).to_lowercase()).value(&s.name).elided(ui);
        },
        |ui| {
            readouts(
                ui,
                &[
                    ("nodes", format!("{:.2} M", r.cells as f64 / 1e6), VALUE),
                    ("cell", format!("{} mm", agentee_core::units::trim(s.cell, 3)), READOUT),
                    ("time", format!("{:.2} s", r.seconds), TRACE),
                ],
            );
            if !s.description.is_empty() {
                note(ui, &s.description, VALUE);
            }
            reading(ui, "layout", s.layout.clone());
            reading(ui, "solver", r.device.clone());
        },
    );
    ui.add_space(8.0);
    readings(ui, &r.readings);
}

pub fn readings(ui: &mut Ui, list: &[agentee_core::sim::Reading]) {
    Line::new().legend("readings").show(ui);
    let cols = [("what", 150.0), ("value", 90.0), ("detail", 150.0)];
    Table::new(&cols, list.len()).show(ui, |i, p, row, at| {
        let x = &list[i];
        cell(p, row, at(0), cols[0].1, &x.label, VALUE);
        cell(p, row, at(1), cols[1].1, &fmt(x.value as f32, &x.unit), TRACE);
        cell(p, row, at(2), cols[2].1, &x.detail, LEGEND);
    });
}
