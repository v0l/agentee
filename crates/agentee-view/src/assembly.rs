use crate::focus::{Context, Focus};
use agentee_core::layout::Layout;
use agentee_core::project::Project;
use agentee_parts::assembly::{Group, Side};
use egui::{Key, Modifiers, Ui};
use egui_bench::prelude::*;

pub struct Guide {
    key: (u64, usize),
    pub groups: Vec<Group>,
    pub step: usize,
    pub done: Vec<bool>,
}

impl Guide {
    pub fn new(project: &Project, i: usize) -> Guide {
        let groups = groups(project, i);
        Guide { key: (project.generation, i), done: vec![false; groups.len()], groups, step: 0 }
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

    fn tick(&mut self) {
        if let Some(d) = self.done.get_mut(self.step) {
            *d = !*d;
        }
        if let Some(next) = (self.step + 1..self.groups.len()).find(|&k| !self.done[k]) {
            self.step = next;
        }
    }

    pub fn keys(&mut self, ui: &Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let key = |k: Key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        if key(Key::ArrowRight) || key(Key::ArrowDown) {
            self.step = (self.step + 1).min(self.groups.len().saturating_sub(1));
        }
        if key(Key::ArrowLeft) || key(Key::ArrowUp) {
            self.step = self.step.saturating_sub(1);
        }
        if key(Key::Space) {
            self.tick();
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

pub fn panel(ui: &mut Ui, guide: &mut Guide) {
    let done = guide.done.iter().filter(|d| **d).count();
    let total = guide.groups.len();
    card(
        ui,
        Some(if done == total { OK } else { READOUT }),
        |ui| {
            Line::new().legend("assembly").value(format!("{done}/{total} placed")).show(ui);
        },
        |ui| {
            if let Some(g) = guide.current() {
                reading(ui, "side", g.side.name().to_string());
                reading(ui, "part", format!("{} {}", g.value, g.package));
                if let Some(b) = &g.bin {
                    reading(ui, "bin", b.clone());
                }
                reading(ui, "refs", format!("{} ({} pcs)", g.refs.join(" "), g.refs.len()));
            }
            ui.add_space(4.0);
            ui.label(legend("space ticks the group, arrows step"));
            ui.add_space(4.0);
            let mut pick = None;
            let mut flip = None;
            for (k, g) in guide.groups.iter().enumerate() {
                ui.horizontal(|ui| {
                    if toggle(ui, "", guide.done[k]).clicked() {
                        flip = Some(k);
                    }
                    let text = format!("{:>2}. {} {}", g.step, g.side.name(), g.line());
                    if ui.selectable_label(k == guide.step, text).clicked() {
                        pick = Some(k);
                    }
                });
            }
            if let Some(k) = flip {
                guide.done[k] = !guide.done[k];
            }
            if let Some(k) = pick {
                guide.step = k;
            }
        },
    );
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

    #[test]
    fn ticking_moves_to_the_next_open_group_and_follows_its_side() {
        let mut g = Guide {
            key: (0, 0),
            groups: vec![
                group(1, Side::Top, &["R1"]),
                group(2, Side::Top, &["R2"]),
                group(3, Side::Bottom, &["R3"]),
            ],
            step: 0,
            done: vec![false, true, false],
        };
        assert!(!g.bottom());
        g.tick();
        assert_eq!((g.step, g.done.clone()), (2, vec![true, true, false]));
        assert!(g.bottom());
        g.tick();
        assert_eq!((g.step, g.done.clone()), (2, vec![true, true, true]));
    }
}
