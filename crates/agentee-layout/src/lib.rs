pub mod detail;
pub mod escape;
pub mod field;
pub mod finish;
pub mod flow;
pub mod global;
pub mod layers;
pub mod pinswap;
pub mod placement;
pub mod planes;
pub mod score;
pub mod tangle;

use agentee_core::board::Board;
use agentee_core::engine::EngineFile;
use agentee_core::geom::P;
use agentee_core::graphic::Bounds;
use agentee_core::layout::{Layout, LayoutFile};
use agentee_core::schematic::Schematic;
use field::CostField;
use score::{Context, Score, Weights};
use serde::Serialize;

pub const DEFAULT_TILE_MM: f64 = 0.25;
pub const DEFAULT_ROUNDS: u32 = 3;

pub struct Model<'a> {
    pub board: &'a Board,
    pub schematic: &'a Schematic,
    pub layout: Layout,
    pub file: LayoutFile,
    pub keepouts: Vec<Vec<P>>,
    pub heat: Vec<(String, f64)>,
    pub placement: Option<placement::PlacePlan>,
    pub escape: Option<escape::EscapePlan>,
    pub planes: Option<planes::PlanesPlan>,
    pub layers: Option<layers::LayerPlan>,
    pub global: Option<global::GlobalPlan>,
    pub detail: Option<detail::DetailPlan>,
    pub text: String,
    pub finished_escape: Option<String>,
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
    fn run(&self, model: &mut Model, cfg: &EngineFile, field: &mut CostField) -> PhaseReport;
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
        let resolved = f.resolve(&cx, &mut d);
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
        escape: None,
        placement: None,
        planes: None,
        layers: None,
        global: None,
        detail: None,
        text: String::new(),
        finished_escape: None,
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
    pub phases: Vec<PhaseReport>,
    pub score: Score,
    pub skipped: Vec<String>,
    #[serde(skip)]
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escape: Option<escape::EscapePlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placement: Option<placement::PlacePlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planes: Option<planes::PlanesPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<layers::LayerPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<global::GlobalPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<detail::DetailPlan>,
}

pub fn phases() -> Vec<Box<dyn Phase>> {
    vec![
        Box::new(placement::Floorplan),
        Box::new(placement::Place),
        Box::new(placement::Legalise),
        Box::new(layers::Layers),
        Box::new(escape::Escape),
        Box::new(planes::Planes),
        Box::new(global::Global),
        Box::new(detail::Detail),
        Box::new(finish::Finish),
    ]
}

pub fn new_field(layout: &Layout, tile: f64) -> CostField {
    let mut b = Bounds::EMPTY;
    layout.outline.iter().for_each(|p| b.add(*p));
    if b.is_empty() {
        b = Bounds { min: [0.0, 0.0], max: [1.0, 1.0] };
    }
    let mut f = CostField::new(b, tile, layout.copper.clone());
    for l in 0..layout.copper.len() {
        f.fill_capacity(l, (tile / 0.2).max(1.0) as f32);
    }
    f
}

pub fn plane_nets(file: &LayoutFile) -> Vec<String> {
    file.zones.iter().map(|z| z.net.clone()).collect()
}

pub fn score_of(model: &Model, field: Option<&CostField>) -> Score {
    let weights = Weights::from_file(&model.layout.engine.score);
    let planes = plane_nets(&model.file);
    let tangle = model.layout.engine.tangle.clone().map(|t| t.weights).unwrap_or_default();
    Score::measure(
        &Context {
            board: model.board,
            schematic: model.schematic,
            layout: &model.layout,
            field,
            keepouts: &model.keepouts,
            heat: &model.heat,
            planes: &planes,
            tangle: &tangle,
        },
        &weights,
    )
}

pub fn run(model: &mut Model, cfg: &Config) -> Result<RunReport, String> {
    let wanted = cfg.engine.phases();
    let all = phases();
    let index = |name: &str| wanted.iter().position(|p| p == name);
    let start = match cfg.only.as_ref().or(cfg.from.as_ref()) {
        Some(f) => index(f).ok_or_else(|| format!("no phase `{f}` in the configured phases"))?,
        None => 0,
    };
    let end = match &cfg.to {
        Some(t) => index(t).ok_or_else(|| format!("no phase `{t}` in the configured phases"))?,
        None => wanted.len().saturating_sub(1),
    };
    let tile = cfg.engine.tile.map(|t| t.to_mm()).unwrap_or(DEFAULT_TILE_MM);
    let mut field = new_field(&model.layout, tile);
    let mut text = cfg.text.clone();
    for name in wanted.iter().skip(start) {
        text = strip_plan(&text, name);
    }
    if text != cfg.text {
        (model.file, model.layout) = (cfg.resolve)(&text)?;
    }
    let mut reports = Vec::new();
    let mut skipped = Vec::new();
    for (i, name) in wanted.iter().enumerate() {
        let run_it = match &cfg.only {
            Some(o) => o == name,
            None => i >= start && i <= end,
        };
        if !run_it {
            continue;
        }
        if cfg.stop.is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed)) {
            skipped.push(format!("{name}: stopped"));
            continue;
        }
        match all.iter().find(|p| p.name() == name) {
            Some(p) => {
                if let Some(w) = cfg.watch {
                    w(Event::Start(name));
                }
                let t0 = std::time::Instant::now();
                model.text = text.clone();
                let mut r = p.run(model, &cfg.engine, &mut field);
                r.ms = t0.elapsed().as_millis();
                let t1 = std::time::Instant::now();
                if let Some(section) = plan_section(model, p.name()) {
                    text = write_plan(&text, p.name(), &section);
                    (model.file, model.layout) = (cfg.resolve)(&text)?;
                }
                if let Some(body) = model.finished_escape.take() {
                    text = write_plan(&text, "escape", &body);
                    (model.file, model.layout) = (cfg.resolve)(&text)?;
                }
                if matches!(p.name(), "floorplan" | "place" | "legalise")
                    && let Some(plan) = &model.placement
                    && !plan.moves.is_empty()
                {
                    text = write_moves(&text, &plan.moves)?;
                    (model.file, model.layout) = (cfg.resolve)(&text)?;
                }
                r.reload_ms = t1.elapsed().as_millis();
                r.score = Some(score_of(model, Some(&field)));
                if let Some(w) = cfg.watch {
                    w(Event::Done(&r));
                }
                reports.push(r);
            }
            None => skipped.push(format!("{name}: not implemented yet")),
        }
    }
    let score = score_of(model, Some(&field));
    Ok(RunReport {
        phases: reports,
        score,
        skipped,
        text,
        placement: model.placement.clone(),
        escape: model.escape.clone(),
        planes: model.planes.clone(),
        layers: model.layers.clone(),
        global: model.global.clone(),
        detail: model.detail.clone(),
    })
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

pub fn plan_section(model: &Model, phase: &str) -> Option<String> {
    let nets = &model.layout.nets;
    match phase {
        "escape" => {
            let plan = model.escape.as_ref()?;
            let mut t = String::new();
            for tr in &plan.tracks {
                t += &track_toml(&nets[tr.net].name, &tr.layer, Some(tr.width), &tr.points);
            }
            for v in &plan.vias {
                t += &via_toml(&nets[v.net].name, v.at, &v.via);
            }
            Some(t)
        }
        "detail" => {
            let plan = model.detail.as_ref()?;
            let mut t = String::new();
            for tr in &plan.tracks {
                t += &track_toml(&tr.net, &tr.layer, tr.width, &tr.points);
            }
            for v in &plan.vias {
                t += &via_toml(&v.net, v.at, &v.via);
            }
            Some(t)
        }
        "planes" => {
            let plan = model.planes.as_ref()?;
            let mut t = String::new();
            for z in &plan.zones {
                let pts: Vec<String> = z.outline.iter().map(|q| pt(*q)).collect();
                t += &format!(
                    "\n[[zones]]\nnet = \"{}\"\nlayers = [\"{}\"]\npriority = {}\noutline = [{}]\n",
                    z.net,
                    z.layer,
                    z.priority,
                    pts.join(", ")
                );
            }
            Some(t)
        }
        _ => None,
    }
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
        t.remove("label");
    }
    Ok(doc.to_string())
}
