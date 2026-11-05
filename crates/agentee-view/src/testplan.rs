use agentee_core::project::Project;
use agentee_core::testplan::{self, Expect, Plan, Row};
use agentee_core::units::trim;
use egui::{Pos2, Rect, RichText, Sense, Ui, Vec2};
use egui_bench::prelude::*;
use egui_bench::theme::{figure, legend_font};

#[derive(Default)]
pub struct State {
    generation: Option<u64>,
    plan: Option<Plan>,
    pub selected: Option<String>,
}

pub fn plan_for(project: &Project) -> Option<Plan> {
    let layout = project.layouts.first().map(|l| &l.item);
    let sch = match layout {
        Some(l) => project.schematics.iter().find(|s| s.name == l.schematic),
        None => None,
    }
    .or_else(|| project.schematics.iter().find(|s| s.item.parent.is_none()))
    .map(|s| &s.item)?;
    let board = layout.and_then(|l| project.boards.iter().find(|b| b.name == l.board));
    Some(testplan::build(sch, layout, board.map(|b| &b.item)))
}

pub fn bring_up_order(plan: &Plan) -> Vec<&Row> {
    let mut rows: Vec<&Row> = plan.rows.iter().collect();
    rows.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| agentee_core::footprint::natural_cmp(&a.reference, &b.reference))
    });
    rows
}

pub fn expected(row: &Row) -> String {
    match &row.expect {
        Expect::Unknown(u) if u == agentee_core::testplan::FIRMWARE => u.clone(),
        Expect::Unknown(_) => "unknown".to_string(),
        Expect::Logic(l) => {
            let t = l.text();
            if t.is_empty() { "logic level".to_string() } else { t }
        }
        Expect::Reading(r) => format!("{} to {} {}", trim(r.min, 3), trim(r.max, 3), r.unit),
    }
}

impl State {
    pub fn load(&mut self, project: &Project) {
        if self.generation == Some(project.generation) {
            return;
        }
        self.generation = Some(project.generation);
        self.plan = plan_for(project);
        if self.row().is_none() {
            self.selected = None;
        }
    }

    fn row(&self) -> Option<&Row> {
        let name = self.selected.as_ref()?;
        self.plan.as_ref()?.rows.iter().find(|r| &r.reference == name)
    }

    fn current(&self) -> Option<&Row> {
        self.row().or_else(|| self.plan.as_ref().and_then(|p| bring_up_order(p).first().copied()))
    }
}

fn tint(row: &Row) -> egui::Color32 {
    if row.is_unknown() {
        WARN
    } else if row.is_assumed() {
        READOUT
    } else {
        OK
    }
}

pub fn list(ui: &mut Ui, st: &mut State, project: &Project) {
    st.load(project);
    let Some(plan) = &st.plan else {
        ui.add(egui::Label::new(legend("This project has no schematic to plan tests for.")).wrap());
        return;
    };
    ui.label(legend("pads in bring-up order"));
    ui.add_space(6.0);
    let current = st.current().map(|r| r.reference.clone());
    let rows = bring_up_order(plan);
    if rows.is_empty() {
        ui.add(egui::Label::new(legend("The schematic has no test points.")).wrap());
        return;
    }
    let mut clicked = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(
        ui,
        24.0,
        rows.len(),
        |ui, range| {
            for r in &rows[range] {
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
                let on = current.as_deref() == Some(r.reference.as_str());
                let p = ui.painter();
                if on {
                    p.rect_filled(rect, 0.0, PANEL);
                    p.rect_filled(
                        Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                        0.0,
                        READOUT,
                    );
                } else if resp.hovered() {
                    p.rect_filled(rect, 0.0, BAND);
                }
                p.circle_filled(Pos2::new(rect.left() + 12.0, rect.center().y), 3.0, tint(r));
                p.text(
                    Pos2::new(rect.left() + 22.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("{}  {}", r.order, r.reference),
                    figure(12.0),
                    if on { VALUE } else { VALUE.gamma_multiply(0.85) },
                );
                p.with_clip_rect(rect).text(
                    Pos2::new(rect.right() - 8.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    &r.net,
                    egui::FontId::proportional(11.5),
                    LEGEND,
                );
                if resp.clicked() {
                    clicked = Some(r.reference.clone());
                }
            }
        },
    );
    if clicked.is_some() {
        st.selected = clicked;
    }
}

pub fn page(ui: &mut Ui, st: &mut State, project: &Project, details: bool) {
    st.load(project);
    let Some(plan) = &st.plan else { return };
    let current = st.current().cloned();
    if details && let Some(row) = &current {
        egui::Panel::right("test-plan-details")
            .resizable(true)
            .default_size(380.0)
            .min_size(320.0)
            .frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin::symmetric(12, 10)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    side(ui, row);
                });
            });
    }
    let mut clicked = None;
    egui::CentralPanel::no_frame().show(ui, |ui| {
        egui::Frame::NONE
            .inner_margin(egui::Margin { left: 18, right: 14, top: 12, bottom: 0 })
            .show(ui, |ui| {
                ui.label(RichText::new(&plan.design).font(figure(26.0)).color(VALUE));
                ui.label(RichText::new(&plan.summary).font(legend_font(13.0)).color(LEGEND));
                ui.add_space(10.0);
                let rows = bring_up_order(plan);
                let cols = [
                    ("step", 44.0),
                    ("pad", 56.0),
                    ("net", 110.0),
                    ("measure", 80.0),
                    ("expected", 180.0),
                ];
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let top = ui.cursor().top();
                    Table::new(&cols, rows.len()).show(ui, |i, p, r, at| {
                        let row = rows[i];
                        cell(p, r, at(0), cols[0].1, &row.order.to_string(), READOUT);
                        cell(p, r, at(1), cols[1].1, &row.reference, VALUE);
                        cell(p, r, at(2), cols[2].1, &row.net, LEGEND);
                        cell(p, r, at(3), cols[3].1, &row.measure, LEGEND);
                        cell(p, r, at(4), cols[4].1, &expected(row), tint(row));
                    });
                    let rect = ui.min_rect();
                    let resp =
                        ui.interact(rect, ui.make_persistent_id("plan-rows"), Sense::click());
                    if resp.clicked()
                        && let Some(pos) = resp.interact_pointer_pos()
                    {
                        let i = ((pos.y - top - egui_bench::table::ROW_H)
                            / egui_bench::table::ROW_H)
                            .floor();
                        if i >= 0.0 && (i as usize) < rows.len() {
                            clicked = Some(rows[i as usize].reference.clone());
                        }
                    }
                });
            });
    });
    if clicked.is_some() {
        st.selected = clicked;
    }
}

fn side(ui: &mut Ui, row: &Row) {
    card(
        ui,
        Some(tint(row)),
        |ui| {
            Line::new().legend("test point").value(&row.reference).elided(ui);
        },
        |ui| {
            match &row.expect {
                Expect::Reading(r) => {
                    readouts(
                        ui,
                        &[
                            (
                                "nominal",
                                r.nominal.map(|n| trim(n, 3)).unwrap_or_else(|| "-".into()),
                                READOUT,
                            ),
                            ("min", trim(r.min, 3), VALUE),
                            ("max", trim(r.max, 3), VALUE),
                        ],
                    );
                    reading(ui, "unit", r.unit.clone());
                    reading(ui, "basis", r.basis.clone());
                }
                Expect::Logic(l) => {
                    chip(ui, "logic level", OK, true);
                    ui.add(egui::Label::new(legend(&l.text())).wrap());
                    reading(ui, "basis", l.basis.clone());
                }
                Expect::Unknown(why) if why == agentee_core::testplan::FIRMWARE => {
                    chip(ui, why, READOUT, true);
                }
                Expect::Unknown(why) => {
                    chip(ui, "unknown", WARN, true);
                    ui.add(egui::Label::new(legend(why)).wrap());
                }
            }
            reading(ui, "step", row.order.to_string());
            reading(ui, "net", row.net.clone());
            reading(ui, "role", row.role.clone());
            reading(ui, "measure", row.measure.clone());
            if let Some(l) = &row.location {
                reading(
                    ui,
                    "location",
                    format!("{}, {} mm, {}", trim(l.x, 2), trim(l.y, 2), l.side),
                );
            }
            if let Some(n) = &row.nearest {
                reading(ui, "nearest", n.clone());
            }
            if let Some(r) = &row.reason {
                reading(ui, "reason", r.clone());
            }
        },
    );
    if !row.conditions.is_empty() {
        ui.add_space(8.0);
        ui.label(legend("conditions"));
        for c in &row.conditions {
            ui.add(egui::Label::new(RichText::new(c).size(12.5).color(VALUE)).wrap());
        }
    }
    if !row.unpowered.is_empty() {
        ui.add_space(8.0);
        ui.label(legend("unpowered until"));
        for c in &row.unpowered {
            ui.add(egui::Label::new(RichText::new(c).size(12.5).color(WARN)).wrap());
        }
    }
    if !row.source.is_empty() {
        ui.add_space(8.0);
        ui.label(legend("sources"));
        for s in &row.source {
            ui.horizontal_wrapped(|ui| {
                let tone = if s.confidence == "assumed" { READOUT } else { OK };
                chip(ui, &s.confidence, tone, true);
                ui.label(
                    RichText::new(format!("{}: {}", s.kind, s.detail)).size(12.0).color(LEGEND),
                );
            });
        }
    }
}
