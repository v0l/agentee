use crate::board3d;
use crate::canvas::View;
use crate::paint::{self, Layers, SymbolStyle};
use agentee_core::diag::Diags;
use agentee_core::footprint::{Footprint, FootprintFile};
use agentee_core::project::Project;
use agentee_core::symbol::{Symbol, SymbolFile};
use agentee_parts::library::{Entry, Library};
use egui::{Pos2, Rect, RichText, Sense, Ui, Vec2};
use egui_bench::prelude::*;
use egui_bench::theme::{figure, legend_font};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Default)]
enum Show {
    #[default]
    Footprint,
    Model,
    Symbol,
}

struct Detail {
    name: String,
    dir: PathBuf,
    footprint: Option<Footprint>,
    symbol: Option<Symbol>,
    datasheet: Option<PathBuf>,
    url: Option<String>,
    fp_view: View,
    sym_view: View,
    scene: Option<(u64, Arc<board3d::Scene>)>,
    orbit: board3d::State,
}

#[derive(Default)]
pub struct Shelf {
    lib: Option<Library>,
    root: PathBuf,
    filter: String,
    shelf_only: bool,
    selected: Option<String>,
    show: Show,
    detail: Option<Detail>,
    count: String,
    bin: String,
    said: Option<(String, bool)>,
    fix: Option<crate::partdialog::FixMatch>,
    add: Option<crate::partdialog::AddPart>,
}

impl Shelf {
    fn load(&mut self, project: &Project) {
        if self.lib.is_none() || self.root != project.root {
            self.root = project.root.clone();
            self.lib = Some(Library::open(Some(&project.root), None));
        }
    }

    fn reload(&mut self) {
        self.lib = Some(Library::open(Some(&self.root), None));
        self.detail = None;
    }

    fn entry(&self) -> Option<&Entry> {
        let name = self.selected.as_ref()?;
        self.lib.as_ref()?.entries.iter().find(|e| &e.name == name)
    }

    fn pick(&mut self, name: String) {
        if self.selected.as_ref() == Some(&name) {
            return;
        }
        self.selected = Some(name);
        self.detail = None;
        self.said = None;
        self.sync_fields();
    }

    fn sync_fields(&mut self) {
        let (count, bin) = match self.entry() {
            Some(e) => (e.self_stock().unwrap_or(0), e.bin().unwrap_or_default().to_string()),
            None => (0, String::new()),
        };
        self.count = count.to_string();
        self.bin = bin;
    }
}

fn read_footprint(path: &Path) -> Option<FootprintFile> {
    agentee_core::project::parse(&std::fs::read_to_string(path).ok()?).ok()
}

fn read_symbol(path: &Path) -> Option<SymbolFile> {
    agentee_core::project::parse(&std::fs::read_to_string(path).ok()?).ok()
}

fn detail(e: &Entry) -> Detail {
    let mut d = Diags::new(&e.name);
    let footprint = e
        .part
        .footprint
        .as_ref()
        .and_then(|f| read_footprint(&e.file(f)))
        .map(|f| f.resolve(&mut d))
        .map(|mut fp| {
            fp.graphics.retain(|g| !matches!(g.shape, agentee_core::graphic::Shape::Text { .. }));
            fp
        });
    let symbol = read_symbol(&e.file(&e.part.symbol)).map(|s| s.resolve(&mut d));
    let datasheet = e.part.datasheet.as_ref().and_then(|ds| {
        let cache = agentee_parts::datasheet::Cache::open(None).ok()?;
        let (entry, copy) = cache.by_hash(&ds.sha256)?;
        Some(entry.pdf(&copy)).filter(|p| p.exists())
    });
    Detail {
        name: e.name.clone(),
        dir: e.dir.clone(),
        footprint,
        symbol,
        datasheet,
        url: e.part.datasheet.as_ref().and_then(|d| d.url.clone()),
        fp_view: View::default(),
        sym_view: View::default(),
        scene: None,
        orbit: board3d::State::default(),
    }
}

pub(crate) fn open_externally(target: &str) -> Result<(), String> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(opener)
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{opener} {target}: {e}"))
}

pub fn list(ui: &mut Ui, shelf: &mut Shelf, project: &Project) {
    shelf.load(project);
    let add = Key::new("add a part").outline(READOUT).min_width(ui.available_width());
    if ui.add(add).clicked() {
        shelf.add = Some(Default::default());
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        field(ui, &mut shelf.filter, "filter");
    });
    ui.add_space(6.0);
    let Some(lib) = &shelf.lib else { return };
    let stocked = lib.entries.iter().filter(|e| e.self_stock().is_some()).count();
    segments(
        ui,
        &mut shelf.shelf_only,
        &[(false, "all", Some(lib.entries.len())), (true, "on the shelf", Some(stocked))],
    );
    ui.add_space(8.0);
    let needle = shelf.filter.to_lowercase();
    let mut rows: Vec<&Entry> = lib
        .entries
        .iter()
        .filter(|e| !shelf.shelf_only || e.self_stock().is_some())
        .filter(|e| {
            needle.is_empty() || {
                let p = &e.part;
                [Some(&e.name), Some(&p.role), p.mpn.as_ref(), p.value.as_ref(), Some(&p.notes)]
                    .into_iter()
                    .flatten()
                    .any(|t| t.to_lowercase().contains(&needle))
            }
        })
        .collect();
    rows.sort_by(|a, b| a.part.role.cmp(&b.part.role).then(a.name.cmp(&b.name)));
    let shown = shelf.selected.as_ref().is_some_and(|n| rows.iter().any(|e| &e.name == n));
    let first = (!shown).then(|| rows.first().map(|e| e.name.clone())).flatten();
    if rows.is_empty() {
        let text = if lib.entries.is_empty() {
            "The library is empty. Add parts with agentee library add."
        } else if shelf.shelf_only && stocked == 0 {
            "Nothing is on the shelf yet. Set a count in the details column or scan bags with agentee library intake."
        } else {
            "No library part matches the filter."
        };
        ui.add(egui::Label::new(legend(text)).wrap());
        return;
    }
    let mut clicked = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let mut role = None;
        for e in rows {
            if role != Some(&e.part.role) {
                if role.is_some() {
                    ui.add_space(8.0);
                }
                ui.label(RichText::new(&e.part.role).font(legend_font(12.5)).color(LEGEND));
                role = Some(&e.part.role);
            }
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
            let on = shelf.selected.as_ref() == Some(&e.name);
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
            let own = e.self_stock();
            let right = match (own, e.bin()) {
                (Some(n), Some(b)) => format!("{n}  {b}"),
                (Some(n), None) => n.to_string(),
                _ => String::new(),
            };
            let tail = p.text(
                Pos2::new(rect.right() - 8.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                right,
                figure(11.0),
                if own.is_some() { OK } else { LEGEND },
            );
            let name_clip =
                Rect::from_min_max(rect.min, Pos2::new(tail.left() - 6.0, rect.bottom()));
            let clipped = p.with_clip_rect(name_clip);
            clipped.text(
                Pos2::new(rect.left() + 12.0, rect.top() + 4.0),
                egui::Align2::LEFT_TOP,
                &e.name,
                egui::FontId::proportional(13.0),
                if on { VALUE } else { VALUE.gamma_multiply(0.85) },
            );
            let package = e.footprint_name.as_deref().map(agentee_parts::assembly::package);
            let sub: Vec<String> = [e.part.value.clone(), package]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .collect();
            clipped.text(
                Pos2::new(rect.left() + 12.0, rect.bottom() - 4.0),
                egui::Align2::LEFT_BOTTOM,
                sub.join(", "),
                egui::FontId::proportional(11.5),
                LEGEND,
            );
            if resp.clicked() {
                clicked = Some(e.name.clone());
            }
        }
    });
    if let Some(n) = clicked.or(first) {
        shelf.pick(n);
    }
}

fn dialogs(ui: &Ui, shelf: &mut Shelf) {
    use crate::partdialog::{Closed, add_part, fix_match};
    let ctx = ui.ctx().clone();
    let root = shelf.root.clone();
    if let Some(f) = shelf.fix.as_mut() {
        match fix_match(&ctx, &root, f) {
            Closed::Open => {}
            Closed::Cancelled => shelf.fix = None,
            Closed::Changed(_) => {
                shelf.fix = None;
                shelf.reload();
                shelf.sync_fields();
            }
        }
    }
    let roles: Vec<String> = {
        let mut r: Vec<String> =
            shelf.lib.iter().flat_map(|l| l.entries.iter().map(|e| e.part.role.clone())).collect();
        r.sort();
        r.dedup();
        r
    };
    if let Some(f) = shelf.add.as_mut() {
        match add_part(&ctx, &root, &roles, f) {
            Closed::Open => {}
            Closed::Cancelled => shelf.add = None,
            Closed::Changed(name) => {
                shelf.add = None;
                shelf.reload();
                if let Some(n) = name {
                    shelf.selected = None;
                    shelf.pick(n);
                }
            }
        }
    }
}

pub fn page(ui: &mut Ui, shelf: &mut Shelf, project: &Project, details: bool) {
    shelf.load(project);
    dialogs(ui, shelf);
    if shelf.entry().is_none() {
        let first = shelf.lib.as_ref().and_then(|l| {
            let mut names: Vec<(&str, &str)> =
                l.entries.iter().map(|e| (e.part.role.as_str(), e.name.as_str())).collect();
            names.sort();
            names.first().map(|n| n.1.to_string())
        });
        shelf.selected = None;
        match first {
            Some(n) => shelf.pick(n),
            None => {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    note(ui, "The library is empty. Add parts with agentee library add.", LEGEND);
                });
                return;
            }
        }
    }
    let Some(e) = shelf.entry().cloned() else { return };
    if shelf.detail.as_ref().is_none_or(|d| d.name != e.name) {
        shelf.detail = Some(detail(&e));
    }
    if details {
        egui::Panel::right("library-details")
            .resizable(true)
            .default_size(380.0)
            .min_size(320.0)
            .frame(egui::Frame::NONE.fill(CHASSIS).inner_margin(egui::Margin::symmetric(12, 10)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    side(ui, shelf, &e);
                });
            });
    }
    egui::CentralPanel::no_frame().show(ui, |ui| {
        egui::Frame::NONE
            .inner_margin(egui::Margin { left: 18, right: 14, top: 12, bottom: 0 })
            .show(ui, |ui| heading_block(ui, shelf, &e));
        ui.add_space(8.0);
        drawing(ui, shelf);
    });
}

fn heading_block(ui: &mut Ui, shelf: &mut Shelf, e: &Entry) {
    let p = &e.part;
    ui.horizontal(|ui| {
        ui.label(RichText::new(&e.name).font(figure(26.0)).color(VALUE));
        ui.add_space(10.0);
        if p.preferred {
            chip(ui, "preferred", OK, true);
        }
        if let Some(n) = e.self_stock() {
            chip(ui, &format!("{n} on the shelf"), OK, true);
        }
    });
    let maker = match (&p.manufacturer, &p.mpn) {
        (Some(m), Some(n)) => format!("{m} {n}"),
        (None, Some(n)) => n.clone(),
        _ => String::new(),
    };
    let what: Vec<String> = [Some(p.role.clone()), p.value.clone(), p.rating.clone()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    ui.label(RichText::new(what.join(", ")).font(legend_font(15.0)).color(VALUE));
    if !maker.is_empty() {
        ui.label(RichText::new(maker).font(legend_font(13.0)).color(LEGEND));
    }
    if !p.notes.is_empty() {
        ui.add(egui::Label::new(RichText::new(&p.notes).size(13.0).color(LEGEND)).wrap());
    }
    ui.add_space(10.0);
    let d = shelf.detail.as_ref();
    ui.horizontal(|ui| {
        let mut options = Vec::new();
        if d.is_some_and(|d| d.footprint.is_some()) {
            options.push((Show::Footprint, "footprint", None));
            options.push((Show::Model, "3d", None));
        }
        if d.is_some_and(|d| d.symbol.is_some()) {
            options.push((Show::Symbol, "symbol", None));
        }
        if !options.iter().any(|o| o.0 == shelf.show)
            && let Some(o) = options.first()
        {
            shelf.show = o.0;
        }
        segments(ui, &mut shelf.show, &options);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let fix = ui
                .add(Key::new("fix match").outline(READOUT))
                .on_hover_text("look the part up at the distributors and correct its part number");
            if fix.clicked() {
                shelf.fix = Some(crate::partdialog::FixMatch::new(
                    &e.name,
                    p.mpn.as_deref(),
                    p.manufacturer.as_deref(),
                    &p.notes,
                ));
            }
            let target = d.and_then(|d| {
                d.datasheet.as_ref().map(|p| p.display().to_string()).or(d.url.clone())
            });
            let k = Key::new("open datasheet").outline(TRACE).enabled(target.is_some());
            let r = ui.add(k);
            let r = match &target {
                Some(t) => r.on_hover_text(t),
                None => r.on_disabled_hover_text("no datasheet is pinned for this part"),
            };
            if r.clicked()
                && let Some(t) = target
                && let Err(err) = open_externally(&t)
            {
                shelf.said = Some((err, false));
            }
        });
    });
}

fn drawing(ui: &mut Ui, shelf: &mut Shelf) {
    let Some(d) = shelf.detail.as_mut() else { return };
    match shelf.show {
        Show::Footprint => {
            let Some(fp) = &d.footprint else { return };
            d.fp_view.max_fit = 2000.0;
            let (resp, xf) = d.fp_view.show(ui, &fp.bounds(), 50.0);
            let p = ui.painter_at(xf.rect);
            paint::grid(&p, &xf, 0.5);
            paint::footprint(&p, &xf, fp, &Layers::default(), resp.hover_pos());
            let size = fp.bounds().size();
            p.text(
                xf.rect.left_bottom() + Vec2::new(10.0, -10.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{}, {:.2} x {:.2} mm, {} pads", fp.name, size[0], size[1], fp.pads.len()),
                egui::FontId::proportional(12.0),
                LEGEND,
            );
        }
        Show::Model => {
            let Some(fp) = &d.footprint else { return };
            let generation = agentee_3d::generation();
            let stale = d.scene.as_ref().is_none_or(|(g, s)| *g != generation || s.pending > 0);
            if stale {
                let scene = board3d::part_scene(fp, &d.dir, agentee_3d::Fetch::Background);
                d.scene = Some((generation, Arc::new(scene)));
            }
            if let Some((_, scene)) = &d.scene {
                board3d::show(ui, scene, &mut d.orbit, true, true);
                if scene.pending > 0 {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
                }
            }
        }
        Show::Symbol => {
            let Some(s) = &d.symbol else { return };
            let padded = crate::pages::symbol_bounds(s, 1);
            d.sym_view.max_fit = 45.0;
            let (resp, xf) = d.sym_view.show(ui, &padded, 40.0);
            let p = ui.painter_at(xf.rect);
            paint::grid(&p, &xf, 1.27);
            let style = SymbolStyle {
                show_hidden: false,
                reference: format!("{}?", s.reference),
                value: s.value.clone(),
                dim: false,
                tips: true,
            };
            paint::symbol(&p, &xf, s, 1, &style, resp.hover_pos());
            if s.units > 1 {
                p.text(
                    xf.rect.left_bottom() + Vec2::new(10.0, -10.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("unit 1 of {}", s.units),
                    egui::FontId::proportional(12.0),
                    LEGEND,
                );
            }
        }
    }
}

fn side(ui: &mut Ui, shelf: &mut Shelf, e: &Entry) {
    let own = e.self_stock().unwrap_or(0);
    card(
        ui,
        Some(if own > 0 { OK } else { LEGEND }),
        |ui| {
            Line::new().legend("on the shelf").show(ui);
        },
        |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(legend("count"));
                    ui.add(egui::TextEdit::singleline(&mut shelf.count).desired_width(90.0));
                });
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(legend("bin"));
                    ui.add(
                        egui::TextEdit::singleline(&mut shelf.bin)
                            .hint_text("A3")
                            .desired_width(120.0),
                    );
                });
            });
            ui.add_space(8.0);
            let count = shelf.count.trim().parse::<u64>();
            let changed = count.as_ref().ok() != Some(&own)
                || shelf.bin.trim() != e.bin().unwrap_or_default();
            ui.horizontal(|ui| {
                let save = Key::new("save").solid(READOUT).enabled(changed && count.is_ok());
                if ui.add(save).clicked() {
                    shelf.said = Some(match save_stock(shelf, e) {
                        Ok(t) => (t, true),
                        Err(t) => (t, false),
                    });
                }
                if changed && ui.add(Key::new("undo").outline(VALUE)).clicked() {
                    shelf.sync_fields();
                }
            });
            if count.is_err() {
                note(ui, "The count is a whole number of parts.", FAULT);
            }
            if let Some((t, ok)) = &shelf.said {
                note(ui, t.clone(), if *ok { OK } else { FAULT });
            }
        },
    );
    ui.add_space(10.0);
    let listings: Vec<String> =
        e.part.stock.iter().filter(|s| s.own == 0).map(|s| s.line()).collect();
    card(
        ui,
        None,
        |ui| {
            Line::new().legend("distributors").show(ui);
        },
        |ui| {
            if listings.is_empty() {
                ui.label(legend("No listing yet. agentee library refresh checks stock and price."));
            }
            for l in listings {
                ui.add(egui::Label::new(RichText::new(l).size(12.5).color(VALUE)).wrap());
                ui.add_space(3.0);
            }
        },
    );
    ui.add_space(10.0);
    card(
        ui,
        None,
        |ui| {
            Line::new().legend("files").show(ui);
        },
        |ui| {
            reading(ui, "symbol", e.part.symbol.clone());
            if let Some(f) = &e.part.footprint {
                reading(ui, "footprint", f.clone());
            }
            if let Some(m) = &e.part.model {
                reading(ui, "3d model", m.clone());
            }
            if let Some(a) = &e.part.added {
                reading(ui, "added", a.clone());
            }
            reading(ui, "kept in", e.dir.display().to_string());
        },
    );
    ui.add_space(10.0);
    if ui.add(Key::new("reload library").outline(VALUE)).clicked() {
        shelf.reload();
        shelf.sync_fields();
    }
}

fn save_stock(shelf: &mut Shelf, e: &Entry) -> Result<String, String> {
    let count: u64 = shelf.count.trim().parse().map_err(|_| "The count is a whole number.")?;
    let part = agentee_parts::library::set_own(e, count, Some(shelf.bin.trim()))?;
    let bin = part.stock.iter().find_map(|s| s.bin.clone());
    shelf.lib = Some(Library::open(Some(&shelf.root), None));
    shelf.sync_fields();
    Ok(match (count, bin) {
        (0, _) => "Saved: none on the shelf.".to_string(),
        (n, Some(b)) => format!("Saved: {n} in bin {b}."),
        (n, None) => format!("Saved: {n} on the shelf."),
    })
}
