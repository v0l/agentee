use crate::pages::{PageState, page};
use agentee_core::project::{ItemRef, Kind, Project};
use agentee_core::{Diagnostic, Severity};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use egui_bench::prelude::*;
use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

type Row = (ItemRef, String, Option<Severity>, Option<f32>, bool);

pub struct App {
    path: PathBuf,
    project: Project,
    error: Option<String>,
    tab: Kind,
    selected: Option<(Kind, String)>,
    filter: String,
    st: PageState,
    events: Option<Receiver<()>>,
    _watcher: Option<notify::RecommendedWatcher>,
    dirty: Option<Instant>,
    loaded: Instant,
    progress: std::collections::HashMap<String, agentee_core::sim::SimProgress>,
    polled: Instant,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, path: PathBuf, select: Option<String>) -> Self {
        egui_bench::install(&cc.egui_ctx);
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let mut app = App {
            path,
            project: Project::default(),
            error: None,
            tab: Kind::Symbol,
            selected: None,
            filter: String::new(),
            st: PageState::default(),
            events: None,
            _watcher: None,
            dirty: None,
            loaded: Instant::now(),
            progress: Default::default(),
            polled: Instant::now() - Duration::from_secs(5),
        };
        app.reload();
        app.watch(cc.egui_ctx.clone());
        if let Some(r) = select.and_then(|n| app.project.find(&n)) {
            app.select(r);
        } else if let Some(r) = app.project.all_refs().first().copied() {
            app.select(r);
        }
        app
    }

    fn watch(&mut self, ctx: egui::Context) {
        let (tx, rx) = channel();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(e) = res
                && e.paths
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
        match Project::load(&self.path) {
            Ok(p) => {
                self.project = p;
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        self.loaded = Instant::now();
    }

    fn select(&mut self, r: ItemRef) {
        let kind = r.kind();
        self.tab = kind;
        self.selected = Some((kind, self.project.name_of(r).to_string()));
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
        if let Some(t) = self.dirty {
            let wait = Duration::from_millis(150);
            if t.elapsed() >= wait {
                self.dirty = None;
                self.reload();
            } else {
                ctx.request_repaint_after(wait - t.elapsed());
            }
        }
    }

    fn header(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            Line::new().legend("agentee").value(self.path.display().to_string()).show(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let e = self.project.count(Severity::Error);
                let w = self.project.count(Severity::Warning);
                lamp(ui, "live", self.events.is_some(), false).on_hover_text(format!(
                    "reloads on save, last {:.0}s ago",
                    self.loaded.elapsed().as_secs_f32()
                ));
                lamp(ui, &format!("{w} warn"), w == 0, false);
                lamp(ui, &format!("{e} err"), e == 0, e > 0);
            });
        });
        ui.add_space(4.0);
        let opts = [
            (Kind::Sim, format!("sims {}", self.project.sims.len())),
            (Kind::Layout, format!("layouts {}", self.project.layouts.len())),
            (Kind::Schematic, format!("schematics {}", self.project.schematics.len())),
            (Kind::Board, format!("boards {}", self.project.boards.len())),
            (Kind::Symbol, format!("symbols {}", self.project.symbols.len())),
            (Kind::Footprint, format!("footprints {}", self.project.footprints.len())),
        ];
        let refs: Vec<(Kind, &str)> = opts.iter().map(|(k, s)| (*k, s.as_str())).collect();
        tabs(ui, &mut self.tab, &refs);
    }

    fn list(&mut self, ui: &mut Ui) {
        field(ui, &mut self.filter, "filter");
        ui.add_space(6.0);
        let needle = self.filter.to_lowercase();
        let rows: Vec<Row> = self
            .project
            .all_refs()
            .into_iter()
            .filter(|r| r.kind() == self.tab)
            .map(|r| {
                let worst = self.project.diags_of(r).iter().map(|d| d.severity).max();
                let name = self.project.name_of(r).to_string();
                let (running, incomplete) = match r {
                    ItemRef::Sim(i) => {
                        let s = &self.project.sims[i].item;
                        (
                            self.progress.get(&name).map(|p| p.fraction()),
                            (s.result.is_none() && s.maps.is_none()) || s.stale,
                        )
                    }
                    _ => (None, false),
                };
                (r, name, worst, running, incomplete)
            })
            .filter(|(_, n, _, _, _)| needle.is_empty() || n.to_lowercase().contains(&needle))
            .collect();
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

    fn failures(&self, ui: &mut Ui) {
        if self.project.failures.is_empty() && self.error.is_none() {
            return;
        }
        card(
            ui,
            Some(FAULT),
            |ui| {
                Line::new().legend("files that did not load").show(ui);
            },
            |ui| {
                if let Some(e) = &self.error {
                    status(ui, false, e);
                }
                for f in &self.project.failures {
                    failure(ui, f);
                }
            },
        );
    }
}

fn failure(ui: &mut Ui, d: &Diagnostic) {
    let file = d.file.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
    Line::new().legend(&d.at).value(file).size(11.0).wrapped(ui);
    Line::new().value(&d.message).tint(FAULT).size(11.5).wrapped(ui);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _f: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        if ctx.input(|i| i.key_pressed(egui::Key::F)) && !ctx.egui_wants_keyboard_input() {
            self.st.view.fitted = false;
        }
        egui::Panel::top("header")
            .frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin {
                left: 12,
                right: 12,
                top: 8,
                bottom: 0,
            }))
            .show(ui, |ui| self.header(ui));
        egui::Panel::left("items")
            .resizable(true)
            .default_size(260.0)
            .min_size(200.0)
            .frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin::symmetric(10, 8)))
            .show(ui, |ui| {
                self.failures(ui);
                self.list(ui);
            });
        let current = self.current();
        self.poll_progress(&ctx);
        self.st.sim_progress = match current {
            Some(r @ ItemRef::Sim(_)) => self.progress.get(self.project.name_of(r)).cloned(),
            _ => None,
        };
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let r = ui.max_rect();
            ui.painter().rect_filled(r, 0.0, CHASSIS);
            ui.painter().line_segment([r.left_top(), r.left_bottom()], Stroke::new(1.0, ETCH));
            match current {
                Some(item) => page(ui, &self.project, item, &mut self.st),
                None => {
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

pub fn run(path: PathBuf, select: Option<String>) -> eframe::Result<()> {
    let title = format!("agentee - {}", path.display());
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 950.0])
            .with_title(title.clone()),
        ..Default::default()
    };
    eframe::run_native(&title, opts, Box::new(move |cc| Ok(Box::new(App::new(cc, path, select)))))
}
