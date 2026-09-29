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
    },
    /// Open the viewer, it reloads when files change
    View {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        select: Option<String>,
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
            let p = ops::load(&path)?;
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
            };
            let png = agentee_view::render_png(&p, r, &opts);
            std::fs::write(&out, png).map_err(|e| format!("{}: {e}", out.display()))?;
            println!("{}", out.display());
            Ok(true)
        }
        Cmd::View { path, select } => {
            agentee_view::run(path, select).map_err(|e| e.to_string())?;
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
