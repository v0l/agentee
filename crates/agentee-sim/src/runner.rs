use agentee_core::Severity;
use agentee_core::project::{ItemRef, Project};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    static CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

pub fn with_cancel<T>(flag: Arc<AtomicBool>, f: impl FnOnce() -> T) -> T {
    CANCEL.with(|c| *c.borrow_mut() = Some(flag));
    let out = f();
    CANCEL.with(|c| *c.borrow_mut() = None);
    out
}

pub(crate) fn cancelled() -> bool {
    CANCEL.with(|c| c.borrow().as_ref().is_some_and(|f| f.load(Ordering::Relaxed)))
}

struct Tracker {
    state: std::sync::Arc<std::sync::Mutex<agentee_core::sim::SimProgress>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    path: PathBuf,
}

impl Tracker {
    fn start(spec: &Path, runs: usize, max_steps: usize) -> Tracker {
        use std::sync::atomic::Ordering;
        let now = agentee_core::sim::now();
        let state = std::sync::Arc::new(std::sync::Mutex::new(agentee_core::sim::SimProgress {
            run: 0,
            runs,
            port: String::new(),
            steps: 0,
            max_steps,
            decay_db: 0.0,
            started: now,
            updated: now,
            pid: std::process::id(),
            phase: "preparing".into(),
        }));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let path = agentee_core::sim::progress_path(spec);
        let write = {
            let (state, path) = (state.clone(), path.clone());
            move || {
                let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                s.updated = agentee_core::sim::now();
                if let Ok(text) = serde_json::to_string(&*s) {
                    let _ = std::fs::write(&path, text);
                }
            }
        };
        write();
        let thread = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    if !stop.load(Ordering::Relaxed) {
                        write();
                    }
                }
            })
        };
        Tracker { state, stop, thread: Some(thread), path }
    }

    fn update(&self, f: impl FnOnce(&mut agentee_core::sim::SimProgress)) {
        f(&mut self.state.lock().unwrap_or_else(|e| e.into_inner()));
    }

    fn phase(&self, phase: &str) {
        self.update(|s| s.phase = phase.to_string());
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn run(
    p: &Project,
    name: &str,
    dry: bool,
    progress: &mut dyn FnMut(&str, usize, f64),
) -> Result<Value, String> {
    let r = p
        .find(&format!("sim:{name}"))
        .or_else(|| p.find(name))
        .ok_or_else(|| format!("no item named `{name}`"))?;
    let ItemRef::Sim(i) = r else {
        return Err(format!("`{name}` is not a simulation"));
    };
    let entry = &p.sims[i];
    if entry.diags.iter().any(|d| d.severity == Severity::Error) {
        return Err(format!("{} has errors, fix them first (agentee check)", entry.name));
    }
    let spec = &entry.item;
    let layout = p.layouts.iter().find(|l| l.name == spec.layout).ok_or("layout is missing")?;
    let board = p.boards.iter().find(|b| b.name == layout.item.board).ok_or("board is missing")?;
    let src = std::fs::read_to_string(&entry.path).map_err(|e| e.to_string())?;
    let tracker = (!dry).then(|| Tracker::start(&entry.path, spec.excite.len(), spec.max_steps));
    let phase = |name: &str| {
        if let Some(t) = &tracker {
            t.phase(name);
        }
    };
    if spec.kind != agentee_core::sim::SimKind::Fdtd {
        phase("solving");
    }
    if spec.kind == agentee_core::sim::SimKind::Cascade {
        return run_cascade(p, entry, &src);
    }
    if spec.kind == agentee_core::sim::SimKind::Channel {
        return run_channel(p, entry, &src);
    }
    if spec.kind == agentee_core::sim::SimKind::Pdn {
        return run_pdn(p, entry, &src);
    }
    if spec.kind != agentee_core::sim::SimKind::Fdtd {
        let hash = agentee_core::sim::hash(&src);
        let mut result = match spec.kind {
            agentee_core::sim::SimKind::Dc => {
                crate::boardsim::dc(&layout.item, &board.item, spec, hash)?
            }
            _ => crate::boardsim::thermal(&layout.item, &board.item, spec, hash)?,
        };
        result.layout_hash = Some(agentee_core::sim::copper_hash(&layout.item, &board.item, None));
        let json_path = agentee_core::sim::result_path(&entry.path);
        std::fs::write(&json_path, serde_json::to_string(&result).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        return Ok(json!({
            "sim": spec.name,
            "kind": spec.kind,
            "cells": result.cells,
            "iterations": result.iterations,
            "residual": result.residual,
            "seconds": (result.seconds * 100.0).round() / 100.0,
            "device": result.device,
            "result": json_path,
            "readings": result.readings,
        }));
    }
    phase("meshing");
    let model = crate::fdtd::model::PcbModel::from_layout(&layout.item, &board.item, spec)?;
    let mut plan = crate::fdtd::plan(
        &model,
        spec.start,
        spec.stop,
        spec.points,
        spec.cell,
        spec.excite.clone(),
        spec.max_steps,
    )?;
    plan.fields = spec.fields.clone();
    plan.far_field = spec.far_field;
    plan.end_db = spec.end_db;
    if dry {
        let g = &plan.sim.grid;
        let min = |v: &[f64]| v.windows(2).map(|w| w[1] - w[0]).fold(f64::MAX, f64::min) * 1e3;
        return Ok(json!({
            "grid": g.dims(),
            "cells_millions": g.cells() as f64 / 1e6,
            "smallest_mm": [min(&g.x), min(&g.y), min(&g.z)],
            "dt_fs": plan.sim.dt * 1e15,
            "end_db": spec.end_db,
            "max_steps": spec.max_steps,
            "runs": plan.excite.len(),
            "inductor_edges": plan.sim.inductors.len(),
        }));
    }
    phase("running");
    let ports: Vec<String> = spec.excite.iter().map(|j| spec.ports[*j].name.clone()).collect();
    let mut report = |port: &str, steps: usize, db: f64| {
        progress(port, steps, db);
        if let Some(t) = &tracker {
            t.update(|s| {
                s.run = ports.iter().position(|p| p == port).unwrap_or(0);
                s.port = port.to_string();
                s.steps = steps;
                s.decay_db = db;
            });
        }
    };
    let outcome =
        crate::fdtd::execute(&plan, &spec.name, agentee_core::sim::hash(&src), &mut report);
    phase("writing");
    let mut result = outcome?;
    result.layout_hash =
        Some(agentee_core::sim::copper_hash(&layout.item, &board.item, spec.region));
    let json_path = agentee_core::sim::result_path(&entry.path);
    std::fs::write(&json_path, serde_json::to_string(&result).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let touch = json_path.with_extension("").with_extension(format!("s{}p", result.ports.len()));
    std::fs::write(&touch, crate::fdtd::touchstone(&result)).map_err(|e| e.to_string())?;
    let marks: Vec<f64> = [0.1, 0.25, 0.5, 0.75, 1.0]
        .iter()
        .map(|k| spec.start + k * (spec.stop - spec.start))
        .collect();
    let mut table = Vec::new();
    for f in marks {
        let k = result.freqs.iter().position(|x| *x >= f - 1.0).unwrap_or(result.freqs.len() - 1);
        let mut row = json!({ "freq_hz": result.freqs[k] });
        for j in (0..result.ports.len()).filter(|j| result.excited[*j]) {
            for i in 0..result.ports.len() {
                row[format!("S{}{} dB", i + 1, j + 1)] =
                    json!((result.db(i, j)[k] * 100.0).round() / 100.0);
            }
        }
        table.push(row);
    }
    Ok(json!({
        "sim": spec.name,
        "ports": result.ports,
        "cells": result.cells,
        "grid": result.grid,
        "steps": result.steps,
        "seconds": (result.seconds * 10.0).round() / 10.0,
        "device": result.device,
        "result": json_path,
        "touchstone": touch,
        "summary": table,
        "readings": result.readings,
    }))
}

fn run_pdn(
    p: &Project,
    entry: &agentee_core::project::Entry<agentee_core::sim::Sim>,
    src: &str,
) -> Result<Value, String> {
    use crate::pdn::{Attach, Part};
    let spec = &entry.item;
    let ps = spec.pdn.as_ref().ok_or("the pdn spec has errors")?;
    let board = p.sims.iter().find(|s| s.name == spec.board).ok_or("the board sim is missing")?;
    let r = board.item.result.as_ref().ok_or("the board sim has not run")?;
    let port = |n: &String| r.ports.iter().position(|x| x == n).ok_or(format!("{n} is not a port"));
    let z0s: Vec<f64> = r
        .ports
        .iter()
        .map(|n| {
            board.item.ports.iter().find(|q| &q.name == n).map(|q| q.impedance).unwrap_or(50.0)
        })
        .collect();
    let z0 = z0s[0];
    if z0s.iter().any(|z| (z - z0).abs() > 1e-9) {
        return Err("every port of the board sim needs the same impedance for a pdn".into());
    }
    let dir = entry.path.parent().unwrap_or(std::path::Path::new("."));
    let mut parts = Vec::new();
    let mut texts = Vec::new();
    for c in &ps.decaps {
        let attach = match &c.model {
            agentee_core::sim::DecapModel::Rlc { c, esl, esr } => {
                Attach::Series { r: *esr, l: *esl, c: Some(*c) }
            }
            agentee_core::sim::DecapModel::File { path, mount } => {
                let file = dir.join(path);
                let text = std::fs::read_to_string(&file)
                    .map_err(|e| format!("{}: {e}", file.display()))?;
                let net = agentee_core::rf::parse_touchstone(&text, 2)?;
                texts.push(text);
                Attach::Measured(agentee_core::rf::as_one_port(&net, *mount, z0).0)
            }
        };
        parts.push(Part { name: c.reference.clone(), port: port(&c.port)?, attach });
    }
    if let Some((vp, vr, vl)) = &ps.vrm {
        parts.push(Part {
            name: "vrm".into(),
            port: port(vp)?,
            attach: Attach::Series { r: *vr, l: *vl, c: None },
        });
    }
    let sinks = ps.sinks.iter().map(port).collect::<Result<Vec<_>, _>>()?;
    let (a, b, n) = ps.band.unwrap_or((1e5, *r.freqs.last().unwrap(), 200));
    let freqs: Vec<f64> =
        (0..n).map(|k| a * (b / a).powf(k as f64 / (n - 1).max(1) as f64)).collect();
    let hash = agentee_core::sim::cascade_hash(agentee_core::sim::hash(src), r.spec_hash, &texts);
    let mut out = crate::pdn::run(&spec.name, r, z0, &sinks, &parts, &freqs, ps.target, hash)?;
    out.layout_hash = r.layout_hash;
    let json_path = agentee_core::sim::result_path(&entry.path);
    std::fs::write(&json_path, serde_json::to_string(&out).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "sim": spec.name,
        "kind": spec.kind,
        "target_ohm": ps.target,
        "result": json_path,
        "readings": out.readings,
    }))
}

fn run_channel(
    p: &Project,
    entry: &agentee_core::project::Entry<agentee_core::sim::Sim>,
    src: &str,
) -> Result<Value, String> {
    use agentee_core::rf::Cx;
    let spec = &entry.item;
    let cs = spec.channel_spec.as_ref().ok_or("the channel spec has errors")?;
    let board = p.sims.iter().find(|s| s.name == spec.board).ok_or("the board sim is missing")?;
    let r = board.item.result.as_ref().ok_or("the board sim has not run")?;
    let port = |n: &String| r.ports.iter().position(|x| x == n).ok_or(format!("{n} is not a port"));
    let ids = cs.through.iter().map(port).collect::<Result<Vec<_>, _>>()?;
    let np = r.ports.len();
    let h: Vec<Cx> = (0..r.freqs.len())
        .map(|k| {
            if cs.differential {
                let m: agentee_core::rf::Matrix = (0..np)
                    .map(|a| (0..np).map(|b| Cx::new(r.s[a][b][k][0], r.s[a][b][k][1])).collect())
                    .collect();
                agentee_core::sparam::mixed_mode(&m, [ids[0], ids[2]], [ids[1], ids[3]])[1][0]
            } else {
                let c = r.s[ids[1]][ids[0]][k];
                Cx::new(c[0], c[1])
            }
        })
        .collect();
    let dir = entry.path.parent().unwrap_or(std::path::Path::new("."));
    let load_model = |r: &agentee_core::sim::IbisRef| -> Result<
        (agentee_core::ibis::Model, agentee_core::ibis::Package),
        String,
    > {
        let path = dir.join(&r.file);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let ibis = agentee_core::ibis::parse(&text).map_err(|e| format!("{}: {e}", r.file))?;
        let model = ibis.models.iter().find(|m| m.name == r.model).cloned().ok_or(format!(
            "{} has no model {}, there is {}",
            r.file,
            r.model,
            ibis.models.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")
        ))?;
        let comp = match &r.component {
            Some(c) => ibis
                .components
                .iter()
                .find(|x| &x.name == c)
                .ok_or(format!("{} has no component {c}", r.file))?,
            None => ibis.components.first().ok_or(format!("{} has no [Component]", r.file))?,
        };
        let pkg = r
            .pin
            .as_ref()
            .and_then(|p| comp.pins.get(p))
            .and_then(|(_, p)| *p)
            .unwrap_or(comp.package);
        Ok((model, pkg))
    };
    let mut rise = cs.rise;
    let mut swing = cs.swing;
    let mut h = h;
    let mut notes: Vec<String> = Vec::new();
    if cs.tx.is_some() || cs.rx.is_some() {
        let z0 = board
            .item
            .ports
            .iter()
            .find(|q| q.name == r.ports[ids[0]])
            .map(|q| q.impedance)
            .unwrap_or(50.0);
        let tx = cs.tx.as_ref().map(&load_model).transpose()?;
        let rx = cs.rx.as_ref().map(&load_model).transpose()?;
        let driver = match &tx {
            Some((m, pkg)) => {
                let r_out =
                    m.output_resistance().ok_or(format!("{} has no usable V-I tables", m.name))?;
                if !cs.rise_given
                    && let Some(t) = m.rise_10_90()
                {
                    rise = t;
                }
                if !cs.swing_given
                    && let Some(v) = m.voltage
                {
                    swing = v;
                }
                notes.push(format!(
                    "{}: {:.1} ohm out, {:.0} ps edge, {:.2} pF die, {:.2} nH package",
                    m.name,
                    r_out,
                    rise * 1e12,
                    m.c_comp * 1e12,
                    pkg.l * 1e9
                ));
                Some(agentee_core::ibis::Driver { r_out, c_comp: m.c_comp, pkg: *pkg })
            }
            None => None,
        };
        let receiver = rx.as_ref().map(|(m, pkg)| {
            notes.push(format!(
                "{}: {:.2} pF die, {:.2} nH package",
                m.name,
                m.c_comp * 1e12,
                pkg.l * 1e9
            ));
            agentee_core::ibis::Receiver { c_comp: m.c_comp, pkg: *pkg }
        });
        h = r
            .freqs
            .iter()
            .enumerate()
            .map(|(k, f)| {
                let c = |i: usize, j: usize| {
                    Cx::new(r.s[ids[i]][ids[j]][k][0], r.s[ids[i]][ids[j]][k][1])
                };
                let s2 = [[c(0, 0), c(0, 1)], [c(1, 0), c(1, 1)]];
                let (emf, zs) = match &driver {
                    Some(d) => d.thevenin(*f),
                    None => (Cx::new(2.0, 0.0), Cx::new(z0, 0.0)),
                };
                let (zl, die) = match &receiver {
                    Some(rc) => rc.load(*f),
                    None => (Cx::new(z0, 0.0), Cx::ONE),
                };
                emf * agentee_core::ibis::terminated(s2, z0, zs, zl) * die
            })
            .collect();
    }
    let params = crate::channel::Params {
        bit_rate: cs.bit_rate,
        rise,
        swing,
        prbs: cs.prbs,
        ctle: cs.ctle.as_ref().map(|(dc, z, poles)| crate::channel::Ctle {
            dc_db: *dc,
            zero: *z,
            poles: poles.clone(),
        }),
        dfe_taps: cs.dfe_taps,
    };
    let texts: Vec<String> = [&cs.tx, &cs.rx]
        .into_iter()
        .flatten()
        .filter_map(|x| std::fs::read_to_string(dir.join(&x.file)).ok())
        .collect();
    let hash = agentee_core::sim::cascade_hash(agentee_core::sim::hash(src), r.spec_hash, &texts);
    let mut out = crate::channel::run(&spec.name, &r.freqs, &h, &params, hash);
    out.layout_hash = r.layout_hash;
    let json_path = agentee_core::sim::result_path(&entry.path);
    std::fs::write(&json_path, serde_json::to_string(&out).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "sim": spec.name,
        "kind": spec.kind,
        "bit_rate": cs.bit_rate,
        "ui_ps": out.ui_ps,
        "result": json_path,
        "readings": out.readings,
        "models": notes,
    }))
}

fn run_cascade(
    p: &Project,
    entry: &agentee_core::project::Entry<agentee_core::sim::Sim>,
    src: &str,
) -> Result<Value, String> {
    let spec = &entry.item;
    let board = p.sims.iter().find(|s| s.name == spec.board).ok_or("the board sim is missing")?;
    let result = board.item.result.as_ref().ok_or("the board sim has not run")?;
    let dir = entry.path.parent().unwrap_or(std::path::Path::new("."));
    let mut placed = Vec::new();
    let mut texts = Vec::new();
    let mut notes: Vec<Value> = Vec::new();
    for d in &spec.devices {
        let file = dir.join(&d.file);
        let text =
            std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let n = agentee_core::rf::ports_from_path(&file).ok_or("device files are named .sNp")?;
        let mut net = agentee_core::rf::parse_touchstone(&text, n)?;
        let ports = d
            .ports
            .iter()
            .map(|q| {
                result.ports.iter().position(|x| x == q).ok_or(format!("{q} is not a board port"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if let (Some(mount), Some(&p0)) = (d.mount, ports.first()) {
            let z = board
                .item
                .ports
                .iter()
                .find(|q| q.name == result.ports[p0])
                .map(|q| q.impedance)
                .unwrap_or(50.0);
            let (one, clamped) = agentee_core::rf::as_one_port(&net, mount, z);
            net = one;
            if let (Some(a), Some(b)) = (clamped.first(), clamped.last()) {
                notes.push(json!(format!(
                    "{}: the fixture data reads as negative resistance at {} points from {:.0} to {:.0} MHz, taken as lossless there",
                    d.file,
                    clamped.len(),
                    a / 1e6,
                    b / 1e6
                )));
            }
        }
        placed.push(crate::cascade::Placed {
            name: d.file.clone(),
            net,
            ports,
            datasheet: d.datasheet.clone(),
        });
        texts.push(text);
    }
    let z0: Vec<f64> = result
        .ports
        .iter()
        .map(|n| {
            board.item.ports.iter().find(|q| &q.name == n).map(|q| q.impedance).unwrap_or(50.0)
        })
        .collect();
    let hash =
        agentee_core::sim::cascade_hash(agentee_core::sim::hash(src), result.spec_hash, &texts);
    let budget = crate::cascade::Budget {
        kelvin: spec.ambient + 273.15,
        bandwidth: spec.bandwidth,
        report: spec.report.clone(),
        after: spec.after.clone(),
    };
    let mut out = crate::cascade::run(&spec.name, result, &z0, &placed, &budget, hash)?;
    out.layout_hash = result.layout_hash;
    let json_path = agentee_core::sim::result_path(&entry.path);
    std::fs::write(&json_path, serde_json::to_string(&out).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let touch = json_path.with_extension("").with_extension(format!("s{}p", out.ports.len()));
    std::fs::write(&touch, crate::fdtd::touchstone(&out)).map_err(|e| e.to_string())?;
    Ok(json!({
        "sim": spec.name,
        "kind": spec.kind,
        "ports": out.ports,
        "points": out.freqs.len(),
        "result": json_path,
        "touchstone": touch,
        "readings": out.readings,
        "notes": notes,
    }))
}
