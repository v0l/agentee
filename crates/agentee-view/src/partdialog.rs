use crate::board3d;
use crate::canvas::View;
use crate::paint::{self, Layers, SymbolStyle};
use agentee_core::diag::Diags;
use agentee_core::footprint::{Footprint, FootprintFile};
use agentee_core::symbol::{Symbol, SymbolFile};
use egui::{RichText, Sense, Ui, Vec2};
use egui_bench::prelude::*;
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, mpsc};

pub struct Job {
    rx: mpsc::Receiver<Result<Vec<Value>, String>>,
    what: &'static str,
}

impl Job {
    pub fn start(
        root: &Path,
        steps: Vec<Vec<String>>,
        what: &'static str,
        ctx: &egui::Context,
    ) -> Job {
        let (tx, rx) = mpsc::channel();
        let (root, ctx) = (root.to_path_buf(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(run(&root, &steps));
            ctx.request_repaint();
        });
        Job { rx, what }
    }

    fn poll(&self) -> Option<Result<Vec<Value>, String>> {
        self.rx.try_recv().ok()
    }
}

fn run(root: &Path, steps: &[Vec<String>]) -> Result<Vec<Value>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for args in steps {
        let o = std::process::Command::new(&exe)
            .args(args)
            .arg("-p")
            .arg(root)
            .output()
            .map_err(|e| format!("{}: {e}", exe.display()))?;
        let text = String::from_utf8_lossy(&o.stdout);
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => out.push(v),
            Err(_) if o.status.success() => out.push(Value::String(text.trim().to_string())),
            Err(_) => {
                let err = String::from_utf8_lossy(&o.stderr);
                let err = err.trim().trim_start_matches("error: ");
                return Err(if err.is_empty() { text.trim().to_string() } else { err.to_string() });
            }
        }
    }
    Ok(out)
}

#[derive(Clone, Default)]
pub struct Candidate {
    pub mpn: String,
    pub manufacturer: String,
    pub description: String,
    pub stock: u64,
    pub at: Vec<String>,
    pub datasheet: Option<String>,
    pub price: Option<String>,
}

fn candidates(v: &Value) -> Vec<Candidate> {
    v["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| Candidate {
            mpn: c["mpn"].as_str().unwrap_or_default().to_string(),
            manufacturer: c["manufacturer"].as_str().unwrap_or_default().to_string(),
            description: c["description"].as_str().unwrap_or_default().to_string(),
            stock: c["stock"].as_u64().unwrap_or(0),
            at: c["at"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str()).map(String::from).collect())
                .unwrap_or_default(),
            datasheet: c["datasheet"].as_str().map(String::from),
            price: c["unit"]
                .as_array()
                .and_then(|u| Some(format!("{:.3} {}", u.first()?.as_f64()?, u.get(1)?.as_str()?))),
        })
        .collect()
}

fn search_steps(query: &str) -> Vec<Vec<String>> {
    let mut args = vec!["library".to_string(), "find".to_string()];
    args.extend(query.split_whitespace().map(String::from));
    args.extend(["--search", "--json", "--limit", "25"].map(String::from));
    vec![args]
}

#[derive(Default)]
pub struct Search {
    pub query: String,
    pub results: Vec<Candidate>,
    pub pick: Option<usize>,
    pub errors: Vec<String>,
    pub searched: bool,
}

impl Search {
    fn take(&mut self, v: &Value) {
        self.results = candidates(v);
        self.pick = None;
        self.searched = true;
        self.errors = v["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| e.as_str())
            .map(String::from)
            .collect();
    }

    fn picked(&self) -> Option<&Candidate> {
        self.pick.and_then(|k| self.results.get(k))
    }

    fn show(&mut self, ui: &mut Ui, job: &mut Option<Job>, root: &Path, hint: &str) -> bool {
        let mut picked = false;
        ui.horizontal(|ui| {
            let busy = job.is_some();
            let ready = !busy && !self.query.trim().is_empty();
            let mut go = false;
            let r = field_then(ui, &mut self.query, hint, 110.0, |ui| {
                go = ui.add(Key::new("search").outline(TRACE).enabled(ready)).clicked();
            });
            let entered = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (go || entered) && ready {
                *job = Some(Job::start(root, search_steps(&self.query), "search", ui.ctx()));
            }
        });
        ui.add_space(6.0);
        if job.as_ref().is_some_and(|j| j.what == "search") {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(legend("asking Mouser, Farnell and DigiKey"));
            });
            return false;
        }
        for e in &self.errors {
            note(ui, e.clone(), WARN);
        }
        if self.searched && self.results.is_empty() {
            note(
                ui,
                "No active part in stock matches. Try the maker's part number or a distributor number.",
                LEGEND,
            );
        }
        egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
            for (k, c) in self.results.iter().enumerate() {
                let on = self.pick == Some(k);
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
                let p = ui.painter();
                if on {
                    p.rect_filled(rect, 2.0, READOUT.gamma_multiply(0.14));
                    p.rect_stroke(
                        rect,
                        2.0,
                        egui::Stroke::new(1.0, READOUT),
                        egui::StrokeKind::Inside,
                    );
                } else if resp.hovered() {
                    p.rect_filled(rect, 2.0, BAND);
                }
                let left = rect.left() + 10.0;
                p.text(
                    egui::pos2(left, rect.top() + 5.0),
                    egui::Align2::LEFT_TOP,
                    format!("{}  {}", c.mpn, c.manufacturer),
                    egui::FontId::proportional(13.5),
                    if on { READOUT } else { VALUE },
                );
                let right = format!(
                    "{} in stock{}  {}",
                    c.stock,
                    c.price.as_ref().map(|p| format!(", {p}")).unwrap_or_default(),
                    c.at.join(", ")
                );
                p.text(
                    egui::pos2(rect.right() - 10.0, rect.top() + 5.0),
                    egui::Align2::RIGHT_TOP,
                    right,
                    egui::FontId::proportional(12.0),
                    LEGEND,
                );
                p.with_clip_rect(egui::Rect::from_min_max(
                    rect.min,
                    egui::pos2(rect.right() - 10.0, rect.bottom()),
                ))
                .text(
                    egui::pos2(left, rect.bottom() - 5.0),
                    egui::Align2::LEFT_BOTTOM,
                    &c.description,
                    egui::FontId::proportional(12.0),
                    LEGEND,
                );
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    self.pick = Some(k);
                    picked = true;
                }
                ui.add_space(2.0);
            }
        });
        picked
    }
}

pub struct FixMatch {
    pub name: String,
    pub current: String,
    pub search: Search,
    pub job: Option<Job>,
    pub said: Option<(String, bool)>,
    pub done: bool,
}

impl FixMatch {
    pub fn new(name: &str, mpn: Option<&str>, maker: Option<&str>, notes: &str) -> FixMatch {
        let distributor_number = notes
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-'))
            .find(|w| w.ends_with("-ND"))
            .map(String::from);
        let query = distributor_number
            .or_else(|| mpn.filter(|m| !m.is_empty()).map(String::from))
            .unwrap_or_else(|| name.to_string());
        let current = match (maker, mpn) {
            (Some(m), Some(n)) => format!("{m} {n}"),
            (None, Some(n)) => n.to_string(),
            _ => "no part number".to_string(),
        };
        FixMatch {
            name: name.to_string(),
            current,
            search: Search { query, ..Default::default() },
            job: None,
            said: None,
            done: false,
        }
    }
}

pub enum Closed {
    Open,
    Cancelled,
    Changed(Option<String>),
}

pub fn fix_match(ctx: &egui::Context, root: &Path, f: &mut FixMatch) -> Closed {
    if let Some(r) = f.job.as_ref().and_then(Job::poll) {
        let what = f.job.take().map(|j| j.what).unwrap_or_default();
        match (what, r) {
            ("search", Ok(v)) => f.search.take(v.first().unwrap_or(&Value::Null)),
            ("apply", Ok(v)) => {
                f.done = true;
                f.said = Some((applied(&v), true));
            }
            (_, Err(e)) => f.said = Some((e, false)),
            _ => {}
        }
    }
    if !f.search.searched && f.job.is_none() && !f.done {
        f.job = Some(Job::start(root, search_steps(&f.search.query), "search", ctx));
    }
    let mut out = Closed::Open;
    egui::Modal::new(egui::Id::new("fix-match")).show(ctx, |ui| {
        ui.set_width(720.0);
        ui.label(RichText::new(format!("Fix the part behind {}", f.name)).size(18.0).color(VALUE));
        ui.label(legend(format!("now {}", f.current)));
        ui.add_space(10.0);
        if !f.done {
            f.search.show(ui, &mut f.job, root, "part number, distributor number or words");
        }
        if let Some((t, ok)) = &f.said {
            ui.add_space(6.0);
            note(ui, t.clone(), if *ok { OK } else { FAULT });
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if f.done {
                if ui.add(Key::new("done").solid(READOUT)).clicked() {
                    out = Closed::Changed(None);
                }
                return;
            }
            let busy = f.job.is_some();
            let label = match f.search.picked() {
                Some(c) => format!("use {} {}", c.manufacturer, c.mpn),
                None => "pick a part above".to_string(),
            };
            let apply =
                Key::new(label).solid(READOUT).enabled(f.search.picked().is_some() && !busy);
            if ui.add(apply).clicked()
                && let Some(c) = f.search.picked().cloned()
            {
                let mut set = vec!["library", "set", &f.name, "--mpn", &c.mpn]
                    .into_iter()
                    .map(String::from)
                    .collect::<Vec<_>>();
                if !c.manufacturer.is_empty() {
                    set.extend(["--manufacturer".to_string(), c.manufacturer.clone()]);
                }
                let refresh = vec!["library".into(), "refresh".into(), f.name.clone()];
                let fill = vec!["library".into(), "fill".into(), f.name.clone()];
                f.said = None;
                f.job = Some(Job::start(root, vec![set, refresh, fill], "apply", ui.ctx()));
            }
            if f.job.as_ref().is_some_and(|j| j.what == "apply") {
                ui.spinner();
                ui.label(legend(
                    "writing the part, checking stock, fetching its datasheet and model",
                ));
            }
            if ui.add(Key::new("cancel").outline(VALUE).enabled(!busy)).clicked() {
                out = Closed::Cancelled;
            }
        });
    });
    out
}

fn applied(steps: &[Value]) -> String {
    let fill = steps.last().cloned().unwrap_or(Value::Null);
    let filled: Vec<String> = fill["filled"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|x| x["filled"].as_array().cloned().unwrap_or_default())
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    let missing: Vec<String> = fill["still_missing"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|x| x["missing"].as_array().cloned().unwrap_or_default())
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    let mut t = "Part number updated and stock checked.".to_string();
    if !filled.is_empty() {
        t += &format!(" Added its {}.", filled.join(" and "));
    }
    if !missing.is_empty() {
        t += &format!(" Still no {}.", missing.join(" or "));
    }
    t
}

#[derive(Default)]
pub struct AddPart {
    pub search: Search,
    pub mpn: String,
    pub name: String,
    pub role: String,
    pub manufacturer: String,
    pub value: String,
    pub rating: String,
    pub notes: String,
    pub symbol: String,
    pub footprint: String,
    pub model: String,
    pub datasheet: String,
    pub job: Option<Job>,
    pub assets: [AssetSearch; 4],
    pub preview: Preview,
    pub said: Option<(String, bool)>,
    pub notes_out: Vec<String>,
    pub added: Option<String>,
}

fn row(ui: &mut Ui, label: &str, text: &mut String, hint: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(Vec2::new(LABEL_W, 30.0), egui::Label::new(legend(label)));
        ui.add(
            egui::TextEdit::singleline(text)
                .font(egui::FontId::proportional(FIELD_FONT))
                .margin(egui::Margin::symmetric(8, 6))
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        );
    });
}

#[derive(Clone, Default)]
pub struct Pick {
    pub source: String,
    pub id: String,
    pub title: String,
    pub detail: String,
}

#[derive(Default)]
pub struct Found {
    pub query: String,
    pub hits: Vec<Pick>,
    pub errors: Vec<String>,
    pub done: bool,
}

#[derive(Default)]
pub struct AssetSearch {
    pub found: Found,
    pub job: Option<Job>,
    pub hover: Option<String>,
}

pub struct Loader<T> {
    spec: String,
    value: Option<Result<T, String>>,
    rx: Option<mpsc::Receiver<Result<T, String>>>,
}

impl<T> Default for Loader<T> {
    fn default() -> Self {
        Loader { spec: String::new(), value: None, rx: None }
    }
}

impl<T: Send + 'static> Loader<T> {
    fn want(
        &mut self,
        spec: &str,
        ctx: &egui::Context,
        load: fn(&str) -> Result<T, String>,
    ) -> Option<&Result<T, String>> {
        if let Some(r) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.value = Some(r);
            self.rx = None;
        }
        if spec != self.spec {
            self.spec = spec.to_string();
            self.rx = None;
            if spec.is_empty() {
                self.value = None;
            } else {
                let (tx, rx) = mpsc::channel();
                let (spec, ctx) = (spec.to_string(), ctx.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(load(&spec));
                    ctx.request_repaint();
                });
                self.rx = Some(rx);
            }
        }
        self.value.as_ref()
    }
}

trait ClonedRef<T> {
    fn cloned_ref(self) -> Option<T>;
}

impl<T: Clone> ClonedRef<T> for Option<&Result<T, String>> {
    fn cloned_ref(self) -> Option<T> {
        self.and_then(|r| r.as_ref().ok()).cloned()
    }
}

impl<T> Loader<T> {
    fn error(&self) -> Option<String> {
        self.value.as_ref().and_then(|r| r.as_ref().err().cloned())
    }
}

#[derive(Clone, Copy, PartialEq, Default)]
enum PreviewTab {
    #[default]
    Footprint,
    Model,
    Symbol,
}

#[derive(Default)]
pub struct Preview {
    tab: PreviewTab,
    symbol: Loader<Symbol>,
    footprint: Loader<Footprint>,
    sym_view: View,
    fp_view: View,
    scene: Option<(u64, String, Arc<board3d::Scene>)>,
    orbit: board3d::State,
}

fn load_symbol(spec: &str) -> Result<Symbol, String> {
    let file: SymbolFile = if spec.ends_with(".sym.toml") {
        let text = std::fs::read_to_string(spec).map_err(|e| format!("{spec}: {e}"))?;
        agentee_core::project::parse(&text).map_err(|e| format!("{spec}: {}", e.1))?
    } else if spec.contains(':') {
        agentee_kicad::import_symbol(spec)?
    } else {
        return Err(format!("{spec}: a project symbol has no preview until it is added"));
    };
    let name = file.name.clone();
    Ok(file.resolve(&mut Diags::new(&name)))
}

fn load_footprint(spec: &str) -> Result<Footprint, String> {
    let file: FootprintFile = if spec.ends_with(".fp.toml") {
        let text = std::fs::read_to_string(spec).map_err(|e| format!("{spec}: {e}"))?;
        agentee_core::project::parse(&text).map_err(|e| format!("{spec}: {}", e.1))?
    } else if spec.contains(':') || spec.ends_with(".kicad_mod") {
        agentee_kicad::import_footprint(spec)?
    } else {
        return Err(format!("{spec}: a project footprint has no preview until it is added"));
    };
    let name = file.name.clone();
    let mut fp = file.resolve(&mut Diags::new(&name));
    fp.graphics.retain(|g| !matches!(g.shape, agentee_core::graphic::Shape::Text { .. }));
    Ok(fp)
}

const LABEL_W: f32 = 112.0;
const FIELD_FONT: f32 = 15.0;

const ASSETS: [&str; 4] = ["symbol", "footprint", "model", "datasheet"];

fn asset_steps(asset: &str, query: &str) -> Vec<Vec<String>> {
    let mut a = vec!["library".to_string(), "assets".to_string(), asset.to_string()];
    a.extend(query.split_whitespace().map(String::from));
    a.extend(["--json", "--limit", "20"].map(String::from));
    vec![a]
}

fn take_found(v: &Value, query: &str) -> Found {
    let text = |h: &Value, k: &str| h[k].as_str().unwrap_or_default().to_string();
    Found {
        query: query.to_string(),
        hits: v["hits"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|h| Pick {
                source: text(h, "source"),
                id: text(h, "id"),
                title: text(h, "title"),
                detail: text(h, "detail"),
            })
            .collect(),
        errors: v["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| e.as_str().map(String::from))
            .collect(),
        done: true,
    }
}

fn drain(job: &mut Option<Job>) -> Option<Result<Vec<Value>, String>> {
    let r = job.as_ref().and_then(Job::poll)?;
    job.take();
    Some(r)
}

fn asset_row(
    ui: &mut Ui,
    label: &str,
    asset: &'static str,
    text: &mut String,
    hint: &str,
    fallback: &str,
    search: &mut AssetSearch,
    root: &Path,
) {
    let query =
        if text.trim().is_empty() { fallback.trim().to_string() } else { text.trim().to_string() };
    let busy = search.job.is_some();
    let ready = !query.is_empty() && !busy;
    let mut go = false;
    ui.horizontal(|ui| {
        ui.add_sized(Vec2::new(LABEL_W, 30.0), egui::Label::new(legend(label)));
        let r = ui.add(
            egui::TextEdit::singleline(text)
                .font(egui::FontId::proportional(FIELD_FONT))
                .margin(egui::Margin::symmetric(8, 6))
                .hint_text(hint)
                .desired_width(ui.available_width() - 104.0),
        );
        let entered = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let clicked = ui.add(Key::new("search").outline(TRACE).enabled(ready)).clicked();
        go = (clicked || entered) && ready;
        if busy {
            ui.spinner();
        }
    });
    if go {
        let q = if text.trim().is_empty() {
            fallback.trim().to_string()
        } else {
            text.trim().to_string()
        };
        search.found = Found { query: q.clone(), ..Default::default() };
        search.job = Some(Job::start(root, asset_steps(asset, &q), asset, ui.ctx()));
    }
    search.hover = None;
    if let Some(p) = results(ui, &search.found, text, asset, &mut search.hover) {
        *text = p.id;
        search.found.done = false;
    }
}

fn results(
    ui: &mut Ui,
    found: &Found,
    current: &str,
    asset: &str,
    hover: &mut Option<String>,
) -> Option<Pick> {
    if !found.done {
        return None;
    }
    let mut pick = None;
    egui::Frame::NONE
        .inner_margin(egui::Margin { left: LABEL_W as i8 + 8, ..Default::default() })
        .show(ui, |ui| {
            for e in &found.errors {
                note(ui, e.clone(), WARN);
            }
            if found.hits.is_empty() {
                note(
                    ui,
                    format!(
                        "No {asset} found for \"{}\". Try fewer or different words.",
                        found.query
                    ),
                    LEGEND,
                );
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt(asset)
                .min_scrolled_height(280.0)
                .max_height(320.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for h in &found.hits {
                        let on = h.id == current;
                        let (rect, resp) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 34.0),
                            Sense::click(),
                        );
                        let p = ui.painter().with_clip_rect(rect);
                        if on {
                            p.rect_filled(rect, 2.0, READOUT.gamma_multiply(0.14));
                        } else if resp.hovered() {
                            p.rect_filled(rect, 2.0, BAND);
                        }
                        let font = egui::FontId::proportional(13.0);
                        p.text(
                            egui::pos2(rect.left() + 8.0, rect.top() + 4.0),
                            egui::Align2::LEFT_TOP,
                            &h.title,
                            font,
                            if on { READOUT } else { VALUE },
                        );
                        p.text(
                            egui::pos2(rect.right() - 8.0, rect.top() + 4.0),
                            egui::Align2::RIGHT_TOP,
                            &h.source,
                            egui::FontId::proportional(11.5),
                            LEGEND,
                        );
                        p.text(
                            egui::pos2(rect.left() + 8.0, rect.bottom() - 4.0),
                            egui::Align2::LEFT_BOTTOM,
                            &h.detail,
                            egui::FontId::proportional(11.5),
                            LEGEND,
                        );
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            if asset == ASSETS[2] {
                                *hover = Some(h.id.clone());
                            }
                        }
                        if resp.clicked() {
                            pick = Some(h.clone());
                        }
                    }
                });
        });
    ui.add_space(4.0);
    pick
}

pub fn add_part(ctx: &egui::Context, root: &Path, roles: &[String], f: &mut AddPart) -> Closed {
    if let Some(r) = f.job.as_ref().and_then(Job::poll) {
        let what = f.job.take().map(|j| j.what).unwrap_or_default();
        match (what, r) {
            ("search", Ok(v)) => f.search.take(v.first().unwrap_or(&Value::Null)),
            ("add", Ok(v)) => {
                let v = v.first().cloned().unwrap_or(Value::Null);
                f.added = v["added"]["name"].as_str().map(String::from);
                f.notes_out = v["notes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|n| n.as_str().map(String::from))
                    .collect();
                f.said = Some((format!("Added {}.", f.added.clone().unwrap_or_default()), true));
            }
            (_, Err(e)) => f.said = Some((e, false)),
            _ => {}
        }
    }
    for search in f.assets.iter_mut() {
        if let Some(r) = drain(&mut search.job) {
            match r {
                Ok(v) => {
                    let q = search.found.query.clone();
                    search.found = take_found(v.first().unwrap_or(&Value::Null), &q);
                }
                Err(e) => {
                    search.found = Found { errors: vec![e], done: true, ..Default::default() };
                }
            }
        }
    }
    let mut out = Closed::Open;
    egui::Modal::new(egui::Id::new("add-part")).show(ctx, |ui| {
        ui.set_width((ctx.content_rect().width() - 120.0).clamp(900.0, 1500.0));
        ui.label(RichText::new("Add a part to the library").size(22.0).color(VALUE));
        ui.add_space(8.0);
        if f.added.is_some() {
            if let Some((t, ok)) = &f.said {
                note(ui, t.clone(), if *ok { OK } else { FAULT });
            }
            for n in &f.notes_out {
                note(ui, n.clone(), LEGEND);
            }
            ui.add_space(10.0);
            if ui.add(Key::new("done").solid(READOUT)).clicked() {
                out = Closed::Changed(f.added.clone());
            }
            return;
        }
        let total = ui.available_width();
        let height = (ctx.content_rect().height() - 220.0).clamp(520.0, 860.0);
        let left_w = (total * 0.58).max(520.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(left_w, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical().id_salt("add-part-form").auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_width(left_w - 16.0);
        ui.label(legend("find it at the distributors to fill the part number, or type it in"));
        ui.add_space(4.0);
        if f.search.show(ui, &mut f.job, root, "part number, distributor number or words")
            && let Some(c) = f.search.picked().cloned()
        {
            f.mpn = c.mpn.clone();
            f.manufacturer = c.manufacturer.clone();
            if f.notes.is_empty() {
                f.notes = c.description.clone();
            }
            if let Some(d) = c.datasheet {
                f.datasheet = d;
            }
        }
        ui.add_space(10.0);
        row(ui, "part number", &mut f.mpn, "the maker's part number, required");
        row(ui, "maker", &mut f.manufacturer, "who makes it");
        row(ui, "role", &mut f.role, "resistor, capacitor, connector, regulator ..., required");
        if !roles.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(LABEL_W + 8.0);
                for r in roles.iter().take(18) {
                    if chip(ui, r, if *r == f.role { READOUT } else { LEGEND }, *r == f.role)
                        .interact(Sense::click())
                        .clicked()
                    {
                        f.role = r.clone();
                    }
                }
            });
        }
        ui.add_space(6.0);
        let (role, mpn) = (f.role.clone(), f.mpn.clone());
        let [sym, fp, model, sheet] = &mut f.assets;
        asset_row(
            ui,
            "symbol",
            ASSETS[0],
            &mut f.symbol,
            "search symbols, or a .sym.toml file, required",
            &role,
            sym,
            root,
        );
        asset_row(
            ui,
            "footprint",
            ASSETS[1],
            &mut f.footprint,
            "blank: the symbol's. Search footprints",
            &role,
            fp,
            root,
        );
        asset_row(
            ui,
            "3d model",
            ASSETS[2],
            &mut f.model,
            "blank: the footprint's. Search models, or a STEP file, URL or ZIP",
            &mpn,
            model,
            root,
        );
        asset_row(
            ui,
            "datasheet",
            ASSETS[3],
            &mut f.datasheet,
            "blank: the distributors'. Search datasheets, or a URL or PDF",
            &mpn,
            sheet,
            root,
        );
        ui.add_space(4.0);
        egui::CollapsingHeader::new(legend("value, rating, notes and library name"))
            .id_salt("add-part-details")
            .default_open(false)
            .show(ui, |ui| {
                row(ui, "value", &mut f.value, "100n, 10k, red");
                row(ui, "rating", &mut f.rating, "2 A, 100 V");
                row(ui, "notes", &mut f.notes, "what it is for");
                row(ui, "library name", &mut f.name, "default the part number");
            });

                    });
                },
            );
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| preview_pane(ui, f),
            );
        });
        if let Some((t, ok)) = &f.said {
            ui.add_space(6.0);
            note(ui, t.clone(), if *ok { OK } else { FAULT });
        }
        ui.add_space(10.0);
        let missing: Vec<&str> =
            [("part number", &f.mpn), ("role", &f.role), ("symbol", &f.symbol)]
                .into_iter()
                .filter(|(_, v)| v.trim().is_empty())
                .map(|(k, _)| k)
                .collect();
        ui.horizontal(|ui| {
            let busy = f.job.is_some();
            let add = Key::new("add part").solid(READOUT).enabled(missing.is_empty() && !busy);
            let add = ui.add(add);
            let add = if missing.is_empty() {
                add
            } else {
                add.on_disabled_hover_text(format!("give the {}", missing.join(", ")))
            };
            if add.clicked() {
                f.said = None;
                f.job = Some(Job::start(root, vec![add_args(f)], "add", ui.ctx()));
            }
            if f.job.as_ref().is_some_and(|j| j.what == "add") {
                ui.spinner();
                ui.label(legend("copying the files, fetching the datasheet, checking stock"));
            }
            if ui.add(Key::new("cancel").outline(VALUE).enabled(!busy)).clicked() {
                out = Closed::Cancelled;
            }
        });

    });
    out
}

fn previewing(f: &AddPart) -> String {
    f.symbol.trim().to_string()
}

fn preview_pane(ui: &mut Ui, f: &mut AddPart) {
    let symbol_spec = previewing(f);
    let fp_spec = f.footprint.trim().to_string();
    let model_hover = f.assets[2].hover.clone();
    let ctx = ui.ctx().clone();
    let pv = &mut f.preview;
    let sym = pv.symbol.want(&symbol_spec, &ctx, load_symbol).cloned_ref();
    let fp = pv.footprint.want(&fp_spec, &ctx, load_footprint).cloned_ref();
    let mut options = Vec::new();
    if fp.is_some() {
        options.push((PreviewTab::Footprint, "footprint", None));
        options.push((PreviewTab::Model, "3d", None));
    }
    if sym.is_some() {
        options.push((PreviewTab::Symbol, "symbol", None));
    }
    if !options.iter().any(|o| o.0 == pv.tab)
        && let Some(o) = options.first()
    {
        pv.tab = o.0;
    }
    if options.is_empty() {
        let note_text = match (pv.symbol.error(), pv.footprint.error()) {
            (Some(e), _) | (_, Some(e)) => e,
            _ => "Search or type a symbol and a footprint to preview the part here.".to_string(),
        };
        let size = ui.available_size();
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter().rect_filled(rect, 2.0, WELL);
        ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, ETCH), egui::StrokeKind::Inside);
        let galley = ui.painter().layout(
            note_text,
            egui::FontId::proportional(14.0),
            LEGEND,
            rect.width() - 40.0,
        );
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, LEGEND);
        return;
    }
    segments(ui, &mut pv.tab, &options);
    ui.add_space(6.0);
    let height = ui.available_height() - 4.0;
    let size = Vec2::new(ui.available_width(), height.max(200.0));
    ui.allocate_ui(size, |ui| match pv.tab {
        PreviewTab::Footprint => {
            let Some(fp) = &fp else { return };
            pv.fp_view.max_fit = 2000.0;
            let (resp, xf) = pv.fp_view.show(ui, &fp.bounds(), 50.0);
            let p = ui.painter_at(xf.rect);
            paint::grid(&p, &xf, 0.5);
            paint::footprint(&p, &xf, fp, &Layers::default(), resp.hover_pos());
            let b = fp.bounds().size();
            p.text(
                xf.rect.left_bottom() + Vec2::new(10.0, -10.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{}, {:.2} x {:.2} mm, {} pads", fp.name, b[0], b[1], fp.pads.len()),
                egui::FontId::proportional(13.0),
                LEGEND,
            );
        }
        PreviewTab::Model => {
            let Some(fp) = &fp else { return };
            let generation = agentee_3d::generation();
            let key = format!("{fp_spec}|{}", model_hover.clone().unwrap_or_default());
            let stale = pv
                .scene
                .as_ref()
                .is_none_or(|(g, k, s)| *g != generation || *k != key || s.pending > 0);
            if stale {
                let mut shown = fp.clone();
                if let Some(m) =
                    model_hover.as_deref().or(Some(f.model.trim()).filter(|m| !m.is_empty()))
                    && !m.starts_with("easyeda:")
                {
                    shown.model = Some(m.to_string());
                }
                let scene =
                    board3d::part_scene(&shown, Path::new("."), agentee_3d::Fetch::Background);
                pv.scene = Some((generation, key, Arc::new(scene)));
            }
            if let Some((_, _, scene)) = &pv.scene {
                board3d::show(ui, scene, &mut pv.orbit, true, true);
                if scene.pending > 0 {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
                }
            }
        }
        PreviewTab::Symbol => {
            let Some(s) = &sym else { return };
            let padded = crate::pages::symbol_bounds(s, 1);
            pv.sym_view.max_fit = 45.0;
            let (resp, xf) = pv.sym_view.show(ui, &padded, 40.0);
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
        }
    });
}

fn add_args(f: &AddPart) -> Vec<String> {
    let mut a: Vec<String> = vec!["library".into(), "add".into(), f.mpn.trim().into()];
    let mut opt = |flag: &str, v: &String| {
        if !v.trim().is_empty() {
            a.push(flag.into());
            a.push(v.trim().into());
        }
    };
    opt("--role", &f.role);
    opt("--symbol", &f.symbol);
    opt("--footprint", &f.footprint);
    opt("--model", &f.model);
    opt("--datasheet", &f.datasheet);
    opt("--manufacturer", &f.manufacturer);
    opt("--value", &f.value);
    opt("--rating", &f.rating);
    opt("--notes", &f.notes);
    opt("--name", &f.name);
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_reply_reads_into_candidates() {
        let v = serde_json::json!({ "candidates": [{
            "mpn": "150080BS75000", "manufacturer": "Wurth Elektronik",
            "description": "LED 0805 blue", "stock": 1200, "unit": [0.21, "EUR"],
            "at": ["Mouser", "Farnell"], "datasheet": "https://x/150080BS75000.pdf"
        }]});
        let c = &candidates(&v)[0];
        assert_eq!(
            (c.mpn.as_str(), c.stock, c.price.as_deref()),
            ("150080BS75000", 1200, Some("0.210 EUR"))
        );
        assert_eq!(c.at, vec!["Mouser", "Farnell"]);
    }

    #[test]
    fn the_add_form_passes_only_what_is_filled() {
        let f = AddPart {
            mpn: " 150080BS75000 ".into(),
            role: "led".into(),
            symbol: "Device:LED".into(),
            notes: "blue status".into(),
            ..Default::default()
        };
        assert_eq!(
            add_args(&f),
            [
                "library",
                "add",
                "150080BS75000",
                "--role",
                "led",
                "--symbol",
                "Device:LED",
                "--notes",
                "blue status"
            ]
        );
    }

    #[test]
    fn fix_match_searches_the_digikey_number_first() {
        let f = FixMatch::new("LED-Red", Some("73249841"), None, "WURTH (DigiKey 732-4984-1-ND)");
        assert_eq!(f.search.query, "732-4984-1-ND");
        let f = FixMatch::new("LED-Red", Some("73249841"), None, "");
        assert_eq!(f.search.query, "73249841");
    }

    #[test]
    fn an_asset_search_reply_reads_into_picks() {
        let v = serde_json::json!({
            "hits": [{ "source": "KiCad", "id": "Device:D_Zener", "title": "D_Zener", "detail": "Device" }],
            "errors": ["EasyEDA: offline"]
        });
        let f = take_found(&v, "zener");
        assert_eq!((f.hits[0].id.as_str(), f.errors.len(), f.done), ("Device:D_Zener", 1, true));
        assert_eq!(
            asset_steps("model", "zener sod-123")[0],
            ["library", "assets", "model", "zener", "sod-123", "--json", "--limit", "20"]
        );
    }
}
