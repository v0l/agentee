use crate::pages::PageState;
use agentee_core::sim::{ChannelResult, Sim};
use egui::{Align2, Color32, ColorImage, Pos2, Rect, Sense, Stroke, TextureOptions, Ui, Vec2};
use egui_bench::prelude::*;

fn image(c: &ChannelResult) -> ColorImage {
    let e = &c.eye;
    let top = e.counts.iter().copied().max().unwrap_or(1).max(1) as f32;
    let pixels = e
        .counts
        .iter()
        .map(|n| {
            if *n == 0 {
                Color32::TRANSPARENT
            } else {
                crate::heat::colour(0.25 + 0.75 * (*n as f32).ln_1p() / top.ln_1p())
            }
        })
        .collect();
    ColorImage::new([e.phases, e.bins], pixels)
}

pub fn canvas(ui: &mut Ui, c: &ChannelResult, index: usize, st: &mut PageState, generation: u64) {
    let key = (generation, index, usize::MAX);
    if st.map_key != Some(key) {
        st.map_tex =
            Some(ui.ctx().load_texture(format!("eye-{index}"), image(c), TextureOptions::LINEAR));
        st.map_key = Some(key);
    }
    let h = ui.available_height();
    section(ui, "eye", &format!("{:.3} Gbps, two unit intervals", c.bit_rate / 1e9), |ui| {
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), (h * 0.58).max(240.0)),
            Sense::hover(),
        );
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, WELL);
        let area =
            Rect::from_min_max(rect.min + Vec2::new(60.0, 12.0), rect.max - Vec2::new(12.0, 28.0));
        if let Some(tex) = &st.map_tex {
            p.image(
                tex.id(),
                area,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let e = &c.eye;
        let y = |mv: f64| {
            area.top() + ((e.v_max_mv - mv) / (e.v_max_mv - e.v_min_mv)) as f32 * area.height()
        };
        let grid = Stroke::new(1.0, ETCH);
        let span = e.v_max_mv - e.v_min_mv;
        let step = [10.0, 20.0, 50.0, 100.0, 200.0, 500.0]
            .into_iter()
            .find(|s| span / s <= 10.0)
            .unwrap_or(1000.0);
        let mut v = (e.v_min_mv / step).ceil() * step;
        while v <= e.v_max_mv {
            p.line_segment([Pos2::new(area.left(), y(v)), Pos2::new(area.right(), y(v))], grid);
            p.text(
                Pos2::new(area.left() - 6.0, y(v)),
                Align2::RIGHT_CENTER,
                format!("{v:.0} mV"),
                theme::figure(10.5),
                LEGEND,
            );
            v += step;
        }
        for k in 0..=4 {
            let x = area.left() + area.width() * k as f32 / 4.0;
            p.line_segment([Pos2::new(x, area.top()), Pos2::new(x, area.bottom())], grid);
            let t = (k as f64 / 2.0 - 1.0) * c.ui_ps;
            p.text(
                Pos2::new(x, area.bottom() + 6.0),
                Align2::CENTER_TOP,
                format!("{t:.0} ps"),
                theme::figure(10.5),
                LEGEND,
            );
        }
        if let Some(hp) = resp.hover_pos()
            && area.contains(hp)
        {
            let mv = e.v_max_mv - ((hp.y - area.top()) / area.height()) as f64 * span;
            let t = ((hp.x - area.left()) / area.width()) as f64 * 2.0 * c.ui_ps - c.ui_ps;
            resp.on_hover_text(format!("{t:.1} ps, {mv:.1} mV"));
        }
    });
    ui.add_space(8.0);
    let size = Vec2::new(ui.available_width(), (ui.available_height() - 40.0).max(160.0));
    section(ui, "pulse response", "one bit at the full swing, time from the main cursor", |ui| {
        let series = vec![("pulse".to_string(), c.pulse_time_ps.clone(), c.pulse_mv.clone())];
        crate::plot::xy_plot(ui, &series, "ps", "mV", size, None);
    });
}

pub fn props(ui: &mut Ui, s: &Sim, c: &ChannelResult, rail: Color32) {
    card(
        ui,
        Some(rail),
        |ui| {
            Line::new().legend("channel").value(&s.name).elided(ui);
        },
        |ui| {
            readouts(
                ui,
                &[
                    ("rate", format!("{:.3} Gbps", c.bit_rate / 1e9), VALUE),
                    ("ui", format!("{:.1} ps", c.ui_ps), READOUT),
                    ("edge", format!("{:.0} ps", c.rise_ps), TRACE),
                ],
            );
            if !s.description.is_empty() {
                note(ui, &s.description, VALUE);
            }
            reading(ui, "board", s.board.clone());
            if let Some(cs) = &s.channel_spec {
                reading(ui, "path", cs.through.join(if cs.differential { ", " } else { " to " }));
            }
        },
    );
    ui.add_space(8.0);
    crate::heat::readings(ui, &c.readings);
}
