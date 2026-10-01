use agentee_core::project::Project;
use egui::Ui;
use egui_bench::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

pub struct Form {
    pub name: String,
    pub board: String,
    pub schematic: String,
    pub place: bool,
    pub seed: u64,
    pub error: Option<String>,
    job: Option<(Instant, mpsc::Receiver<Result<String, String>>)>,
}

pub enum Outcome {
    Open,
    Cancelled,
    Created(String),
}

impl Form {
    pub fn new(project: &Project) -> Form {
        let schematic = tops(project).first().cloned().unwrap_or_default();
        let board = project.boards.first().map(|b| b.name.clone()).unwrap_or_default();
        let taken = |n: &str| project.layouts.iter().any(|l| l.name == n);
        let name = (1..)
            .map(|k| if k == 1 { schematic.clone() } else { format!("{schematic}-{k}") })
            .find(|n| !n.is_empty() && !taken(n))
            .unwrap_or_else(|| "layout".into());
        Form { name, board, schematic, place: true, seed: 1, error: None, job: None }
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}

fn tops(project: &Project) -> Vec<String> {
    project.schematics.iter().filter(|s| s.item.parent.is_none()).map(|s| s.name.clone()).collect()
}

fn file_stem(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' })
        .collect()
}

pub fn starter(name: &str, board: &str, schematic: &str) -> String {
    let mut doc = toml_edit::DocumentMut::new();
    doc["name"] = toml_edit::value(name);
    doc["board"] = toml_edit::value(board);
    doc["schematic"] = toml_edit::value(schematic);
    doc.to_string()
}

pub fn create(
    project: &Project,
    root: &Path,
    f: &Form,
) -> Result<(PathBuf, String, Option<agentee_core::project::LayoutInputs>), String> {
    let name = f.name.trim();
    if name.is_empty() {
        return Err("name the layout".into());
    }
    if project.layouts.iter().any(|l| l.name == name) {
        return Err(format!("a layout named `{name}` exists"));
    }
    let path =
        root.join(format!("{}{}", file_stem(name), agentee_core::project::Kind::Layout.ext()));
    if path.exists() {
        return Err(format!("{} exists", path.display()));
    }
    let text = starter(name, &f.board, &f.schematic);
    let inputs =
        if f.place { Some(project.inputs_for(&path, &f.board, &f.schematic)?) } else { None };
    Ok((path, text, inputs))
}

pub fn show(ctx: &egui::Context, project: &Project, root: &Path, f: &mut Form) -> Outcome {
    if let Some((_, rx)) = &f.job
        && let Ok(r) = rx.try_recv()
    {
        f.job = None;
        match r {
            Ok(name) => return Outcome::Created(name),
            Err(e) => f.error = Some(e),
        }
    }
    let mut out = Outcome::Open;
    egui::Modal::new(egui::Id::new("new-layout")).show(ctx, |ui: &mut Ui| {
        ui.set_width(380.0);
        modal_title(ui, "new layout");
        let busy = f.job.is_some();
        ui.add_enabled_ui(!busy, |ui| {
            row(ui, "name", |ui| {
                field(ui, &mut f.name, "name");
            });
            row(ui, "board", |ui| {
                choice(
                    ui,
                    "new-board",
                    &mut f.board,
                    project.boards.iter().map(|b| (b.name.clone(), b.name.clone())),
                );
            });
            row(ui, "schematic", |ui| {
                choice(
                    ui,
                    "new-sch",
                    &mut f.schematic,
                    tops(project).into_iter().map(|s| (s.clone(), s)),
                );
            });
            ui.checkbox(&mut f.place, "place every part with the placer");
            if f.place {
                row(ui, "seed", |ui| {
                    ui.add(egui::DragValue::new(&mut f.seed).range(1..=1_000_000));
                });
            }
        });
        if let Some((t, _)) = &f.job {
            readouts(ui, &[("placing", format!("{:.0} s", t.elapsed().as_secs_f32()), READOUT)]);
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if let Some(e) = &f.error {
            status(ui, false, e);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new("create")).clicked() {
                match create(project, root, f) {
                    Ok((path, text, inputs)) => {
                        f.error = None;
                        let (tx, rx) = mpsc::channel();
                        let wake = ctx.clone();
                        let name = f.name.trim().to_string();
                        let seed = f.seed;
                        std::thread::spawn(move || {
                            let r = write(&path, &text, inputs.as_ref(), seed).map(|_| name);
                            let _ = tx.send(r);
                            wake.request_repaint();
                        });
                        f.job = Some((Instant::now(), rx));
                    }
                    Err(e) => f.error = Some(e),
                }
            }
            if ui.add_enabled(!busy, egui::Button::new("cancel")).clicked() {
                out = Outcome::Cancelled;
            }
        });
    });
    out
}

pub fn write(
    path: &Path,
    text: &str,
    inputs: Option<&agentee_core::project::LayoutInputs>,
    seed: u64,
) -> Result<(), String> {
    let text = match inputs {
        Some(inputs) => {
            let opts = agentee_core::place::PlaceOptions { seed, ..Default::default() };
            agentee_layout::start::place_text(inputs, text, &opts)?.0
        }
        None => text.to_string(),
    };
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}
