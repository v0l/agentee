use agentee_core::calc::{self, TraceGeometry};
use agentee_core::project::{ItemRef, Kind, Project};
use agentee_core::units::{Amps, Kelvin, Length, Ohms};
use agentee_core::{Diagnostic, Severity};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn load(path: &Path) -> Result<Project, String> {
    Project::load(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load_checked(path: &Path) -> Result<Project, String> {
    let mut p = load(path)?;
    agentee_sim::checks::apply(&mut p);
    Ok(p)
}

pub fn find(p: &Project, name: &str) -> Result<ItemRef, String> {
    p.find(name).ok_or_else(|| {
        let mut names: Vec<&str> = p.all_refs().into_iter().map(|r| p.name_of(r)).collect();
        names.sort();
        let near: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| {
                n.to_lowercase().contains(&name.to_lowercase())
                    || name.to_lowercase().contains(&n.to_lowercase())
            })
            .take(8)
            .collect();
        if near.is_empty() {
            format!("no item named `{name}` in {}", p.root.display())
        } else {
            format!("no item named `{name}`, did you mean: {}", near.join(", "))
        }
    })
}

pub fn diagnostics(p: &Project, item: Option<ItemRef>, min: Severity) -> Vec<&Diagnostic> {
    let all: Vec<&Diagnostic> = match item {
        Some(r) => p.diags_of(r).iter().collect(),
        None => p.diagnostics(),
    };
    all.into_iter().filter(|d| d.severity >= min).collect()
}

pub fn check_report(p: &Project, item: Option<ItemRef>, min: Severity) -> (String, Value, bool) {
    let diags = diagnostics(p, item, min);
    let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
    let warnings = diags.iter().filter(|d| d.severity == Severity::Warning).count();
    let mut text = String::new();
    for d in &diags {
        text += &format!("{d}\n");
    }
    let scope = match item {
        Some(r) => p.name_of(r).to_string(),
        None => format!(
            "{} sims, {} layouts, {} schematics, {} boards, {} symbols, {} footprints",
            p.sims.len(),
            p.layouts.len(),
            p.schematics.len(),
            p.boards.len(),
            p.symbols.len(),
            p.footprints.len()
        ),
    };
    text += &format!("{scope}: {errors} errors, {warnings} warnings\n");
    let v =
        json!({ "ok": errors == 0, "errors": errors, "warnings": warnings, "diagnostics": diags });
    (text, v, errors == 0)
}

pub fn list(p: &Project) -> Value {
    let row = |r: ItemRef, kind: Kind| {
        let d = p.diags_of(r);
        json!({
            "kind": kind,
            "name": p.name_of(r),
            "file": p.path_of(r),
            "errors": d.iter().filter(|x| x.severity == Severity::Error).count(),
            "warnings": d.iter().filter(|x| x.severity == Severity::Warning).count(),
        })
    };
    let mut items = Vec::new();
    for r in p.all_refs() {
        items.push(row(r, r.kind()));
    }
    json!({ "root": p.root, "items": items, "unloadable": p.failures })
}

pub fn show(p: &Project, r: ItemRef) -> Value {
    match r {
        ItemRef::Sim(i) => {
            let s = &p.sims[i].item;
            json!({
                "kind": "sim",
                "file": p.sims[i].path,
                "sim": s,
                "result": s.result.as_ref().map(|r| json!({
                    "ports": r.ports,
                    "freqs_hz": [r.freqs.first(), r.freqs.last(), r.freqs.len()],
                    "cells": r.cells,
                    "steps": r.steps,
                    "seconds": r.seconds,
                    "path": agentee_core::sim::result_path(&p.sims[i].path),
                })),
                "diagnostics": p.sims[i].diags,
            })
        }
        ItemRef::Schematic(i) => {
            let s = &p.schematics[i].item;
            let nets: Vec<Value> = s
                .nets
                .iter()
                .map(|n| {
                    json!({
                        "name": n.name,
                        "class": n.class,
                        "style": n.style,
                        "pins": n.pins.iter().map(|r| s.pin_label(*r)).collect::<Vec<_>>(),
                        "wires": n.wires,
                        "drawn_by_hand": n.drawn_by_hand,
                    })
                })
                .collect();
            let parts: Vec<Value> = s
                .parts
                .iter()
                .map(|part| {
                    let pins: Vec<Value> = part
                        .pins()
                        .map(|(k, pin)| json!({ "number": pin.number, "name": pin.name, "at": part.pin_at(k), "outward": part.pin_outward(k) }))
                        .collect();
                    let b = part.bounds();
                    json!({
                        "ref": part.reference,
                        "value": part.value,
                        "symbol": part.symbol_name,
                        "footprint": part.footprint,
                        "at": part.at,
                        "rotation": part.rotation,
                        "mirror": part.mirror,
                        "unit": part.unit,
                        "bounds_mm": { "min": b.min, "max": b.max },
                        "pins": pins,
                    })
                })
                .collect();
            json!({ "kind": "schematic", "file": p.schematics[i].path, "name": s.name, "parts": parts, "nets": nets, "diagnostics": p.schematics[i].diags })
        }
        ItemRef::Layout(i) => {
            let l = &p.layouts[i].item;
            let parts: Vec<Value> = l
                .parts
                .iter()
                .map(|part| {
                    let pads: Vec<Value> = part
                        .pads
                        .iter()
                        .map(|q| {
                            let mut b = agentee_core::graphic::Bounds::EMPTY;
                            q.outlines.iter().flatten().for_each(|c| b.add(*c));
                            json!({ "number": q.number, "net": q.net.map(|n| l.nets[n].name.clone()), "center": b.center(), "size": b.size(), "layers": q.copper })
                        })
                        .collect();
                    json!({ "ref": part.reference, "footprint": part.footprint_name, "at": part.at, "rotation": part.rotation, "bottom": part.bottom, "pads": pads })
                })
                .collect();
            let ratsnest: Vec<Value> = l
                .ratsnest
                .iter()
                .map(|(a, b, n)| json!({ "net": l.nets[*n].name, "from": a, "to": b }))
                .collect();
            json!({
                "kind": "layout",
                "file": p.layouts[i].path,
                "name": l.name,
                "board": l.board,
                "schematic": l.schematic,
                "outline": l.outline,
                "copper_layers": l.copper,
                "parts": parts,
                "nets": l.nets,
                "tracks": l.tracks,
                "vias": l.vias,
                "zones": l.zones.iter().map(|z| json!({ "net": l.nets[z.net].name, "layer": z.layer, "islands_removed": z.islands_removed })).collect::<Vec<_>>(),
                "unrouted": ratsnest,
                "diagnostics": p.layouts[i].diags,
            })
        }
        ItemRef::Board(i) => {
            let b = &p.boards[i].item;
            json!({
                "kind": "board",
                "file": p.boards[i].path,
                "board": b,
                "copper_layers": b.stackup.copper_names(),
                "finished_thickness_mm": b.stackup.thickness().to_mm(),
                "trace_analysis": b.analyze(),
                "diagnostics": p.boards[i].diags,
            })
        }
        ItemRef::Symbol(i) => {
            let s = &p.symbols[i].item;
            json!({
                "kind": "symbol",
                "file": p.symbols[i].path,
                "symbol": s,
                "diagnostics": p.symbols[i].diags,
            })
        }
        ItemRef::Footprint(i) => {
            let f = &p.footprints[i].item;
            let b = f.bounds();
            let cy = f.courtyard("F");
            json!({
                "kind": "footprint",
                "file": p.footprints[i].path,
                "footprint": f,
                "pad_numbers": f.pad_numbers(),
                "bounds_mm": { "min": b.min, "max": b.max },
                "courtyard_mm": if cy.is_empty() { Value::Null } else { json!({ "min": cy.min, "max": cy.max }) },
                "diagnostics": p.footprints[i].diags,
            })
        }
    }
}

fn write_new(
    dir: &Path,
    stem: &str,
    kind: Kind,
    text: &str,
    force: bool,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{}{}", agentee_kicad::file_stem(stem), kind.ext()));
    if path.exists() && !force {
        return Err(format!("{} exists, pass force to overwrite", path.display()));
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

pub fn import_footprint(spec: &str, dir: &Path, force: bool) -> Result<PathBuf, String> {
    let f = agentee_kicad::import_footprint(spec)?;
    let text = toml::to_string(&f).map_err(|e| e.to_string())?;
    let header = format!("# imported from KiCad {spec}\n");
    write_new(dir, &f.name, Kind::Footprint, &(header + &text), force)
}

pub enum FootprintPick {
    None,
    Default,
    Spec(String),
}

pub fn import_symbol(
    spec: &str,
    dir: &Path,
    fp_dir: &Path,
    pick: FootprintPick,
    force: bool,
) -> Result<(Vec<PathBuf>, Vec<String>), String> {
    let mut s = agentee_kicad::import_symbol(spec)?;
    let mut written = Vec::new();
    let mut notes = Vec::new();
    let fp = match pick {
        FootprintPick::None => None,
        FootprintPick::Spec(f) => Some(f),
        FootprintPick::Default => match s.footprint.clone() {
            Some(f) if f.contains(':') => Some(f),
            _ => {
                let filters = if s.footprint_filters.is_empty() {
                    String::new()
                } else {
                    format!(", its footprint filters are {}", s.footprint_filters.join(" "))
                };
                notes.push(format!(
                    "`{}` names no default footprint{filters}; import one and set `footprint`",
                    s.name
                ));
                None
            }
        },
    };
    if let Some(f) = fp {
        written.push(import_footprint(&f, fp_dir, force)?);
        s.footprint = f.rsplit(':').next().map(str::to_string);
    }
    let text = toml::to_string(&s).map_err(|e| e.to_string())?;
    let header = format!("# imported from KiCad {spec}\n");
    written.insert(0, write_new(dir, &s.name, Kind::Symbol, &(header + &text), force)?);
    Ok((written, notes))
}

pub fn new_item(kind: Kind, name: &str, dir: &Path) -> Result<PathBuf, String> {
    let text = match kind {
        Kind::Board => crate::templates::board(name),
        Kind::Symbol => crate::templates::symbol(name),
        Kind::Footprint => crate::templates::footprint(name),
        Kind::Schematic => format!("name = \"{name}\"\n"),
        Kind::Layout => format!("name = \"{name}\"\n"),
        Kind::Sim => format!(
            "name = \"{name}\"\n\n[frequency]\nstart = \"100MHz\"\nstop = \"4GHz\"\n\n[[ports]]\nname = \"IN\"\npad = \"J1.1\"\n"
        ),
    };
    write_new(dir, name, kind, &text, false)
}

pub fn parse<T>(s: &str, f: fn(&str) -> Result<T, String>, what: &str) -> Result<T, String> {
    f(s).map_err(|e| format!("{what}: {e}"))
}

pub fn trace_width(
    current: &str,
    copper: &str,
    rise: &str,
    internal: bool,
) -> Result<Value, String> {
    let i = parse(current, Amps::parse, "current")?;
    let t = parse(copper, Length::parse, "copper")?;
    let dt = parse(rise, Kelvin::parse, "rise")?;
    let w = calc::ipc2221_width(i.0, dt.0, t.to_mm(), !internal);
    Ok(json!({
        "current": i.to_string(),
        "copper": t.to_string(),
        "temp_rise": dt.to_string(),
        "layer": if internal { "internal" } else { "external" },
        "min_width_mm": (w * 1e4).round() / 1e4,
        "min_width_mil": (w / agentee_core::units::MM_PER_MIL * 100.0).round() / 100.0,
        "method": "IPC-2221",
    }))
}

pub fn run_sim(
    p: &Project,
    name: &str,
    dry: bool,
    progress: &mut dyn FnMut(&str, usize, f64),
) -> Result<Value, String> {
    let r = find(p, &format!("sim:{name}")).or_else(|_| find(p, name))?;
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
    let model = agentee_sim::fdtd::model::PcbModel::from_layout(&layout.item, &board.item, spec)?;
    let plan = agentee_sim::fdtd::plan(
        &model,
        spec.start,
        spec.stop,
        spec.points,
        spec.cell,
        spec.excite.clone(),
        spec.max_steps,
    )?;
    if dry {
        let g = &plan.sim.grid;
        let min = |v: &[f64]| v.windows(2).map(|w| w[1] - w[0]).fold(f64::MAX, f64::min) * 1e3;
        let min_steps = ((1.5 / spec.start) / plan.sim.dt) as usize;
        return Ok(json!({
            "grid": g.dims(),
            "cells_millions": g.cells() as f64 / 1e6,
            "smallest_mm": [min(&g.x), min(&g.y), min(&g.z)],
            "dt_fs": plan.sim.dt * 1e15,
            "min_steps": min_steps.min(spec.max_steps),
            "max_steps": spec.max_steps,
            "runs": plan.excite.len(),
            "inductor_edges": plan.sim.inductors.len(),
        }));
    }
    let result =
        agentee_sim::fdtd::execute(&plan, &spec.name, agentee_core::sim::hash(&src), progress)?;
    let json_path = agentee_core::sim::result_path(&entry.path);
    std::fs::write(&json_path, serde_json::to_string(&result).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let touch = json_path.with_extension("").with_extension(format!("s{}p", result.ports.len()));
    std::fs::write(&touch, agentee_sim::fdtd::touchstone(&result)).map_err(|e| e.to_string())?;
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
    }))
}

pub struct FieldQuery<'a> {
    pub board: Option<&'a str>,
    pub layer: &'a str,
    pub width: Option<&'a str>,
    pub netclass: Option<&'a str>,
    pub gap: Option<&'a str>,
    pub coplanar_gap: Option<&'a str>,
    pub mask: bool,
    pub fine: bool,
}

pub fn field_solve(p: &Project, q: &FieldQuery) -> Result<Value, String> {
    let board = match q.board {
        Some(n) => match find(p, &format!("board:{n}"))? {
            ItemRef::Board(i) => &p.boards[i].item,
            _ => unreachable!(),
        },
        None => match p.boards.as_slice() {
            [one] => &one.item,
            [] => return Err("no board in the project".into()),
            _ => return Err("several boards, name one with board".into()),
        },
    };
    let class = q
        .netclass
        .map(|n| board.netclasses.iter().find(|c| c.name == n).ok_or(format!("no netclass `{n}`")))
        .transpose()?;
    let len = |s: Option<&str>, what| {
        s.map(|v| parse(v, Length::parse, what).map(Length::to_mm)).transpose()
    };
    let width = len(q.width, "width")?
        .or(class.map(|c| c.track_width.to_mm()))
        .ok_or("give width or netclass")?;
    let trace = agentee_sim::xsection::Trace {
        width,
        diff_gap: len(q.gap, "gap")?.or(class.and_then(|c| c.diff_gap.map(Length::to_mm))),
        coplanar_gap: len(q.coplanar_gap, "coplanar_gap")?
            .or(class.and_then(|c| c.coplanar_gap.map(Length::to_mm))),
    };
    let t0 = std::time::Instant::now();
    let r = agentee_sim::xsection::line(board, q.layer, &trace, q.mask, q.fine)?;
    let geometry = board.stackup.geometry(q.layer);
    let line = calc::Line { diff_gap_mm: trace.diff_gap, coplanar_gap_mm: trace.coplanar_gap };
    let formula = geometry.map(|g| (g.impedance(width, line) * 100.0).round() / 100.0);
    Ok(json!({
        "board": board.name,
        "layer": q.layer,
        "width_mm": width,
        "diff_gap_mm": trace.diff_gap,
        "coplanar_gap_mm": trace.coplanar_gap,
        "solder_mask": q.mask,
        "field": r,
        "closed_form_uncoated_ohm": formula,
        "seconds": (t0.elapsed().as_secs_f64() * 1000.0).round() / 1000.0,
    }))
}

pub struct ImpedanceQuery<'a> {
    pub project: Option<&'a Project>,
    pub board: Option<&'a str>,
    pub layer: Option<&'a str>,
    pub width: Option<&'a str>,
    pub gap: Option<&'a str>,
    pub coplanar_gap: Option<&'a str>,
    pub target: Option<&'a str>,
    pub h: Option<&'a str>,
    pub er: Option<f64>,
    pub t: Option<&'a str>,
}

pub fn impedance(q: &ImpedanceQuery) -> Result<Value, String> {
    let geometry = match (q.h, q.er) {
        (Some(h), Some(er)) => TraceGeometry::Microstrip {
            h_mm: parse(h, Length::parse, "h")?.to_mm(),
            er,
            t_mm: parse(q.t.unwrap_or("1oz"), Length::parse, "t")?.to_mm(),
        },
        _ => {
            let p = q.project.ok_or("give a project with a board, or --h and --er")?;
            let board = match q.board {
                Some(n) => match find(p, n)? {
                    ItemRef::Board(i) => &p.boards[i].item,
                    _ => return Err(format!("`{n}` is not a board")),
                },
                None => match p.boards.as_slice() {
                    [one] => &one.item,
                    [] => return Err("no board in the project".into()),
                    _ => return Err("several boards, name one with board".into()),
                },
            };
            let layer = q.layer.unwrap_or("F.Cu");
            board.stackup.geometry(layer).ok_or_else(|| {
                format!(
                    "`{layer}` is not a copper layer of {} ({})",
                    board.name,
                    board.stackup.copper_names().join(", ")
                )
            })?
        }
    };
    let gap = calc::Line {
        diff_gap_mm: q.gap.map(|g| parse(g, Length::parse, "gap")).transpose()?.map(Length::to_mm),
        coplanar_gap_mm: q
            .coplanar_gap
            .map(|g| parse(g, Length::parse, "coplanar_gap"))
            .transpose()?
            .map(Length::to_mm),
    };
    if gap.coplanar_gap_mm.is_some() && !geometry.is_external() {
        return Err("coplanar_gap is only modelled on outer layers".into());
    }
    let mut v = json!({ "geometry": geometry, "line": gap });
    if let Some(w) = q.width {
        let w = parse(w, Length::parse, "width")?;
        v["width_mm"] = json!(w.to_mm());
        v["impedance_ohm"] = json!((geometry.impedance(w.to_mm(), gap) * 100.0).round() / 100.0);
    }
    if let Some(t) = q.target {
        let t = parse(t, Ohms::parse, "target")?;
        v["target_ohm"] = json!(t.0);
        v["width_for_target_mm"] = match geometry.width_for(t.0, gap) {
            Some(w) => json!((w * 1e4).round() / 1e4),
            None => Value::Null,
        };
    }
    if q.width.is_none() && q.target.is_none() {
        return Err("give width, target, or both".into());
    }
    Ok(v)
}
