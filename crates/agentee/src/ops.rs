use agentee_core::calc::{self, TraceGeometry};
use agentee_core::project::{ItemRef, Kind, Project};
use agentee_core::units::{Amps, Kelvin, Length, Ohms};
use agentee_core::{Diagnostic, Severity};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn load(path: &Path) -> Result<Project, String> {
    Project::load(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load_footprints(path: &Path) -> Result<Project, String> {
    Project::load_footprints(path).map_err(|e| format!("{}: {e}", path.display()))
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
                "logic": s.logic_result.as_ref().map(|r| json!({
                    "passed": r.passed,
                    "failures": r.failures,
                    "readings": r.readings,
                    "vcd": r.vcd,
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
                "board_cutouts": l.board_cutouts,
                "copper_layers": l.copper,
                "parts": parts,
                "nets": l.nets,
                "tracks": l.tracks,
                "vias": l.vias,
                "zones": l.zones.iter().map(|z| json!({ "net": l.nets[z.net].name, "layer": z.layer, "islands_removed": z.islands_removed })).collect::<Vec<_>>(),
                "unrouted": ratsnest,
                "silk": l.silk,
                "watermark": l.watermark,
                "pairs": l.pairs,
                "match_groups": l.match_groups,
                "interfaces": l.interfaces,
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

pub fn import_board(
    path: &Path,
    dir: &Path,
    force: bool,
) -> Result<(Vec<PathBuf>, Vec<String>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let pro = std::fs::read_to_string(path.with_extension("kicad_pro")).ok();
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("board");
    let name = agentee_kicad::file_stem(name);
    let b = agentee_kicad::board::import_board(&text, pro.as_deref(), &name)?;
    let header = format!("# imported from KiCad {}\n", path.display());
    let mut written = Vec::new();
    let emit = |stem: &str, kind: Kind, body: String, sub: &str, written: &mut Vec<PathBuf>| {
        let d = if sub.is_empty() { dir.to_path_buf() } else { dir.join(sub) };
        write_new(&d, stem, kind, &(header.clone() + &body), force).map(|p| written.push(p))
    };
    let t = |r: Result<String, toml::ser::Error>| r.map_err(|e| e.to_string());
    emit(&name, Kind::Board, t(toml::to_string(&b.board))?, "", &mut written)?;
    emit(&name, Kind::Schematic, t(toml::to_string(&b.schematic))?, "", &mut written)?;
    emit(&name, Kind::Layout, t(toml::to_string(&b.layout))?, "", &mut written)?;
    for f in &b.footprints {
        emit(&f.name, Kind::Footprint, t(toml::to_string(f))?, "footprints", &mut written)?;
    }
    for s in &b.symbols {
        emit(&s.name, Kind::Symbol, t(toml::to_string(s))?, "symbols", &mut written)?;
    }
    let mut notes = b.notes;
    if !b.layout.zones.is_empty() {
        let fill = write_fills(&load(dir)?, &name)?;
        notes.push(format!("stored {} zone fills in the layout", fill["fills"]));
    }
    Ok((written, notes))
}

pub fn write_fills(p: &Project, name: &str) -> Result<Value, String> {
    let r = find(p, &format!("pcb:{name}")).or_else(|_| find(p, name))?;
    let ItemRef::Layout(i) = r else {
        return Err(format!("`{name}` is not a layout"));
    };
    let entry = &p.layouts[i];
    let layout = &entry.item;
    #[derive(serde::Serialize)]
    struct Fills {
        fills: Vec<agentee_core::layout::FillFile>,
    }
    let fills = Fills {
        fills: layout
            .fill_keys
            .iter()
            .zip(&layout.zones)
            .map(|(k, z)| agentee_core::layout::fill_file(k, z))
            .collect(),
    };
    let path = &entry.path;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    doc.remove("fills");
    let mut out = doc.to_string().trim_end().to_string();
    out.push('\n');
    if !fills.fills.is_empty() {
        out.push('\n');
        out += &toml::to_string(&fills).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, &out).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(json!({
        "layout": layout.name,
        "file": path,
        "fills": fills.fills.len(),
        "refilled": layout.fill_keys.iter().filter(|k| !k.stored).count(),
        "points": fills.fills.iter().flat_map(|f| &f.rings).map(|r| r.len()).sum::<usize>(),
    }))
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
    agentee_sim::runner::run(p, name, dry, progress)
}

pub fn fetch_models(p: &Project) -> Result<Value, String> {
    let mut generated: Vec<&str> = Vec::new();
    let mut wanted: Vec<String> = Vec::new();
    for f in p.footprints.iter().map(|f| &f.item) {
        let own = f.model.as_deref().is_some_and(|m| agentee_3d::in_project(m, &p.root).is_some());
        if !own && agentee_3d::parametric::generate(f).is_some() {
            generated.push(&f.name);
        } else if let Some(m) = &f.model {
            wanted.push(m.clone());
        }
    }
    wanted.sort();
    wanted.dedup();
    let mut models = Vec::new();
    for m in wanted {
        let local = agentee_3d::locate(&m, &p.root);
        let fetched = local.is_none();
        let row = match agentee_3d::get(&m, &p.root, agentee_3d::Fetch::Blocking) {
            agentee_3d::Status::Ready(mesh) => json!({
                "model": m,
                "path": agentee_3d::locate(&m, &p.root),
                "fetched": fetched,
                "triangles": mesh.triangles(),
            }),
            agentee_3d::Status::Missing(e) => json!({ "model": m, "error": e }),
            agentee_3d::Status::Pending => json!({ "model": m, "error": "pending" }),
        };
        models.push(row);
    }
    Ok(json!({ "cache": agentee_3d::cache_dir(), "generated": generated.len(), "models": models }))
}

pub fn serpentine(
    from: &str,
    to: &str,
    add: &str,
    amplitude: &str,
    pitch: &str,
) -> Result<Value, String> {
    let point = |v: &str| -> Result<[f64; 2], String> {
        let parts: Vec<f64> = v
            .split(',')
            .map(|x| x.trim().parse::<f64>())
            .collect::<Result<_, _>>()
            .map_err(|_| format!("`{v}` is not x,y"))?;
        match parts.as_slice() {
            [x, y] => Ok([*x, *y]),
            _ => Err(format!("`{v}` is not x,y")),
        }
    };
    let mm = |v: &str, what| parse(v, Length::parse, what).map(Length::to_mm);
    let pts = agentee_core::layout::serpentine(
        point(from)?,
        point(to)?,
        mm(add, "add")?,
        mm(amplitude, "amplitude")?,
        mm(pitch, "pitch")?,
    )?;
    let len: f64 = pts
        .windows(2)
        .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
        .sum();
    Ok(json!({
        "points": pts.iter().map(|p| [(p[0] * 1e4).round() / 1e4, (p[1] * 1e4).round() / 1e4]).collect::<Vec<_>>(),
        "length_mm": len,
    }))
}

pub struct SparamQuery<'a> {
    pub tdr: Option<&'a str>,
    pub rise: Option<&'a str>,
    pub pair: Option<&'a str>,
    pub xtalk: Option<&'a str>,
}

fn parse_time(v: &str) -> Option<f64> {
    let v = v.trim().to_lowercase();
    for (suffix, scale) in [("fs", 1e-15), ("ps", 1e-12), ("ns", 1e-9), ("us", 1e-6), ("s", 1.0)] {
        if let Some(n) = v.strip_suffix(suffix) {
            return n.trim().parse::<f64>().ok().map(|x| x * scale);
        }
    }
    None
}

fn thin(v: &[f64], n: usize) -> Vec<f64> {
    let step = (v.len() / n.max(1)).max(1);
    v.iter().step_by(step).map(|x| (x * 1000.0).round() / 1000.0).collect()
}

pub fn sparam(p: &Project, name: &str, q: &SparamQuery) -> Result<Value, String> {
    use agentee_core::rf::Cx;
    use agentee_core::sparam as sp;
    let r = find(p, &format!("sim:{name}")).or_else(|_| find(p, name))?;
    let ItemRef::Sim(i) = r else { return Err(format!("`{name}` is not a simulation")) };
    let res = p.sims[i].item.result.as_ref().ok_or("not run yet")?;
    let np = res.ports.len();
    let port = |n: &str| -> Result<usize, String> {
        res.ports
            .iter()
            .position(|x| x == n)
            .or_else(|| n.parse::<usize>().ok().filter(|k| *k >= 1 && *k <= np).map(|k| k - 1))
            .ok_or(format!("no port `{n}`, there is {}", res.ports.join(", ")))
    };
    let at = |k: usize| -> agentee_core::rf::Matrix {
        (0..np)
            .map(|a| (0..np).map(|b| Cx::new(res.s[a][b][k][0], res.s[a][b][k][1])).collect())
            .collect()
    };
    let full = res.excited.iter().all(|e| *e);
    let (mut worst_gain, mut worst_f, mut recip) = (0.0f64, 0.0, 0.0f64);
    if full {
        for (k, f) in res.freqs.iter().enumerate() {
            let m = at(k);
            let g = sp::passivity(&m);
            if g > worst_gain {
                worst_gain = g;
                worst_f = *f;
            }
            recip = recip.max(sp::reciprocity(&m));
        }
    }
    let fmax = *res.freqs.last().unwrap();
    let rise = match q.rise {
        Some(v) => parse_time(v).ok_or(format!("cannot read `{v}` as a time, like 35ps"))?,
        None => (1.3 / fmax).max(10e-12),
    };
    let mut out = json!({
        "sim": res.name,
        "ports": res.ports,
        "fully_excited": full,
        "passivity": if full { json!({ "largest_gain": worst_gain, "at_hz": worst_f, "passive": worst_gain <= 1.0 + 1e-3 }) } else { Value::Null },
        "reciprocity_error": if full { json!(recip) } else { Value::Null },
    });
    if let Some(t) = q.tdr {
        let k = port(t)?;
        let s11: Vec<Cx> = res.s[k][k].iter().map(|c| Cx::new(c[0], c[1])).collect();
        let st = sp::step(&res.freqs, &s11, rise, Some(40.0 * rise));
        let z0 = p.sims[i].item.ports.get(k).map(|x| x.impedance).unwrap_or(50.0);
        let z = sp::tdr_impedance(&st, z0);
        let settled: Vec<(f64, f64)> = st
            .time_ps
            .iter()
            .zip(&z)
            .filter(|(t, _)| **t > 2.0 * rise * 1e12)
            .map(|(t, z)| (*t, *z))
            .collect();
        let lo = settled.iter().copied().min_by(|a, b| a.1.total_cmp(&b.1));
        let hi = settled.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1));
        out["tdr"] = json!({
            "port": res.ports[k],
            "rise_ps": st.rise_ps,
            "warning": st.warning,
            "lowest": lo.map(|x| json!({ "ohm": x.1, "at_ps": x.0 })),
            "highest": hi.map(|x| json!({ "ohm": x.1, "at_ps": x.0 })),
            "time_ps": thin(&st.time_ps, 400),
            "ohm": thin(&z, 400),
        });
    }
    if let Some(spec) = q.pair {
        let ids = spec.split(',').map(|x| port(x.trim())).collect::<Result<Vec<_>, _>>()?;
        let [a, b, c, d] = ids.as_slice() else {
            return Err("pair is IN+,IN-,OUT+,OUT-".into());
        };
        if !full {
            return Err("mixed mode needs every port driven".into());
        }
        let mm: Vec<[[Cx; 4]; 4]> =
            (0..res.freqs.len()).map(|k| sp::mixed_mode(&at(k), [*a, *c], [*b, *d])).collect();
        let db = |i: usize, j: usize| -> Vec<f64> {
            mm.iter().map(|m| (m[i][j].db() * 100.0).round() / 100.0).collect()
        };
        out["mixed_mode"] = json!({
            "freq_hz": res.freqs,
            "sdd21_db": db(1, 0),
            "sdd11_db": db(0, 0),
            "scc21_db": db(3, 2),
            "scd21_db": db(3, 0),
            "sdc21_db": db(1, 2),
        });
    }
    if let Some(spec) = q.xtalk {
        let ids = spec.split(',').map(|x| port(x.trim())).collect::<Result<Vec<_>, _>>()?;
        let [from, to] = ids.as_slice() else { return Err("xtalk is FROM,TO".into()) };
        if !res.excited[*from] {
            return Err(format!("{} was not driven", res.ports[*from]));
        }
        let s: Vec<Cx> = res.s[*to][*from].iter().map(|c| Cx::new(c[0], c[1])).collect();
        let st = sp::step(&res.freqs, &s, rise, Some(40.0 * rise));
        let peak =
            st.value.iter().copied().fold(0.0f64, |a, v| if v.abs() > a.abs() { v } else { a });
        let worst =
            s.iter().zip(&res.freqs).map(|(c, f)| (c.db(), *f)).max_by(|a, b| a.0.total_cmp(&b.0));
        out["crosstalk"] = json!({
            "from": res.ports[*from],
            "to": res.ports[*to],
            "worst_db": worst.map(|w| w.0),
            "worst_at_hz": worst.map(|w| w.1),
            "step_peak": peak,
            "rise_ps": st.rise_ps,
        });
    }
    Ok(out)
}

pub fn fab(p: &Project, name: &str, out: &std::path::Path) -> Result<Value, String> {
    let r = find(p, &format!("pcb:{name}")).or_else(|_| find(p, name))?;
    let ItemRef::Layout(i) = r else {
        return Err(format!("`{name}` is not a layout"));
    };
    let entry = &p.layouts[i];
    let errors: Vec<String> = entry
        .diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.message.clone())
        .collect();
    if !errors.is_empty() {
        return Err(format!("{} has errors, fix them first: {}", entry.name, errors.join("; ")));
    }
    let layout = &entry.item;
    let board = p.boards.iter().find(|b| b.name == layout.board).ok_or("board is missing")?;
    let sch =
        p.schematics.iter().find(|s| s.name == layout.schematic).ok_or("schematic is missing")?;
    let report = agentee_fab::package(layout, &board.item, &sch.item, out)?;
    let mut files = report.files.clone();
    for (side, show, hide) in [
        (
            "top",
            vec!["F.Fab", "F.SilkS", "Edge.Cuts"],
            vec!["In1.Cu", "In2.Cu", "B.Cu", "B.SilkS", "B.Fab"],
        ),
        (
            "bottom",
            vec!["B.Fab", "B.SilkS", "Edge.Cuts"],
            vec!["F.Cu", "In1.Cu", "In2.Cu", "F.SilkS", "F.Fab"],
        ),
    ] {
        let opts = agentee_view::RenderOptions {
            width: 2000,
            height: 1400,
            scale: 1.0,
            unit: 1,
            panels: false,
            hidden_pins: false,
            show: show.into_iter().map(String::from).collect(),
            hide: hide.into_iter().map(String::from).collect(),
            region: None,
        };
        let png = agentee_view::render_png(p, r, &opts);
        let name = format!("assembly-{side}.png");
        std::fs::write(out.join(&name), png).map_err(|e| e.to_string())?;
        files.push(name);
    }
    let warnings = entry.diags.iter().filter(|d| d.severity == Severity::Warning).count();
    Ok(json!({
        "layout": entry.name,
        "dir": report.dir,
        "files": files,
        "parts_placed": report.parts_placed,
        "bom_lines": report.bom_lines,
        "holes": report.holes,
        "warnings": warnings,
    }))
}

pub fn drc(p: &Project, name: &str, list: bool) -> Result<(String, Value), String> {
    use agentee_core::drc;
    let r = find(p, &format!("pcb:{name}")).or_else(|_| find(p, name))?;
    let (board, setup, diags) = match r {
        ItemRef::Layout(i) => {
            let e = &p.layouts[i];
            let b = p
                .boards
                .iter()
                .find(|b| b.name == e.item.board)
                .ok_or_else(|| format!("board `{}` is missing", e.item.board))?;
            let setup = drc::Setup::of(&drc::Ctx::of_layout(&b.item, &e.item));
            let diags: Vec<&Diagnostic> = e.diags.iter().filter(|d| d.rule.is_some()).collect();
            (&b.item, setup, diags)
        }
        ItemRef::Board(i) => (&p.boards[i].item, drc::Setup::of_board(&p.boards[i].item), vec![]),
        _ => return Err(format!("`{name}` is not a layout or a board")),
    };
    if list {
        let rules = drc::status(board, &setup);
        let mut text = String::new();
        for r in &rules {
            let state = match (r.enabled, r.applies) {
                (false, _) => "disabled".to_string(),
                (true, true) => format!("applies ({})", r.when),
                (true, false) => format!("skipped, needs {}", r.when),
            };
            let sev = format!("{:?}", r.severity).to_lowercase();
            let cat = serde_json::to_value(r.category).unwrap_or_default();
            text += &format!(
                "{:<22} {:<9} {:<8} {state}: {}\n",
                r.id,
                cat.as_str().unwrap_or_default(),
                sev,
                r.summary
            );
        }
        return Ok((text, json!({ "setup": setup, "rules": rules })));
    }
    let mut text = String::new();
    for d in &diags {
        text += &format!("{d}\n");
    }
    let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
    text += &format!("{}: {errors} DRC errors, {} DRC diagnostics\n", p.name_of(r), diags.len());
    Ok((text, json!({ "setup": setup, "diagnostics": diags })))
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
    pub sweep: Option<&'a str>,
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
    let loss = match q.sweep {
        None => None,
        Some(spec) => {
            let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
            let [a, b, n] = parts.as_slice() else {
                return Err("sweep is START,STOP,POINTS like 10MHz,20GHz,21".into());
            };
            let f = |v: &str| agentee_core::sim::freq(v).ok_or(format!("cannot read `{v}`"));
            let (a, b) = (f(a)?, f(b)?);
            let n: usize = n.parse().map_err(|_| format!("cannot read `{n}` as a count"))?;
            let freqs: Vec<f64> = (0..n.max(1))
                .map(|k| if n <= 1 { a } else { a * (b / a).powf(k as f64 / (n - 1) as f64) })
                .collect();
            let model = agentee_sim::loss::Model {
                roughness: agentee_sim::loss::Roughness {
                    rms_um: board.stackup.roughness_um,
                    huray_radius_um: board.stackup.huray.map(|h| h.0),
                    huray_ratio: board.stackup.huray.map(|h| h.1),
                },
                ..Default::default()
            };
            Some(agentee_sim::loss::line(board, q.layer, &trace, q.mask, q.fine, &model, &freqs)?)
        }
    };
    let pair = match trace.diff_gap {
        Some(_) => Some(agentee_sim::xsection::pair(board, q.layer, &trace, q.mask, q.fine)?),
        None => None,
    };
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
        "pair": pair,
        "loss": loss,
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

pub fn unroute(p: &Project, name: &str, nets: &[String]) -> Result<usize, String> {
    let r = find(p, &format!("pcb:{name}")).or_else(|_| find(p, name))?;
    let ItemRef::Layout(i) = r else {
        return Err(format!("`{name}` is not a layout"));
    };
    let path = &p.layouts[i].path;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let mut removed = 0;
    for key in ["tracks", "vias"] {
        if let Some(arr) = doc.get_mut(key).and_then(|v| v.as_array_of_tables_mut()) {
            let before = arr.len();
            arr.retain(|t| {
                let net = t.get("net").and_then(|v| v.as_str()).unwrap_or("");
                !nets.iter().any(|g| agentee_core::layout::glob(g, net))
            });
            removed += before - arr.len();
        }
    }
    std::fs::write(path, doc.to_string()).map_err(|e| e.to_string())?;
    Ok(removed)
}

pub fn route(
    p: &Project,
    name: &str,
    opts: &agentee_core::route::RouteOptions,
    write: bool,
) -> Result<Value, String> {
    let r = find(p, &format!("pcb:{name}")).or_else(|_| find(p, name))?;
    let ItemRef::Layout(i) = r else {
        return Err(format!("`{name}` is not a layout"));
    };
    let entry = &p.layouts[i];
    let layout = &entry.item;
    let board = p.boards.iter().find(|b| b.name == layout.board).ok_or("board is missing")?;
    let result = agentee_core::route::route(layout, &board.item, opts)?;
    if write {
        append_route(&entry.path, &format!("agentee route {}", opts.nets.join(" ")), &result)?;
    }
    Ok(json!({
        "layout": entry.name,
        "written": write,
        "connections": result.connections,
        "routed": result.routed,
        "tracks": result.tracks.len(),
        "vias": result.vias.len(),
        "failed": result.failed,
    }))
}

fn append_route(
    path: &Path,
    header: &str,
    result: &agentee_core::route::RouteResult,
) -> Result<(), String> {
    if result.tracks.is_empty() && result.vias.is_empty() {
        return Ok(());
    }
    let f = |v: f64| {
        let s = format!("{:.4}", v);
        let s = s.trim_end_matches('0');
        if s.ends_with('.') { format!("{s}0") } else { s.to_string() }
    };
    let pt = |q: [f64; 2]| format!("[{}, {}]", f(q[0]), f(q[1]));
    let mut text = format!("\n# {header}\n");
    for t in &result.tracks {
        let pts: Vec<String> = t.points.iter().map(|q| pt(*q)).collect();
        let width = t.width.map(|w| format!("width = {}\n", f(w))).unwrap_or_default();
        text += &format!(
            "\n[[tracks]]\nnet = \"{}\"\nlayer = \"{}\"\n{width}points = [{}]\n",
            t.net,
            t.layer,
            pts.join(", ")
        );
    }
    for v in &result.vias {
        text +=
            &format!("\n[[vias]]\nnet = \"{}\"\nat = {}\nvia = \"{}\"\n", v.net, pt(v.at), v.via);
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    std::io::Write::write_all(&mut file, text.as_bytes()).map_err(|e| e.to_string())
}

pub fn tune(
    p: &Project,
    name: &str,
    opts: &agentee_core::tune::TuneOptions,
    write: bool,
) -> Result<Value, String> {
    let r = find(p, &format!("pcb:{name}")).or_else(|_| find(p, name))?;
    let ItemRef::Layout(i) = r else {
        return Err(format!("`{name}` is not a layout"));
    };
    let entry = &p.layouts[i];
    let board = p.boards.iter().find(|b| b.name == entry.item.board).ok_or("board is missing")?;
    let result = agentee_core::tune::tune(&entry.item, &board.item, opts)?;
    if write && !result.edits.is_empty() {
        let path = &entry.path;
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
        let tracks = doc
            .get_mut("tracks")
            .and_then(|v| v.as_array_of_tables_mut())
            .ok_or("the layout has no [[tracks]]")?;
        let round = |v: f64| (v * 1e4).round() / 1e4;
        for e in &result.edits {
            let t = tracks.get_mut(e.track).ok_or("track index out of range")?;
            let mut arr = toml_edit::Array::new();
            for q in &e.points {
                let mut pt = toml_edit::Array::new();
                pt.push(round(q[0]));
                pt.push(round(q[1]));
                arr.push(pt);
            }
            t["points"] = toml_edit::value(arr);
        }
        std::fs::write(path, doc.to_string()).map_err(|e| e.to_string())?;
    }
    Ok(json!({
        "layout": entry.name,
        "written": write,
        "tuned": result.tuned,
        "failed": result.failed,
        "over": result.over,
        "tracks_changed": result.edits.len(),
    }))
}

pub fn neck(
    root: &Path,
    name: &str,
    opts: &agentee_core::neck::NeckOptions,
    write: bool,
) -> Result<Value, String> {
    let p = load(root)?;
    let r = find(&p, &format!("pcb:{name}")).or_else(|_| find(&p, name))?;
    let ItemRef::Layout(i) = r else {
        return Err(format!("`{name}` is not a layout"));
    };
    let entry = &p.layouts[i];
    let board = p.boards.iter().find(|b| b.name == entry.item.board).ok_or("board is missing")?;
    let result = agentee_core::neck::neck(&entry.item, &board.item, opts)?;
    let mut fills = Value::Null;
    if write && !result.edits.is_empty() {
        let path = &entry.path;
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
        let tracks = doc
            .get_mut("tracks")
            .and_then(|v| v.as_array_of_tables_mut())
            .ok_or("the layout has no [[tracks]]")?;
        let round = |v: f64| (v * 1e6).round() / 1e6;
        let points = |pts: &[[f64; 2]]| {
            let mut arr = toml_edit::Array::new();
            for q in pts {
                let mut pt = toml_edit::Array::new();
                pt.push(round(q[0]));
                pt.push(round(q[1]));
                arr.push(pt);
            }
            toml_edit::value(arr)
        };
        let mut added = Vec::new();
        for e in &result.edits {
            let t = tracks.get_mut(e.track).ok_or("track index out of range")?;
            t["points"] = points(&e.points);
            for n in &e.necks {
                let mut neck = toml_edit::Table::new();
                for key in ["net", "layer"] {
                    if let Some(v) = t.get(key) {
                        neck[key] = v.clone();
                    }
                }
                neck["width"] = toml_edit::value(round(n.width));
                neck["points"] = points(&n.points);
                added.push(neck);
            }
        }
        for neck in added {
            tracks.push(neck);
        }
        std::fs::write(path, doc.to_string()).map_err(|e| e.to_string())?;
        if !entry.item.zones.is_empty() {
            fills = write_fills(&load(root)?, name)?["fills"].clone();
        }
    }
    Ok(json!({
        "layout": entry.name,
        "written": write,
        "necked": result.necked,
        "failed": result.failed,
        "tracks_changed": result.edits.len(),
        "tracks_added": result.edits.iter().map(|e| e.necks.len()).sum::<usize>(),
        "fills": fills,
    }))
}

pub fn silk(root: &std::path::Path, name: &str, hide: bool, write: bool) -> Result<Value, String> {
    let mut moved: std::collections::BTreeMap<String, usize> = Default::default();
    let mut hidden: Vec<String> = Vec::new();
    let mut left: Vec<String> = Vec::new();
    for pass in 0..8 {
        let p = load(root)?;
        let r = find(&p, &format!("pcb:{name}")).or_else(|_| find(&p, name))?;
        let ItemRef::Layout(i) = r else {
            return Err(format!("`{name}` is not a layout"));
        };
        let entry = &p.layouts[i];
        let fixes = &entry.item.label_fixes;
        left = fixes.iter().map(|f| f.reference.clone()).collect();
        if fixes.is_empty() || !write {
            if !write {
                return Ok(json!({ "layout": entry.name, "written": false, "fixes": fixes }));
            }
            break;
        }
        let path = &entry.path;
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
        let parts = doc
            .get_mut("footprints")
            .and_then(|v| v.as_array_of_tables_mut())
            .ok_or("the layout has no [[footprints]]")?;
        let mut changed = 0;
        for f in fixes {
            let Some(t) = parts
                .iter_mut()
                .find(|t| t.get("ref").and_then(|v| v.as_str()) == Some(f.reference.as_str()))
            else {
                continue;
            };
            let tries = moved.entry(f.reference.clone()).or_default();
            *tries += 1;
            let mut label =
                t.get("label").and_then(|v| v.as_inline_table()).cloned().unwrap_or_default();
            match f.at {
                Some(at) if *tries <= 3 => {
                    let mut pt = toml_edit::Array::new();
                    pt.push((at[0] * 100.0).round() / 100.0);
                    pt.push((at[1] * 100.0).round() / 100.0);
                    label.insert("at", pt.into());
                    if f.rotation != 0.0 {
                        label.insert("rotation", f.rotation.into());
                    } else {
                        label.remove("rotation");
                    }
                }
                _ if hide => {
                    label.insert("hide", true.into());
                    hidden.push(f.reference.clone());
                }
                _ => continue,
            }
            t["label"] = toml_edit::value(label);
            changed += 1;
        }
        std::fs::write(path, doc.to_string()).map_err(|e| e.to_string())?;
        if changed == 0 || pass == 7 {
            break;
        }
    }
    hidden.sort();
    hidden.dedup();
    Ok(json!({
        "layout": name,
        "written": true,
        "moved": moved.keys().filter(|k| !hidden.contains(k)).collect::<Vec<_>>(),
        "hidden": hidden,
        "still_failing": left,
    }))
}

pub struct TestpointOptions {
    pub nets: Vec<String>,
    pub side: Option<String>,
    pub pitch: f64,
    pub write: bool,
}

fn layout_index(p: &Project, name: &str) -> Result<usize, String> {
    match find(p, &format!("pcb:{name}")).or_else(|_| find(p, name))? {
        ItemRef::Layout(i) => Ok(i),
        _ => Err(format!("`{name}` is not a layout")),
    }
}

fn design_sheets(p: &Project, root: &str) -> Vec<PathBuf> {
    let mut names = vec![root.to_string()];
    let mut k = 0;
    while k < names.len() {
        for e in &p.schematics {
            if e.item.parent.as_deref() == Some(names[k].as_str()) && !names.contains(&e.name) {
                names.push(e.name.clone());
            }
        }
        k += 1;
    }
    names
        .iter()
        .filter_map(|n| p.schematics.iter().find(|e| &e.name == n).map(|e| e.path.clone()))
        .collect()
}

fn edit_toml(path: &Path) -> Result<toml_edit::DocumentMut, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.parse().map_err(|e| format!("{}: {e}", path.display()))
}

fn add_to_sheet(path: &Path, reference: &str, net: &str) -> Result<bool, String> {
    let mut doc = edit_toml(path)?;
    let Some(nets) = doc.get_mut("nets").and_then(|v| v.as_array_of_tables_mut()) else {
        return Ok(false);
    };
    let Some(entry) = nets.iter_mut().find(|t| t.get("name").and_then(|v| v.as_str()) == Some(net))
    else {
        return Ok(false);
    };
    let Some(pins) = entry.get_mut("pins").and_then(|v| v.as_array_mut()) else {
        return Ok(false);
    };
    pins.push(format!("{reference}.1"));
    let at = |t: &toml_edit::Table| -> Option<(f64, f64)> {
        let a = t.get("at")?.as_array()?;
        let n = |v: &toml_edit::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64));
        Some((n(a.get(0)?)?, n(a.get(1)?)?))
    };
    let parts = doc.get("parts").and_then(|v| v.as_array_of_tables());
    let is_tp = |t: &toml_edit::Table| {
        t.get("symbol").and_then(|v| v.as_str()) == Some(agentee_core::testpoint::SYMBOL)
    };
    let placed: Vec<(f64, f64)> =
        parts.map(|a| a.iter().filter_map(at).collect()).unwrap_or_default();
    let others: Vec<(f64, f64)> =
        parts.map(|a| a.iter().filter(|t| !is_tp(t)).filter_map(at).collect()).unwrap_or_default();
    let x = others.iter().map(|p| p.0).fold(0.0, f64::max) + 12.7;
    let x = (x / 2.54).ceil() * 2.54;
    let column = placed.iter().filter(|p| (p.0 - x).abs() < 1e-6).count();
    let y = others.iter().map(|p| p.1).fold(f64::MAX, f64::min);
    let y = if y == f64::MAX { 20.32 } else { (y / 2.54).round() * 2.54 } + column as f64 * 7.62;
    let mut part = toml_edit::Table::new();
    part["ref"] = toml_edit::value(reference);
    part["symbol"] = toml_edit::value(agentee_core::testpoint::SYMBOL);
    part["value"] = toml_edit::value("TP");
    part["footprint"] = toml_edit::value(agentee_core::testpoint::PAD_FOOTPRINT);
    let mut pt = toml_edit::Array::new();
    pt.push((x * 100.0).round() / 100.0);
    pt.push((y * 100.0).round() / 100.0);
    part["at"] = toml_edit::value(pt);
    match doc.get_mut("parts").and_then(|v| v.as_array_of_tables_mut()) {
        Some(a) => a.push(part),
        None => {
            let mut a = toml_edit::ArrayOfTables::new();
            a.push(part);
            doc["parts"] = toml_edit::Item::ArrayOfTables(a);
        }
    }
    std::fs::write(path, doc.to_string()).map_err(|e| e.to_string())?;
    Ok(true)
}

fn ensure_library(root: &Path, p: &Project) -> Result<Vec<String>, String> {
    use agentee_core::testpoint as tp;
    let mut written = Vec::new();
    if !p.footprints.iter().any(|f| f.item.name == tp::PAD_FOOTPRINT) {
        let path = root.join("footprints").join(format!("{}.fp.toml", tp::PAD_FOOTPRINT));
        std::fs::create_dir_all(root.join("footprints")).map_err(|e| e.to_string())?;
        std::fs::write(&path, tp::PAD_FOOTPRINT_TOML).map_err(|e| e.to_string())?;
        written.push(path.display().to_string());
    }
    if !p.symbols.iter().any(|s| s.item.name == tp::SYMBOL) {
        let path = root.join("symbols").join(format!("{}.sym.toml", tp::SYMBOL));
        std::fs::create_dir_all(root.join("symbols")).map_err(|e| e.to_string())?;
        std::fs::write(&path, tp::SYMBOL_TOML).map_err(|e| e.to_string())?;
        written.push(path.display().to_string());
    }
    Ok(written)
}

type Placed = (String, String, [f64; 2], Option<String>, Option<[f64; 2]>);

pub fn testpoints(root: &Path, name: &str, o: &TestpointOptions) -> Result<Value, String> {
    use agentee_core::testpoint as tp;
    let p = load(root)?;
    let i = layout_index(&p, name)?;
    let entry = &p.layouts[i];
    let layout = &entry.item;
    let board = &p.boards.iter().find(|b| b.name == layout.board).ok_or("board is missing")?.item;
    let mut spec = layout.test.clone();
    if let Some(side) = &o.side {
        let mut d = agentee_core::diag::Diags::new("test");
        let file = tp::TestFile { side: Some(side.clone()), ..Default::default() };
        spec.side = file.resolve(&mut d).side;
        if !d.list.is_empty() {
            return Err(format!("side `{side}` is not F or B"));
        }
    }
    if !o.nets.is_empty() {
        spec.nets = o.nets.clone();
    }
    let mut targets = Vec::new();
    let mut have = Vec::new();
    let mut exempt = Vec::new();
    for (ni, net) in layout.nets.iter().enumerate() {
        if !tp::wanted(&spec, board, net)
            || !layout.parts.iter().any(|p| p.pads.iter().any(|q| q.net == Some(ni)))
        {
            continue;
        }
        if tp::has_access(&spec, &layout.parts, &layout.vias, ni) {
            have.push(net.name.clone());
        } else if let Some(why) = tp::exempt(board, &layout.pairs, ni, net) {
            exempt.push(format!("{} ({why})", net.name));
        } else {
            targets.push(ni);
        }
    }
    let spots = tp::place(layout, board, &spec, &targets, o.pitch);
    let mut used: Vec<u32> = p
        .schematics
        .iter()
        .flat_map(|s| s.item.parts.iter())
        .filter_map(|q| q.reference.strip_prefix("TP").and_then(|n| n.parse().ok()))
        .collect();
    let mut next = || {
        let n = (1..).find(|n| !used.contains(n)).unwrap_or(1);
        used.push(n);
        format!("TP{n}")
    };
    let sheets = design_sheets(&p, &layout.schematic);
    let mut placed = Vec::new();
    let mut failed = Vec::new();
    for s in &spots {
        let net = layout.nets[s.net].name.clone();
        let Some(at) = s.at else {
            failed.push(
                json!({ "net": net, "reason": "no clear spot on the probe side near its copper" }),
            );
            continue;
        };
        let reference = next();
        let mut sheet = None;
        if o.write {
            for path in &sheets {
                if add_to_sheet(path, &reference, &net)? {
                    sheet = Some(path.display().to_string());
                    break;
                }
            }
            if sheet.is_none() {
                failed.push(json!({ "net": net, "reason": "no schematic sheet lists this net" }));
                continue;
            }
        }
        placed.push((reference, net, at, sheet, s.via));
    }
    let base = json!({
        "layout": entry.name,
        "side": spec.side,
        "already_probed": have,
        "exempt": exempt,
    });
    let listed = |placed: &[Placed]| -> Vec<Value> {
        placed
            .iter()
            .map(|(r, n, a, s, v)| json!({ "ref": r, "net": n, "at": a, "sheet": s, "via": v }))
            .collect()
    };
    if !o.write || placed.is_empty() {
        let mut out = base;
        out["written"] = json!(false);
        out["placed"] = json!(listed(&placed));
        out["failed"] = json!(failed);
        return Ok(out);
    }
    let library = ensure_library(entry.path.parent().unwrap_or(root), &p)?;
    let f = |v: f64| (v * 1e4).round() / 1e4;
    let mut text = format!("\n# agentee testpoints {}\n", o.nets.join(" "));
    for (r, _, at, _, _) in &placed {
        text += &format!("\n[[footprints]]\nref = \"{r}\"\nat = [{}, {}]\n", f(at[0]), f(at[1]));
        if spec.bottom() {
            text += "side = \"bottom\"\n";
        }
    }
    for (_, net, at, _, via) in &placed {
        let Some(v) = via else { continue };
        text += &format!(
            "\n[[tracks]]\nnet = \"{net}\"\nlayer = \"{}\"\npoints = [[{}, {}], [{}, {}]]\n",
            spec.copper(),
            f(at[0]),
            f(at[1]),
            f(v[0]),
            f(v[1])
        );
        text += &format!("\n[[vias]]\nnet = \"{net}\"\nat = [{}, {}]\n", f(v[0]), f(v[1]));
    }
    let path = entry.path.clone();
    let layout_name = entry.name.clone();
    drop(p);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    std::io::Write::write_all(&mut file, text.as_bytes()).map_err(|e| e.to_string())?;

    let p = load(root)?;
    let i = layout_index(&p, &layout_name)?;
    let mut layout = p.layouts[i].item.clone();
    let board = &p.boards.iter().find(|b| b.name == layout.board).ok_or("board is missing")?.item;
    let pads: Vec<(usize, Vec<Vec<[f64; 2]>>)> = layout
        .parts
        .iter()
        .filter(|q| placed.iter().any(|(r, ..)| *r == q.reference))
        .flat_map(|q| q.pads.iter().filter_map(|x| x.net.map(|n| (n, x.outlines.clone()))))
        .collect();
    let on_pad = |c: [f64; 2], n: usize| {
        pads.iter().any(|(pn, o)| {
            *pn == n
                && o.iter().any(|ring| {
                    agentee_core::geom::point_in_polygon(c, ring)
                        || agentee_core::drc::edge_distance(ring, c) < 1e-3
                })
        })
    };
    layout.ratsnest.retain(|(a, b, n)| on_pad(*a, *n) || on_pad(*b, *n));
    let route_nets: Vec<String> = {
        let mut v: Vec<String> =
            layout.ratsnest.iter().map(|(_, _, n)| layout.nets[*n].name.clone()).collect();
        v.sort();
        v.dedup();
        v
    };
    let routed = if route_nets.is_empty() {
        agentee_core::route::RouteResult::default()
    } else {
        let opts = agentee_core::route::RouteOptions { nets: route_nets, ..Default::default() };
        agentee_core::route::route(&layout, board, &opts)?
    };
    append_route(&path, "agentee testpoints routes", &routed)?;
    drop(p);

    let mut labels: std::collections::BTreeMap<String, Value> = Default::default();
    for pass in 0..2 {
        let p = load(root)?;
        let i = layout_index(&p, &layout_name)?;
        let fixes: Vec<_> = p.layouts[i]
            .item
            .label_fixes
            .iter()
            .filter(|f| placed.iter().any(|(r, ..)| *r == f.reference))
            .cloned()
            .collect();
        if fixes.is_empty() {
            break;
        }
        let mut doc = edit_toml(&path)?;
        let Some(parts) = doc.get_mut("footprints").and_then(|v| v.as_array_of_tables_mut()) else {
            break;
        };
        for fx in &fixes {
            let Some(t) = parts
                .iter_mut()
                .find(|t| t.get("ref").and_then(|v| v.as_str()) == Some(fx.reference.as_str()))
            else {
                continue;
            };
            let mut label = toml_edit::InlineTable::new();
            match fx.at {
                Some(at) if pass == 0 => {
                    let mut pt = toml_edit::Array::new();
                    pt.push((at[0] * 100.0).round() / 100.0);
                    pt.push((at[1] * 100.0).round() / 100.0);
                    label.insert("at", pt.into());
                    if fx.rotation != 0.0 {
                        label.insert("rotation", fx.rotation.into());
                    }
                    labels.insert(fx.reference.clone(), json!({ "moved": at }));
                }
                _ => {
                    label.insert("hide", true.into());
                    labels.insert(fx.reference.clone(), json!({ "hidden": true }));
                }
            }
            t["label"] = toml_edit::value(label);
        }
        std::fs::write(&path, doc.to_string()).map_err(|e| e.to_string())?;
    }
    let mut out = base;
    out["written"] = json!(true);
    out["placed"] = json!(listed(&placed));
    out["failed"] = json!(failed);
    out["library"] = json!(library);
    out["connections"] = json!(routed.connections);
    out["routed"] = json!(routed.routed);
    out["tracks"] = json!(routed.tracks.len());
    out["vias"] = json!(routed.vias.len());
    out["unrouted"] = json!(routed.failed);
    out["labels"] = json!(labels);
    Ok(out)
}
