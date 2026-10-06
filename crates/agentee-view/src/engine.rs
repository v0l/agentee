use crate::edit::Editor;
use agentee_core::engine::PHASES;
use agentee_core::project::Project;
use agentee_core::units::{Length, trim};
use agentee_layout::pinswap::PinSwap;
use agentee_layout::score::Score;
use agentee_layout::start::Reset;
use agentee_layout::{Event, PhaseReport, Run, RunReport};
use egui::Ui;
use egui_bench::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Instant;
use toml_edit::{Item, Value};

enum Msg {
    Start(String),
    Done(PhaseReport),
    Finished(Box<Result<RunReport, String>>),
}

struct Job {
    rx: mpsc::Receiver<Msg>,
    stop: Arc<AtomicBool>,
    started: Instant,
    phase: Option<(String, Instant)>,
    done: Vec<PhaseReport>,
    base: String,
}

struct Last {
    phases: Vec<PhaseReport>,
    score: Score,
    skipped: Vec<String>,
    seconds: f32,
}

type Swapped = (String, Result<PinSwap, String>);
type TextJob = (String, Instant, mpsc::Receiver<Result<String, String>>);

#[derive(Default)]
pub struct Panel {
    pub from: Option<String>,
    pub to: Option<String>,
    job: Option<Job>,
    last: Option<Last>,
    error: Option<String>,
    swap_job: Option<mpsc::Receiver<Swapped>>,
    swap: Option<Swapped>,
    swap_note: Option<String>,
    text_job: Option<TextJob>,
    pub reset: Option<ResetForm>,
}

#[derive(Clone, Copy)]
pub struct ResetForm {
    pub what: Reset,
    pub place: bool,
    pub seed: u64,
}

impl Default for ResetForm {
    fn default() -> Self {
        ResetForm {
            what: Reset { routing: true, via_rules: false, zones: false, labels: false },
            place: false,
            seed: 1,
        }
    }
}

#[derive(Clone, Copy)]
enum Knob {
    Mm(f64),
    Int(i64),
    Flag(bool),
}

const KNOBS: &[(&str, &str, Knob, &str)] = &[
    ("", "rounds", Knob::Int(3), "global and detail repetitions"),
    ("", "place_rounds", Knob::Int(3), "placement passes driven by hot tiles"),
    ("place", "seed", Knob::Int(1), "seed of the placer, try others for other layouts"),
    ("access", "via_in_pad", Knob::Flag(false), "allow filled vias in BGA and SMD pads"),
    ("global", "tile", Knob::Mm(1.0), "tile of the global route"),
    ("global", "rounds", Knob::Int(30), "negotiation rounds of the global route"),
    ("global", "via_cost", Knob::Mm(1.0), "track length a layer change costs in the global route"),
    ("detail", "grid", Knob::Mm(0.05), "routing grid"),
    ("detail", "rounds", Knob::Int(30), "negotiation rounds of the detail route"),
    ("detail", "via_cost", Knob::Mm(3.0), "track length a via costs"),
    ("detail", "bend_cost", Knob::Mm(0.1), "track length a 45 degree bend costs"),
    ("detail", "fences", Knob::Flag(true), "keep foreign nets out of BGA fields"),
];

pub fn running(ed: &Editor) -> bool {
    ed.engine.job.is_some() || ed.engine.text_job.is_some()
}

pub fn reset(ed: &mut Editor, ctx: &egui::Context, project: &Project, i: usize, form: ResetForm) {
    let text = match agentee_layout::start::reset(&ed.text(), &form.what) {
        Ok(t) => t,
        Err(e) => {
            ed.engine.error = Some(e);
            return;
        }
    };
    if !form.place {
        ed.replace_text(ctx, project, i, &text);
        return;
    }
    let inputs = match ed.inputs(project, i) {
        Ok(x) => x,
        Err(e) => {
            ed.engine.error = Some(e);
            return;
        }
    };
    let (tx, rx) = mpsc::channel();
    let wake = ctx.clone();
    std::thread::spawn(move || {
        let opts = agentee_core::place::PlaceOptions { seed: form.seed, ..Default::default() };
        let r = agentee_layout::start::place_text(&inputs, &text, &opts).map(|(t, _)| t);
        let _ = tx.send(r);
        wake.request_repaint();
    });
    ed.engine.error = None;
    ed.engine.text_job = Some(("placing".into(), Instant::now(), rx));
}

pub fn reset_dialog(ui: &mut Ui, project: &Project, i: usize, ed: &mut Editor) {
    let Some(mut form) = ed.engine.reset else { return };
    let ctx = ui.ctx().clone();
    let mut apply = false;
    let mut close = false;
    egui::Modal::new(egui::Id::new(("reset-layout", i))).show(&ctx, |ui| {
        ui.set_width(380.0);
        modal_title(ui, "reset layout");
        let w = &mut form.what;
        ui.checkbox(&mut w.routing, "routing: tracks, vias and the engine's plans");
        ui.checkbox(&mut w.via_rules, "via rules: [[fanouts]] and [[stitching]]");
        ui.checkbox(&mut w.zones, "zones and their stored fills");
        ui.checkbox(&mut w.labels, "silk label positions");
        ui.checkbox(&mut form.place, "placement: place every unlocked part again");
        if form.place {
            row(ui, "seed", |ui| {
                ui.add(egui::DragValue::new(&mut form.seed).range(1..=1_000_000));
            });
        }
        ui.add_space(6.0);
        note(ui, "the result is unsaved in the window, ctrl+Z takes it back", LEGEND);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let any = form.place || form.what != Reset::default();
            if ui.add_enabled(any, egui::Button::new("reset")).clicked() {
                apply = true;
            }
            if ui.button("cancel").clicked() {
                close = true;
            }
        });
    });
    ed.engine.reset = if apply || close { None } else { Some(form) };
    if apply {
        reset(ed, &ctx, project, i, form);
    }
}

pub fn start(
    ed: &mut Editor,
    ctx: &egui::Context,
    project: &Project,
    i: usize,
    from: Option<String>,
    to: Option<String>,
    only: Option<String>,
) {
    let inputs = match ed.inputs(project, i) {
        Ok(x) => x,
        Err(e) => {
            ed.engine.error = Some(e);
            return;
        }
    };
    let text = ed.text();
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let halt = stop.clone();
    let wake = ctx.clone();
    let base = text.clone();
    std::thread::spawn(move || {
        let send = |m: Msg| {
            let _ = tx.send(m);
            wake.request_repaint();
        };
        let watch = |e: Event| match e {
            Event::Start(n) => send(Msg::Start(n.to_string())),
            Event::Done(r) => send(Msg::Done(r.clone())),
        };
        let run = Run { from, to, only, watch: Some(&watch), stop: Some(&halt) };
        send(Msg::Finished(Box::new(agentee_layout::run_text(&inputs, &text, &run))));
    });
    ed.engine.error = None;
    ed.engine.job =
        Some(Job { rx, stop, started: Instant::now(), phase: None, done: Vec::new(), base });
}

pub fn poll(ed: &mut Editor, ctx: &egui::Context, project: &Project, i: usize) {
    if let Some((_, _, rx)) = &ed.engine.text_job {
        match rx.try_recv() {
            Ok(r) => {
                ed.engine.text_job = None;
                match r {
                    Ok(t) => ed.replace_text(ctx, project, i, &t),
                    Err(e) => ed.engine.error = Some(e),
                }
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
            Err(mpsc::TryRecvError::Disconnected) => ed.engine.text_job = None,
        }
    }
    if let Some(rx) = &ed.engine.swap_job
        && let Ok(r) = rx.try_recv()
    {
        ed.engine.swap_job = None;
        ed.engine.swap = Some(r);
    }
    let mut finished = None;
    if let Some(job) = &mut ed.engine.job {
        while let Ok(m) = job.rx.try_recv() {
            match m {
                Msg::Start(n) => job.phase = Some((n, Instant::now())),
                Msg::Done(r) => {
                    job.phase = None;
                    job.done.push(r);
                }
                Msg::Finished(r) => finished = Some(*r),
            }
        }
        if finished.is_none() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
    let Some(r) = finished else { return };
    let Some(job) = ed.engine.job.take() else { return };
    match r {
        Ok(r) => {
            if r.text != job.base {
                ed.replace_text(ctx, project, i, &r.text);
            }
            ed.engine.last = Some(Last {
                phases: r.phases,
                score: r.score,
                skipped: r.skipped,
                seconds: job.started.elapsed().as_secs_f32(),
            });
        }
        Err(e) => ed.engine.error = Some(e),
    }
}

pub fn panel(ui: &mut Ui, project: &Project, i: usize, ed: &mut Editor) {
    let ctx = ui.ctx().clone();
    let configured = ed.layout(project, i).engine.phases();
    let busy = running(ed);
    card_frame(ui, busy, |ui| {
        let mut from = ed.engine.from.clone().unwrap_or_else(|| configured[0].clone());
        let mut to = ed.engine.to.clone().unwrap_or_else(|| configured.last().cloned().unwrap());
        if !configured.contains(&from) {
            from = configured[0].clone();
        }
        if !configured.contains(&to) {
            to = configured.last().cloned().unwrap_or_default();
        }
        let options = || configured.iter().map(|p| (p.clone(), p.clone()));
        row(ui, "from", |ui| {
            choice(ui, "engine-from", &mut from, options());
        });
        row(ui, "to", |ui| {
            choice(ui, "engine-to", &mut to, options());
        });
        ed.engine.from = Some(from.clone());
        ed.engine.to = Some(to.clone());
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("run"))
                .on_hover_text("the phases from..to, the result lands in the editor unsaved")
                .clicked()
            {
                start(ed, &ctx, project, i, Some(from.clone()), Some(to.clone()), None);
            }
            if ui.add_enabled(!busy, egui::Button::new(format!("run {from} only"))).clicked() {
                start(ed, &ctx, project, i, None, None, Some(from.clone()));
            }
            if let Some(job) = &ed.engine.job
                && ui
                    .button("stop")
                    .on_hover_text("stops after the phase that is running")
                    .clicked()
            {
                job.stop.store(true, Ordering::Relaxed);
            }
        });
        ui.horizontal(|ui| {
            let planes = !agentee_core::tie::plane_nets(ed.layout(project, i)).is_empty();
            if ui
                .add_enabled(!busy && planes, egui::Button::new("tie plane pads"))
                .on_hover_text("a stub and a via beside every SMD pad of a net with a zone")
                .on_disabled_hover_text("no net has a zone yet")
                .clicked()
            {
                ed.tie_planes(&ctx, project, i, &[]);
            }
        });
        if let Some(job) = &ed.engine.job {
            let now = match &job.phase {
                Some((p, t)) => format!("{p}  {:.0} s", t.elapsed().as_secs_f32()),
                None => "starting".into(),
            };
            readouts(
                ui,
                &[
                    ("running", now, READOUT),
                    ("total", format!("{:.0} s", job.started.elapsed().as_secs_f32()), VALUE),
                ],
            );
            phase_table(ui, &job.done);
        }
        if let Some((what, t, _)) = &ed.engine.text_job {
            readouts(
                ui,
                &[(what.as_str(), format!("{:.0} s", t.elapsed().as_secs_f32()), READOUT)],
            );
        }
        if let Some(e) = &ed.engine.error {
            status(ui, false, e);
        }
        if let Some(last) = &ed.engine.last
            && !busy
        {
            ui.add_space(4.0);
            Line::new()
                .legend("last run")
                .value(format!("{:.1} s, score {}", last.seconds, trim(last.score.total, 1)))
                .show(ui);
            egui::CollapsingHeader::new(legend("phase times")).id_salt("engine-times").show(
                ui,
                |ui| {
                    phase_table(ui, &last.phases);
                    for s in &last.skipped {
                        note(ui, s, LEGEND);
                    }
                },
            );
            egui::CollapsingHeader::new(legend("score terms"))
                .id_salt("engine-score")
                .show(ui, |ui| score_table(ui, &last.score));
        }
        ui.add_space(4.0);
        egui::CollapsingHeader::new(legend("phases to run")).id_salt("engine-phases").show(
            ui,
            |ui| {
                phases(ui, &ctx, project, i, ed, &configured);
            },
        );
        egui::CollapsingHeader::new(legend("settings")).id_salt("engine-knobs").show(ui, |ui| {
            knobs(ui, &ctx, project, i, ed);
        });
    });
}

fn card_frame(ui: &mut Ui, busy: bool, body: impl FnOnce(&mut Ui)) {
    card(
        ui,
        Some(if busy { READOUT } else { TRACE }),
        |ui| {
            Line::new().legend("layout engine").value("place and route").show(ui);
        },
        body,
    );
}

fn phase_table(ui: &mut Ui, phases: &[PhaseReport]) {
    if phases.is_empty() {
        return;
    }
    let cols = [("phase", 90.0), ("time", 60.0), ("score", 80.0), ("failed", 60.0)];
    Table::new(&cols, phases.len()).show(ui, |k, p, r, at| {
        let ph = &phases[k];
        let col = if ph.changed { VALUE } else { LEGEND };
        cell(p, r, at(0), cols[0].1, &ph.phase, col);
        cell(p, r, at(1), cols[1].1, &format!("{:.1} s", ph.ms as f32 / 1000.0), TRACE);
        let score = ph.score.as_ref().map(|s| trim(s.total, 1)).unwrap_or_default();
        cell(p, r, at(2), cols[2].1, &score, READOUT);
        let (n, c) = if ph.failed.is_empty() {
            ("".into(), OK)
        } else {
            (ph.failed.len().to_string(), FAULT)
        };
        cell(p, r, at(3), cols[3].1, &n, c);
    });
    let notes: Vec<String> = phases
        .iter()
        .flat_map(|ph| {
            let failed = ph.failed.iter().map(move |f| format!("{}: {f}", ph.phase));
            let notes = ph.notes.iter().map(move |n| format!("{}: {n}", ph.phase));
            failed.chain(notes)
        })
        .collect();
    if !notes.is_empty() {
        egui::CollapsingHeader::new(legend(format!("{} notes", notes.len())))
            .id_salt(("engine-notes", phases.len()))
            .show(ui, |ui| {
                for n in notes.iter().take(60) {
                    Line::new().value(n).size(11.0).wrapped(ui);
                }
            });
    }
}

fn score_table(ui: &mut Ui, score: &Score) {
    let mut terms: Vec<(&String, &agentee_layout::score::Term)> =
        score.terms.iter().filter(|(_, t)| t.measured && t.weighted != 0.0).collect();
    terms.sort_by(|a, b| b.1.weighted.total_cmp(&a.1.weighted));
    if terms.is_empty() {
        return;
    }
    let cols = [("term", 100.0), ("raw", 70.0), ("weight", 60.0), ("score", 80.0)];
    Table::new(&cols, terms.len().min(8)).show(ui, |k, p, r, at| {
        let (name, t) = terms[k];
        cell(p, r, at(0), cols[0].1, name, VALUE);
        cell(p, r, at(1), cols[1].1, &trim(t.raw, 2), TRACE);
        cell(p, r, at(2), cols[2].1, &trim(t.weight, 2), LEGEND);
        cell(p, r, at(3), cols[3].1, &trim(t.weighted, 1), READOUT);
    });
}

fn phases(
    ui: &mut Ui,
    ctx: &egui::Context,
    project: &Project,
    i: usize,
    ed: &mut Editor,
    configured: &[String],
) {
    let mut on: Vec<String> = configured.to_vec();
    ui.horizontal_wrapped(|ui| {
        for p in PHASES {
            let lit = on.iter().any(|x| x == p);
            if toggle(ui, p, lit).clicked() {
                if lit {
                    on.retain(|x| x != p);
                } else {
                    on.push(p.to_string());
                }
            }
        }
    });
    if on != configured && !on.is_empty() {
        let ordered: Vec<&str> =
            PHASES.iter().copied().filter(|p| on.iter().any(|x| x == p)).collect();
        let value = (ordered.len() != PHASES.len()).then(|| {
            let mut a = toml_edit::Array::new();
            ordered.iter().for_each(|p| a.push(*p));
            Value::Array(a)
        });
        ed.engine_set(ctx, project, i, "", "phases", value);
    }
}

fn number(item: Option<&Item>) -> Option<f64> {
    let v = item?.as_value()?;
    v.as_float()
        .or_else(|| v.as_integer().map(|i| i as f64))
        .or_else(|| v.as_str().and_then(|s| Length::parse(s).ok()).map(Length::to_mm))
}

fn knobs(ui: &mut Ui, ctx: &egui::Context, project: &Project, i: usize, ed: &mut Editor) {
    let mut change: Option<(&str, &str, Option<Value>)> = None;
    let mut section = None;
    for (sec, key, kind, help) in KNOBS {
        if section != Some(*sec) {
            section = Some(*sec);
            ui.add_space(4.0);
            Line::new().legend(if sec.is_empty() { "engine" } else { sec }).show(ui);
        }
        let item = ed.engine_get(sec, key);
        let set = item.is_some();
        row_help(ui, key, help, |ui| {
            let picked = match *kind {
                Knob::Mm(d) => {
                    let mut v = number(item).unwrap_or(d);
                    let r = ui.add(
                        egui::DragValue::new(&mut v)
                            .speed(0.01)
                            .range(0.0..=100.0)
                            .max_decimals(3)
                            .suffix(" mm")
                            .update_while_editing(false),
                    );
                    r.changed().then(|| Value::from((v * 1e4).round() / 1e4))
                }
                Knob::Int(d) => {
                    let mut v = number(item).map(|x| x as i64).unwrap_or(d);
                    let r = ui.add(
                        egui::DragValue::new(&mut v)
                            .speed(0.2)
                            .range(0..=100000)
                            .update_while_editing(false),
                    );
                    r.changed().then(|| Value::from(v))
                }
                Knob::Flag(d) => {
                    let mut v = item.and_then(Item::as_bool).unwrap_or(d);
                    ui.checkbox(&mut v, "").changed().then(|| Value::from(v))
                }
            };
            if let Some(v) = picked {
                change = Some((sec, key, Some(v)));
            }
            if set && ui.small_button("default").clicked() {
                change = Some((sec, key, None));
            }
        });
    }
    if let Some((sec, key, v)) = change {
        ed.engine_set(ctx, project, i, sec, key, v);
    }
}

pub fn pinswap(ui: &mut Ui, project: &Project, i: usize, ed: &mut Editor, part: &str) {
    let ctx = ui.ctx().clone();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let busy = ed.engine.swap_job.is_some() || running(ed);
        if ui
            .add_enabled(!busy, egui::Button::new("pinswap"))
            .on_hover_text("reassign swappable I/O of this chip to untangle the ratsnest")
            .clicked()
        {
            start_swap(ed, project, i, part, &ctx);
        }
        if ed.engine.swap_job.is_some() {
            lamp(ui, "searching", true, false);
        }
    });
    let Some((who, r)) = &ed.engine.swap else { return };
    if who != part {
        return;
    }
    match r {
        Err(e) => status(ui, false, e),
        Ok(s) => {
            readouts(
                ui,
                &[
                    ("movable", s.movable_nets.to_string(), VALUE),
                    ("tangle", format!("{} > {}", trim(s.before, 1), trim(s.after, 1)), TRACE),
                    ("swaps", s.swaps.len().to_string(), READOUT),
                ],
            );
            if !s.swaps.is_empty() && ui.button("write to the schematic").clicked() {
                let swaps = s.swaps.clone();
                ed.engine.swap_note = Some(write_swaps(project, part, &swaps));
                ed.engine.swap = None;
            }
        }
    }
    if let Some(n) = &ed.engine.swap_note {
        note(ui, n, LEGEND);
    }
}

fn start_swap(ed: &mut Editor, project: &Project, i: usize, part: &str, ctx: &egui::Context) {
    let inputs = match ed.inputs(project, i) {
        Ok(x) => x,
        Err(e) => {
            ed.engine.swap = Some((part.to_string(), Err(e)));
            return;
        }
    };
    let layout = ed.layout(project, i).clone();
    let text = ed.text();
    let part = part.to_string();
    let (tx, rx) = mpsc::channel();
    let wake = ctx.clone();
    std::thread::spawn(move || {
        let r = agentee_core::project::parse::<agentee_core::layout::LayoutFile>(&text)
            .map_err(|(at, m)| format!("{at}: {m}"))
            .and_then(|file| {
                agentee_layout::pinswap::run(
                    &layout,
                    &inputs.board,
                    &inputs.schematic,
                    &file,
                    &part,
                    1,
                )
            });
        let _ = tx.send((part, r));
        wake.request_repaint();
    });
    ed.engine.swap_job = Some(rx);
    ed.engine.swap = None;
    ed.engine.swap_note = None;
}

fn write_swaps(project: &Project, part: &str, swaps: &[agentee_layout::pinswap::Swap]) -> String {
    let mut written = 0;
    for s in &project.schematics {
        let Ok(src) = std::fs::read_to_string(&s.path) else { continue };
        let out = agentee_layout::pinswap::rewrite(&src, part, swaps);
        if out != src {
            if let Err(e) = std::fs::write(&s.path, &out) {
                return format!("{}: {e}", s.path.display());
            }
            written += 1;
        }
    }
    format!("{} swaps written into {written} schematic files", swaps.len())
}
