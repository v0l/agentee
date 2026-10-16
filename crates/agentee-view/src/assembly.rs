use crate::canvas::View;
use crate::focus::{Context, Focus, placed_bounds};
use crate::paint::{self, Layers, Xf};
use agentee_core::layout::Layout;
use agentee_core::project::Project;
use agentee_parts::assembly::{Group, Side};
use egui::{Key as KeyCode, Modifiers, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use egui_bench::prelude::*;
use egui_bench::theme::{figure, legend_font};

const STEP_W: f32 = 360.0;
const RAIL_H: f32 = 54.0;

pub struct Guide {
    key: (u64, usize),
    pub groups: Vec<Group>,
    pub step: usize,
    pub done: Vec<bool>,
    view: View,
    layers: Layers,
}

impl Guide {
    pub fn new(project: &Project, i: usize) -> Guide {
        let groups = groups(project, i);
        Guide {
            key: (project.generation, i),
            done: vec![false; groups.len()],
            groups,
            step: 0,
            view: View::default(),
            layers: crate::pcb::default_layers(),
        }
    }

    pub fn refresh(&mut self, project: &Project, i: usize) {
        if self.key == (project.generation, i) {
            return;
        }
        let fresh = groups(project, i);
        let was = |g: &Group| self.groups.iter().position(|o| o.refs == g.refs && o.side == g.side);
        self.done = fresh.iter().map(|g| was(g).is_some_and(|k| self.done[k])).collect();
        self.step = self.step.min(fresh.len().saturating_sub(1));
        self.groups = fresh;
        self.key = (project.generation, i);
    }

    pub fn current(&self) -> Option<&Group> {
        self.groups.get(self.step)
    }

    pub fn focus(&self, l: &Layout) -> Focus {
        let mut f = Focus { context: Context::Dim, ..Default::default() };
        if let Some(g) = self.current() {
            f.parts =
                (0..l.parts.len()).filter(|&pi| g.refs.contains(&l.parts[pi].reference)).collect();
        }
        f
    }

    pub fn bottom(&self) -> bool {
        self.current().is_some_and(|g| g.side == Side::Bottom)
    }

    pub fn progress(&self) -> (usize, usize) {
        (self.done.iter().filter(|d| **d).count(), self.groups.len())
    }

    fn finished(&self) -> bool {
        !self.done.is_empty() && self.done.iter().all(|d| *d)
    }

    fn tick(&mut self) {
        if let Some(d) = self.done.get_mut(self.step) {
            *d = !*d;
        }
        if self.done.get(self.step) == Some(&true)
            && let Some(next) = self.next_open(self.step)
        {
            self.step = next;
        }
    }

    fn next_open(&self, from: usize) -> Option<usize> {
        let n = self.groups.len();
        (1..n).map(|k| (from + k) % n).find(|&k| !self.done[k])
    }

    fn upcoming(&self, count: usize) -> Vec<usize> {
        (self.step + 1..self.groups.len()).filter(|&k| !self.done[k]).take(count).collect()
    }

    fn parts(&self) -> (usize, usize) {
        let total = self.groups.iter().map(|g| g.refs.len()).sum();
        let placed = self.groups.iter().zip(&self.done).filter(|(_, d)| **d);
        (placed.map(|(g, _)| g.refs.len()).sum(), total)
    }

    pub fn keys(&mut self, ui: &Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let key = |k: KeyCode| ui.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        if key(KeyCode::ArrowRight) || key(KeyCode::ArrowDown) {
            self.step = (self.step + 1).min(self.groups.len().saturating_sub(1));
        }
        if key(KeyCode::ArrowLeft) || key(KeyCode::ArrowUp) {
            self.step = self.step.saturating_sub(1);
        }
        if key(KeyCode::Space) || key(KeyCode::Enter) {
            self.tick();
        }
        if key(KeyCode::F) {
            self.view.fitted = false;
        }
    }
}

fn groups(project: &Project, i: usize) -> Vec<Group> {
    let l = &project.layouts[i].item;
    let Some(sch) = project.schematics.iter().find(|s| s.name == l.schematic) else {
        return Vec::new();
    };
    let lib = agentee_parts::library::Library::open(Some(&project.root), None);
    agentee_parts::assembly::groups(l, &sch.item, &lib)
}

fn kind(reference: &str) -> &'static str {
    let prefix: String = reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    match prefix.as_str() {
        "R" | "RN" => "resistor",
        "RV" => "trimmer",
        "RT" | "TH" => "thermistor",
        "C" | "CP" => "capacitor",
        "L" => "inductor",
        "FB" => "ferrite bead",
        "D" | "DZ" | "ZD" => "diode",
        "LED" => "LED",
        "Q" | "M" | "T" => "transistor",
        "U" | "IC" => "IC",
        "Y" | "X" => "crystal",
        "J" | "P" | "CN" => "connector",
        "SW" | "S" => "switch",
        "F" => "fuse",
        "K" => "relay",
        "BZ" | "LS" => "buzzer",
        "BT" => "battery holder",
        "TP" => "test point",
        "PS" => "power module",
        _ => "part",
    }
}

fn describe(g: &Group) -> String {
    let what = kind(&g.refs[0]);
    let mut s = format!("{} {what}", g.package);
    if what == "LED" && g.package.is_empty() {
        s = "LED".into();
    }
    s
}

pub fn page(ui: &mut Ui, project: &Project, i: usize, guide: &mut Guide) {
    guide.refresh(project, i);
    guide.keys(ui);
    if guide.bottom() != guide.view.flip {
        guide.view.flip = !guide.view.flip;
        guide.layers.mirror();
    }
    let frame =
        |x: i8, y: i8| egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin::symmetric(x, y));
    egui::Panel::bottom("assembly-rail")
        .resizable(false)
        .exact_size(RAIL_H)
        .frame(frame(16, 8))
        .show(ui, |ui| rail(ui, guide));
    egui::Panel::left("assembly-step")
        .resizable(false)
        .exact_size(STEP_W)
        .frame(frame(22, 18))
        .show(ui, |ui| instruction(ui, guide));
    egui::CentralPanel::no_frame().show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(legend("drag to pan, scroll to zoom, F or double click fits the board"));
        });
        ui.add_space(4.0);
        board(ui, &project.layouts[i].item, guide);
    });
}

fn instruction(ui: &mut Ui, guide: &mut Guide) {
    let total = guide.groups.len();
    if total == 0 {
        ui.label(value("This layout has no placed parts to assemble."));
        return;
    }
    if guide.finished() {
        ui.label(RichText::new("Board done").font(figure(40.0)).color(OK));
        ui.add_space(6.0);
        ui.label(value(format!("All {total} groups are placed.")));
        ui.add_space(18.0);
        if ui.add(Key::new("start again").outline(VALUE)).clicked() {
            guide.done.iter_mut().for_each(|d| *d = false);
            guide.step = 0;
        }
        return;
    }
    let step = guide.step;
    let g = guide.groups[step].clone();
    let done = guide.done[step];
    let tint = if done { OK } else { READOUT };

    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Step {}", g.step)).font(legend_font(15.0)).color(VALUE));
        ui.label(RichText::new(format!("of {total}")).font(legend_font(15.0)).color(LEGEND));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            lamp(ui, &format!("{} side", g.side.name()), true, false);
        });
    });
    ui.add_space(14.0);

    let big = if g.value.chars().count() > 9 { 34.0 } else { 58.0 };
    ui.add(egui::Label::new(RichText::new(&g.value).font(figure(big)).color(tint)).wrap());
    ui.label(RichText::new(describe(&g)).font(legend_font(19.0)).color(VALUE));
    if let Some(m) = g.mpn.as_ref().filter(|m| !m.starts_with(&g.value)) {
        ui.label(RichText::new(m).font(legend_font(13.0)).color(LEGEND));
    }
    ui.add_space(18.0);

    ui.horizontal(|ui| {
        hero(ui, "to place", &g.refs.len().to_string(), "pcs", tint);
        ui.add_space(28.0);
        if let Some(b) = &g.bin {
            hero(ui, "from bin", b, "", VALUE);
        }
    });
    ui.add_space(14.0);

    let refs = g.refs.join("  ");
    ui.add(
        egui::Label::new(RichText::new(refs).font(egui::FontId::monospace(15.0)).color(VALUE))
            .wrap(),
    );
    ui.add_space(10.0);
    if g.bin.is_none() {
        note(ui, "No bin recorded for this part.", LEGEND);
    }
    if g.hot {
        note(ui, "Large thermal pad: hot air or a hot plate.", WARN);
    }
    if g.tht {
        note(ui, "Through-hole: solder from the other side.", TRACE);
    }
    if let Some(own) = g.own {
        let left = own.saturating_sub(g.refs.len() as u64);
        let c = if own < g.refs.len() as u64 { FAULT } else { LEGEND };
        note(ui, format!("{own} on the shelf, {left} after this board."), c);
    }

    ui.add_space(18.0);
    let label = if done { "placed, undo" } else { "placed" };
    let k = Key::new(label).min_width(STEP_W - 44.0);
    let k = if done { k.outline(OK) } else { k.solid(READOUT) };
    if ui.add(k).clicked() {
        guide.tick();
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let half = (STEP_W - 52.0) / 2.0;
        let back = ui.add(Key::new("back").outline(VALUE).min_width(half).enabled(step > 0));
        if back.clicked() {
            guide.step = step - 1;
        }
        let skip = Key::new("skip").outline(VALUE).min_width(half).enabled(step + 1 < total);
        if ui.add(skip).clicked() {
            guide.step = step + 1;
        }
    });
    ui.add_space(6.0);
    ui.label(legend("space marks placed, arrows step"));

    let next = guide.upcoming(5);
    if next.is_empty() {
        return;
    }
    ui.add_space(22.0);
    ui.label(RichText::new("Next").font(legend_font(15.0)).color(LEGEND));
    ui.add_space(4.0);
    for k in next {
        let n = &guide.groups[k];
        let side =
            if n.side != g.side { format!(", {} side", n.side.name()) } else { String::new() };
        let text = format!("{:>2}  {} {}, {} pcs{side}", n.step, n.value, n.package, n.refs.len());
        let r = ui.add(
            egui::Label::new(RichText::new(text).font(egui::FontId::monospace(13.5)).color(LEGEND))
                .truncate()
                .sense(Sense::click()),
        );
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if r.clicked() {
            guide.step = k;
        }
    }
}

fn rail(ui: &mut Ui, guide: &mut Guide) {
    let (placed, total) = guide.parts();
    let groups_done = guide.done.iter().filter(|d| **d).count();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{placed} of {total} parts placed"))
                .font(legend_font(14.0))
                .color(VALUE),
        );
        ui.label(
            RichText::new(format!("{groups_done} of {} groups", guide.groups.len()))
                .font(legend_font(14.0))
                .color(LEGEND),
        );
    });
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 14.0), Sense::click());
    if total == 0 {
        return;
    }
    let sides: Vec<Side> = guide.groups.iter().map(|g| g.side).fold(Vec::new(), |mut v, s| {
        if v.last() != Some(&s) {
            v.push(s);
        }
        v
    });
    let gap = 10.0;
    let seams = sides.len().saturating_sub(1) as f32 * gap + guide.groups.len() as f32;
    let per = (rect.width() - seams).max(0.0) / total as f32;
    let p = ui.painter_at(rect);
    let mut x = rect.left();
    let mut hovered = None;
    for (k, g) in guide.groups.iter().enumerate() {
        if k > 0 && guide.groups[k - 1].side != g.side {
            x += gap;
        }
        let w = per * g.refs.len() as f32;
        let r = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(w.max(1.0), rect.height()));
        let fill = if k == guide.step {
            READOUT
        } else if guide.done[k] {
            OK.gamma_multiply(0.75)
        } else {
            BAND
        };
        p.rect_filled(r, 1.0, fill);
        if resp.hover_pos().is_some_and(|h| h.x >= r.left() && h.x < r.right() + 1.0) {
            hovered = Some(k);
        }
        x += w + 1.0;
    }
    if let Some(k) = hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            guide.step = k;
        }
        let g = &guide.groups[k];
        resp.on_hover_text(format!("{}. {} side, {}", g.step, g.side.name(), g.line()));
    }
}

fn board(ui: &mut Ui, l: &Layout, guide: &mut Guide) {
    guide.view.max_fit = 2000.0;
    let (resp, xf) = guide.view.show_with(ui, &l.bounds(), 36.0, false);
    let p = ui.painter_at(xf.rect);
    paint::grid(&p, &xf, 1.0);
    let focus = guide.focus(l);
    crate::pcb::layout(&p, &xf, l, &guide.layers, resp.hover_pos(), false, Some(&focus));
    markers(&p, &xf, l, guide);
}

fn markers(p: &egui::Painter, xf: &Xf, l: &Layout, guide: &Guide) {
    let index = |r: &str| l.parts.iter().position(|q| q.reference == r);
    for (g, _) in guide.groups.iter().zip(&guide.done).filter(|(_, d)| **d) {
        for pi in g.refs.iter().filter_map(|r| index(r)) {
            let b = placed_bounds(&l.parts[pi]);
            if b.is_empty() {
                continue;
            }
            p.circle_filled(xf.world(b.center()), 3.0, OK);
        }
    }
    let Some(g) = guide.current() else { return };
    let tint = if guide.done[guide.step] { OK } else { READOUT };
    for pi in g.refs.iter().filter_map(|r| index(r)) {
        let b = placed_bounds(&l.parts[pi]);
        if b.is_empty() {
            continue;
        }
        let [w, h] = b.size();
        let c = xf.world(b.center());
        let r = (xf.len(w.hypot(h) / 2.0) + 4.0).max(10.0);
        p.circle_stroke(c, r, Stroke::new(2.0, tint));
        for d in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
            p.line_segment([c + d * (r + 2.0), c + d * (r + 8.0)], Stroke::new(2.0, tint));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(step: usize, side: Side, refs: &[&str]) -> Group {
        Group {
            step,
            side,
            value: "1k".into(),
            package: "0402".into(),
            footprint: "R_0402_1005Metric".into(),
            mpn: None,
            refs: refs.iter().map(|r| r.to_string()).collect(),
            tht: false,
            hot: false,
            height: None,
            library: None,
            bin: None,
            own: None,
        }
    }

    fn guide(done: Vec<bool>) -> Guide {
        Guide {
            key: (0, 0),
            groups: vec![
                group(1, Side::Top, &["R1"]),
                group(2, Side::Top, &["R2", "R4"]),
                group(3, Side::Bottom, &["R3"]),
            ],
            step: 0,
            done,
            view: View::default(),
            layers: crate::pcb::default_layers(),
        }
    }

    #[test]
    fn placing_moves_to_the_next_open_group_and_follows_its_side() {
        let mut g = guide(vec![false, true, false]);
        assert!(!g.bottom());
        g.tick();
        assert_eq!((g.step, g.done.clone()), (2, vec![true, true, false]));
        assert!(g.bottom());
        assert_eq!(g.parts(), (3, 4));
        g.tick();
        assert!(g.finished());
        g.tick();
        assert_eq!((g.step, g.done.clone()), (2, vec![true, true, false]));
    }

    #[test]
    fn a_group_reads_as_package_and_kind() {
        assert_eq!(describe(&group(1, Side::Top, &["R7"])), "0402 resistor");
        assert_eq!(kind("LED3"), "LED");
        assert_eq!(kind("U12"), "IC");
    }
}
