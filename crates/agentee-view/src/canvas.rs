use crate::paint::Xf;
use agentee_core::graphic::Bounds;
use egui::{Pos2, Rect, Response, Sense, Ui, Vec2};
use egui_bench::theme::{ETCH, WELL};

#[derive(Clone, Copy, Debug)]
pub struct View {
    pub center: [f64; 2],
    pub scale: f32,
    pub fitted: bool,
    pub max_fit: f32,
}

impl Default for View {
    fn default() -> Self {
        View { center: [0.0, 0.0], scale: 20.0, fitted: false, max_fit: 2000.0 }
    }
}

impl View {
    pub fn fit(&mut self, rect: Rect, b: &Bounds, margin: f32) {
        if b.is_empty() {
            *self = View { fitted: true, max_fit: self.max_fit, ..Default::default() };
            return;
        }
        let [w, h] = b.size();
        let avail = rect.size() - Vec2::splat(margin * 2.0);
        let sx = avail.x / (w.max(0.5) as f32);
        let sy = avail.y / (h.max(0.5) as f32);
        self.scale = sx.min(sy).clamp(0.5, self.max_fit);
        self.center = [(b.min[0] + b.max[0]) / 2.0, (b.min[1] + b.max[1]) / 2.0];
        self.fitted = true;
    }

    pub fn xf(&self, rect: Rect) -> Xf {
        Xf {
            rect,
            center: self.center,
            scale: self.scale,
            max_stroke: f32::MAX,
            local: agentee_core::geom::Transform::IDENTITY,
        }
    }

    pub fn show(&mut self, ui: &mut Ui, bounds: &Bounds, margin: f32) -> (Response, Xf) {
        self.show_with(ui, bounds, margin, false)
    }

    pub fn pan(&mut self, d: Vec2) {
        self.center[0] -= (d.x / self.scale) as f64;
        self.center[1] -= (d.y / self.scale) as f64;
    }

    pub fn show_with(
        &mut self,
        ui: &mut Ui,
        bounds: &Bounds,
        margin: f32,
        editing: bool,
    ) -> (Response, Xf) {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        ui.painter().rect_filled(rect, 0.0, WELL);
        ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, ETCH), egui::StrokeKind::Inside);
        if !self.fitted || (!editing && resp.double_clicked()) {
            self.fit(rect, bounds, margin);
        }
        let panning = if editing {
            resp.dragged_by(egui::PointerButton::Middle)
                || resp.dragged_by(egui::PointerButton::Secondary)
        } else {
            resp.dragged()
        };
        if panning {
            self.pan(resp.drag_delta());
        }
        if let Some(hover) = resp.hover_pos() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = zoom * (scroll * 0.0025).exp();
            if (factor - 1.0).abs() > 1e-4 {
                let before = self.xf(rect).mm(hover);
                self.scale = (self.scale * factor).clamp(0.5, 4000.0);
                let after = self.xf(rect).mm(hover);
                self.center[0] += before[0] - after[0];
                self.center[1] += before[1] - after[1];
            }
        }
        (resp, self.xf(rect))
    }
}

pub fn cursor_readout(ui: &Ui, xf: &Xf, hover: Option<Pos2>) {
    let Some(h) = hover else { return };
    let [x, y] = xf.mm(h);
    let s = format!("{x:8.3}  {y:8.3} mm");
    let p = ui.painter();
    let at = xf.rect.left_bottom() + Vec2::new(8.0, -6.0);
    p.text(
        at,
        egui::Align2::LEFT_BOTTOM,
        s,
        egui_bench::theme::figure(11.0),
        egui_bench::theme::LEGEND,
    );
}
