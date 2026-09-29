use agentee_core::calc::{self, TraceGeometry};
use agentee_core::project::{ItemRef, Kind, Project};
use agentee_core::units::{Amps, Kelvin, Length, Ohms};
use agentee_core::{Diagnostic, Severity};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn load(path: &Path) -> Result<Project, String> {
    Project::load(path).map_err(|e| format!("{}: {e}", path.display()))
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
            "{} boards, {} symbols, {} footprints",
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
        let kind = match r {
            ItemRef::Board(_) => Kind::Board,
            ItemRef::Symbol(_) => Kind::Symbol,
            ItemRef::Footprint(_) => Kind::Footprint,
        };
        items.push(row(r, kind));
    }
    json!({ "root": p.root, "items": items, "unloadable": p.failures })
}

pub fn show(p: &Project, r: ItemRef) -> Value {
    match r {
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

pub struct ImpedanceQuery<'a> {
    pub project: Option<&'a Project>,
    pub board: Option<&'a str>,
    pub layer: Option<&'a str>,
    pub width: Option<&'a str>,
    pub gap: Option<&'a str>,
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
    let gap = q.gap.map(|g| parse(g, Length::parse, "gap")).transpose()?.map(Length::to_mm);
    let mut v = json!({ "geometry": geometry, "differential": gap.is_some() });
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
