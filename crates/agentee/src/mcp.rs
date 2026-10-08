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
(footprints). Change schematics, layouts and board specs with the edit tool, all the commands of one change in \
one call. Write symbols and footprints directly, reading format_reference first. Then call check and fix every \
error. Never rewrite the TOML from a script. Use render_item to see what you made. Prefer importing KiCad parts (kicad_search, then \
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
            "name": "layout",
            "description": "Run the layout engine on a layout: the configured [engine] phases in order, then the score per term with the worst offenders of each. Phases not implemented yet are listed as skipped. With search it tries many settings in parallel (placement seeds by default, or the [engine.search] knobs), screens them on the cheap stages, runs the best few in full and writes the one that routes best, with its settings pinned in [engine]. Use search instead of rerunning place and route by hand with different seeds.",
            "inputSchema": s(json!({
                "name": { "type": "string" },
                "from": { "type": "string", "description": "start at this phase" },
                "to": { "type": "string", "description": "stop after this phase" },
                "only": { "type": "string", "description": "run one phase" },
                "dry_run": { "type": "boolean", "description": "report without writing the plan" },
                "search": { "type": ["boolean", "integer"], "description": "true, or how many settings to try (default 16)" },
                "keep": { "type": "integer", "description": "with search, how many of the screen's best get the full run (default 4)" },
            }), &["name"]),
        },
        {
            "name": "stackups",
            "description": "Stackup presets for a board's `stackup.preset`: JLCPCB and PCBWay builds and generic HDI builds with layer thicknesses and er. Filter the list, or give name to get one preset's layers.",
            "inputSchema": s(json!({
                "name": { "type": "string", "description": "one preset, returns its layers" },
                "fab": { "type": "string", "description": "jlcpcb, pcbway or generic" },
                "layers": { "type": "integer", "description": "copper layer count" },
                "thickness": { "type": "number", "description": "finished thickness in mm, within 10%" },
                "search": { "type": "string", "description": "substring of name or description" },
            }), &[]),
        },
        {
            "name": "edit",
            "description": "Edit a schematic (sch), layout (pcb) or board spec (board) with commands, one per line, in one load and one check at the end. It keeps the file's comments and layout and returns the check report of what it touched. A command that names a missing pin, part or net fails before anything is written. Send `help` alone for the commands of that target. Commands: sch add/remove/move/set/net/connect/disconnect/nc/unnc/note; pcb place/unplace/track/untrack/via/unvia/zone/unzone/pair/text/fanout/stitch/watermark/title/test; board class/unclass/via/unvia/outline/cutout/stackup.",
            "inputSchema": s(json!({
                "target": { "type": "string", "enum": ["sch", "pcb", "board"] },
                "item": { "type": "string", "description": "item name, optional when the project has one of that kind" },
                "commands": { "type": "string", "description": "one command per line, e.g. `add R1 R 10k --footprint R_0402_1005Metric` then `net MID R1.2 R2.1 --class Signal`" },
            }), &["target", "commands"]),
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
            "description": "Render an item to PNG exactly as the viewer shows it and return the image. Without the side panels the image is cropped to the drawing, so width and height are the largest it gets.",
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
                "focus": { "type": "array", "items": { "type": "string" }, "description": "schematic or layout only: zoom to these parts, nets or pins (U1, SPI_*, U1.3) and fade the rest" },
                "context": { "type": "string", "enum": ["dim", "hide", "show"], "default": "dim", "description": "what happens to everything outside focus" },
                "rulers": { "type": "boolean", "description": "label mm coordinates along the edges, to pick the next region" },
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
            "description": "Write a starter file for a board, symbol, footprint, schematic, layout or sim, to edit from. A sim starts as an FDTD run of a layout, or with sim_kind logic as a logic sim of a schematic (clock, reset, a clocked assertion) whose nets you rename.",
            "inputSchema": s(json!({
                "kind": { "type": "string", "enum": ["board", "symbol", "footprint", "schematic", "layout", "sim"] },
                "name": { "type": "string" },
                "dir": { "type": "string", "description": "relative to the project" },
                "sim_kind": { "type": "string", "enum": ["fdtd", "logic"], "description": "with kind sim, default fdtd" },
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
            "description": "Run a simulation (*.sim.toml): an FDTD run of a layout on the GPU (ports on pads, lumped models for passives, S-parameters saved as JSON and Touchstone next to the spec), any other kind (dc, thermal, cascade, channel, pdn), or a logic sim of the schematic netlist (stimulus, assertions, setup and hold, a VCD next to the spec). Render the sim afterwards to see the plot or the waveforms.",
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
            "name": "tie",
            "description": "Tie every SMD pad of a plane net (a net with a [[zones]] entry) to the nearest plane layer with a short stub and a via beside the pad, away from the part body, and append them to the layout file. Use this for ground and supply pads instead of routing tracks between them; `route` with a glob skips plane nets.",
            "inputSchema": s(json!({
                "name": { "type": "string" },
                "nets": { "type": "string", "description": "comma separated plane nets, globs allowed, default every net with a zone" },
                "dry_run": { "type": "boolean" },
            }), &["name"]),
        },
        {
            "name": "route",
            "description": "Autoroute the ratsnest of the named nets of a layout on a grid, keeping each class's width, clearance, layers and via, and append the tracks and vias to the layout file. Existing copper is never moved unless reroute is set.",
            "inputSchema": s(json!({
                "name": { "type": "string" },
                "nets": { "type": "string", "description": "comma separated, * and ? globs" },
                "layers": { "type": "string", "description": "comma separated copper layers, default all" },
                "grid": { "type": "number", "default": 0.05 },
                "via": { "type": "string", "description": "comma separated [[vias]] names from the board to choose from, default the class vias" },
                "via_cost": { "type": "number", "default": 3.0 },
                "bend_cost": { "type": "number", "default": 0.1, "description": "mm of track per 45 degree bend, three times that for 90" },
                "pairs": { "type": "boolean" },
                "via_in_pad": { "type": "boolean", "description": "allow vias fully inside SMD pads (filled and capped); by default vias keep off every SMD pad" },
                "reroute": { "type": "boolean", "description": "remove these nets' tracks and vias first" },
                "dry_run": { "type": "boolean" },
            }), &["name", "nets"]),
        },
        {
            "name": "fill",
            "description": "Fill the zones of a layout and store the copper at the end of its file, so loads and the viewer skip the fill until the zone's copper, clearances or neighbours change.",
            "inputSchema": s(json!({ "name": { "type": "string" } }), &["name"]),
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
            "name": "neck",
            "description": "Neck down track ends that enter a pad narrower than the track or break clearance near the pad: the end becomes a separate [[tracks]] entry with an explicit width, the smallest of the class width, the pad's smaller side and the widest that keeps clearance, rounded down to 0.01 mm, never under min_track_width and never longer than the class neckdown. taper steps the width down over a short chain of segments. Stored zone fills are refreshed.",
            "inputSchema": s(json!({
                "name": { "type": "string" },
                "nets": { "type": "string", "description": "comma separated globs, default all" },
                "taper": { "type": "boolean" },
                "dry_run": { "type": "boolean" },
            }), &["name"]),
        },
        {
            "name": "place",
            "description": "Place the layout's parts automatically as a quick start: connectors and edge-mount parts on the board edges (never an RF and a USB connector on one edge unless it must), mounting holes and fiducials in the corners, the largest chips near the centre, each IC's decoupling caps, crystal and pull-ups clustered at the pins they serve, placed by weighted wirelength, then legalised on a 0.05 mm grid with no courtyard overlaps and refined by simulated annealing. Writes at, rotation and side into [[footprints]] and refreshes the stored fills. Parts with locked = true stay put.",
            "inputSchema": s(json!({
                "name": { "type": "string" },
                "parts": { "type": "string", "description": "comma separated globs of the parts to place, default all" },
                "keep_placed": { "type": "boolean", "description": "leave every part that already has a placement" },
                "side": { "type": "string", "description": "F, B or both, default F" },
                "seed": { "type": "integer", "description": "default 1; the same seed gives the same placement" },
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
            "name": "testpoints",
            "description": "Add a test pad to each net that needs probe access and has none: a TestPoint part (TestPoint_Pad_D1.0mm footprint, both written into the project if missing) joined to the net in the schematic sheet that lists it, a footprint on the probe side in free space near the net's copper, and an autorouted track and via from the net's copper. Nets default to the layout's [test] nets; impedance and pair nets are skipped.",
            "inputSchema": s(json!({
                "name": { "type": "string" },
                "nets": { "type": "string", "description": "comma separated, * and ? globs, any case; default the [test] nets" },
                "side": { "type": "string", "description": "probe side F or B, default the [test] side (B)" },
                "pitch": { "type": "number", "default": 2.54, "description": "grid and least spacing of the test pads, mm" },
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
            "name": "export",
            "description": "Write a layout as one STEP assembly for enclosure CAD: the board as a solid with its drilled holes and cutouts, each part's STEP model embedded and placed, and VRML, generated and missing models as surfaces or boxes. Board bottom at z = 0, X and Y as the layout with Y up.",
            "inputSchema": s(json!({ "name": { "type": "string" }, "out": { "type": "string", "description": "output .step file, relative to the project" } }), &["name", "out"]),
        },
        {
            "name": "parts",
            "description": "Price a layout's or schematic's BOM at Mouser and Farnell and find cheaper equivalents: the same part at the other distributor, resistors and ceramic capacitors with the same value, package, tolerance, voltage and dielectric (never a downgrade), generic discretes (2N7002, S1D, SS14, SMAJ..) by name and package, indicator LEDs by colour and package, and the distributor's suggested replacement for parts going obsolete. Costs are at the needed quantity, buying up to a price break when that is cheaper. Keys come from ~/.config/agentee/distributors.toml ([mouser] api_key, [farnell] api_key and store).",
            "inputSchema": s(json!({
                "name": { "type": "string", "description": "layout or schematic" },
                "boards": { "type": "integer", "default": 1 },
                "alternatives": { "type": "boolean", "default": true },
                "distributors": { "type": "array", "items": { "type": "string" }, "description": "mouser, farnell; default every one with a key" },
                "farnell_store": { "type": "string", "description": "e.g. uk.farnell.com, ie.farnell.com, www.newark.com" },
                "refs": { "type": "array", "items": { "type": "string" }, "description": "only the BOM lines holding these references" },
                "order": { "type": "string", "description": "directory, relative to the project, to write order sheets to instead of returning the report: NAME-order.csv and NAME-<distributor>.csv, each with the lines to buy there first, then the lines bought at the other distributor, then what it lacks" },
                "spares": { "type": "boolean", "default": false, "description": "with order: 0402/0603 resistors and capacitors to the next 10 above need + 5, one spare per D, Q, U and F line, or the part's `spares` field" },
            }), &["name"]),
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

fn png_size(png: &[u8]) -> (u32, u32) {
    let be = |k: usize| u32::from_be_bytes(png[k..k + 4].try_into().unwrap());
    (be(16), be(20))
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
        "layout" => {
            let r = ops::layout_engine(
                root,
                arg(a, "name").ok_or("name is required")?,
                &ops::LayoutArgs {
                    from: arg(a, "from").map(String::from),
                    to: arg(a, "to").map(String::from),
                    only: arg(a, "only").map(String::from),
                    write: !flag(a, "dry_run"),
                    search: match (a.get("search"), a.get("keep").and_then(Value::as_u64)) {
                        (Some(Value::Bool(false)) | None, None) => None,
                        (s, keep) => Some(agentee_layout::search::Ask {
                            tries: s.and_then(Value::as_u64).filter(|&n| n > 0).map(|n| n as usize),
                            keep: keep.map(|n| n as usize),
                        }),
                    },
                },
            )?;
            Ok(ok(vec![text(pretty(&r))]))
        }
        "stackups" => {
            if let Some(n) = arg(a, "name") {
                return Ok(ok(vec![text(pretty(&ops::stackup(n)?))]));
            }
            let q = ops::StackupQuery {
                fab: arg(a, "fab"),
                layers: a.get("layers").and_then(Value::as_u64).map(|n| n as usize),
                thickness_mm: a.get("thickness").and_then(Value::as_f64),
                search: arg(a, "search"),
            };
            Ok(ok(vec![text(ops::stackups_text(&ops::stackups(&q)))]))
        }
        "edit" => {
            let kind = match arg(a, "target") {
                Some("sch") => Kind::Schematic,
                Some("pcb") => Kind::Layout,
                Some("board") => Kind::Board,
                _ => return Err("target is sch, pcb or board".into()),
            };
            let commands = arg(a, "commands").ok_or("commands is required")?;
            if commands.trim() == "help" {
                let mut t = String::new();
                for cmd in crate::edit::commands(kind) {
                    t += &crate::edit::usage(kind, cmd);
                    t += "\n";
                }
                return Ok(ok(vec![text(t)]));
            }
            let lines: Vec<String> = commands.lines().map(str::to_string).collect();
            let item = match arg(a, "item") {
                Some(n) => n.to_string(),
                None => crate::edit::only_item(root, kind)?,
            };
            let (t, _, clean) = crate::edit::run_lines(root, kind, &item, &lines)?;
            Ok(json!({ "content": [text(t)], "isError": !clean }))
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
                focus: strings(a, "focus"),
                context: arg(a, "context").unwrap_or("dim").parse()?,
                rulers: flag(a, "rulers"),
                ..Default::default()
            };
            let png = agentee_view::render_png(&p, r, &opts)?;
            let (w, h) = png_size(&png);
            let mut note = format!("{} ({w}x{h})", p.name_of(r));
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
                Some("sim") => Kind::Sim,
                _ => {
                    return Err("kind is board, symbol, footprint, schematic, layout or sim".into());
                }
            };
            let sim = ops::SimTemplate::parse(arg(a, "sim_kind").unwrap_or("fdtd"))?;
            let default = match kind {
                Kind::Symbol => "symbols",
                Kind::Footprint => "footprints",
                _ => ".",
            };
            let path = ops::new_item(
                kind,
                sim,
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
        "tie" => {
            let name = arg(a, "name").ok_or("name is required")?;
            let nets: Vec<String> = arg(a, "nets")
                .map(|v| {
                    v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
                })
                .unwrap_or_default();
            let r = ops::tie(&ops::load(root)?, name, &nets, !flag(a, "dry_run"))?;
            Ok(ok(vec![text(pretty(&r))]))
        }
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
                via: if a.get("via").is_some_and(Value::is_array) {
                    strings(a, "via")
                } else {
                    split("via")
                },
                via_cost: a.get("via_cost").and_then(Value::as_f64).unwrap_or(3.0),
                bend_cost: a.get("bend_cost").and_then(Value::as_f64).unwrap_or(0.1),
                pairs: flag(a, "pairs"),
                via_in_pad: flag(a, "via_in_pad"),
                ..Default::default()
            };
            Ok(ok(vec![text(pretty(&ops::route(&ops::load(root)?, name, &opts, !dry_run)?))]))
        }
        "testpoints" => {
            let nets: Vec<String> = arg(a, "nets")
                .map(|v| {
                    v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
                })
                .unwrap_or_default();
            let opts = ops::TestpointOptions {
                nets,
                side: arg(a, "side").map(str::to_string),
                pitch: a.get("pitch").and_then(Value::as_f64).unwrap_or(2.54),
                write: !flag(a, "dry_run"),
            };
            Ok(ok(vec![text(pretty(&ops::testpoints(
                root,
                arg(a, "name").ok_or("name is required")?,
                &opts,
            )?))]))
        }
        "neck" => {
            let nets: Vec<String> = arg(a, "nets")
                .map(|v| {
                    v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
                })
                .unwrap_or_else(|| vec!["*".into()]);
            let opts = agentee_core::neck::NeckOptions { nets, taper: flag(a, "taper") };
            Ok(ok(vec![text(pretty(&ops::neck(
                root,
                arg(a, "name").ok_or("name is required")?,
                &opts,
                !flag(a, "dry_run"),
            )?))]))
        }
        "place" => {
            let parts: Vec<String> = arg(a, "parts")
                .map(|v| {
                    v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
                })
                .unwrap_or_default();
            let args = ops::PlaceArgs {
                parts,
                keep_placed: flag(a, "keep_placed"),
                side: arg(a, "side").unwrap_or("F").to_string(),
                seed: a.get("seed").and_then(Value::as_u64).unwrap_or(1),
                write: !flag(a, "dry_run"),
            };
            Ok(ok(vec![text(pretty(&ops::place(
                root,
                arg(a, "name").ok_or("name is required")?,
                &args,
            )?))]))
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
            let p = ops::load_footprints(root)?;
            Ok(ok(vec![text(pretty(&ops::fetch_models(&p)?))]))
        }
        "fill" => Ok(ok(vec![text(pretty(&ops::write_fills(
            &ops::load(root)?,
            arg(a, "name").ok_or("name is required")?,
        )?))])),
        "export" => {
            let p = ops::load(root)?;
            let out = root.join(arg(a, "out").ok_or("out is required")?);
            Ok(ok(vec![text(pretty(&ops::export(
                &p,
                arg(a, "name").ok_or("name is required")?,
                &out,
            )?))]))
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
        "parts" => {
            let p = ops::load(root)?;
            let distributors = strings(a, "distributors");
            let refs = strings(a, "refs");
            let name = arg(a, "name").ok_or("name is required")?;
            let q = ops::PartsQuery {
                boards: int(a, "boards", 1) as u32,
                alternatives: a.get("alternatives").and_then(Value::as_bool).unwrap_or(true),
                distributors: &distributors,
                farnell_store: arg(a, "farnell_store"),
                refs: &refs,
                config: None,
            };
            if let Some(dir) = arg(a, "order") {
                let spares = a.get("spares").and_then(Value::as_bool).unwrap_or(false);
                return Ok(ok(vec![text(pretty(&ops::order(
                    &p,
                    name,
                    &q,
                    spares,
                    &root.join(dir),
                )?))]));
            }
            let r = ops::parts(&p, name, &q)?;
            Ok(ok(vec![text(pretty(&serde_json::to_value(&r).map_err(|e| e.to_string())?))]))
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
                "serverInfo": { "name": "agentee", "version": concat!(env!("CARGO_PKG_VERSION"), " ", env!("AGENTEE_BUILD_ID")) },
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
    fn edit_runs_a_batch_on_a_named_schematic() {
        let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
        let dir = std::env::temp_dir().join(format!("agentee-mcp-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("symbols")).unwrap();
        std::fs::create_dir_all(dir.join("footprints")).unwrap();
        for f in ["symbols/R.sym.toml", "footprints/R_0402_1005Metric.fp.toml"] {
            std::fs::copy(lna.join(f), dir.join(f)).unwrap();
        }
        std::fs::write(dir.join("a.sch.toml"), "name = \"a\"\n").unwrap();
        std::fs::write(dir.join("b.sch.toml"), "name = \"b\"\n").unwrap();
        let edit = |args: Value| {
            let c = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": "edit", "arguments": args } });
            handle(&dir, &c).unwrap()
        };

        let r = edit(json!({ "target": "sch", "commands": "add R1 R 1k" }));
        assert_eq!(r["result"]["isError"], true, "{r}");
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("a, b"), "{r}");

        let r = edit(
            json!({ "target": "sch", "item": "b", "commands": "add R1 R 1k\nadd R2 R 10k\nnet MID R1.2 R2.1" }),
        );
        assert_eq!(r["result"]["isError"], false, "{r}");
        let text = std::fs::read_to_string(dir.join("b.sch.toml")).unwrap();
        assert!(text.contains("pins = [\"R1.2\", \"R2.1\"]"), "{text}");

        let r = edit(json!({ "target": "pcb", "commands": "help" }));
        assert!(
            r["result"]["content"][0]["text"].as_str().unwrap().contains("track NET LAYER"),
            "{r}"
        );
    }

    #[test]
    fn new_item_writes_a_logic_sim_that_runs_on_the_counter() {
        let logic = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logic");
        let dir = std::env::temp_dir().join(format!("agentee-new-logic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "symbols", "footprints"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            for e in std::fs::read_dir(logic.join(sub)).unwrap() {
                let path = e.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                if name.ends_with(".toml") && !name.ends_with(".sim.toml") {
                    std::fs::copy(&path, dir.join(sub).join(name)).unwrap();
                }
            }
        }
        let call = json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "new_item", "arguments": { "kind": "sim", "sim_kind": "logic", "name": "starter" } } });
        let r = handle(&dir, &call).unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        let path = dir.join("starter.sim.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("kind = \"logic\""));
        let text = text
            .replace("# ignore = [\"J1\"]", "ignore = [\"J1\", \"J2\"]")
            .replace("# schematic = \"top\"", "schematic = \"counter\"");
        std::fs::write(&path, text).unwrap();
        let p = agentee_core::Project::load(&dir).unwrap();
        let s = p.sims.iter().find(|s| s.name == "starter").unwrap();
        let errors: Vec<&str> = s
            .diags
            .iter()
            .filter(|d| d.severity == agentee_core::Severity::Error)
            .map(|d| d.message.as_str())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        let spec = s.item.logic.as_ref().unwrap();
        let (res, _) = agentee_sim::logic::run(spec, "starter", 0, "starter.vcd");
        assert_eq!((res.passed, res.failures.len()), (1, 0), "{:?}", res.failures);
        let bad = json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "new_item", "arguments": { "kind": "sim", "sim_kind": "spice", "name": "x" } } });
        assert_eq!(handle(&dir, &bad).unwrap()["result"]["isError"], true);
        let fdtd = json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": { "name": "new_item", "arguments": { "kind": "sim", "name": "rf" } } });
        handle(&dir, &fdtd).unwrap();
        let rf = std::fs::read_to_string(dir.join("rf.sim.toml")).unwrap();
        assert!(rf.contains("[frequency]") && !rf.contains("logic"));
    }

    #[test]
    fn tool_errors_are_results_not_protocol_errors() {
        let call = json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "show_item", "arguments": { "name": "missing" } } });
        let r = handle(&demo(), &call).unwrap();
        assert_eq!(r["result"]["isError"], true);
    }

    #[test]
    fn testpoints_adds_probed_pads_to_a_copy_of_the_lna() {
        let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
        let dir = std::env::temp_dir().join(format!("agentee-testpoints-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["", "symbols", "footprints"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            for e in std::fs::read_dir(lna.join(sub)).unwrap() {
                let path = e.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                if name.ends_with(".toml") && !name.ends_with(".sim.toml") {
                    std::fs::copy(&path, dir.join(sub).join(name)).unwrap();
                }
            }
        }
        ops::write_fills(&ops::load(&dir).unwrap(), "lna").unwrap();
        let call = json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "testpoints", "arguments": { "name": "lna", "nets": "VCC,VBIAS" } } });
        let r = handle(&dir, &call).unwrap();
        assert_ne!(r["result"]["isError"], true, "{r}");
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["placed"].as_array().unwrap().len(), 2, "{v}");
        assert!(v["unrouted"].as_array().unwrap().is_empty(), "{v}");
        let p = ops::load(&dir).unwrap();
        let errors: Vec<_> =
            p.layouts[0].diags.iter().filter(|d| d.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(v["fills"]["fills"].as_u64().is_some_and(|n| n > 0), "{v}");
        let keys = &p.layouts[0].item.fill_keys;
        assert!(!keys.is_empty() && keys.iter().all(|k| k.stored), "stored fills are stale");
        assert!(!p.layouts[0].diags.iter().any(|d| d.rule.as_deref() == Some("test-access")));
        let tps = p.layouts[0].item.parts.iter().filter(|q| q.reference.starts_with("TP"));
        assert!(tps.clone().count() == 2 && tps.clone().all(|q| q.bottom));
        let sch = std::fs::read_to_string(dir.join("lna.sch.toml")).unwrap();
        assert!(sch.contains("\"TP1.1\"") && sch.contains("\"TP2.1\""));
    }

    #[test]
    fn neck_rewrites_wide_track_ends_and_refreshes_fills() {
        let lna = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
        let dir = std::env::temp_dir().join(format!("agentee-neck-mcp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for f in ["symbols/R.sym.toml", "footprints/R_0402_1005Metric.fp.toml"] {
            std::fs::create_dir_all(dir.join(f).parent().unwrap()).unwrap();
            std::fs::copy(lna.join(f), dir.join(f)).unwrap();
        }
        std::fs::write(
            dir.join("t.board.toml"),
            "name = \"t\"\nfab = \"jlcpcb\"\n[outline]\nsize = [20, 10]\n[stackup]\npreset = \"jlcpcb-2l-1.6mm\"\n[[netclasses]]\nname = \"Default\"\ntrack_width = \"0.2mm\"\nclearance = \"0.15mm\"\n[[netclasses]]\nname = \"Power\"\ntrack_width = \"0.8mm\"\nclearance = \"0.15mm\"\n",
        )
        .unwrap();
        let mut sch = String::from("name = \"t\"\nboard = \"t\"\n");
        let mut pcb = String::from("name = \"t\"\nboard = \"t\"\nschematic = \"t\"\n");
        for (i, (r, x)) in [("R1", 5.0), ("R2", 12.0)].iter().enumerate() {
            sch += &format!(
                "\n[[parts]]\nref = \"{r}\"\nsymbol = \"R\"\nvalue = \"x\"\nfootprint = \"R_0402_1005Metric\"\nat = [{}, 20.32]\n",
                10.16 * (i + 1) as f64
            );
            pcb += &format!(
                "\n[[footprints]]\nref = \"{r}\"\nat = [{x}, 5]\nlabel = {{ hide = true }}\n"
            );
        }
        sch += "\n[[nets]]\nname = \"P\"\nclass = \"Power\"\npins = [\"R1.2\", \"R2.1\"]\n";
        sch += "\n[[nets]]\nname = \"A\"\npins = [\"R1.1\"]\n\n[[nets]]\nname = \"GND\"\npins = [\"R2.2\"]\n";
        pcb += "\n[[tracks]]\nnet = \"P\"\nlayer = \"F.Cu\"\npoints = [[5.51, 5], [11.49, 5]]\n";
        pcb += "\n[[zones]]\nnet = \"GND\"\nlayers = [\"B.Cu\"]\n";
        std::fs::write(dir.join("t.sch.toml"), sch).unwrap();
        std::fs::write(dir.join("t.pcb.toml"), &pcb).unwrap();
        let call = |dry: bool| {
            let c = json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "neck", "arguments": { "name": "t", "dry_run": dry } } });
            let r = handle(&dir, &c).unwrap();
            assert_ne!(r["result"]["isError"], true, "{r}");
            let v: Value =
                serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
            v
        };
        let v = call(true);
        assert_eq!(v["necked"].as_array().unwrap().len(), 2, "{v}");
        assert_eq!(std::fs::read_to_string(dir.join("t.pcb.toml")).unwrap(), pcb);
        let v = call(false);
        assert_eq!(v["tracks_changed"], 1, "{v}");
        assert_eq!(v["tracks_added"], 2, "{v}");
        assert_eq!(v["fills"], 1, "{v}");
        let text = std::fs::read_to_string(dir.join("t.pcb.toml")).unwrap();
        assert_eq!(text.matches("[[tracks]]").count(), 3, "{text}");
        assert!(text.contains("[[fills]]"), "{text}");
        let p = ops::load(&dir).unwrap();
        let errors: Vec<_> =
            p.layouts[0].diags.iter().filter(|d| d.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            p.layouts[0].diags.iter().filter(|d| d.rule.as_deref() == Some("neckdown")).count(),
            2
        );
        assert!(p.layouts[0].item.fill_keys.iter().all(|k| k.stored));
        assert!(call(false)["necked"].as_array().unwrap().is_empty());
    }

    #[test]
    fn check_passes_on_the_demo() {
        let (_, v, ok) = ops::check_report(&ops::load(&demo()).unwrap(), None, Severity::Warning);
        assert!(ok, "{v}");
    }
}
