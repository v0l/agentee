use crate::pages::{PageState, page};
use agentee_core::Severity;
use agentee_core::project::{ItemRef, Kind, Project};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use egui_bench::prelude::*;
use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

fn changes_content(kind: &notify::EventKind) -> bool {
    use notify::EventKind;
    use notify::event::{AccessKind, AccessMode, ModifyKind};
    match kind {
        EventKind::Create(_) | EventKind::Remove(_) => true,
        EventKind::Modify(ModifyKind::Metadata(_)) => false,
        EventKind::Modify(_) => true,
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        _ => false,
    }
}

type Row = (ItemRef, String, Option<Severity>, Option<f32>, bool);

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Item(Kind),
    Assembly,
    Library,
}

const PANELS: [(&str, &str); 3] = [
    ("list", "the list of items on the left"),
    ("details", "the details column on the right"),
    ("checks", "the check results along the bottom"),
];

fn tab_strip(
    ui: &mut Ui,
    current: &mut Tab,
    groups: &[Vec<(Tab, &str, Option<usize>)>],
    panels: &mut [bool; 3],
) {
    const TAB_H: f32 = 24.0;
    const GAP: f32 = 22.0;
    const SEAM: f32 = 40.0;
    let width = ui.available_width();
    let (strip, _) = ui.allocate_exact_size(Vec2::new(width, TAB_H + 2.0), Sense::hover());
    let p = ui.painter().clone();
    let mut x = strip.left();
    let mut mark = None;
    for (g, group) in groups.iter().enumerate() {
        if g > 0 {
            let seam = x + SEAM / 2.0 - GAP;
            p.line_segment(
                [Pos2::new(seam, strip.top() + 5.0), Pos2::new(seam, strip.bottom() - 7.0)],
                Stroke::new(1.0, ETCH),
            );
            x += SEAM - GAP;
        }
        for (tab, label, count) in group {
            let on = *current == *tab;
            let mut job = egui::text::LayoutJob::default();
            job.append(
                label,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(14.0),
                    color: if on { VALUE } else { LEGEND },
                    ..Default::default()
                },
            );
            if let Some(n) = count {
                job.append(
                    &n.to_string(),
                    6.0,
                    egui::TextFormat {
                        font_id: egui_bench::theme::figure(11.0),
                        color: if on { READOUT } else { LEGEND.gamma_multiply(0.7) },
                        ..Default::default()
                    },
                );
            }
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let w = galley.size().x;
            let hit =
                Rect::from_min_size(Pos2::new(x - 6.0, strip.top()), Vec2::new(w + 12.0, TAB_H));
            let r = ui.interact(hit, ui.id().with(("tab", *label)), Sense::click());
            if r.hovered() && !on {
                p.rect_filled(hit, 3.0, BAND);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if r.clicked() {
                *current = *tab;
            }
            p.galley(Pos2::new(x, strip.top() + (TAB_H - galley.size().y) / 2.0), galley, VALUE);
            if on {
                mark = Some((x, w));
            }
            x += w + GAP;
        }
    }
    p.rect_filled(
        Rect::from_min_size(Pos2::new(strip.left(), strip.bottom() - 1.0), Vec2::new(width, 1.0)),
        0.0,
        ETCH,
    );
    if let Some((x, w)) = mark {
        p.rect_filled(
            Rect::from_min_size(Pos2::new(x, strip.bottom() - 2.0), Vec2::new(w, 2.0)),
            0.0,
            READOUT,
        );
    }
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                Pos2::new(x, strip.top()),
                Pos2::new(strip.right(), strip.bottom() - 2.0),
            ))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    for (k, (name, what)) in PANELS.iter().enumerate().rev() {
        let verb = if panels[k] { "hide" } else { "show" };
        if toggle(&mut right, name, panels[k]).on_hover_text(format!("{verb} {what}")).clicked() {
            panels[k] = !panels[k];
        }
    }
    right.label(legend("panels"));
}

enum Leave {
    Quit,
    Open(PathBuf),
}

const SUFFIXES: [&str; 6] = [
    agentee_core::project::BOARD_EXT,
    agentee_core::project::SYMBOL_EXT,
    agentee_core::project::FOOTPRINT_EXT,
    agentee_core::project::SCHEMATIC_EXT,
    agentee_core::project::LAYOUT_EXT,
    agentee_core::project::SIM_EXT,
];

pub struct App {
    path: PathBuf,
    project: Project,
    error: Option<String>,
    tab: Tab,
    selected: Option<(Kind, String)>,
    assembly: Option<String>,
    show_list: bool,
    library: crate::library::Shelf,
    last: std::collections::HashMap<Kind, String>,
    guides: std::collections::HashMap<String, crate::assembly::Guide>,
    filter: String,
    st: PageState,
    events: Option<Receiver<()>>,
    _watcher: Option<notify::RecommendedWatcher>,
    dirty: Option<Instant>,
    loading: Option<Receiver<Result<Project, String>>>,
    loaded: Instant,
    progress: std::collections::HashMap<String, agentee_core::sim::SimProgress>,
    polled: Instant,
    ctx: egui::Context,
    leaving: Option<Leave>,
    browser: Option<Browser>,
    new_layout: Option<crate::newlayout::Form>,
    pending_select: Option<String>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext,
        path: PathBuf,
        select: Option<String>,
        view_3d: bool,
        assemble: bool,
    ) -> Self {
        egui_bench::install(&cc.egui_ctx);
        if let Some(state) = &cc.wgpu_render_state {
            egui_bench::viewer3d::install(state);
        }
        let repaint = cc.egui_ctx.clone();
        agentee_3d::set_waker(move || repaint.request_repaint());
        let mut app = App {
            path: PathBuf::new(),
            project: Project::default(),
            error: None,
            tab: Tab::Item(Kind::Symbol),
            selected: None,
            assembly: None,
            show_list: true,
            library: Default::default(),
            last: Default::default(),
            guides: Default::default(),
            filter: String::new(),
            st: PageState::default(),
            events: None,
            _watcher: None,
            dirty: None,
            loading: None,
            loaded: Instant::now(),
            progress: Default::default(),
            polled: Instant::now() - Duration::from_secs(5),
            ctx: cc.egui_ctx.clone(),
            leaving: None,
            browser: None,
            new_layout: None,
            pending_select: None,
        };
        app.st.view_3d = view_3d;
        let tab = match select.as_deref() {
            Some("library") => Some(Tab::Library),
            Some("assembly") => Some(Tab::Assembly),
            _ => None,
        };
        app.open(path, select.filter(|_| tab.is_none()));
        if let Some(t) = tab {
            app.tab = t;
        }
        if assemble {
            app.tab = Tab::Assembly;
            app.assembly = match &app.selected {
                Some((Kind::Layout, name)) => Some(name.clone()),
                _ => None,
            };
        }
        app
    }

    fn open(&mut self, path: PathBuf, select: Option<String>) {
        self.path = std::fs::canonicalize(&path).unwrap_or(path);
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "agentee - {}",
            self.path.display()
        )));
        self.st = PageState { view_3d: self.st.view_3d, ..PageState::default() };
        self.project = Project::default();
        self.selected = None;
        self.assembly = None;
        self.guides.clear();
        self.filter.clear();
        self.progress.clear();
        self.polled = Instant::now() - Duration::from_secs(5);
        self._watcher = None;
        self.events = None;
        self.dirty = None;
        self.loading = None;
        self.new_layout = None;
        self.pending_select = None;
        self.reload();
        self.watch(self.ctx.clone());
        self.library = Default::default();
        let refs = self.project.all_refs();
        let first = |k: Kind| refs.iter().find(|r| r.kind() == k).copied();
        let pick = select
            .and_then(|n| self.project.find(&n))
            .or_else(|| first(Kind::Layout))
            .or_else(|| first(Kind::Schematic))
            .or_else(|| refs.first().copied());
        if let Some(r) = pick {
            self.select(r);
        }
    }

    fn browse(&mut self) {
        let dir = if self.path.is_dir() {
            self.path.clone()
        } else {
            self.path.parent().map(PathBuf::from).unwrap_or_default()
        };
        self.browser = Some(
            Browser::new("open a project folder or file", dir).suffixes(&SUFFIXES).folders(true),
        );
    }

    fn show_browser(&mut self, ctx: &egui::Context) {
        let Some(b) = &mut self.browser else { return };
        let target = match b.show(ctx) {
            Picked::Browsing => return,
            Picked::Cancelled => None,
            Picked::File(p) | Picked::Folder(p) => Some(p),
        };
        self.browser = None;
        match target {
            Some(p) if self.st.dirty() > 0 => self.leaving = Some(Leave::Open(p)),
            Some(p) => self.open(p, None),
            None => {}
        }
    }

    fn watch(&mut self, ctx: egui::Context) {
        let (tx, rx) = channel();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(e) = res else { return };
            if !changes_content(&e.kind) {
                return;
            }
            if e.paths.iter().any(|p| agentee_3d::is_model(p)) {
                agentee_3d::models_changed();
                ctx.request_repaint();
            }
            if e.paths
                .iter()
                .any(|p| Kind::of(p).is_some() || p.to_string_lossy().ends_with(".result.json"))
            {
                let _ = tx.send(());
                ctx.request_repaint();
            }
        });
        if let Ok(mut w) = watcher {
            let mode = if self.path.is_dir() {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            let target = if self.path.is_dir() {
                self.path.clone()
            } else {
                self.path.parent().map(PathBuf::from).unwrap_or_default()
            };
            if w.watch(&target, mode).is_ok() {
                self._watcher = Some(w);
                self.events = Some(rx);
            }
        }
    }

    fn reload(&mut self) {
        self.apply(Project::load(&self.path).map_err(|e| e.to_string()));
    }

    fn apply(&mut self, loaded: Result<Project, String>) {
        match loaded {
            Ok(p) => {
                self.project = p;
                self.error = None;
                self.st.sync(&self.ctx, &self.project);
                if let Some(n) = self.pending_select.clone()
                    && let Some(r) = self.project.find(&format!("pcb:{n}"))
                {
                    self.pending_select = None;
                    self.select(r);
                }
            }
            Err(e) => self.error = Some(e),
        }
        self.loaded = Instant::now();
    }

    fn reload_in_background(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let (path, ctx) = (self.path.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Project::load(&path).map_err(|e| e.to_string()));
            ctx.request_repaint();
        });
        self.loading = Some(rx);
    }

    fn select(&mut self, r: ItemRef) {
        let kind = r.kind();
        self.tab = Tab::Item(kind);
        let name = self.project.name_of(r).to_string();
        self.last.insert(kind, name.clone());
        self.selected = Some((kind, name));
    }

    fn current(&self) -> Option<ItemRef> {
        let (kind, name) = self.selected.as_ref()?;
        self.project
            .all_refs()
            .into_iter()
            .find(|r| r.kind() == *kind && self.project.name_of(*r) == name)
    }

    fn poll_progress(&mut self, ctx: &egui::Context) {
        if self.polled.elapsed() >= Duration::from_millis(500) {
            self.polled = Instant::now();
            self.progress = self
                .project
                .sims
                .iter()
                .filter_map(|e| {
                    agentee_core::sim::SimProgress::load(&e.path).map(|p| (e.name.clone(), p))
                })
                .collect();
        }
        if !self.progress.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.events {
            while rx.try_recv().is_ok() {
                self.dirty = Some(Instant::now());
            }
        }
        if let Some(rx) = &self.loading {
            match rx.try_recv() {
                Ok(loaded) => {
                    self.loading = None;
                    self.apply(loaded);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.loading = None,
            }
        }
        if let Some(t) = self.dirty {
            let wait = Duration::from_millis(150);
            if t.elapsed() >= wait {
                self.dirty = None;
                self.reload_in_background(ctx);
            } else {
                ctx.request_repaint_after(wait - t.elapsed());
            }
        }
    }

    fn header(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            Line::new().legend("agentee").value(self.path.display().to_string()).show(ui);
            if toggle(ui, "open", self.browser.is_some())
                .on_hover_text("open another project folder or file, ctrl+O")
                .clicked()
            {
                self.browse();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let e = self.project.count(Severity::Error);
                let w = self.project.count(Severity::Warning);
                let unsaved = self.st.dirty();
                if unsaved > 0 {
                    lamp(ui, &format!("{unsaved} unsaved"), false, true)
                        .on_hover_text("layouts with edits not written yet, ctrl+S saves");
                }
                lamp(ui, "live", self.events.is_some(), false).on_hover_text(format!(
                    "reloads on save, last {:.0}s ago",
                    self.loaded.elapsed().as_secs_f32()
                ));
                let checks = "show or hide the check results of the selected item";
                let warn = lamp(ui, &format!("{w} warn"), w == 0, false);
                let err = lamp(ui, &format!("{e} err"), e == 0, e > 0);
                if warn.interact(Sense::click()).on_hover_text(checks).clicked()
                    || err.interact(Sense::click()).on_hover_text(checks).clicked()
                {
                    self.st.show_checks = !self.st.show_checks;
                }
            });
        });
        ui.add_space(6.0);
        let p = &self.project;
        let item = |k: Kind, label: &'static str, n: usize| (Tab::Item(k), label, Some(n));
        let groups = [
            vec![
                item(Kind::Schematic, "schematics", p.schematics.len()),
                item(Kind::Layout, "layouts", p.layouts.len()),
                item(Kind::Board, "boards", p.boards.len()),
                item(Kind::Sim, "sims", p.sims.len()),
            ],
            vec![(Tab::Assembly, "assembly", None), (Tab::Library, "library", None)],
            vec![
                item(Kind::Symbol, "symbols", p.symbols.len()),
                item(Kind::Footprint, "footprints", p.footprints.len()),
            ],
        ];
        let mut tab = self.tab;
        let mut panels = [self.show_list, self.st.show_details, self.st.show_checks];
        tab_strip(ui, &mut tab, &groups, &mut panels);
        [self.show_list, self.st.show_details, self.st.show_checks] = panels;
        if tab != self.tab {
            self.switch(tab);
        }
    }

    fn switch(&mut self, tab: Tab) {
        self.tab = tab;
        let Tab::Item(kind) = tab else { return };
        if self.selected.as_ref().is_some_and(|(k, _)| *k == kind) {
            return;
        }
        let refs = self.project.all_refs();
        let of_kind = |r: &&ItemRef| r.kind() == kind;
        let last = self.last.get(&kind).cloned();
        let pick = refs
            .iter()
            .filter(of_kind)
            .find(|r| last.as_deref() == Some(self.project.name_of(**r)))
            .or_else(|| refs.iter().find(of_kind))
            .copied();
        match pick {
            Some(r) => self.select(r),
            None => self.selected = None,
        }
    }

    fn assembly_list(&mut self, ui: &mut Ui) {
        ui.label(legend("boards to build"));
        ui.add_space(6.0);
        let current = self.assembly_layout();
        let mut clicked = None;
        for (i, e) in self.project.layouts.iter().enumerate() {
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
            let on = current == Some(i);
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
            p.with_clip_rect(rect).text(
                Pos2::new(rect.left() + 12.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &e.name,
                egui::FontId::proportional(13.0),
                if on { VALUE } else { VALUE.gamma_multiply(0.85) },
            );
            if let Some((done, total)) = self.guides.get(&e.name).map(|g| g.progress()) {
                p.text(
                    Pos2::new(rect.right() - 8.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    format!("{done}/{total}"),
                    egui_bench::theme::figure(11.5),
                    if done == total && total > 0 { OK } else { READOUT },
                );
            }
            if resp.clicked() {
                clicked = Some(e.name.clone());
            }
        }
        if let Some(n) = clicked {
            self.assembly = Some(n);
        }
    }

    fn assembly_layout(&self) -> Option<usize> {
        let by_name = |n: &String| self.project.layouts.iter().position(|e| &e.name == n);
        self.assembly.as_ref().and_then(by_name).or((!self.project.layouts.is_empty()).then_some(0))
    }

    fn list(&mut self, ui: &mut Ui) {
        let tab = match self.tab {
            Tab::Item(k) => k,
            Tab::Assembly => return self.assembly_list(ui),
            Tab::Library => return crate::library::list(ui, &mut self.library, &self.project),
        };
        if tab == Kind::Layout {
            let dir = self.path.is_dir();
            if ui
                .add_enabled(dir, egui::Button::new("new layout"))
                .on_disabled_hover_text("open a project directory to add files to it")
                .clicked()
            {
                self.new_layout = Some(crate::newlayout::Form::new(&self.project));
            }
            ui.add_space(6.0);
        }
        field(ui, &mut self.filter, "filter");
        ui.add_space(6.0);
        let needle = self.filter.to_lowercase();
        let rows: Vec<Row> = self
            .project
            .all_refs()
            .into_iter()
            .filter(|r| r.kind() == tab)
            .map(|r| {
                let worst = self.project.diags_of(r).iter().map(|d| d.severity).max();
                let name = self.project.name_of(r).to_string();
                let (running, incomplete) = match r {
                    ItemRef::Sim(i) => {
                        let s = &self.project.sims[i].item;
                        (
                            self.progress.get(&name).map(|p| p.fraction()),
                            (s.result.is_none()
                                && s.maps.is_none()
                                && s.channel.is_none()
                                && s.logic_result.is_none()
                                && s.circuit_result.is_none())
                                || s.stale,
                        )
                    }
                    _ => (None, false),
                };
                (r, name, worst, running, incomplete)
            })
            .filter(|(_, n, _, _, _)| needle.is_empty() || n.to_lowercase().contains(&needle))
            .collect();
        let mut rows = rows;
        if tab == Kind::Schematic {
            for row in rows.iter_mut() {
                if let ItemRef::Schematic(i) = row.0
                    && let Some(parent) = &self.project.schematics[i].item.parent
                {
                    row.1 = format!("{parent} \u{203a} {}", row.1);
                }
            }
            rows.sort_by(|a, b| a.1.cmp(&b.1));
        }
        let current = self.current();
        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(
            ui,
            20.0,
            rows.len(),
            |ui, range| {
                for (r, name, worst, running, incomplete) in &rows[range] {
                    let (rect, resp) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::click());
                    let on = current == Some(*r);
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
                    let dot = match (worst, running, incomplete) {
                        (Some(Severity::Error), _, _) => FAULT,
                        (_, Some(_), _) => READOUT,
                        (Some(Severity::Warning), _, _) | (_, _, true) => WARN,
                        _ => OK,
                    };
                    if let Some(f) = running {
                        p.text(
                            Pos2::new(rect.right() - 6.0, rect.center().y),
                            egui::Align2::RIGHT_CENTER,
                            format!("{:.0}%", f * 100.0),
                            egui_bench::theme::figure(11.0),
                            READOUT,
                        );
                    }
                    p.circle_filled(Pos2::new(rect.left() + 12.0, rect.center().y), 3.0, dot);
                    let col = if on { VALUE } else { VALUE.gamma_multiply(0.85) };
                    p.with_clip_rect(rect).text(
                        Pos2::new(rect.left() + 22.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        name,
                        egui::FontId::proportional(12.5),
                        col,
                    );
                    if resp.clicked() {
                        clicked = Some(*r);
                    }
                }
            },
        );
        if let Some(r) = clicked {
            self.select(r);
        }
    }

    fn error(&self, ui: &mut Ui) {
        let Some(e) = &self.error else { return };
        card(
            ui,
            Some(FAULT),
            |ui| {
                Line::new().legend("error").show(ui);
            },
            |ui| status(ui, false, e),
        );
    }
}

impl App {
    fn guard_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.st.dirty() > 0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.leaving = Some(Leave::Quit);
        }
        if self.leaving.is_none() {
            return;
        }
        let mut close = false;
        egui::Modal::new(egui::Id::new("unsaved-edits")).show(ctx, |ui| {
            modal_title(ui, "unsaved layout edits");
            note(ui, format!("{} layouts have edits that are not saved.", self.st.dirty()), VALUE);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let verb = match self.leaving {
                    Some(Leave::Open(_)) => "open",
                    _ => "quit",
                };
                if ui.button(format!("save and {verb}")).clicked() {
                    match self.st.save_all() {
                        Ok(()) => close = true,
                        Err(e) => self.error = Some(e),
                    }
                }
                if ui.button(format!("{verb} without saving")).clicked() {
                    self.st.editors.clear();
                    close = true;
                }
                if ui.button("cancel").clicked() {
                    self.leaving = None;
                }
            });
        });
        if close {
            match self.leaving.take() {
                Some(Leave::Open(p)) => self.open(p, None),
                _ => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
    }
}

impl App {
    fn assembly_page(&mut self, ui: &mut Ui) {
        let Some(i) = self.assembly_layout() else {
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                note(ui, "This project has no layouts to assemble.", Color32::from_gray(160));
            });
            return;
        };
        let name = self.project.layouts[i].name.clone();
        let guide = self
            .guides
            .entry(name)
            .or_insert_with(|| crate::assembly::Guide::new(&self.project, i));
        crate::assembly::page(ui, &self.project, i, guide);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _f: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.guard_close(&ctx);
        self.poll(&ctx);
        self.show_browser(&ctx);
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
            self.browse();
        }
        if let Some(f) = &mut self.new_layout {
            match crate::newlayout::show(&ctx, &self.project, &self.project.root, f) {
                crate::newlayout::Outcome::Open => {}
                crate::newlayout::Outcome::Cancelled => self.new_layout = None,
                crate::newlayout::Outcome::Created(name) => {
                    self.new_layout = None;
                    self.pending_select = Some(name);
                    self.dirty = Some(Instant::now());
                }
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F)) && !ctx.egui_wants_keyboard_input() {
            self.st.view.fitted = false;
        }
        egui::Panel::top("header")
            .frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin {
                left: 12,
                right: 12,
                top: 8,
                bottom: 2,
            }))
            .show(ui, |ui| self.header(ui));
        if self.show_list {
            egui::Panel::left("items")
                .resizable(true)
                .default_size(240.0)
                .min_size(180.0)
                .frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin::symmetric(10, 8)))
                .show(ui, |ui| {
                    self.error(ui);
                    self.list(ui);
                });
        }
        let current = self.current();
        self.poll_progress(&ctx);
        self.st.runs.poll();
        if self.st.runs.any() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        self.st.sim_progress = match current {
            Some(r @ ItemRef::Sim(_)) => self.progress.get(self.project.name_of(r)).cloned(),
            _ => None,
        };
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let r = ui.max_rect();
            ui.painter().rect_filled(r, 0.0, CHASSIS);
            ui.painter().line_segment([r.left_top(), r.left_bottom()], Stroke::new(1.0, ETCH));
            match self.tab {
                Tab::Assembly => return self.assembly_page(ui),
                Tab::Library => {
                    return crate::library::page(
                        ui,
                        &mut self.library,
                        &self.project,
                        self.st.show_details,
                    );
                }
                Tab::Item(_) => {}
            }
            match current {
                Some(item) => page(ui, &self.project, item, &mut self.st),
                None => {
                    if !self.project.failures.is_empty() {
                        egui::Panel::bottom("diagnostics")
                            .resizable(true)
                            .default_size(150.0)
                            .frame(
                                egui::Frame::NONE
                                    .fill(CHASSIS)
                                    .inner_margin(egui::Margin::symmetric(10, 6)),
                            )
                            .show(ui, |ui| {
                                crate::pages::diagnostics(
                                    ui,
                                    &crate::pages::with_failures(&self.project, &[]),
                                    true,
                                )
                            });
                    }
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        note(
                            ui,
                            "Nothing selected. Pick an item on the left.",
                            Color32::from_gray(160),
                        );
                    });
                }
            }
        });
    }
}

pub fn run(
    path: PathBuf,
    select: Option<String>,
    view_3d: bool,
    assemble: bool,
) -> eframe::Result<()> {
    let title = format!("agentee - {}", path.display());
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 950.0])
            .with_title(title.clone())
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png"))
                    .unwrap_or_default(),
            ),
        ..Default::default()
    };
    eframe::run_native(
        &title,
        opts,
        Box::new(move |cc| Ok(Box::new(App::new(cc, path, select, view_3d, assemble)))),
    )
}
