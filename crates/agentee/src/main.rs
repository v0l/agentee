mod mcp;
mod ops;
mod templates;

use agentee_core::Severity;
use agentee_core::project::Kind;
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
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
}

#[derive(Clone, Copy, ValueEnum)]
enum LibKind {
    Symbol,
    Footprint,
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
    /// Write a starter board, symbol or footprint
    New {
        kind: NewKind,
        name: String,
        #[arg(short, long)]
        dir: Option<PathBuf>,
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
    /// Run an FDTD simulation (*.sim.toml) on the GPU and save S-parameters
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
        /// Via from the board to use, default the net class via
        #[arg(long)]
        via: Option<String>,
        /// Cost of a via in mm of track
        #[arg(long, default_value_t = 1.0)]
        via_cost: f64,
        /// Route differential pairs as coupled pairs where they fit
        #[arg(long)]
        pairs: bool,
        /// Remove the existing tracks and vias of these nets first and route them again
        #[arg(long)]
        reroute: bool,
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
    /// Trace calculators
    Calc {
        #[command(subcommand)]
        calc: Calc,
    },
    /// Print the file format reference
    Docs,
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
            };
            let png = agentee_view::render_png(&p, r, &opts);
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
        Cmd::New { kind, name, dir } => {
            let (kind, default) = match kind {
                NewKind::Board => (Kind::Board, "."),
                NewKind::Symbol => (Kind::Symbol, "symbols"),
                NewKind::Footprint => (Kind::Footprint, "footprints"),
                NewKind::Schematic => (Kind::Schematic, "."),
                NewKind::Layout => (Kind::Layout, "."),
            };
            let path = ops::new_item(kind, &name, &dir.unwrap_or_else(|| default.into()))?;
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
            let p = ops::load(&path)?;
            print_json(&ops::fetch_models(&p)?);
            Ok(true)
        }
        Cmd::Fab { name, project, out } => {
            let p = ops::load(&project)?;
            print_json(&ops::fab(&p, &name, &out)?);
            Ok(true)
        }
        Cmd::Route {
            name,
            project,
            nets,
            layers,
            grid,
            via,
            via_cost,
            pairs,
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
                pairs,
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
        Cmd::Docs => {
            print!("{FORMAT}");
            Ok(true)
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
