pub mod access;
pub mod constraints;
pub mod detail;
pub mod escape;
pub mod finish;
pub mod flow;
pub mod global;
pub mod negotiate;
pub mod pinswap;
pub mod placement;
pub mod planes;
pub mod score;
pub mod start;
pub mod tangle;

use agentee_core::board::Board;
use agentee_core::engine::{EngineFile, stage_of};
use agentee_core::geom::P;
use agentee_core::layout::{Layout, LayoutFile};
use agentee_core::route::{RoutedTrack, RoutedVia};
use agentee_core::schematic::Schematic;
use score::{Context, Score, Weights};
use serde::Serialize;

pub const ROUTE: &str = "route";
pub const PLANES: &str = "planes";
pub const PLACE: &str = "place";
pub const RETIRED_PLANS: &[&str] = &["detail", "escape", "tie", "global"];

pub struct Model<'a> {
    pub board: &'a Board,
    pub schematic: &'a Schematic,
    pub layout: Layout,
    pub file: LayoutFile,
    pub keepouts: Vec<Vec<P>>,
    pub heat: Vec<(String, f64)>,
    pub constraints: Option<constraints::Groups>,
    pub placement: Option<placement::PlacePlan>,
    pub access: Option<access::AccessPlan>,
    pub planes: Option<planes::PlanesPlan>,
    pub global: Option<global::GlobalPlan>,
    pub detail: Option<detail::DetailPlan>,
    pub hot: Vec<placement::Hot>,
    pub base: Option<negotiate::Base>,
    pub hist: Option<Vec<f32>>,
    pub warm: Option<negotiate::Warm>,
    pub pass: usize,
    pub text: String,
}

impl Model<'_> {
    pub fn ensure_base(&mut self, opts: &negotiate::Options) -> Result<(), String> {
        if self.base.is_none() {
            self.base = Some(negotiate::Base::new(&self.layout, self.board, opts)?);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PhaseReport {
    pub phase: String,
    pub ms: u128,
    pub reload_ms: u128,
    pub changed: bool,
    pub notes: Vec<String>,
    pub failed: Vec<String>,
    pub score: Option<Score>,
}

pub trait Phase {
    fn name(&self) -> &'static str;
    fn run(&self, model: &mut Model, cfg: &EngineFile) -> PhaseReport;
}

pub struct Config<'a> {
    pub engine: EngineFile,
    pub from: Option<String>,
    pub to: Option<String>,
    pub only: Option<String>,
    pub text: String,
    pub resolve: &'a dyn Fn(&str) -> Result<(LayoutFile, Layout), String>,
    pub watch: Option<&'a dyn Fn(Event)>,
    pub stop: Option<&'a std::sync::atomic::AtomicBool>,
}

pub enum Event<'a> {
    Start(&'a str),
    Done(&'a PhaseReport),
}

pub struct Run<'a> {
    pub from: Option<String>,
    pub to: Option<String>,
    pub only: Option<String>,
    pub watch: Option<&'a dyn Fn(Event)>,
    pub stop: Option<&'a std::sync::atomic::AtomicBool>,
}

pub fn run_text(
    inputs: &agentee_core::project::LayoutInputs,
    text: &str,
    a: &Run,
) -> Result<RunReport, String> {
    let footprints: std::collections::HashMap<&str, &agentee_core::footprint::Footprint> =
        inputs.footprints.iter().map(|(n, f)| (n.as_str(), f)).collect();
    let dir = inputs.path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let first: LayoutFile =
        agentee_core::project::parse(text).map_err(|(at, m)| format!("{at}: {m}"))?;
    let heat = agentee_core::place::thermal_heat(&inputs.sims, &first.name);
    let resolve = |t: &str| -> Result<(LayoutFile, Layout), String> {
        let f: LayoutFile =
            agentee_core::project::parse(t).map_err(|(at, m)| format!("{at}: {m}"))?;
        let cx = agentee_core::layout::Context {
            dir: dir.clone(),
            board: &inputs.board,
            schematic: &inputs.schematic,
            footprints: footprints.clone(),
            heat: heat.clone(),
        };
        let mut d = agentee_core::diag::Diags::new(&f.name);
        let resolved = agentee_core::layout::without_checks(|| f.resolve(&cx, &mut d));
        Ok((f, resolved))
    };
    let (file, layout) = resolve(text)?;
    let keepouts: Vec<Vec<P>> = file
        .place
        .as_ref()
        .map(|s| s.keepouts.iter().map(|k| k.iter().map(|q| q.to_mm()).collect()).collect())
        .unwrap_or_default();
    let engine = file.engine.clone().unwrap_or_default();
    let mut model = Model {
        board: &inputs.board,
        schematic: &inputs.schematic,
        layout,
        file,
        keepouts,
        heat: heat.clone(),
        constraints: None,
        placement: None,
        access: None,
        planes: None,
        global: None,
        detail: None,
        hot: Vec::new(),
        base: None,
        hist: None,
        warm: None,
        pass: 0,
        text: String::new(),
    };
    let cfg = Config {
        engine,
        from: a.from.clone(),
        to: a.to.clone(),
        only: a.only.clone(),
        text: text.to_string(),
        resolve: &resolve,
        watch: a.watch,
        stop: a.stop,
    };
    run(&mut model, &cfg)
}

#[derive(Clone, Debug, Serialize)]
pub struct RunReport {
    pub time: TimeReport,
    pub phases: Vec<PhaseReport>,
    pub score: Score,
    pub skipped: Vec<String>,
    #[serde(skip)]
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<constraints::Groups>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placement: Option<placement::PlacePlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<access::AccessPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planes: Option<planes::PlanesPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<global::GlobalPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<detail::DetailPlan>,
}

pub fn plane_nets(file: &LayoutFile) -> Vec<String> {
    file.zones.iter().map(|z| z.net.clone()).collect()
}

pub fn score_of(model: &Model) -> Score {
    let weights = Weights::from_file(&model.layout.engine.score);
    let planes = plane_nets(&model.file);
    let tangle = model.layout.engine.tangle.clone().map(|t| t.weights).unwrap_or_default();
    Score::measure(
        &Context {
            board: model.board,
            schematic: model.schematic,
            layout: &model.layout,
            overflow: model.global.as_ref().map(|g| g.overflow),
            access: model.access.as_ref().filter(|a| a.pads > 0).map(|a| a.stuck.len()),
            copper_overlap: model.detail.as_ref().map(|d| d.overlap_left),
            keepouts: &model.keepouts,
            heat: &model.heat,
            planes: &planes,
            tangle: &tangle,
        },
        &weights,
    )
}

type Snapshot =
    (usize, String, Option<detail::DetailPlan>, Option<negotiate::Warm>, Option<Vec<f32>>, u32);

struct Driver<'c, 'a> {
    cfg: &'c Config<'a>,
    text: String,
    reports: Vec<PhaseReport>,
    skipped: Vec<String>,
    discarded: Vec<(String, f64)>,
    search: negotiate::Spend,
    unfilled: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TimeReport {
    pub total_ms: f64,
    pub reload_ms: f64,
    pub stages: Vec<(String, f64)>,
    pub discarded: Vec<(String, f64)>,
    pub search: negotiate::Spend,
}

impl TimeReport {
    pub fn table(&self) -> String {
        let s = |v: f64| format!("{:>8.1} s", v / 1000.0);
        let mut t = format!("{:<52}{}\n", "time", s(self.total_ms));
        for (name, ms) in &self.stages {
            t += &format!("  {:<50}{}\n", name, s(*ms));
        }
        t += &format!("  {:<50}{}\n", "resolving the file again", s(self.reload_ms));
        let thrown = self.discarded.iter().map(|d| d.1).fold(0.0, |a, b| a + b);
        t += &format!("{:<52}{}\n", "thrown away", s(thrown));
        for (what, ms) in &self.discarded {
            t += &format!("  {:<50}{}\n", what, s(*ms));
        }
        if self.search.work_ms > 0.0 {
            t += "detail search\n";
            for line in self.search.summary().lines() {
                t += &format!("  {line}\n");
            }
        }
        t
    }
}

fn ms_since(t: std::time::Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

impl Driver<'_, '_> {
    fn stopped(&self) -> bool {
        self.cfg.stop.is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn reload_unfilled(&mut self, model: &mut Model) -> Result<u128, String> {
        let t = std::time::Instant::now();
        (model.file, model.layout) =
            agentee_core::layout::without_fills(|| (self.cfg.resolve)(&self.text))?;
        model.base = None;
        model.hist = None;
        model.warm = None;
        self.unfilled = true;
        Ok(t.elapsed().as_millis())
    }

    fn reload(&mut self, model: &mut Model) -> Result<u128, String> {
        let t = std::time::Instant::now();
        self.unfilled = false;
        (model.file, model.layout) = (self.cfg.resolve)(&self.text)?;
        model.base = None;
        model.hist = None;
        model.warm = None;
        Ok(t.elapsed().as_millis())
    }

    fn start(&self, name: &str) -> std::time::Instant {
        if let Some(w) = self.cfg.watch {
            w(Event::Start(name));
        }
        std::time::Instant::now()
    }

    fn done(&mut self, model: &Model, mut r: PhaseReport, t0: std::time::Instant, reload_ms: u128) {
        r.ms = t0.elapsed().as_millis().saturating_sub(reload_ms);
        r.reload_ms = reload_ms;
        r.score = Some(score_of(model));
        if let Some(w) = self.cfg.watch {
            w(Event::Done(&r));
        }
        self.reports.push(r);
    }

    fn step(&mut self, model: &mut Model, p: &dyn Phase) {
        let t0 = self.start(p.name());
        model.text = self.text.clone();
        let r = p.run(model, &self.cfg.engine);
        self.done(model, r, t0, 0);
    }

    fn write_route(&mut self, model: &mut Model) -> Result<u128, String> {
        let Some(d) = model.detail.as_ref() else { return Ok(0) };
        self.text = write_plan(&self.text, ROUTE, &route_toml(&d.tracks, &d.vias));
        self.reload(model)
    }
}

fn selected(cfg: &Config) -> Result<Vec<String>, String> {
    let wanted = cfg.engine.phases();
    let pick = |n: &str| -> Result<usize, String> {
        let s = stage_of(n).ok_or_else(|| format!("no stage `{n}`"))?;
        wanted
            .iter()
            .position(|w| w == s)
            .ok_or_else(|| format!("stage `{n}` is not in the configured stages"))
    };
    if let Some(o) = &cfg.only {
        return Ok(vec![wanted[pick(o)?].clone()]);
    }
    let start = cfg.from.as_deref().map(pick).transpose()?.unwrap_or(0);
    let end = cfg.to.as_deref().map(pick).transpose()?.unwrap_or(wanted.len().saturating_sub(1));
    Ok(wanted.get(start..=end).map(|s| s.to_vec()).unwrap_or_default())
}

pub fn run(model: &mut Model, cfg: &Config) -> Result<RunReport, String> {
    let chosen = selected(cfg)?;
    let has = |s: &str| chosen.iter().any(|x| x == s);
    let routes = has("global") || has("detail");
    let started = std::time::Instant::now();
    let mut d = Driver {
        cfg,
        text: cfg.text.clone(),
        reports: Vec::new(),
        skipped: Vec::new(),
        discarded: Vec::new(),
        search: negotiate::Spend::default(),
        unfilled: false,
    };
    if routes {
        for p in std::iter::once(ROUTE).chain(RETIRED_PLANS.iter().copied()) {
            d.text = strip_plan(&d.text, p);
        }
    }
    if has("access") {
        d.text = strip_plan(&d.text, PLANES);
    }
    if d.text != cfg.text {
        d.reload(model)?;
    }
    if has("finish") && !has("detail") {
        model.detail = route_from_text(&d.text);
    }
    let dopts = detail::options(&cfg.engine);
    if has("constraints") {
        d.step(model, &constraints::Constraints);
    }
    let place_rounds = if has("place") { cfg.engine.place_rounds.unwrap_or(3).max(1) } else { 1 };
    let rounds = cfg.engine.rounds.unwrap_or(3).max(1);
    let mut best_pass: Option<Snapshot> = None;
    let mut best_pass_ms: Option<(u32, f64)> = None;
    for pass in 0..place_rounds {
        let tp = std::time::Instant::now();
        let already = d.discarded.len();
        if d.stopped() {
            d.skipped.push("stopped".into());
            break;
        }
        if has("place") {
            if pass > 0 && model.hot.is_empty() {
                break;
            }
            let t0 = d.start("place");
            model.text = d.text.clone();
            model.pass = pass as usize;
            let r = placement::Place.run(model, &cfg.engine);
            let mut reload_ms = 0;
            if let Some(plan) = &model.placement
                && !plan.moves.is_empty()
            {
                d.text = write_moves(&d.text, &plan.moves)?;
                d.text = if plan.under.is_empty() {
                    strip_plan(&d.text, PLACE)
                } else {
                    let mut t = String::new();
                    for u in &plan.under {
                        if let Some((layer, from, width)) = &u.stub {
                            t += &track_toml(&u.net, layer, Some(*width), &[*from, u.at]);
                        }
                        t += &via_toml(&u.net, u.at, &u.via);
                    }
                    write_plan(&d.text, PLACE, &t)
                };
                if !plan.texts.is_empty() {
                    let mut doc: toml_edit::DocumentMut =
                        d.text.parse().map_err(|e| format!("{e}"))?;
                    start::move_board_texts(&mut doc, &plan.texts);
                    d.text = doc.to_string();
                }
                reload_ms = d.reload_unfilled(model)?;
                for _ in 0..3 {
                    let (moved, _) = model.layout.settle_labels(model.board);
                    if moved.is_empty() {
                        break;
                    }
                    let mut doc: toml_edit::DocumentMut =
                        d.text.parse().map_err(|e| format!("{e}"))?;
                    start::write_labels(&mut doc, &moved)?;
                    d.text = doc.to_string();
                    reload_ms += d.reload_unfilled(model)?;
                }
            }
            model.hot.clear();
            d.done(model, r, t0, reload_ms);
        }
        if has("access") && !d.stopped() {
            let t0 = d.start("access");
            model.text = d.text.clone();
            let mut r = access::Access.run(model, &cfg.engine);
            let mut reload_ms = 0;
            if let Some(section) = planes_toml(model) {
                d.text = write_plan(&d.text, PLANES, &section);
                reload_ms = d.reload(model)?;
            } else if d.unfilled {
                reload_ms = d.reload(model)?;
            }
            match model.ensure_base(&dopts) {
                Ok(()) => access::pin_access(model, &mut r, &dopts),
                Err(e) => r.failed.push(e),
            }
            d.done(model, r, t0, reload_ms);
        }
        if routes {
            if d.unfilled {
                d.reload(model)?;
            }
            let reps = if has("place") && place_rounds > 1 { 1 } else { rounds };
            route_reps(&mut d, model, &has, &dopts, reps, pass, None);
        }
        model.hot = model.global.as_ref().map(|g| g.hot.clone()).unwrap_or_default();
        if let Some(x) = &model.detail {
            let tile = model.global.as_ref().map(|g| g.tile).unwrap_or(1.0);
            for f in &x.failed {
                model.hot.push(placement::Hot {
                    at: [(f.from[0] + f.to[0]) / 2.0, (f.from[1] + f.to[1]) / 2.0],
                    size: tile,
                    overflow: 0.5,
                });
            }
        }
        if !has("place") {
            break;
        }
        let missing = model.detail.as_ref().map(|x| x.connections - x.routed).unwrap_or(usize::MAX);
        let step = (model.detail.as_ref().map(|x| x.connections).unwrap_or(0) / 100).max(1);
        let gained = best_pass.as_ref().is_none_or(|b| missing + step <= b.0);
        let pass_ms = ms_since(tp) - d.discarded[already..].iter().map(|x| x.1).sum::<f64>();
        if best_pass.as_ref().is_none_or(|b| missing < b.0) {
            best_pass = Some((
                missing,
                d.text.clone(),
                model.detail.clone(),
                model.warm.clone(),
                model.hist.clone(),
                pass,
            ));
            if let Some((k, t)) = best_pass_ms.replace((pass, pass_ms)) {
                d.discarded.push((
                    format!("placement pass {}, the rest, beaten by a later pass", k + 1),
                    t,
                ));
            }
        } else {
            d.discarded.push((
                format!("placement pass {}, the rest, routed less than the best pass", pass + 1),
                pass_ms,
            ));
        }
        if !gained {
            break;
        }
    }
    if let Some((_, text, plan, warm, hist, pass)) = best_pass {
        if text != d.text {
            d.text = text;
            d.reload(model)?;
            model.detail = plan.clone();
            model.warm = warm;
            model.hist = hist;
        }
        let clean = plan.as_ref().is_some_and(|x| x.overlap_left == 0 && x.failed.is_empty());
        if routes && place_rounds > 1 && rounds > 1 && !clean && !d.stopped() {
            let start = plan.map(|p| (p.routed, p));
            route_reps(&mut d, model, &has, &dopts, rounds - 1, pass, start);
        }
    }
    if has("detail") {
        d.write_route(model)?;
        for _ in 0..2 {
            if d.stopped() || !repair(&mut d, model, &dopts)? {
                break;
            }
        }
    }
    if has("finish") && !d.stopped() {
        let t0 = d.start("finish");
        let mut r = finish::Finish.run(model, &cfg.engine);
        let mut reload_ms = 0;
        if r.changed {
            reload_ms += d.write_route(model)?;
            if finish::check_spread(model, &cfg.engine, &mut r) {
                reload_ms += d.write_route(model)?;
            }
        }
        let before = r.changed;
        r.changed = false;
        finish::tune(model, &mut r);
        if r.changed {
            reload_ms += d.write_route(model)?;
        }
        let tuned = r.changed;
        r.changed = false;
        finish::neck(model, &mut r);
        finish::dedouble(model, &mut r);
        finish::trim_pours(model, &mut r);
        if r.changed {
            reload_ms += d.write_route(model)?;
        }
        let mut joined = finish::close_joints(model, &cfg.engine, &mut r);
        finish::drop_fragments(model, &mut r);
        joined |= finish::close_joints(model, &cfg.engine, &mut r);
        if joined || r.changed {
            finish::dedouble(model, &mut r);
            r.changed = true;
            reload_ms += d.write_route(model)?;
        }
        if finish::widen(model, &cfg.engine, &mut r) {
            r.changed = true;
            reload_ms += d.write_route(model)?;
        }
        r.changed |= before || tuned;
        d.done(model, r, t0, reload_ms);
    }
    if d.unfilled {
        d.reload(model)?;
    }
    let score = score_of(model);
    let mut stages: Vec<(String, f64)> = Vec::new();
    for r in &d.reports {
        match stages.iter_mut().find(|x| x.0 == r.phase) {
            Some(x) => x.1 += r.ms as f64,
            None => stages.push((r.phase.clone(), r.ms as f64)),
        }
    }
    let time = TimeReport {
        total_ms: ms_since(started),
        reload_ms: d.reports.iter().map(|r| r.reload_ms as f64).sum(),
        stages,
        discarded: d.discarded,
        search: d.search,
    };
    Ok(RunReport {
        time,
        phases: d.reports,
        score,
        skipped: d.skipped,
        text: d.text,
        constraints: model.constraints.clone(),
        placement: model.placement.clone(),
        access: model.access.clone(),
        planes: model.planes.clone(),
        global: model.global.clone(),
        detail: model.detail.clone(),
    })
}

fn route_reps(
    d: &mut Driver,
    model: &mut Model,
    has: &dyn Fn(&str) -> bool,
    dopts: &negotiate::Options,
    reps: u32,
    pass: u32,
    start: Option<(usize, detail::DetailPlan)>,
) {
    let cfg = d.cfg;
    let mut best = start.as_ref().map(|s| s.0).unwrap_or(0);
    let mut kept: Option<detail::DetailPlan> = start.map(|s| s.1);
    let mut kept_rep: Option<(u32, f64)> = None;
    let first = if kept.is_some() { 1 } else { 0 };
    for rep in first..first + reps {
        let tr = std::time::Instant::now();
        if d.stopped() {
            d.skipped.push("stopped".into());
            break;
        }
        if has("global") {
            let t0 = d.start("global");
            let r = match model.ensure_base(dopts) {
                Ok(()) => global::Global.run(model, &cfg.engine),
                Err(e) => {
                    PhaseReport { phase: "global".into(), failed: vec![e], ..Default::default() }
                }
            };
            d.done(model, r, t0, 0);
        }
        if has("detail") {
            d.step(model, &detail::Detail);
            if let Some(x) = &model.detail {
                d.search.absorb(&x.spend);
            }
        }
        let rep_ms = ms_since(tr);
        let routed = model.detail.as_ref().map(|x| x.routed).unwrap_or(0);
        let clean =
            model.detail.as_ref().is_some_and(|x| x.overlap_left == 0 && x.failed.is_empty());
        let step = (model.detail.as_ref().map(|x| x.connections).unwrap_or(0) / 100).max(1);
        let label = |k: u32, why: &str| format!("pass {} route {}, {why}", pass + 1, k + 1);
        if routed < best + step && kept.is_some() {
            if routed > best {
                kept = model.detail.clone();
                if let Some((k, t)) = kept_rep.replace((rep, rep_ms)) {
                    d.discarded.push((label(k, "beaten by the next route"), t));
                }
            } else {
                d.discarded.push((label(rep, "routed no more than the one before"), rep_ms));
            }
            break;
        }
        if let Some((k, t)) = kept_rep.replace((rep, rep_ms)) {
            d.discarded.push((label(k, "beaten by the next route"), t));
        }
        best = routed;
        kept = model.detail.clone();
        if clean || !has("global") || !has("detail") {
            break;
        }
    }
    if kept.is_some() {
        model.detail = kept;
    }
}

fn repair(d: &mut Driver, model: &mut Model, opts: &negotiate::Options) -> Result<bool, String> {
    let Some(plan) = model.detail.as_ref() else { return Ok(false) };
    let stray: Vec<String> = model
        .layout
        .nets
        .iter()
        .filter(|n| n.unrouted > 0 && !plan.failed.iter().any(|f| f.net == n.name))
        .map(|n| n.name.clone())
        .collect();
    if stray.is_empty() {
        return Ok(false);
    }
    let t0 = d.start("detail");
    let o = negotiate::Options { nets: stray.clone(), ..opts.clone() };
    let mut r = PhaseReport { phase: "detail".into(), ..Default::default() };
    let mut gained = false;
    match negotiate::Base::new(&model.layout, model.board, &o) {
        Ok(base) => {
            let out = negotiate::route_on(&model.layout, &base, &o, &negotiate::Guide::default());
            d.search.absorb(&out.spend);
            let res = out.result;
            r.notes.push(format!(
                "repair on the refilled pours: {} nets, {} of {} connections",
                stray.len(),
                res.routed,
                res.connections
            ));
            r.failed = res.failed.iter().map(|f| format!("{}: {}", f.net, f.reason)).collect();
            gained = res.routed > 0;
            let plan = model.detail.as_mut().expect("detail plan");
            plan.failed.retain(|f| !stray.contains(&f.net));
            plan.failed.extend(res.failed);
            plan.tracks.extend(res.tracks);
            plan.vias.extend(res.vias);
        }
        Err(e) => r.failed.push(e),
    }
    let ms = if gained { d.write_route(model)? } else { 0 };
    r.changed = gained;
    if !gained {
        d.discarded.push(("repair that joined nothing".into(), ms_since(t0)));
    }
    d.done(model, r, t0, ms);
    Ok(gained)
}

fn stitching_toml(model: &Model) -> String {
    if !model.file.stitching.is_empty() {
        return String::new();
    }
    let b = model.board;
    let outer = [model.layout.copper.first(), model.layout.copper.last()];
    let Some(ground) = model
        .file
        .zones
        .iter()
        .filter(|z| z.layers.iter().any(|l| outer.contains(&Some(l))))
        .map(|z| z.net.as_str())
        .find(|n| agentee_core::place::is_ground(n))
    else {
        return String::new();
    };
    let rf: Vec<String> = model
        .layout
        .nets
        .iter()
        .filter(|n| agentee_core::place::is_rf_class(b, &n.class))
        .filter(|n| b.netclasses.iter().any(|c| c.name == n.class && c.coplanar_gap.is_some()))
        .map(|n| format!("\"{}\"", n.name))
        .collect();
    if rf.is_empty() {
        return String::new();
    }
    format!("\n[[stitching]]\nnet = \"{ground}\"\nfence = [{}]\n", rf.join(", "))
}

fn planes_toml(model: &Model) -> Option<String> {
    let zones = model.planes.as_ref().map(|p| p.zones.as_slice()).unwrap_or_default();
    let mut t = stitching_toml(model);
    for z in zones {
        let pts: Vec<String> = z.outline.iter().map(|q| pt(*q)).collect();
        t += &format!(
            "\n[[zones]]\nnet = \"{}\"\nlayers = [\"{}\"]\npriority = {}\noutline = [{}]\n",
            z.net,
            z.layer,
            z.priority,
            pts.join(", ")
        );
    }
    (!t.is_empty()).then_some(t)
}

pub fn route_toml(tracks: &[RoutedTrack], vias: &[RoutedVia]) -> String {
    let mut t = String::new();
    for tr in tracks {
        t += &track_toml(&tr.net, &tr.layer, tr.width, &tr.points);
    }
    for v in vias {
        t += &via_toml(&v.net, v.at, &v.via);
    }
    t
}

fn route_from_text(text: &str) -> Option<detail::DetailPlan> {
    let start = format!("# plan {ROUTE}\n");
    let end = format!("# end plan {ROUTE}\n");
    let a = text.find(&start)? + start.len();
    let b = text[a..].find(&end)? + a;
    let doc: toml::Table = text[a..b].parse().ok()?;
    let point = |v: &toml::Value| -> Option<P> {
        let q = v.as_array()?;
        let num = |x: &toml::Value| x.as_float().or_else(|| x.as_integer().map(|i| i as f64));
        Some([num(q.first()?)?, num(q.get(1)?)?])
    };
    let mut plan = detail::DetailPlan::default();
    for t in doc.get("tracks").and_then(|v| v.as_array()).into_iter().flatten() {
        let t = t.as_table()?;
        plan.tracks.push(RoutedTrack {
            net: t.get("net")?.as_str()?.to_string(),
            layer: t.get("layer")?.as_str()?.to_string(),
            width: t.get("width").and_then(|w| w.as_float()),
            points: t.get("points")?.as_array()?.iter().filter_map(point).collect(),
        });
    }
    for v in doc.get("vias").and_then(|v| v.as_array()).into_iter().flatten() {
        let v = v.as_table()?;
        plan.vias.push(RoutedVia {
            net: v.get("net")?.as_str()?.to_string(),
            at: point(v.get("at")?)?,
            via: v.get("via")?.as_str()?.to_string(),
        });
    }
    Some(plan)
}

fn fmt(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0');
    if s.ends_with('.') { format!("{s}0") } else { s.to_string() }
}

fn pt(q: P) -> String {
    format!("[{}, {}]", fmt(q[0]), fmt(q[1]))
}

pub(crate) fn track_toml(net: &str, layer: &str, width: Option<f64>, points: &[P]) -> String {
    let pts: Vec<String> = points.iter().map(|q| pt(*q)).collect();
    let w = width.map(|w| format!("width = {}\n", fmt(w))).unwrap_or_default();
    format!(
        "\n[[tracks]]\nnet = \"{net}\"\nlayer = \"{layer}\"\n{w}points = [{}]\n",
        pts.join(", ")
    )
}

pub(crate) fn via_toml(net: &str, at: P, via: &str) -> String {
    format!("\n[[vias]]\nnet = \"{net}\"\nat = {}\nvia = \"{via}\"\n", pt(at))
}

pub fn routed_toml(r: &agentee_core::route::RouteResult) -> String {
    let mut t = String::new();
    for tr in &r.tracks {
        t += &track_toml(&tr.net, &tr.layer, tr.width, &tr.points);
    }
    for v in &r.vias {
        t += &via_toml(&v.net, v.at, &v.via);
    }
    t
}

pub fn strip_plan(text: &str, phase: &str) -> String {
    let start = format!("# plan {phase}\n");
    let end = format!("# end plan {phase}\n");
    let mut kept = text.to_string();
    while let (Some(a), Some(b)) = (kept.find(&start), kept.find(&end)) {
        if b < a {
            break;
        }
        kept = format!("{}{}", &kept[..a], &kept[b + end.len()..]);
    }
    kept
}

pub fn write_plan(text: &str, phase: &str, body: &str) -> String {
    let kept = strip_plan(text, phase);
    format!("{}\n\n# plan {phase}\n{body}\n# end plan {phase}\n", kept.trim_end_matches('\n'))
}

pub fn write_moves(text: &str, moves: &[placement::Move]) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let parts = doc
        .get_mut("footprints")
        .and_then(|v| v.as_array_of_tables_mut())
        .ok_or("[[footprints]] is not an array of tables")?;
    for m in moves {
        let Some(t) = parts
            .iter_mut()
            .find(|t| t.get("ref").and_then(|v| v.as_str()) == Some(m.reference.as_str()))
        else {
            continue;
        };
        let num = |v: &toml_edit::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64));
        let point_of = |i: Option<&toml_edit::Item>| -> Option<P> {
            let a = i?.as_array()?;
            Some([num(a.get(0)?)?, num(a.get(1)?)?])
        };
        let old_at = point_of(t.get("at"));
        let old_rot = t.get("rotation").and_then(|v| v.as_value()).and_then(num).unwrap_or(0.0);
        let mut pt = toml_edit::Array::new();
        pt.push(m.at[0]);
        pt.push(m.at[1]);
        t["at"] = toml_edit::value(pt);
        let rot = m.rotation.rem_euclid(360.0);
        if rot.abs() < 1e-6 {
            t.remove("rotation");
        } else {
            t["rotation"] = toml_edit::value(rot);
        }
        if m.bottom {
            t["side"] = toml_edit::value("bottom");
        } else {
            t.remove("side");
        }
        if m.hide_label {
            let mut hidden = toml_edit::InlineTable::new();
            hidden.insert("hide", true.into());
            t["label"] = toml_edit::value(hidden);
            continue;
        }
        let label = t.get_mut("label").and_then(|l| l.as_table_like_mut());
        if let (Some(label), Some(from)) = (label, old_at)
            && let Some(la) = point_of(label.get("at"))
        {
            let off = agentee_core::geom::rotate(
                [la[0] - from[0], la[1] - from[1]],
                m.rotation - old_rot,
            );
            let mut q = toml_edit::Array::new();
            q.push(((m.at[0] + off[0]) * 1e4).round() / 1e4);
            q.push(((m.at[1] + off[1]) * 1e4).round() / 1e4);
            label.insert("at", toml_edit::value(q));
        }
    }
    Ok(doc.to_string())
}
