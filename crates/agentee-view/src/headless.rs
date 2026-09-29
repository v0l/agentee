use crate::pages::{PageState, page};
use crate::raster::{Canvas, Textures, encode_png};
use agentee_core::project::{ItemRef, Project};
use egui::{Pos2, RawInput, Rect, Vec2, ViewportId};
use egui_bench::theme::CHASSIS;

#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub unit: u32,
    pub panels: bool,
    pub hidden_pins: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            width: 1400,
            height: 900,
            scale: 1.0,
            unit: 1,
            panels: true,
            hidden_pins: false,
        }
    }
}

pub fn render_png(project: &Project, item: ItemRef, opts: &RenderOptions) -> Vec<u8> {
    let (w, h, rgba) = render_rgba(project, item, opts);
    encode_png(w, h, &rgba)
}

pub fn render_rgba(
    project: &Project,
    item: ItemRef,
    opts: &RenderOptions,
) -> (usize, usize, Vec<u8>) {
    let ctx = egui::Context::default();
    egui_bench::install(&ctx);
    let ppp = opts.scale.max(0.25);
    let size = Vec2::new(opts.width as f32 / ppp, opts.height as f32 / ppp);
    let mut st = PageState {
        interactive: false,
        panels: opts.panels,
        unit: opts.unit.max(1),
        show_hidden: opts.hidden_pins,
        ..Default::default()
    };
    st.select(item);
    st.unit = opts.unit.max(1);
    let mut textures = Textures::default();
    let mut last = None;
    for _ in 0..3 {
        let mut raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        };
        raw.viewports.entry(ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
        let out = ctx.run_ui(raw, |ui| {
            egui::Frame::NONE.fill(CHASSIS).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                page(ui, project, item, &mut st);
            });
        });
        textures.apply(&out.textures_delta);
        last = Some(out);
    }
    let out = last.unwrap();
    let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
    let (pw, ph) = (opts.width as usize, opts.height as usize);
    let mut canvas = Canvas::new(pw, ph);
    let bg = CHASSIS.to_array().map(|c| c as f32 / 255.0);
    canvas.rgba.fill(bg);
    canvas.draw(&prims, &textures, out.pixels_per_point);
    (pw, ph, canvas.to_rgba8())
}
