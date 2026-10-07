#![recursion_limit = "256"]

mod edit;
mod mcp;
mod ops;
mod templates;

use agentee_core::Severity;
use agentee_core::project::Kind;
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub const FORMAT: &str = include_str!("../../../docs/format.md");

#[derive(Parser)]
#[command(
    name = "agentee",
    version,
    about = "Electronic design for agents: board spec, symbols and footprints as TOML"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum NewKind {
    Board,
    Symbol,
    Footprint,
    Schematic,
    Layout,
    Sim,
}

#[derive(Clone, Copy, ValueEnum)]
enum LibKind {
    Symbol,
    Footprint,
    /// A whole .kicad_pcb: board, layout, a netlist schematic, footprints and pin symbols
    Board,
}

#[derive(Subcommand)]
enum Cmd {
    /// Load every board, symbol and footprint under PATH and report problems
    Check {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Only this item
        #[arg(long)]
        item: Option<String>,
        /// Also print info notes
        #[arg(long)]
        info: bool,
        #[arg(long)]
        json: bool,
    },
    /// Design rule checks of a layout: its rule diagnostics, or every rule with --list
    Drc {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Print every rule with its id, category, severity and whether it applies here
        #[arg(long)]
        list: bool,
        #[arg(long)]
        json: bool,
    },
    /// List the items in a project
    List {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Print the resolved model of one item as JSON
    Show {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
    },
    /// Render an item to PNG as the viewer draws it
    Render {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1400)]
        width: u32,
        #[arg(long, default_value_t = 900)]
        height: u32,
        /// Pixels per point
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
        #[arg(long, default_value_t = 1)]
        unit: u32,
        /// Only the drawing, no side panels
        #[arg(long)]
        canvas_only: bool,
        #[arg(long)]
        hidden_pins: bool,
        /// Layers to turn on, comma separated (F.Fab,F.Mask,In1.Cu)
        #[arg(long, value_delimiter = ',')]
        show: Vec<String>,
        /// Layers to turn off, comma separated
        #[arg(long, value_delimiter = ',')]
        hide: Vec<String>,
        /// Zoom to x0,y0,x1,y1 in mm
        #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
        region: Option<Vec<f64>>,
        /// Zoom to these parts, nets or pins (U1,SPI_*,U1.3) and fade the rest
        #[arg(long, value_delimiter = ',')]
        focus: Vec<String>,
        /// What happens to everything outside --focus: dim, hide or show
        #[arg(long, default_value = "dim")]
        context: agentee_view::Context,
        /// Label mm coordinates along the edges
        #[arg(long)]
        rulers: bool,
    },
    /// Open the viewer, it reloads when files change
    View {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        select: Option<String>,
        /// Open layouts in the 3D view
        #[arg(long = "3d")]
        view_3d: bool,
    },
    /// Serve the project over MCP on stdio
    Mcp {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Write a starter board, symbol, footprint, schematic, layout or sim
    New {
        kind: NewKind,
        name: String,
        #[arg(short, long)]
        dir: Option<PathBuf>,
        /// The sim to start from: an FDTD run of a layout, or a logic sim of a schematic
        #[arg(long = "kind", value_enum, default_value = "fdtd")]
        sim_kind: ops::SimTemplate,
    },
    /// Import from the installed KiCad libraries
    Import {
        kind: LibKind,
        /// Library:Name, a .kicad_sym path with :Name, or a .kicad_mod path
        spec: String,
        #[arg(short, long)]
        dir: Option<PathBuf>,
        /// With a symbol, also import its default footprint
        #[arg(long)]
        with_footprint: bool,
        /// With a symbol, import this footprint (Library:Name) and link it
        #[arg(long)]
        footprint: Option<String>,
        /// Where footprints go when importing with a symbol
        #[arg(long, default_value = "footprints")]
        footprint_dir: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Search KiCad library names; every word must match Library:Name
    Search {
        kind: LibKind,
        query: Vec<String>,
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    /// Run a simulation (*.sim.toml): FDTD, dc, thermal, cascade, channel, pdn or logic
    Sim {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Only mesh it and print the grid, time step and step count
        #[arg(long)]
        dry_run: bool,
    },
    /// Analyse a sim result: TDR, mixed mode, crosstalk, passivity and reciprocity
    Sparam {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Port for a TDR (name or number)
        #[arg(long)]
        tdr: Option<String>,
        /// 10-90% rise time, default 1.3 / the top frequency
        #[arg(long)]
        rise: Option<String>,
        /// Mixed mode: IN+,IN-,OUT+,OUT-
        #[arg(long)]
        pair: Option<String>,
        /// Crosstalk from one port to another: FROM,TO
        #[arg(long)]
        xtalk: Option<String>,
    },
    /// Download the KiCad 3D models (VRML) the project's footprints name, for the 3D view
    Models {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Write the fab package for a layout: Gerbers, drill, BOM, placement, notes
    Fab {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Write the board and every part model of a layout as one STEP assembly, for enclosure CAD
    Export {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Output file, .step or .stp
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Stock, price and cheaper equivalents for a layout's or schematic's BOM from Mouser and Farnell, with keys in ~/.config/agentee/distributors.toml
    Parts {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Boards to buy for
        #[arg(long, default_value_t = 1)]
        boards: u32,
        /// Distributors to ask, comma separated (mouser, farnell), default every one with a key
        #[arg(long, value_delimiter = ',')]
        distributor: Vec<String>,
        /// Farnell store, e.g. uk.farnell.com, ie.farnell.com, de.farnell.com, www.newark.com
        #[arg(long)]
        farnell_store: Option<String>,
        /// Only these references, comma separated
        #[arg(long, value_delimiter = ',')]
        refs: Vec<String>,
        /// Write order sheets to this directory instead: NAME-order.csv and one NAME-<distributor>.csv per distributor, each with the lines to buy there first, then the lines bought at the other one, then what it lacks
        #[arg(long)]
        order: Option<PathBuf>,
        /// With --order, add the hand assembly allowance: 0402/0603 resistors and capacitors to the next 10 above need + 5, one spare per D, Q, U and F line, or the part's `spares` field
        #[arg(long)]
        spares: bool,
        /// Price what is on the BOM without searching for cheaper equivalents
        #[arg(long)]
        no_alternatives: bool,
        /// Key file instead of ~/.config/agentee/distributors.toml
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Fill the zones of a layout and store the copper in its file, so loads skip the fill
    Fill {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
    },
    /// Route nets of a layout on a grid and append the tracks and vias to its file
    Route {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Nets to route, globs allowed, comma separated or repeated
        #[arg(long, required = true, value_delimiter = ',')]
        nets: Vec<String>,
        /// Copper layers to route on, default every layer
        #[arg(long, value_delimiter = ',')]
        layers: Vec<String>,
        /// Grid cell in mm
        #[arg(long, default_value_t = 0.05)]
        grid: f64,
        /// Vias from the board to use, comma separated or repeated, default the net class vias
        #[arg(long, value_delimiter = ',')]
        via: Vec<String>,
        /// Cost of a via in mm of track
        #[arg(long, default_value_t = 3.0)]
        via_cost: f64,
        /// Cost of a 45 degree bend in mm of track, three times that for 90
        #[arg(long, default_value_t = 0.1)]
        bend_cost: f64,
        /// Route differential pairs as coupled pairs where they fit
        #[arg(long)]
        pairs: bool,
        /// Allow vias fully inside SMD pads (filled and capped), default keep them off every pad
        #[arg(long)]
        via_in_pad: bool,
        /// Remove the existing tracks and vias of these nets first and route them again
        #[arg(long)]
        reroute: bool,
        /// Report without writing the file
        #[arg(long)]
        dry_run: bool,
    },
    /// Tie every SMD pad of a plane net (a net with a zone) to its plane with a stub and a via
    /// beside the pad, and append them to the layout file
    Tie {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Plane nets to tie, globs allowed, default every net with a zone
        #[arg(long, value_delimiter = ',')]
        nets: Vec<String>,
        /// Report without writing the file
        #[arg(long)]
        dry_run: bool,
    },
    /// Meander the short net of every pair over its skew limit and every match group member
    /// short of its target, and write the new points into the tracks
    Tune {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Nets to tune, globs allowed, comma separated
        #[arg(long, default_value = "*", value_delimiter = ',')]
        nets: Vec<String>,
        /// Largest bump height in mm, default tries 1.2 mm down to 0.2 mm
        #[arg(long)]
        amplitude: Option<f64>,
        /// Bump pitch in mm, default three track widths
        #[arg(long)]
        pitch: Option<f64>,
        /// Report without writing the file
        #[arg(long)]
        dry_run: bool,
    },
    /// Neck down track ends that enter a pad narrower than the track or break clearance near it:
    /// the end becomes a separate narrower track within the class neckdown length
    Neck {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Nets to neck down, globs allowed, comma separated
        #[arg(long, default_value = "*", value_delimiter = ',')]
        nets: Vec<String>,
        /// Step the width down in a short chain of segments instead of one neck
        #[arg(long)]
        taper: bool,
        /// Report without writing the file
        #[arg(long)]
        dry_run: bool,
    },
    /// Place the schematic's parts on the board: connectors on edges, holes and fiducials in
    /// corners, large chips central, passives clustered by the netlist, then legalised and refined
    Place {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Parts to place, globs allowed, comma separated; default every part
        #[arg(long, value_delimiter = ',')]
        parts: Vec<String>,
        /// Leave every part that already has a placement where it is
        #[arg(long)]
        keep_placed: bool,
        /// Sides to place on: F, B or both
        #[arg(long, default_value = "F")]
        side: String,
        /// Seed for the refinement, the same seed gives the same placement
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Report without writing the file
        #[arg(long)]
        dry_run: bool,
    },
    /// Move every failing silk reference to the clear spot check suggests, again until they
    /// settle, and hide the ones with nowhere to go when asked
    Silk {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Hide references that have no clear spot
        #[arg(long)]
        hide: bool,
        /// List the fixes without writing the file
        #[arg(long)]
        dry_run: bool,
    },
    /// Add a test pad to each net that has no probe access: a TestPoint part in the schematic,
    /// a footprint on the probe side near the net's copper, and a routed track (and via) to it
    Testpoints {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Nets to give a test pad, globs allowed, comma separated; default the [test] nets
        #[arg(long, value_delimiter = ',')]
        nets: Vec<String>,
        /// Probe side, F or B; default the [test] side (B)
        #[arg(long)]
        side: Option<String>,
        /// Grid the pads sit on and the least spacing between them, mm
        #[arg(long, default_value_t = 2.54)]
        pitch: f64,
        /// List the spots without writing any file
        #[arg(long)]
        dry_run: bool,
    },
    /// Trace calculators
    Calc {
        #[command(subcommand)]
        calc: Calc,
    },
    /// Run the layout engine on a layout: the configured stages in order, then the score per term
    Layout {
        name: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        /// Start at this stage, keeping the results of the stages before it
        #[arg(long)]
        from: Option<String>,
        /// Stop after this stage
        #[arg(long)]
        to: Option<String>,
        /// Run one stage only
        #[arg(long)]
        only: Option<String>,
        /// Report without writing the plan into the layout file
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Reassign a chip's swappable I/O (same bank, pairs as pairs, clock pins kept on clock pins) to untangle the ratsnest
    Pinswap {
        name: String,
        #[arg(long)]
        part: String,
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Rewrite the schematic pin references
        #[arg(long)]
        write: bool,
    },
    /// List the fab stackup presets a board's `stackup.preset` can name, or print one
    Stackups {
        /// Print this preset's layers as JSON
        name: Option<String>,
        /// jlcpcb, pcbway or generic
        #[arg(long)]
        fab: Option<String>,
        /// Copper layer count
        #[arg(long)]
        layers: Option<usize>,
        /// Finished thickness in mm, matched within 10%
        #[arg(long)]
        thickness: Option<f64>,
        /// Substring of the name or description, like 1080 or 2oz
        #[arg(long)]
        search: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Edit a schematic, layout or board with commands instead of writing TOML by hand
    ///
    /// Every edit takes the item name and a command. `agentee edit sch help` lists them with
    /// their arguments. A list of commands also works: `agentee edit sch NAME -` reads them from
    /// stdin, one per line, and `agentee edit sch NAME FILE` from a file, one load, one check at
    /// the end. NAME can be left out when the project has one item of that kind. It rewrites the TOML, keeps the comments and layout of the file, and prints the
    /// check report of the files it touched.
    ///
    /// Pin references are REF.PIN, by number (U1.3) or by a unique pin name (U1.VCC). The pin
    /// numbers that came back are in the JSON facts.
    Edit {
        /// schematic, layout or board
        #[arg(value_enum)]
        target: EditKind,
        /// Item name, or `-` for stdin or a script file when the project has one item of the kind
        name: String,
        /// Arguments of the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
        /// List the item's parts and nets, or its tracks, vias and zones
        #[arg(long)]
        list: bool,
        #[arg(long)]
        json: bool,
    },
    /// Print the file format reference
    Docs,
}

#[derive(Clone, Copy, ValueEnum)]
enum EditKind {
    Sch,
    Pcb,
    Board,
}

impl EditKind {
    fn kind(self) -> Kind {
        match self {
            EditKind::Sch => Kind::Schematic,
            EditKind::Pcb => Kind::Layout,
            EditKind::Board => Kind::Board,
        }
    }
}

#[derive(Subcommand)]
enum Calc {
    /// Minimum track width for a current (IPC-2221)
    TraceWidth {
        #[arg(long)]
        current: String,
        #[arg(long, default_value = "1oz")]
        copper: String,
        #[arg(long, default_value = "10C")]
        rise: String,
        #[arg(long)]
        internal: bool,
    },
    /// Meander points that add length to a straight segment
    Serpentine {
        /// Segment start x,y in mm
        #[arg(long)]
        from: String,
        /// Segment end x,y in mm
        #[arg(long)]
        to: String,
        /// Length to add, e.g. 2.5mm
        #[arg(long)]
        add: String,
        /// Largest bump height
        #[arg(long, default_value = "0.6mm")]
        amplitude: String,
        /// Distance between the legs of a bump
        #[arg(long, default_value = "0.4mm")]
        pitch: String,
    },
    /// Solve the trace cross-section on the GPU (2D field solver)
    Field {
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        #[arg(long)]
        board: Option<String>,
        #[arg(long, default_value = "F.Cu")]
        layer: String,
        #[arg(long)]
        width: Option<String>,
        /// Take width and gaps from this net class
        #[arg(long)]
        netclass: Option<String>,
        #[arg(long)]
        gap: Option<String>,
        #[arg(long)]
        coplanar_gap: Option<String>,
        /// Leave off the solder mask the stackup puts over outer layers
        #[arg(long)]
        no_mask: bool,
        #[arg(long)]
        fine: bool,
        /// Loss sweep START,STOP,POINTS (log spaced), e.g. 10MHz,20GHz,21
        #[arg(long)]
        sweep: Option<String>,
    },
    /// Impedance on a board layer, or the width for a target
    Impedance {
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
        #[arg(long)]
        board: Option<String>,
        #[arg(long)]
        layer: Option<String>,
        #[arg(long)]
        width: Option<String>,
        #[arg(long)]
        gap: Option<String>,
        /// Gap to the ground pour either side (grounded coplanar, outer layers)
        #[arg(long)]
        coplanar_gap: Option<String>,
        #[arg(long)]
        target: Option<String>,
        /// Dielectric height for a bare microstrip, instead of a board
        #[arg(long)]
        h: Option<String>,
        #[arg(long)]
        er: Option<f64>,
        #[arg(long)]
        t: Option<String>,
    },
}

fn print_json(v: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn run(cli: Cli) -> Result<bool, String> {
    match cli.cmd {
        Cmd::Check { path, item, info, json } => {
            let p = ops::load_checked(&path)?;
            let item = item.map(|n| ops::find(&p, &n)).transpose()?;
            let min = if info { Severity::Info } else { Severity::Warning };
            let (text, v, ok) = ops::check_report(&p, item, min);
            if json {
                print_json(&v);
            } else {
                print!("{text}");
            }
            Ok(ok)
        }
        Cmd::Drc { name, project, list, json } => {
            let p = ops::load(&project)?;
            let (text, v) = ops::drc(&p, &name, list)?;
            if json {
                print_json(&v);
            } else {
                print!("{text}");
            }
            Ok(true)
        }
        Cmd::List { path } => {
            print_json(&ops::list(&ops::load(&path)?));
            Ok(true)
        }
        Cmd::Show { name, project } => {
            let p = ops::load(&project)?;
            let r = ops::find(&p, &name)?;
            print_json(&ops::show(&p, r));
            Ok(true)
        }
        Cmd::Render {
            name,
            project,
            out,
            width,
            height,
            scale,
            unit,
            canvas_only,
            hidden_pins,
            show,
            hide,
            region,
            focus,
            context,
            rulers,
        } => {
            let p = ops::load(&project)?;
            let r = ops::find(&p, &name)?;
            let opts = agentee_view::RenderOptions {
                width,
                height,
                scale,
                unit,
                panels: !canvas_only,
                hidden_pins,
                show,
                hide,
                region: region.filter(|r| r.len() == 4).map(|r| [r[0], r[1], r[2], r[3]]),
                focus,
                context,
                rulers,
            };
            let png = agentee_view::render_png(&p, r, &opts)?;
            std::fs::write(&out, png).map_err(|e| format!("{}: {e}", out.display()))?;
            println!("{}", out.display());
            Ok(true)
        }
        Cmd::View { path, select, view_3d } => {
            agentee_view::run(path, select, view_3d).map_err(|e| e.to_string())?;
            Ok(true)
        }
        Cmd::Mcp { path } => {
            mcp::serve(&path).map_err(|e| e.to_string())?;
            Ok(true)
        }
        Cmd::New { kind, name, dir, sim_kind } => {
            let (kind, default) = match kind {
                NewKind::Board => (Kind::Board, "."),
                NewKind::Symbol => (Kind::Symbol, "symbols"),
                NewKind::Footprint => (Kind::Footprint, "footprints"),
                NewKind::Schematic => (Kind::Schematic, "."),
                NewKind::Layout => (Kind::Layout, "."),
                NewKind::Sim => (Kind::Sim, "."),
            };
            let path =
                ops::new_item(kind, sim_kind, &name, &dir.unwrap_or_else(|| default.into()))?;
            println!("{}", path.display());
            Ok(true)
        }
        Cmd::Import { kind, spec, dir, with_footprint, footprint, footprint_dir, force } => {
            let (written, notes) = match kind {
                LibKind::Symbol => {
                    let pick = match (footprint, with_footprint) {
                        (Some(f), _) => ops::FootprintPick::Spec(f),
                        (None, true) => ops::FootprintPick::Default,
                        (None, false) => ops::FootprintPick::None,
                    };
                    ops::import_symbol(
                        &spec,
                        &dir.unwrap_or_else(|| "symbols".into()),
                        &footprint_dir,
                        pick,
                        force,
                    )?
                }
                LibKind::Footprint => (
                    vec![ops::import_footprint(
                        &spec,
                        &dir.unwrap_or_else(|| "footprints".into()),
                        force,
                    )?],
                    Vec::new(),
                ),
                LibKind::Board => ops::import_board(
                    std::path::Path::new(&spec),
                    &dir.unwrap_or_else(|| ".".into()),
                    force,
                )?,
            };
            for w in written {
                println!("{}", w.display());
            }
            for n in notes {
                eprintln!("note: {n}");
            }
            Ok(true)
        }
        Cmd::Search { kind, query, limit } => {
            let q = query.join(" ");
            let hits = match kind {
                LibKind::Symbol => agentee_kicad::search_symbols(&q, limit),
                LibKind::Footprint => agentee_kicad::search_footprints(&q, limit),
                LibKind::Board => return Err("search covers symbols and footprints".into()),
            };
            for h in &hits {
                println!("{}:{}", h.library, h.name);
            }
            Ok(!hits.is_empty())
        }
        Cmd::Calc {
            calc:
                Calc::Field {
                    project,
                    board,
                    layer,
                    width,
                    netclass,
                    gap,
                    coplanar_gap,
                    no_mask,
                    fine,
                    sweep,
                },
        } => {
            let p = ops::load(&project)?;
            let v = ops::field_solve(
                &p,
                &ops::FieldQuery {
                    board: board.as_deref(),
                    layer: &layer,
                    width: width.as_deref(),
                    netclass: netclass.as_deref(),
                    gap: gap.as_deref(),
                    coplanar_gap: coplanar_gap.as_deref(),
                    mask: !no_mask,
                    fine,
                    sweep: sweep.as_deref(),
                },
            )?;
            print_json(&v);
            Ok(true)
        }
        Cmd::Calc { calc: Calc::Serpentine { from, to, add, amplitude, pitch } } => {
            print_json(&ops::serpentine(&from, &to, &add, &amplitude, &pitch)?);
            Ok(true)
        }
        Cmd::Calc { calc: Calc::TraceWidth { current, copper, rise, internal } } => {
            print_json(&ops::trace_width(&current, &copper, &rise, internal)?);
            Ok(true)
        }
        Cmd::Calc {
            calc:
                Calc::Impedance { project, board, layer, width, gap, coplanar_gap, target, h, er, t },
        } => {
            let p = if h.is_some() { None } else { Some(ops::load(&project)?) };
            let v = ops::impedance(&ops::ImpedanceQuery {
                project: p.as_ref(),
                board: board.as_deref(),
                layer: layer.as_deref(),
                width: width.as_deref(),
                gap: gap.as_deref(),
                coplanar_gap: coplanar_gap.as_deref(),
                target: target.as_deref(),
                h: h.as_deref(),
                er,
                t: t.as_deref(),
            })?;
            print_json(&v);
            Ok(true)
        }
        Cmd::Sim { name, project, dry_run } => {
            let p = ops::load(&project)?;
            let mut last = std::time::Instant::now();
            let v = ops::run_sim(&p, &name, dry_run, &mut |port, steps, db| {
                if last.elapsed().as_secs_f64() > 2.0 {
                    eprintln!("{port}: {steps} steps, fields down {db:.1} dB");
                    last = std::time::Instant::now();
                }
            })?;
            print_json(&v);
            Ok(true)
        }
        Cmd::Sparam { name, project, tdr, rise, pair, xtalk } => {
            let p = ops::load(&project)?;
            let q = ops::SparamQuery {
                tdr: tdr.as_deref(),
                rise: rise.as_deref(),
                pair: pair.as_deref(),
                xtalk: xtalk.as_deref(),
            };
            print_json(&ops::sparam(&p, &name, &q)?);
            Ok(true)
        }
        Cmd::Models { path } => {
            let p = ops::load_footprints(&path)?;
            print_json(&ops::fetch_models(&p)?);
            Ok(true)
        }
        Cmd::Parts {
            name,
            project,
            boards,
            distributor,
            farnell_store,
            refs,
            order,
            spares,
            no_alternatives,
            config,
            json,
        } => {
            let p = ops::load(&project)?;
            let q = ops::PartsQuery {
                boards,
                alternatives: !no_alternatives,
                distributors: &distributor,
                farnell_store: farnell_store.as_deref(),
                refs: &refs,
                config: config.as_deref(),
            };
            if let Some(dir) = order {
                print_json(&ops::order(&p, &name, &q, spares, &dir)?);
                return Ok(true);
            }
            let r = ops::parts(&p, &name, &q)?;
            if json {
                print_json(&serde_json::to_value(&r).map_err(|e| e.to_string())?);
            } else {
                print!("{}", ops::parts_text(&r));
            }
            Ok(r.errors.is_empty())
        }
        Cmd::Fill { name, project } => {
            let p = ops::load(&project)?;
            print_json(&ops::write_fills(&p, &name)?);
            Ok(true)
        }
        Cmd::Fab { name, project, out } => {
            let p = ops::load(&project)?;
            print_json(&ops::fab(&p, &name, &out)?);
            Ok(true)
        }
        Cmd::Export { name, project, out } => {
            let p = ops::load(&project)?;
            print_json(&ops::export(&p, &name, &out)?);
            Ok(true)
        }
        Cmd::Tie { name, project, nets, dry_run } => {
            let p = ops::load(&project)?;
            let r = ops::tie(&p, &name, &nets, !dry_run)?;
            let ok = r["failed"].as_array().is_some_and(|f| f.is_empty());
            print_json(&r);
            Ok(ok)
        }
        Cmd::Route {
            name,
            project,
            nets,
            layers,
            grid,
            via,
            via_cost,
            bend_cost,
            pairs,
            via_in_pad,
            reroute,
            dry_run,
        } => {
            if reroute && !dry_run {
                let p = ops::load(&project)?;
                ops::unroute(&p, &name, &nets)?;
            }
            let p = ops::load(&project)?;
            let opts = agentee_core::route::RouteOptions {
                nets,
                layers,
                grid,
                via,
                via_cost,
                bend_cost,
                pairs,
                via_in_pad,
                ..Default::default()
            };
            let r = ops::route(&p, &name, &opts, !dry_run)?;
            let ok = r["failed"].as_array().is_some_and(|f| f.is_empty());
            print_json(&r);
            Ok(ok)
        }
        Cmd::Tune { name, project, nets, amplitude, pitch, dry_run } => {
            let p = ops::load(&project)?;
            let opts = agentee_core::tune::TuneOptions { nets, amplitude, pitch };
            let r = ops::tune(&p, &name, &opts, !dry_run)?;
            let ok = r["failed"].as_array().is_some_and(|f| f.is_empty());
            print_json(&r);
            Ok(ok)
        }
        Cmd::Neck { name, project, nets, taper, dry_run } => {
            let opts = agentee_core::neck::NeckOptions { nets, taper };
            let r = ops::neck(&project, &name, &opts, !dry_run)?;
            let ok = r["failed"].as_array().is_some_and(|f| f.is_empty());
            print_json(&r);
            Ok(ok)
        }
        Cmd::Place { name, project, parts, keep_placed, side, seed, dry_run } => {
            let a = ops::PlaceArgs { parts, keep_placed, side, seed, write: !dry_run };
            let r = ops::place(&project, &name, &a)?;
            let ok = r["failed"].as_array().is_none_or(|f| f.is_empty());
            print_json(&r);
            Ok(ok)
        }
        Cmd::Silk { name, project, hide, dry_run } => {
            let r = ops::silk(&project, &name, hide, !dry_run)?;
            let ok = r["still_failing"].as_array().is_none_or(|f| f.is_empty());
            print_json(&r);
            Ok(ok)
        }
        Cmd::Testpoints { name, project, nets, side, pitch, dry_run } => {
            let opts = ops::TestpointOptions { nets, side, pitch, write: !dry_run };
            let r = ops::testpoints(&project, &name, &opts)?;
            let ok = r["failed"].as_array().is_none_or(|f| f.is_empty())
                && r["unrouted"].as_array().is_none_or(|f| f.is_empty());
            print_json(&r);
            Ok(ok)
        }
        Cmd::Layout { name, project, from, to, only, dry_run, json } => {
            let r = ops::layout_engine(
                &project,
                &name,
                &ops::LayoutArgs { from, to, only, write: !dry_run },
            )?;
            if json {
                print_json(&r);
            } else {
                for s in r["skipped"].as_array().into_iter().flatten() {
                    println!("skipped {}", s.as_str().unwrap_or(""));
                }
                print!("{}", r["score_table"].as_str().unwrap_or(""));
                print!("{}", r["time_table"].as_str().unwrap_or(""));
            }
            Ok(true)
        }
        Cmd::Pinswap { name, part, project, seed, write } => {
            print_json(&ops::pinswap(&project, &name, &part, seed, write)?);
            Ok(true)
        }
        Cmd::Stackups { name, fab, layers, thickness, search, json } => {
            if let Some(name) = name {
                print_json(&ops::stackup(&name)?);
                return Ok(true);
            }
            let q = ops::StackupQuery {
                fab: fab.as_deref(),
                layers,
                thickness_mm: thickness,
                search: search.as_deref(),
            };
            let list = ops::stackups(&q);
            if json {
                print_json(&serde_json::to_value(&list).unwrap_or_default());
            } else {
                print!("{}", ops::stackups_text(&list));
            }
            Ok(true)
        }
        Cmd::Edit { target, name, args, list, json } => {
            let root = PathBuf::from(".");
            if name == "help" && args.is_empty() {
                println!("{}", edit_help(target.kind()));
                return Ok(true);
            }
            if list {
                let mut s = edit::Session::open(&root)?;
                let path = s.target(target.kind(), &name)?;
                let v = match target.kind() {
                    Kind::Schematic => edit::sch::show(&s, &path),
                    _ => edit::pcb::show(&s, &path),
                };
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                return Ok(true);
            }
            let kind = target.kind();
            let script_arg = |a: &str| {
                a == "-" || (!edit::commands(kind).contains(&a) && Path::new(a).is_file())
            };
            let (text, v, ok) = if args.is_empty() && script_arg(&name) {
                let item = edit::only_item(&root, kind)?;
                edit::run_lines(&root, kind, &item, &script_lines(&name)?)?
            } else if let [script] = args.as_slice()
                && script_arg(script)
            {
                edit::run_lines(&root, kind, &name, &script_lines(script)?)?
            } else if args.is_empty() {
                return Err("give a command: `agentee edit sch ITEM help` lists them".into());
            } else {
                edit::run_one(&root, target.kind(), &name, &args)?
            };
            report(text, &v, json, ok)
        }
        Cmd::Docs => {
            print!("{FORMAT}");
            Ok(true)
        }
    }
}

fn report(text: String, v: &serde_json::Value, json: bool, ok: bool) -> Result<bool, String> {
    if json {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    } else {
        print!("{text}");
    }
    Ok(ok)
}

fn script_lines(source: &str) -> Result<Vec<String>, String> {
    if source == "-" {
        return read_lines(&mut std::io::stdin().lock());
    }
    let file = std::fs::File::open(source).map_err(|e| format!("{source}: {e}"))?;
    read_lines(&mut std::io::BufReader::new(file))
}

fn read_lines(r: &mut dyn std::io::BufRead) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for line in std::io::BufRead::lines(r) {
        lines.push(line.map_err(|e| e.to_string())?);
    }
    Ok(lines)
}

fn edit_help(kind: Kind) -> String {
    let mut text = String::new();
    let stem = match kind {
        Kind::Schematic => "sch",
        Kind::Layout => "pcb",
        _ => "board",
    };
    text += &format!(
        "agentee edit {stem} ITEM COMMAND [args]   (sch = sch:NAME, pcb = pcb:NAME)\n\
         agentee edit {stem} ITEM -   commands on stdin, one per line\n\
         agentee edit {stem} ITEM FILE   commands from a file\n\
         ITEM can be left out of the last two when the project has one {}\n\n",
        match kind {
            Kind::Schematic => "schematic",
            Kind::Layout => "layout",
            _ => "board",
        }
    );
    for cmd in edit::commands(kind) {
        text += &format!("  {}\n", edit::usage(kind, cmd));
    }
    text
}

fn main() -> ExitCode {
    agentee_core::version::set_build(env!("CARGO_PKG_VERSION"), env!("AGENTEE_BUILD_ID"));
    match run(Cli::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
