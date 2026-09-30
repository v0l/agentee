use crate::ops;
use agentee_core::Severity;
use agentee_core::project::Kind;
use agentee_view::RenderOptions;
use base64::Engine;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

const PROTOCOL: &str = "2025-06-18";

const INSTRUCTIONS: &str = "agentee is an electronic design package for agents. The design lives in TOML files: \
*.board.toml (board spec: stackup, fab rules, vias, net classes), *.sym.toml (schematic symbols) and *.fp.toml \
(footprints). Read format_reference before writing any file. Edit the files directly, then call check and fix \
every error. Use render_item to see what you made. Prefer importing KiCad parts (kicad_search, then \
import_kicad_symbol / import_kicad_footprint) over drawing them by hand.";

fn tools() -> Value {
    let s = |props: Value, required: &[&str]| json!({ "type": "object", "properties": props, "required": required });
    json!([
        {
            "name": "format_reference",
            "description": "The file format reference for boards, symbols and footprints, with examples. Read this before writing files.",
            "inputSchema": s(json!({}), &[]),
        },
        {
            "name": "check",
            "description": "Load the project and report errors and warnings for every item, or one item.",
            "inputSchema": s(json!({
                "item": { "type": "string", "description": "only this board, symbol or footprint" },
                "include_info": { "type": "boolean", "description": "also list info notes" },
            }), &[]),
        },
        {
            "name": "drc",
            "description": "Design rule checks of a layout. With list = true: every DRC rule with its id, category, severity, and whether it applies to this board and why. Without: the layout's diagnostics that carry a rule id. Rule ids go in the board's [drc] disable list or severity table.",
            "inputSchema": s(json!({
                "name": { "type": "string", "description": "layout or board" },
                "list": { "type": "boolean" },
            }), &["name"]),
        },
        {
            "name": "list_items",
            "description": "Every board, symbol and footprint in the project with its file and error counts.",
            "inputSchema": s(json!({}), &[]),
        },
        {
            "name": "show_item",
            "description": "The resolved model of one item as JSON: pins with positions, expanded pad arrays, stackup with computed trace geometry, impedance and current analysis.",
            "inputSchema": s(json!({ "name": { "type": "string" } }), &["name"]),
        },
        {
            "name": "render_item",
            "description": "Render an item to PNG exactly as the viewer shows it and return the image.",
            "inputSchema": s(json!({
                "name": { "type": "string" },
                "width": { "type": "integer", "default": 1400 },
                "height": { "type": "integer", "default": 900 },
                "unit": { "type": "integer", "description": "symbol unit, 1 based", "default": 1 },
                "canvas_only": { "type": "boolean", "description": "drop the side panels" },
                "hidden_pins": { "type": "boolean" },
                "save_to": { "type": "string", "description": "also write the PNG here, relative to the project" },
                "show": { "type": "array", "items": { "type": "string" }, "description": "layers to turn on, e.g. F.Fab, F.Mask, In1.Cu" },
                "hide": { "type": "array", "items": { "type": "string" } },
                "region": { "type": "array", "items": { "type": "number" }, "description": "zoom to [x0, y0, x1, y1] in mm" },
            }), &["name"]),
        },
        {
            "name": "kicad_search",
            "description": "Search the installed KiCad libraries by name. Every word must match Library:Name.",
            "inputSchema": s(json!({
                "query": { "type": "string" },
                "kind": { "type": "string", "enum": ["symbol", "footprint"] },
                "limit": { "type": "integer", "default": 40 },
            }), &["query", "kind"]),
        },
        {
            "name": "import_kicad_symbol",
            "description": "Convert a KiCad symbol (Library:Name) into a .sym.toml in the project. with_footprint also imports its default footprint, or give footprint to pick one.",
            "inputSchema": s(json!({
                "spec": { "type": "string", "description": "Library:Name, e.g. Amplifier_Operational:LM358" },
                "with_footprint": { "type": "boolean" },
                "footprint": { "type": "string", "description": "Library:Name of the footprint to import and link" },
                "dir": { "type": "string", "description": "relative to the project, default symbols/" },
                "force": { "type": "boolean" },
            }), &["spec"]),
        },
        {
            "name": "import_kicad_footprint",
            "description": "Convert a KiCad footprint (Library:Name) into a .fp.toml in the project.",
            "inputSchema": s(json!({
                "spec": { "type": "string", "description": "Library:Name, e.g. Package_SO:SOIC-8_3.9x4.9mm_P1.27mm" },
                "dir": { "type": "string", "description": "relative to the project, default footprints/" },
                "force": { "type": "boolean" },
            }), &["spec"]),
        },
        {
            "name": "new_item",
            "description": "Write a starter file for a board, symbol or footprint that passes check, to edit from.",
            "inputSchema": s(json!({
                "kind": { "type": "string", "enum": ["board", "symbol", "footprint", "schematic", "layout"] },
                "name": { "type": "string" },
                "dir": { "type": "string", "description": "relative to the project" },
            }), &["kind", "name"]),
        },
        {
            "name": "trace_width",
            "description": "Minimum track width for a current (IPC-2221).",
            "inputSchema": s(json!({
                "current": { "type": "string", "description": "e.g. 2A or 500mA" },
                "copper": { "type": "string", "default": "1oz" },
                "temp_rise": { "type": "string", "default": "10C" },
                "internal": { "type": "boolean" },
            }), &["current"]),
        },
        {
            "name": "run_sim",
            "description": "Run an FDTD simulation (*.sim.toml) of a layout on the GPU: ports on pads, lumped models for passives, S-parameters saved as JSON and Touchstone next to the spec. Render the sim afterwards to see the plot.",
            "inputSchema": s(json!({ "name": { "type": "string" } }), &["name"]),
        },
        {
            "name": "serpentine",
            "description": "Points for a track that replaces a straight segment and adds a given length as trombone bumps to one side. Paste them into the track's points.",
            "inputSchema": s(json!({
                "from": { "type": "string", "description": "x,y in mm" },
                "to": { "type": "string", "description": "x,y in mm" },
                "add": { "type": "string", "description": "e.g. 2.5mm" },
                "amplitude": { "type": "string", "default": "0.6mm" },
                "pitch": { "type": "string", "default": "0.4mm" },
            }), &["from", "to", "add"]),
        },
        {
            "name": "route",
            "description": "Autoroute the ratsnest of the named nets of a layout on a grid, keeping each class's width, clearance, layers and via, and append the tracks and vias to the layout file. Existing copper is never moved unless reroute is set.",
            "inputSchema": s(json!({
                "name": { "type": "string" },
                "nets": { "type": "string", "description": "comma separated, * and ? globs" },
                "layers": { "type": "string", "description": "comma separated copper layers, default all" },
                "grid": { "type": "number", "default": 0.05 },
                "via": { "type": "string" },
                "via_cost": { "type": "number", "default": 3.0 },
                "bend_cost": { "type": "number", "default": 0.1, "description": "mm of track per 45 degree bend, three times that for 90" },
                "pairs": { "type": "boolean" },
                "reroute": { "type": "boolean", "description": "remove these nets' tracks and vias first" },
                "dry_run": { "type": "boolean" },
            }), &["name", "nets"]),
        },
        {
            "name": "tune",
            "description": "Length-match a layout: meander the short net of every pair over its skew limit and every match group member short of its target, on the longest segments where the bumps clear every other net, and write the points back into the tracks.",
            "inputSchema": s(json!({
                "name": { "type": "string" },
                "nets": { "type": "string", "description": "comma separated globs, default all" },
                "amplitude": { "type": "number", "description": "largest bump height in mm" },
                "pitch": { "type": "number", "description": "bump pitch in mm" },
                "dry_run": { "type": "boolean" },
            }), &["name"]),
        },
        {
            "name": "silk",
            "description": "Move every silk reference that check flags to the clear spot it suggests, repeating until the labels settle, and optionally hide the ones that have nowhere to go. Writes label = { at, rotation } or hide = true into the footprints.",
            "inputSchema": s(json!({
                "name": { "type": "string" },
                "hide": { "type": "boolean" },
                "dry_run": { "type": "boolean" },
            }), &["name"]),
        },
        {
            "name": "sparam",
            "description": "Analyse a finished sim or cascade: passivity and reciprocity, a TDR of one port (impedance against time with a Gaussian edge), mixed-mode Sdd/Scc/Scd for a pair given as IN+,IN-,OUT+,OUT-, and crosstalk FROM,TO in frequency and as a step.",
            "inputSchema": s(json!({
                "name": { "type": "string" },
                "tdr": { "type": "string", "description": "port name or number" },
                "rise": { "type": "string", "description": "10-90% rise, e.g. 35ps" },
                "pair": { "type": "string" },
                "xtalk": { "type": "string" },
            }), &["name"]),
        },
        {
            "name": "models",
            "description": "Find the 3D models (STEP or VRML) the project's footprints name, downloading missing KiCad library models into the cache. Reports each model's path and triangle count, or why it is missing.",
            "inputSchema": s(json!({}), &[]),
        },
        {
            "name": "fab",
            "description": "Write the fab package for a layout into a folder: RS-274X Gerbers (X2) per copper, mask, paste, silk and edge layer, Excellon drills, a zip of those for upload, BOM (generic and JLCPCB), pick-and-place, fab notes and assembly drawings. Refuses while the layout has errors.",
            "inputSchema": s(json!({ "name": { "type": "string" }, "out": { "type": "string", "description": "output folder, relative to the project" } }), &["name", "out"]),
        },
        {
            "name": "field_solve",
            "description": "Solve a trace cross-section with the GPU field solver: impedance, effective permittivity, C and L per metre, delay. Includes solder mask, thickness, coplanar grounds and differential pairs. Within 0.5% of exact references with fine = true.",
            "inputSchema": s(json!({
                "board": { "type": "string" },
                "layer": { "type": "string", "default": "F.Cu" },
                "netclass": { "type": "string", "description": "take width and gaps from this class" },
                "width": { "type": "string" },
                "gap": { "type": "string", "description": "differential pair gap" },
                "coplanar_gap": { "type": "string" },
                "no_mask": { "type": "boolean" },
                "fine": { "type": "boolean" },
                "sweep": { "type": "string", "description": "loss sweep START,STOP,POINTS, e.g. 10MHz,20GHz,21: R, L, G, C, Z0 and dB/in per frequency with skin effect, stackup roughness and a causal (Djordjevic-Sarkar) dielectric" },
            }), &[]),
        },
        {
            "name": "impedance",
            "description": "Trace impedance on a board layer for a width (and pair gap), or the width that hits a target.",
            "inputSchema": s(json!({
                "board": { "type": "string", "description": "board name, optional when there is one" },
                "layer": { "type": "string", "default": "F.Cu" },
                "width": { "type": "string" },
                "gap": { "type": "string", "description": "differential pair gap" },
                "coplanar_gap": { "type": "string", "description": "gap to the ground pour either side, grounded coplanar on outer layers" },
                "target": { "type": "string", "description": "e.g. 50ohm" },
            }), &[]),
        },
    ])
}

fn text(s: impl Into<String>) -> Value {
    json!({ "type": "text", "text": s.into() })
}

fn ok(content: Vec<Value>) -> Value {
    json!({ "content": content, "isError": false })
}

fn fail(msg: impl Into<String>) -> Value {
    json!({ "content": [text(msg)], "isError": true })
}

fn arg<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}

fn flag(a: &Value, k: &str) -> bool {
    a.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn strings(a: &Value, k: &str) -> Vec<String> {
    a.get(k)
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn int(a: &Value, k: &str, d: u64) -> u64 {
    a.get(k).and_then(Value::as_u64).unwrap_or(d)
}

fn under(root: &Path, rel: Option<&str>, default: &str) -> PathBuf {
    root.join(rel.unwrap_or(default))
}

fn call(root: &Path, name: &str, a: &Value) -> Result<Value, String> {
    let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap_or_default();
    match name {
        "format_reference" => Ok(ok(vec![text(crate::FORMAT)])),
        "check" => {
            let p = ops::load_checked(root)?;
            let item = arg(a, "item").map(|n| ops::find(&p, n)).transpose()?;
            let min = if flag(a, "include_info") { Severity::Info } else { Severity::Warning };
            let (t, _, _) = ops::check_report(&p, item, min);
            Ok(ok(vec![text(t)]))
        }
        "drc" => {
            let p = ops::load(root)?;
            let (t, _) = ops::drc(&p, arg(a, "name").ok_or("name is required")?, flag(a, "list"))?;
            Ok(ok(vec![text(t)]))
        }
        "list_items" => Ok(ok(vec![text(pretty(&ops::list(&ops::load(root)?)))])),
        "show_item" => {
            let p = ops::load(root)?;
            let r = ops::find(&p, arg(a, "name").ok_or("name is required")?)?;
            Ok(ok(vec![text(pretty(&ops::show(&p, r)))]))
        }
        "render_item" => {
            let p = ops::load(root)?;
            let r = ops::find(&p, arg(a, "name").ok_or("name is required")?)?;
            let opts = RenderOptions {
                width: int(a, "width", 1400).clamp(200, 4000) as u32,
                height: int(a, "height", 900).clamp(200, 4000) as u32,
                unit: int(a, "unit", 1) as u32,
                panels: !flag(a, "canvas_only"),
                hidden_pins: flag(a, "hidden_pins"),
                show: strings(a, "show"),
                hide: strings(a, "hide"),
                region: a.get("region").and_then(Value::as_array).filter(|v| v.len() == 4).map(
                    |v| {
                        let f = |i: usize| v[i].as_f64().unwrap_or(0.0);
                        [f(0), f(1), f(2), f(3)]
                    },
                ),
                ..Default::default()
            };
            let png = agentee_view::render_png(&p, r, &opts);
            let mut note = format!("{} ({}x{})", p.name_of(r), opts.width, opts.height);
            if let Some(dest) = arg(a, "save_to") {
                let path = root.join(dest);
                std::fs::write(&path, &png).map_err(|e| format!("{}: {e}", path.display()))?;
                note += &format!(", saved to {}", path.display());
            }
            let data = base64::engine::general_purpose::STANDARD.encode(&png);
            Ok(ok(vec![
                json!({ "type": "image", "data": data, "mimeType": "image/png" }),
                text(note),
            ]))
        }
        "kicad_search" => {
            let q = arg(a, "query").ok_or("query is required")?;
            let limit = int(a, "limit", 40) as usize;
            let hits = match arg(a, "kind") {
                Some("footprint") => agentee_kicad::search_footprints(q, limit),
                _ => agentee_kicad::search_symbols(q, limit),
            };
            let lines: Vec<String> =
                hits.iter().map(|h| format!("{}:{}", h.library, h.name)).collect();
            Ok(ok(vec![text(if lines.is_empty() {
                "no matches".into()
            } else {
                lines.join("\n")
            })]))
        }
        "import_kicad_symbol" => {
            let spec = arg(a, "spec").ok_or("spec is required")?;
            let pick = match (arg(a, "footprint"), flag(a, "with_footprint")) {
                (Some(f), _) => ops::FootprintPick::Spec(f.to_string()),
                (None, true) => ops::FootprintPick::Default,
                (None, false) => ops::FootprintPick::None,
            };
            let (written, notes) = ops::import_symbol(
                spec,
                &under(root, arg(a, "dir"), "symbols"),
                &root.join("footprints"),
                pick,
                flag(a, "force"),
            )?;
            let list: Vec<String> = written.iter().map(|p| p.display().to_string()).collect();
            let mut msg = format!("wrote {}", list.join(", "));
            for n in notes {
                msg += &format!("\nnote: {n}");
            }
            Ok(ok(vec![text(msg)]))
        }
        "import_kicad_footprint" => {
            let spec = arg(a, "spec").ok_or("spec is required")?;
            let path = ops::import_footprint(
                spec,
                &under(root, arg(a, "dir"), "footprints"),
                flag(a, "force"),
            )?;
            Ok(ok(vec![text(format!("wrote {}", path.display()))]))
        }
        "new_item" => {
            let kind = match arg(a, "kind") {
                Some("board") => Kind::Board,
                Some("symbol") => Kind::Symbol,
                Some("footprint") => Kind::Footprint,
                Some("schematic") => Kind::Schematic,
                Some("layout") => Kind::Layout,
                _ => return Err("kind is board, symbol, footprint, schematic or layout".into()),
            };
            let default = match kind {
                Kind::Symbol => "symbols",
                Kind::Footprint => "footprints",
                _ => ".",
            };
            let path = ops::new_item(
                kind,
                arg(a, "name").ok_or("name is required")?,
                &under(root, arg(a, "dir"), default),
            )?;
            let body = std::fs::read_to_string(&path).unwrap_or_default();
            Ok(ok(vec![text(format!("wrote {}\n\n{body}", path.display()))]))
        }
        "trace_width" => {
            let v = ops::trace_width(
                arg(a, "current").ok_or("current is required")?,
                arg(a, "copper").unwrap_or("1oz"),
                arg(a, "temp_rise").unwrap_or("10C"),
                flag(a, "internal"),
            )?;
            Ok(ok(vec![text(pretty(&v))]))
        }
        "run_sim" => {
            let p = ops::load(root)?;
            let v = ops::run_sim(
                &p,
                arg(a, "name").ok_or("name is required")?,
                flag(a, "dry_run"),
                &mut |_, _, _| {},
            )?;
            Ok(ok(vec![text(pretty(&v))]))
        }
        "serpentine" => Ok(ok(vec![text(pretty(&ops::serpentine(
            arg(a, "from").ok_or("from is required")?,
            arg(a, "to").ok_or("to is required")?,
            arg(a, "add").ok_or("add is required")?,
            arg(a, "amplitude").unwrap_or("0.6mm"),
            arg(a, "pitch").unwrap_or("0.4mm"),
        )?))])),
        "route" => {
            let name = arg(a, "name").ok_or("name is required")?;
            let split = |k: &str| -> Vec<String> {
                arg(a, k)
                    .map(|v| {
                        v.split(',')
                            .map(|x| x.trim().to_string())
                            .filter(|x| !x.is_empty())
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let nets = split("nets");
            let dry_run = flag(a, "dry_run");
            if flag(a, "reroute") && !dry_run {
                ops::unroute(&ops::load(root)?, name, &nets)?;
            }
            let opts = agentee_core::route::RouteOptions {
                nets,
                layers: split("layers"),
                grid: a.get("grid").and_then(Value::as_f64).unwrap_or(0.05),
                via: arg(a, "via").map(str::to_string),
                via_cost: a.get("via_cost").and_then(Value::as_f64).unwrap_or(3.0),
                bend_cost: a.get("bend_cost").and_then(Value::as_f64).unwrap_or(0.1),
                pairs: flag(a, "pairs"),
                ..Default::default()
            };
            Ok(ok(vec![text(pretty(&ops::route(&ops::load(root)?, name, &opts, !dry_run)?))]))
        }
        "silk" => Ok(ok(vec![text(pretty(&ops::silk(
            root,
            arg(a, "name").ok_or("name is required")?,
            flag(a, "hide"),
            !flag(a, "dry_run"),
        )?))])),
        "tune" => {
            let nets: Vec<String> = arg(a, "nets")
                .map(|v| {
                    v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
                })
                .unwrap_or_else(|| vec!["*".into()]);
            let opts = agentee_core::tune::TuneOptions {
                nets,
                amplitude: a.get("amplitude").and_then(Value::as_f64),
                pitch: a.get("pitch").and_then(Value::as_f64),
            };
            let name = arg(a, "name").ok_or("name is required")?;
            Ok(ok(vec![text(pretty(&ops::tune(
                &ops::load(root)?,
                name,
                &opts,
                !flag(a, "dry_run"),
            )?))]))
        }
        "sparam" => {
            let p = ops::load(root)?;
            let q = ops::SparamQuery {
                tdr: arg(a, "tdr"),
                rise: arg(a, "rise"),
                pair: arg(a, "pair"),
                xtalk: arg(a, "xtalk"),
            };
            Ok(ok(vec![text(pretty(&ops::sparam(
                &p,
                arg(a, "name").ok_or("name is required")?,
                &q,
            )?))]))
        }
        "models" => {
            let p = ops::load(root)?;
            Ok(ok(vec![text(pretty(&ops::fetch_models(&p)?))]))
        }
        "fab" => {
            let p = ops::load(root)?;
            let out = root.join(arg(a, "out").ok_or("out is required")?);
            Ok(ok(vec![text(pretty(&ops::fab(
                &p,
                arg(a, "name").ok_or("name is required")?,
                &out,
            )?))]))
        }
        "field_solve" => {
            let p = ops::load(root)?;
            let v = ops::field_solve(
                &p,
                &ops::FieldQuery {
                    board: arg(a, "board"),
                    layer: arg(a, "layer").unwrap_or("F.Cu"),
                    width: arg(a, "width"),
                    netclass: arg(a, "netclass"),
                    gap: arg(a, "gap"),
                    coplanar_gap: arg(a, "coplanar_gap"),
                    mask: !flag(a, "no_mask"),
                    fine: flag(a, "fine"),
                    sweep: arg(a, "sweep"),
                },
            )?;
            Ok(ok(vec![text(pretty(&v))]))
        }
        "impedance" => {
            let p = ops::load(root)?;
            let v = ops::impedance(&ops::ImpedanceQuery {
                project: Some(&p),
                board: arg(a, "board"),
                layer: arg(a, "layer"),
                width: arg(a, "width"),
                gap: arg(a, "gap"),
                coplanar_gap: arg(a, "coplanar_gap"),
                target: arg(a, "target"),
                h: None,
                er: None,
                t: None,
            })?;
            Ok(ok(vec![text(pretty(&v))]))
        }
        _ => Err(format!("unknown tool `{name}`")),
    }
}

fn handle(root: &Path, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL);
            Ok(json!({
                "protocolVersion": asked,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "agentee", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(call(root, name, &args).unwrap_or_else(fail))
        }
        m if m.starts_with("notifications/") => return None,
        m => Err(json!({ "code": -32601, "message": format!("method `{m}` not found") })),
    };
    let id = id?;
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

pub fn serve(root: &Path) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let v: Vec<Value> = batch.iter().filter_map(|m| handle(root, m)).collect();
                (!v.is_empty()).then_some(Value::Array(v))
            }
            Ok(msg) => handle(root, &msg),
            Err(e) => Some(
                json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } }),
            ),
        };
        if let Some(r) = reply {
            serde_json::to_writer(&mut out, &r)?;
            out.write_all(b"\n")?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo")
    }

    #[test]
    fn initialize_and_list() {
        let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2024-11-05" } });
        let r = handle(&demo(), &init).unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
        assert!(
            handle(&demo(), &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
                .is_none()
        );
        let tools =
            handle(&demo(), &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).unwrap();
        assert!(tools["result"]["tools"].as_array().unwrap().len() > 5);
    }

    #[test]
    fn tool_errors_are_results_not_protocol_errors() {
        let call = json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "show_item", "arguments": { "name": "missing" } } });
        let r = handle(&demo(), &call).unwrap();
        assert_eq!(r["result"]["isError"], true);
    }

    #[test]
    fn check_passes_on_the_demo() {
        let (_, v, ok) = ops::check_report(&ops::load(&demo()).unwrap(), None, Severity::Warning);
        assert!(ok, "{v}");
    }
}
