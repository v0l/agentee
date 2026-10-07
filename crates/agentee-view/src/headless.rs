use crate::focus::Context;
use crate::pages::{PageState, page};
use crate::raster::{Canvas, Textures, encode_png};
use agentee_core::graphic::Bounds;
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
    pub focus: Vec<String>,
    pub context: Context,
    pub rulers: bool,
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
            focus: Vec::new(),
            context: Context::Dim,
            rulers: false,
        }
    }
}

pub fn render_png(
    project: &Project,
    item: ItemRef,
    opts: &RenderOptions,
) -> Result<Vec<u8>, String> {
    let (w, h, rgba) = render_rgba(project, item, opts)?;
    Ok(encode_png(w, h, &rgba))
}

fn focus(
    project: &Project,
    item: ItemRef,
    opts: &RenderOptions,
) -> Result<Option<(crate::focus::Focus, Bounds)>, String> {
    if opts.focus.is_empty() {
        return Ok(None);
    }
    match item {
        ItemRef::Schematic(i) => {
            crate::focus::schematic(&project.schematics[i].item, &opts.focus, opts.context)
        }
        ItemRef::Layout(i) => {
            crate::focus::layout(&project.layouts[i].item, &opts.focus, opts.context)
        }
        _ => Err("focus works on schematics and layouts".into()),
    }
    .map(Some)
}

fn cropped(project: &Project, item: ItemRef, st: &PageState, size: Vec2) -> Option<Vec2> {
    if st.panels || st.view_3d {
        return None;
    }
    let zoomed = st.region.is_some();
    let (bounds, margin, max_fit) = match item {
        ItemRef::Symbol(i) => (
            crate::pages::symbol_bounds(&project.symbols[i].item, st.unit),
            40.0,
            if zoomed { 4000.0 } else { 45.0 },
        ),
        ItemRef::Footprint(i) => (project.footprints[i].item.bounds(), 40.0, 2000.0),
        ItemRef::Schematic(i) => {
            (project.schematics[i].item.bounds(), 50.0, if zoomed { 4000.0 } else { 45.0 })
        }
        ItemRef::Layout(i) => (project.layouts[i].item.bounds(), 30.0, 2000.0),
        _ => return None,
    };
    let b = st.region.unwrap_or(bounds);
    if b.is_empty() {
        return None;
    }
    let [w, h] = b.size().map(|v| v.max(0.5) as f32);
    let avail = size - Vec2::splat(margin * 2.0);
    let scale = (avail.x / w).min(avail.y / h).clamp(0.5, max_fit);
    let fit = Vec2::new(w, h) * scale + Vec2::splat(margin * 2.0);
    Some(fit.min(size).max(Vec2::splat(64.0)))
}

pub fn render_rgba(
    project: &Project,
    item: ItemRef,
    opts: &RenderOptions,
) -> Result<(usize, usize, Vec<u8>), String> {
    let focused = focus(project, item, opts)?;
    let ctx = egui::Context::default();
    egui_bench::install(&ctx);
    let ppp = opts.scale.max(0.25);
    let mut st = PageState {
        interactive: false,
        panels: opts.panels,
        unit: opts.unit.max(1),
        show_hidden: opts.hidden_pins,
        rulers: opts.rulers,
        ..Default::default()
    };
    st.select(item);
    st.unit = opts.unit.max(1);
    if opts.show.iter().any(|x| x == "back") {
        st.view.flip = true;
        st.pcb_layers.mirror();
    }
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
        let mut b = Bounds::EMPTY;
        b.add([x0, y0]);
        b.add([x1, y1]);
        b
    });
    if let Some((f, b)) = focused {
        st.region = st.region.or(Some(b));
        st.focus = Some(f);
    }
    let full = Vec2::new(opts.width as f32 / ppp, opts.height as f32 / ppp);
    let size = cropped(project, item, &st, full).unwrap_or(full);
    let (pw, ph) = ((size.x * ppp).round() as usize, (size.y * ppp).round() as usize);
    let size = Vec2::new(pw as f32, ph as f32) / ppp;
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
    let mut canvas = Canvas::new(pw, ph);
    let bg = CHASSIS.to_array().map(|c| c as f32 / 255.0);
    canvas.rgba.fill(bg);
    canvas.draw(&prims, &textures, out.pixels_per_point);
    let mut rgba = canvas.to_rgba8();
    let ph = match st.content_bottom.filter(|_| !st.panels) {
        Some(bottom) => (((bottom + 2.0) * ppp).ceil() as usize).clamp(64, ph),
        None => ph,
    };
    rgba.truncate(pw * ph * 4);
    Ok((pw, ph, rgba))
}
