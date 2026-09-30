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
    pub show: Vec<String>,
    pub hide: Vec<String>,
    pub region: Option<[f64; 4]>,
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
            show: Vec::new(),
            hide: Vec::new(),
            region: None,
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
    for layers in [&mut st.layers, &mut st.pcb_layers] {
        layers.hidden.retain(|h| !opts.show.contains(h));
        layers.hidden.extend(opts.hide.iter().cloned());
    }
    st.view_3d = opts.show.iter().any(|x| x == "3d" || x == "3d-top" || x == "3d-bottom");
    st.show_parts = !opts.hide.iter().any(|x| x == "parts");
    if let Some([x0, y0, x1, y1]) = opts.region {
        let c = [((x0 + x1) / 2.0) as f32, -((y0 + y1) / 2.0) as f32, 0.0];
        st.camera.focus = Some((c, ((x1 - x0).abs().max((y1 - y0).abs())) as f32));
    }
    if opts.show.iter().any(|x| x == "3d-top") {
        st.camera.pitch = 1.5;
        st.camera.yaw = 0.0;
    }
    if opts.show.iter().any(|x| x == "3d-bottom") {
        st.camera.pitch = -1.5;
        st.camera.yaw = 0.0;
    }
    if let ItemRef::Sim(i) = item {
        let s = &project.sims[i];
        st.sim_progress = agentee_core::sim::SimProgress::load(&s.path);
        st.show_fields = opts.show.iter().any(|x| x == "fields");
        st.wave.read(&s.name, &opts.show);
        st.show_tdr = opts.show.iter().any(|x| x == "tdr");
        let maps = s.item.maps.as_ref().map(|m| &m.maps[..]).or(s
            .item
            .result
            .as_ref()
            .map(|r| &r.maps[..]));
        if let Some(maps) = maps {
            let hits = |m: &agentee_core::sim::LayerMap| {
                opts.show.iter().filter(|x| **x == m.quantity || **x == m.layer).count()
            };
            if let Some((k, _)) = maps
                .iter()
                .enumerate()
                .filter(|(_, m)| hits(m) > 0)
                .max_by_key(|(k, m)| (hits(m), usize::MAX - k))
            {
                st.map_index = k;
            }
        }
    }
    st.region = opts.region.map(|[x0, y0, x1, y1]| {
        let mut b = agentee_core::graphic::Bounds::EMPTY;
        b.add([x0, y0]);
        b.add([x1, y1]);
        b
    });
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
