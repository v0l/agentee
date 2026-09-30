use crate::board::{Board, BoardFile, Rules, fab_rules};
use crate::diag::{Diagnostic, Diags, Severity};
use crate::footprint::{Footprint, FootprintFile, natural_cmp};
use crate::graphic::Bounds;
use crate::layout::{Context, Layout, LayoutFile};
use crate::schematic::{Library, Schematic, SchematicFile};
use crate::sim::{Sim, SimFile, SimResult};
use crate::symbol::{Symbol, SymbolFile};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const BOARD_EXT: &str = ".board.toml";
pub const SYMBOL_EXT: &str = ".sym.toml";
pub const FOOTPRINT_EXT: &str = ".fp.toml";
pub const SCHEMATIC_EXT: &str = ".sch.toml";
pub const LAYOUT_EXT: &str = ".pcb.toml";
pub const SIM_EXT: &str = ".sim.toml";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Board,
    Symbol,
    Footprint,
    Schematic,
    Layout,
    Sim,
}

impl Kind {
    pub fn of(path: &Path) -> Option<Kind> {
        let name = path.file_name()?.to_str()?;
        if name.ends_with(BOARD_EXT) {
            Some(Kind::Board)
        } else if name.ends_with(SYMBOL_EXT) {
            Some(Kind::Symbol)
        } else if name.ends_with(FOOTPRINT_EXT) {
            Some(Kind::Footprint)
        } else if name.ends_with(SCHEMATIC_EXT) {
            Some(Kind::Schematic)
        } else if name.ends_with(LAYOUT_EXT) {
            Some(Kind::Layout)
        } else if name.ends_with(SIM_EXT) {
            Some(Kind::Sim)
        } else {
            None
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Kind::Board => BOARD_EXT,
            Kind::Symbol => SYMBOL_EXT,
            Kind::Footprint => FOOTPRINT_EXT,
            Kind::Schematic => SCHEMATIC_EXT,
            Kind::Layout => LAYOUT_EXT,
            Kind::Sim => SIM_EXT,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry<T> {
    pub name: String,
    pub path: PathBuf,
    pub item: T,
    pub diags: Vec<Diagnostic>,
}

impl<T> Entry<T> {
    pub fn worst(&self) -> Option<Severity> {
        self.diags.iter().map(|d| d.severity).max()
    }

    pub fn count(&self, s: Severity) -> usize {
        self.diags.iter().filter(|d| d.severity == s).count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemRef {
    Board(usize),
    Symbol(usize),
    Footprint(usize),
    Schematic(usize),
    Layout(usize),
    Sim(usize),
}

impl ItemRef {
    pub fn kind(self) -> Kind {
        match self {
            ItemRef::Board(_) => Kind::Board,
            ItemRef::Symbol(_) => Kind::Symbol,
            ItemRef::Footprint(_) => Kind::Footprint,
            ItemRef::Schematic(_) => Kind::Schematic,
            ItemRef::Layout(_) => Kind::Layout,
            ItemRef::Sim(_) => Kind::Sim,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Project {
    pub root: PathBuf,
    pub generation: u64,
    pub boards: Vec<Entry<Board>>,
    pub symbols: Vec<Entry<Symbol>>,
    pub footprints: Vec<Entry<Footprint>>,
    pub schematics: Vec<Entry<Schematic>>,
    pub layouts: Vec<Entry<Layout>>,
    pub sims: Vec<Entry<Sim>>,
    pub failures: Vec<Diagnostic>,
}

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                walk(&p, out)?;
            }
        } else if Kind::of(&p).is_some() {
            out.push(p);
        }
    }
    Ok(())
}

fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let before = &src[..offset.min(src.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.len() - before.rfind('\n').map(|i| i + 1).unwrap_or(0) + 1;
    (line, col)
}

pub fn parse<T: serde::de::DeserializeOwned>(src: &str) -> Result<T, (String, String)> {
    toml::from_str(src).map_err(|e| {
        let at =
            e.span().map(|s| line_col(src, s.start)).map(|(l, c)| format!("line {l}, col {c}"));
        (at.unwrap_or_default(), e.message().trim().to_string())
    })
}

fn flatten(
    file: &SchematicFile,
    all: &[(PathBuf, SchematicFile)],
    lib: &Library,
    stack: &mut Vec<String>,
    d: &mut Diags,
) -> (SchematicFile, Vec<crate::schematic::SheetFrame>) {
    if file.sheets.is_empty() {
        return (file.clone(), Vec::new());
    }
    stack.push(file.name.clone());
    let mut sheets = Vec::new();
    for name in &file.sheets {
        if stack.contains(name) {
            d.error("sheets", format!("sheet `{name}` includes itself"));
            continue;
        }
        let Some((_, child)) = all.iter().find(|(_, s)| &s.name == name) else {
            d.error("sheets", format!("no schematic named `{name}`"));
            continue;
        };
        let (child, _) = flatten(child, all, lib, stack, &mut Diags::new(name));
        let bounds = child.resolve(lib, &mut Diags::new(name)).bounds();
        sheets.push((child, bounds));
    }
    stack.pop();
    let refs: Vec<(&SchematicFile, Bounds)> = sheets.iter().map(|(s, b)| (s, *b)).collect();
    file.merge(&refs, d)
}

fn tag(mut d: Diags, path: &Path) -> Vec<Diagnostic> {
    for x in &mut d.list {
        x.file = Some(path.to_path_buf());
    }
    d.list
}

impl Project {
    pub fn load(path: &Path) -> std::io::Result<Project> {
        Self::load_kinds(path, |_| true)
    }

    pub fn load_footprints(path: &Path) -> std::io::Result<Project> {
        Self::load_kinds(path, |k| matches!(k, Kind::Board | Kind::Footprint))
    }

    fn load_kinds(path: &Path, keep: fn(Kind) -> bool) -> std::io::Result<Project> {
        let (root, files) = if path.is_dir() {
            let mut v = Vec::new();
            walk(path, &mut v)?;
            (path.to_path_buf(), v)
        } else {
            if Kind::of(path).is_none() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "{} is not a {BOARD_EXT}, {SYMBOL_EXT} or {FOOTPRINT_EXT} file",
                        path.display()
                    ),
                ));
            }
            (path.parent().unwrap_or(Path::new(".")).to_path_buf(), vec![path.to_path_buf()])
        };
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut p = Project { root, generation, ..Default::default() };
        let mut sym_files = Vec::new();
        let mut fp_files = Vec::new();
        let mut sch_files = Vec::new();
        let mut pcb_files = Vec::new();
        let mut sim_files = Vec::new();
        for f in files.into_iter().filter(|f| Kind::of(f).is_some_and(keep)) {
            let src = match std::fs::read_to_string(&f) {
                Ok(s) => s,
                Err(e) => {
                    p.fail(&f, "", e.to_string());
                    continue;
                }
            };
            match Kind::of(&f) {
                Some(Kind::Board) => match parse::<BoardFile>(&src) {
                    Ok(b) => {
                        let mut d = Diags::new(&b.name);
                        let item = b.resolve(&mut d);
                        item.check(&mut d);
                        p.boards.push(Entry {
                            name: item.name.clone(),
                            diags: tag(d, &f),
                            path: f,
                            item,
                        });
                    }
                    Err((at, msg)) => p.fail(&f, &at, msg),
                },
                Some(Kind::Symbol) => match parse::<SymbolFile>(&src) {
                    Ok(s) => sym_files.push((f, s)),
                    Err((at, msg)) => p.fail(&f, &at, msg),
                },
                Some(Kind::Footprint) => match parse::<FootprintFile>(&src) {
                    Ok(s) => fp_files.push((f, s)),
                    Err((at, msg)) => p.fail(&f, &at, msg),
                },
                Some(Kind::Schematic) => match parse::<SchematicFile>(&src) {
                    Ok(s) => sch_files.push((f, s)),
                    Err((at, msg)) => p.fail(&f, &at, msg),
                },
                Some(Kind::Layout) => match parse::<LayoutFile>(&src) {
                    Ok(s) => pcb_files.push((f, s)),
                    Err((at, msg)) => p.fail(&f, &at, msg),
                },
                Some(Kind::Sim) => match parse::<SimFile>(&src) {
                    Ok(s) => sim_files.push((f, s, crate::sim::hash(&src))),
                    Err((at, msg)) => p.fail(&f, &at, msg),
                },
                None => {}
            }
        }
        let rules = p.rules();
        for (f, file) in fp_files {
            let mut d = Diags::new(&file.name);
            let item = file.resolve(&mut d);
            item.check(&rules, &mut d);
            p.footprints.push(Entry { name: item.name.clone(), diags: tag(d, &f), path: f, item });
        }
        for (f, file) in sym_files {
            let mut d = Diags::new(&file.name);
            let item = file.resolve(&mut d);
            item.check(&mut d);
            p.symbols.push(Entry { name: item.name.clone(), diags: tag(d, &f), path: f, item });
        }
        p.cross_check();
        let sheet_names: std::collections::HashSet<String> =
            sch_files.iter().flat_map(|(_, s)| s.sheets.iter().cloned()).collect();
        for (f, file) in &sch_files {
            let mut d = Diags::new(&file.name);
            let board = p.pick_board(file.board.as_deref(), &mut d);
            let lib = Library {
                symbols: p.symbols.iter().map(|e| (e.name.as_str(), &e.item)).collect(),
                footprints: p.footprints.iter().map(|e| (e.name.as_str(), &e.item)).collect(),
                netclasses: board.map(|b| b.netclasses.iter().map(|n| n.name.clone()).collect()),
            };
            let (whole, frames) = flatten(file, &sch_files, &lib, &mut Vec::new(), &mut d);
            let mut item = whole.resolve(&lib, &mut d);
            item.check_as(&lib, &mut d, sheet_names.contains(&file.name));
            item.sheets = frames;
            item.parent = sch_files
                .iter()
                .find(|(_, s)| s.sheets.contains(&file.name))
                .map(|(_, s)| s.name.clone());
            p.schematics.push(Entry {
                name: item.name.clone(),
                diags: tag(d, f),
                path: f.clone(),
                item,
            });
        }
        for (f, file) in pcb_files {
            let mut d = Diags::new(&file.name);
            let board = p.pick_board(file.board.as_deref(), &mut d).cloned();
            let sch = p.pick_schematic(file.schematic.as_deref(), &mut d).cloned();
            let (Some(board), Some(schematic)) = (board, sch) else {
                p.failures.extend(tag(d, &f));
                continue;
            };
            let cx = Context {
                dir: f.parent().map(Path::to_path_buf).unwrap_or_default(),
                board: &board,
                schematic: &schematic,
                footprints: p.footprints.iter().map(|e| (e.name.as_str(), &e.item)).collect(),
            };
            let item = file.resolve(&cx, &mut d);
            p.layouts.push(Entry { name: item.name.clone(), diags: tag(d, &f), path: f, item });
        }
        let cascade = |f: &SimFile| {
            matches!(
                f.kind,
                Some(
                    crate::sim::SimKind::Cascade
                        | crate::sim::SimKind::Channel
                        | crate::sim::SimKind::Pdn
                )
            )
        };
        sim_files.sort_by_key(|(_, f, _)| cascade(f));
        let layouts_of: Vec<(String, Option<String>)> =
            sim_files.iter().map(|(_, f, _)| (f.name.clone(), f.layout.clone())).collect();
        for (f, file, mut hash) in sim_files {
            let mut d = Diags::new(&file.name);
            if file.kind == Some(crate::sim::SimKind::Logic) {
                match p.load_logic(&f, &file, hash, &mut d) {
                    Some(item) => p.sims.push(Entry {
                        name: item.name.clone(),
                        diags: tag(d, &f),
                        path: f,
                        item,
                    }),
                    None => p.failures.extend(tag(d, &f)),
                }
                continue;
            }
            let own = if cascade(&file) && file.layout.is_none() {
                layouts_of
                    .iter()
                    .find(|(n, _)| Some(n) == file.board.as_ref())
                    .and_then(|(_, l)| l.clone())
            } else {
                file.layout.clone()
            };
            let layout = match &own {
                Some(n) => p.layouts.iter().find(|l| &l.name == n),
                None => p.layouts.first().filter(|_| p.layouts.len() == 1),
            };
            let Some(layout) = layout else {
                d.error("layout", "name the layout to simulate with `layout`");
                p.failures.extend(tag(d, &f));
                continue;
            };
            let copper = layout.item.copper.clone();
            let mut item = file.resolve(&layout.item, &copper, &mut d);
            if file.kind == Some(crate::sim::SimKind::Cascade) {
                hash = p.check_cascade(&f, &item, hash, &mut d);
            }
            if file.kind == Some(crate::sim::SimKind::Channel) {
                hash = p.check_channel(&f, &item, hash, &mut d);
            }
            if file.kind == Some(crate::sim::SimKind::Pdn) {
                hash = p.check_pdn(&f, &item, hash, &mut d);
            }
            let path = crate::sim::result_path(&f);
            let text = std::fs::read_to_string(&path).ok();
            let fdtd = text.as_deref().and_then(|t| serde_json::from_str::<SimResult>(t).ok());
            let maps =
                text.as_deref().and_then(|t| serde_json::from_str::<crate::sim::MapResult>(t).ok());
            let channel = text
                .as_deref()
                .and_then(|t| serde_json::from_str::<crate::sim::ChannelResult>(t).ok());
            let saved_hash = fdtd
                .as_ref()
                .map(|r| r.spec_hash)
                .or(maps.as_ref().map(|r| r.spec_hash))
                .or(channel.as_ref().map(|r| r.spec_hash));
            match saved_hash {
                Some(h) if h == hash => {}
                Some(_) => {
                    d.info(
                        "result",
                        "the spec changed since the last run, the result shown is stale",
                    );
                    item.stale = true;
                }
                None => d.info("result", "not run yet, `agentee sim` runs it"),
            }
            item.copper_then = fdtd
                .as_ref()
                .and_then(|r| r.layout_hash)
                .or(maps.as_ref().and_then(|r| r.layout_hash))
                .or(channel.as_ref().and_then(|r| r.layout_hash));
            item.copper_now = if cascade(&file) {
                p.sims.iter().find(|s| s.name == item.board).and_then(|s| s.item.copper_now)
            } else {
                p.boards
                    .iter()
                    .find(|b| b.name == layout.item.board)
                    .map(|b| crate::sim::copper_hash(&layout.item, &b.item, item.region))
            };
            if item.copper_then.is_some() && item.copper_then != item.copper_now {
                d.info(
                    "result",
                    "the layout changed since the last run, the result shown is stale",
                );
                item.stale = true;
            }
            item.result = fdtd;
            item.maps = maps;
            item.channel = channel;
            p.sims.push(Entry { name: item.name.clone(), diags: tag(d, &f), path: f, item });
        }
        p.measure_interfaces();
        Ok(p)
    }

    fn measure_interfaces(&mut self) {
        let sims: Vec<crate::interface::Measured> = self
            .sims
            .iter()
            .map(|s| crate::interface::Measured {
                name: &s.name,
                stale: s.item.stale,
                untracked: s.item.copper_then.is_none(),
                fdtd: s.item.result.as_ref(),
                channel: s.item.channel.as_ref(),
            })
            .collect();
        let mut found = Vec::new();
        for (i, l) in self.layouts.iter().enumerate() {
            let mut d = Diags::new(l.name.clone());
            for iface in &l.item.interfaces {
                crate::interface::measure(iface, &sims, &mut d);
            }
            found.push((i, tag(d, &l.path)));
        }
        for (i, diags) in found {
            self.layouts[i].diags.extend(diags);
        }
    }

    fn load_logic(&self, path: &Path, file: &SimFile, src: u64, d: &mut Diags) -> Option<Sim> {
        let tops: Vec<&Entry<Schematic>> =
            self.schematics.iter().filter(|s| s.item.parent.is_none()).collect();
        let sch = match &file.schematic {
            Some(n) => self.schematics.iter().find(|s| &s.name == n),
            None => match tops.as_slice() {
                [one] => Some(*one),
                _ => None,
            },
        };
        let Some(sch) = sch else {
            d.error("schematic", "name the schematic to simulate with `schematic`");
            return None;
        };
        let mut item = file.resolve_logic(&sch.item, d);
        let hash = crate::logic::logic_hash(
            src,
            item.logic.as_ref().map(|l| l.netlist_hash).unwrap_or_default(),
        );
        let text = std::fs::read_to_string(crate::sim::result_path(path)).ok();
        let result =
            text.as_deref().and_then(|t| serde_json::from_str::<crate::logic::LogicResult>(t).ok());
        match &result {
            Some(r) if r.spec_hash == hash => {
                for f in &r.failures {
                    d.error("result", f.clone());
                }
            }
            Some(_) => {
                d.info(
                    "result",
                    "the spec or the schematic changed since the last run, the result shown is stale",
                );
                item.stale = true;
            }
            None => d.info("result", "not run yet, `agentee sim` runs it"),
        }
        item.logic_result = result;
        Some(item)
    }

    fn check_pdn(&self, path: &Path, sim: &Sim, hash: u64, d: &mut Diags) -> u64 {
        let Some(board) = self.sims.iter().find(|s| s.name == sim.board) else {
            if !sim.board.is_empty() {
                d.error("board", format!("no sim named `{}`", sim.board));
            }
            return hash;
        };
        let Some(r) = &board.item.result else {
            d.error("board", format!("`{}` has not run yet", sim.board));
            return hash;
        };
        let Some(spec) = &sim.pdn else { return hash };
        let mut texts = Vec::new();
        let mut named: Vec<&String> = spec.sinks.iter().collect();
        named.extend(spec.decaps.iter().map(|c| &c.port));
        if let Some(v) = &spec.vrm {
            named.push(&v.0);
        }
        for n in named {
            if !r.ports.contains(n) {
                d.error(
                    "ports",
                    format!(
                        "`{n}` is not a port of {}, there is {}",
                        sim.board,
                        r.ports.join(", ")
                    ),
                );
            }
        }
        if !r.excited.iter().all(|e| *e) {
            d.error("board", format!("`{}` must drive every port (drop `excite`)", sim.board));
        }
        for c in &spec.decaps {
            if let crate::sim::DecapModel::File { path: f, .. } = &c.model {
                match std::fs::read_to_string(path.parent().unwrap_or(Path::new(".")).join(f)) {
                    Ok(t) => texts.push(t),
                    Err(e) => d.error("decaps", format!("cannot read {f}: {e}")),
                }
            }
        }
        if board.item.stale {
            d.warn("board", format!("the result of `{}` is stale", sim.board));
        }
        crate::sim::cascade_hash(hash, r.spec_hash, &texts)
    }

    fn check_channel(&self, path: &Path, sim: &Sim, hash: u64, d: &mut Diags) -> u64 {
        let mut texts = Vec::new();
        if let Some(spec) = &sim.channel_spec {
            for r in [&spec.tx, &spec.rx].into_iter().flatten() {
                let f = path.parent().unwrap_or(Path::new(".")).join(&r.file);
                match std::fs::read_to_string(&f) {
                    Ok(t) => match crate::ibis::parse(&t) {
                        Ok(ib) if ib.models.iter().any(|m| m.name == r.model) => texts.push(t),
                        Ok(_) => d.error("ibis", format!("{} has no model {}", r.file, r.model)),
                        Err(e) => d.error("ibis", format!("{}: {e}", r.file)),
                    },
                    Err(e) => d.error("ibis", format!("cannot read {}: {e}", r.file)),
                }
            }
        }
        let Some(board) = self.sims.iter().find(|s| s.name == sim.board) else {
            if !sim.board.is_empty() {
                d.error("board", format!("no sim named `{}`", sim.board));
            }
            return hash;
        };
        let Some(r) = &board.item.result else {
            d.error("board", format!("`{}` has not run yet", sim.board));
            return hash;
        };
        if let Some(spec) = &sim.channel_spec {
            for n in &spec.through {
                if !r.ports.contains(n) {
                    d.error(
                        "through",
                        format!(
                            "`{n}` is not a port of {}, there is {}",
                            sim.board,
                            r.ports.join(", ")
                        ),
                    );
                }
            }
            let drive: Vec<&String> = if spec.differential {
                spec.through[..2].iter().collect()
            } else {
                vec![&spec.through[0]]
            };
            let all = spec.differential;
            for n in drive {
                if let Some(k) = r.ports.iter().position(|x| x == n)
                    && !r.excited[k]
                {
                    d.error("board", format!("`{}` did not drive {n}", sim.board));
                }
            }
            if all && !r.excited.iter().all(|e| *e) {
                d.warn("board", "mixed mode is exact only when every port was driven");
            }
        }
        if board.item.stale {
            d.warn("board", format!("the result of `{}` is stale", sim.board));
        }
        crate::sim::cascade_hash(hash, r.spec_hash, &texts)
    }

    fn check_cascade(&self, path: &Path, sim: &Sim, hash: u64, d: &mut Diags) -> u64 {
        let Some(board) = self.sims.iter().find(|s| s.name == sim.board) else {
            if !sim.board.is_empty() {
                d.error("board", format!("no FDTD sim named `{}`", sim.board));
            }
            return hash;
        };
        if board.item.kind != crate::sim::SimKind::Fdtd {
            d.error("board", format!("`{}` is not an FDTD sim", sim.board));
            return hash;
        }
        let names: Vec<&str> = board.item.ports.iter().map(|p| p.name.as_str()).collect();
        let mut used: Vec<&str> = Vec::new();
        let mut texts = Vec::new();
        for (i, dev) in sim.devices.iter().enumerate() {
            let at = format!("devices[{i}] {}", dev.file);
            let file = path.parent().unwrap_or(Path::new(".")).join(&dev.file);
            let n = crate::rf::ports_from_path(&file);
            match std::fs::read_to_string(&file) {
                Err(e) => d.error(&at, format!("cannot read {}: {e}", file.display())),
                Ok(text) => {
                    match n {
                        None => d.error(&at, "name the file .s2p, .s3p and so on"),
                        Some(n) if dev.mount.is_some() && (n != 2 || dev.ports.len() != 1) => d.error(
                            &at,
                            "a mounted part is a .s2p fixture measurement joined to one board port",
                        ),
                        Some(n) if dev.mount.is_none() && n != dev.ports.len() => d.error(
                            &at,
                            format!("the file has {n} ports, `ports` lists {}", dev.ports.len()),
                        ),
                        Some(n) => {
                            if let Err(e) = crate::rf::parse_touchstone(&text, n) {
                                d.error(&at, e);
                            }
                        }
                    }
                    texts.push(text);
                }
            }
            for p in &dev.ports {
                if !names.contains(&p.as_str()) {
                    d.error(&at, format!("`{p}` is not a port of {}", sim.board));
                } else if used.contains(&p.as_str()) {
                    d.error(&at, format!("`{p}` is joined to two devices"));
                }
                used.push(p);
            }
        }
        if used.len() >= names.len() {
            d.error("devices", "every board port is joined to a device, none is left to measure");
        }
        match &board.item.result {
            None => d.error(
                "board",
                format!("`{}` has not run yet, `agentee sim {}` first", sim.board, sim.board),
            ),
            Some(r) => {
                let missing: Vec<&str> = r
                    .ports
                    .iter()
                    .zip(&r.excited)
                    .filter(|(_, e)| !**e)
                    .map(|(n, _)| n.as_str())
                    .collect();
                if !missing.is_empty() {
                    d.error(
                        "board",
                        format!(
                            "`{}` did not excite {}, a cascade needs every port driven (drop `excite`)",
                            sim.board,
                            missing.join(", ")
                        ),
                    );
                }
                if board.item.stale {
                    d.warn("board", format!("the result of `{}` is stale", sim.board));
                }
                return crate::sim::cascade_hash(hash, r.spec_hash, &texts);
            }
        }
        hash
    }

    fn pick_board(&self, name: Option<&str>, d: &mut Diags) -> Option<&Board> {
        match name {
            Some(n) => {
                let b = self.boards.iter().find(|b| b.name == n).map(|b| &b.item);
                if b.is_none() {
                    d.error("board", format!("no board named `{n}`"));
                }
                b
            }
            None => match self.boards.as_slice() {
                [one] => Some(&one.item),
                [] => None,
                _ => {
                    d.error("board", "several boards in the project, name one with `board`");
                    None
                }
            },
        }
    }

    fn pick_schematic(&self, name: Option<&str>, d: &mut Diags) -> Option<&Schematic> {
        let found = match name {
            Some(n) => self.schematics.iter().find(|b| b.name == n).map(|b| &b.item),
            None => match self.schematics.as_slice() {
                [one] => Some(&one.item),
                _ => None,
            },
        };
        if found.is_none() {
            d.error("schematic", "name the schematic this layout places with `schematic`");
        }
        found
    }

    fn fail(&mut self, path: &Path, at: &str, message: String) {
        let item = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.failures.push(Diagnostic {
            severity: Severity::Error,
            file: Some(path.to_path_buf()),
            item,
            at: at.to_string(),
            message,
            rule: None,
        });
    }

    pub fn rules(&self) -> Rules {
        match self.boards.as_slice() {
            [one] => one.item.rules.clone(),
            _ => fab_rules("generic").unwrap(),
        }
    }

    pub fn footprint(&self, name: &str) -> Option<&Entry<Footprint>> {
        let short = name.rsplit(':').next().unwrap_or(name);
        self.footprints.iter().find(|f| f.item.name == name || f.item.name == short)
    }

    fn cross_check(&mut self) {
        let dupes = |names: Vec<&str>| -> Vec<String> {
            let mut seen = std::collections::HashSet::new();
            names.into_iter().filter(|n| !seen.insert(*n)).map(str::to_string).collect()
        };
        for n in dupes(self.symbols.iter().map(|s| s.item.name.as_str()).collect()) {
            for s in self.symbols.iter_mut().filter(|s| s.item.name == n) {
                push(s, Severity::Error, "name", format!("another symbol is also named `{n}`"));
            }
        }
        for n in dupes(self.footprints.iter().map(|s| s.item.name.as_str()).collect()) {
            for s in self.footprints.iter_mut().filter(|s| s.item.name == n) {
                push(s, Severity::Error, "name", format!("another footprint is also named `{n}`"));
            }
        }
        let mut notes = Vec::new();
        for (i, s) in self.symbols.iter().enumerate() {
            let Some(fp_name) = &s.item.footprint else { continue };
            let Some(fp) = self.footprint(fp_name) else {
                notes.push((
                    i,
                    Severity::Warning,
                    format!("footprint `{fp_name}` is not in this project"),
                ));
                continue;
            };
            let pads = fp.item.pad_numbers();
            let mut pins: Vec<&str> = s.item.pins.iter().map(|p| p.number.as_str()).collect();
            pins.sort_by(|a, b| natural_cmp(a, b));
            pins.dedup();
            let missing: Vec<&str> = pins.iter().filter(|n| !pads.contains(n)).copied().collect();
            let unused: Vec<&str> = pads.iter().filter(|n| !pins.contains(n)).copied().collect();
            if !missing.is_empty() {
                notes.push((
                    i,
                    Severity::Error,
                    format!("pins {} have no pad on `{}`", missing.join(", "), fp.item.name),
                ));
            }
            if !unused.is_empty() {
                notes.push((
                    i,
                    Severity::Warning,
                    format!("pads {} on `{}` have no pin", unused.join(", "), fp.item.name),
                ));
            }
        }
        for (i, sev, msg) in notes {
            push(&mut self.symbols[i], sev, "footprint", msg);
        }
    }

    pub fn find(&self, name: &str) -> Option<ItemRef> {
        let (kind, name) = match name.split_once(':') {
            Some((k, n)) => match k {
                "board" => (Some(Kind::Board), n),
                "symbol" | "sym" => (Some(Kind::Symbol), n),
                "footprint" | "fp" => (Some(Kind::Footprint), n),
                "schematic" | "sch" => (Some(Kind::Schematic), n),
                "layout" | "pcb" => (Some(Kind::Layout), n),
                "sim" => (Some(Kind::Sim), n),
                _ => (None, n),
            },
            None => (None, name),
        };
        let refs = self.all_refs();
        let pick = |f: &dyn Fn(&str) -> bool| {
            refs.iter().copied().find(|r| kind.is_none_or(|k| r.kind() == k) && f(self.name_of(*r)))
        };
        pick(&|n| n == name).or_else(|| pick(&|n| n.eq_ignore_ascii_case(name)))
    }

    pub fn name_of(&self, r: ItemRef) -> &str {
        match r {
            ItemRef::Board(i) => &self.boards[i].name,
            ItemRef::Symbol(i) => &self.symbols[i].name,
            ItemRef::Footprint(i) => &self.footprints[i].name,
            ItemRef::Schematic(i) => &self.schematics[i].name,
            ItemRef::Layout(i) => &self.layouts[i].name,
            ItemRef::Sim(i) => &self.sims[i].name,
        }
    }

    pub fn diags_of(&self, r: ItemRef) -> &[Diagnostic] {
        match r {
            ItemRef::Board(i) => &self.boards[i].diags,
            ItemRef::Symbol(i) => &self.symbols[i].diags,
            ItemRef::Footprint(i) => &self.footprints[i].diags,
            ItemRef::Schematic(i) => &self.schematics[i].diags,
            ItemRef::Layout(i) => &self.layouts[i].diags,
            ItemRef::Sim(i) => &self.sims[i].diags,
        }
    }

    pub fn path_of(&self, r: ItemRef) -> &Path {
        match r {
            ItemRef::Board(i) => &self.boards[i].path,
            ItemRef::Symbol(i) => &self.symbols[i].path,
            ItemRef::Footprint(i) => &self.footprints[i].path,
            ItemRef::Schematic(i) => &self.schematics[i].path,
            ItemRef::Layout(i) => &self.layouts[i].path,
            ItemRef::Sim(i) => &self.sims[i].path,
        }
    }

    pub fn all_refs(&self) -> Vec<ItemRef> {
        (0..self.sims.len())
            .map(ItemRef::Sim)
            .chain((0..self.layouts.len()).map(ItemRef::Layout))
            .chain((0..self.schematics.len()).map(ItemRef::Schematic))
            .chain((0..self.boards.len()).map(ItemRef::Board))
            .chain((0..self.symbols.len()).map(ItemRef::Symbol))
            .chain((0..self.footprints.len()).map(ItemRef::Footprint))
            .collect()
    }

    pub fn diagnostics(&self) -> Vec<&Diagnostic> {
        let mut v: Vec<&Diagnostic> = self.failures.iter().collect();
        for r in self.all_refs() {
            v.extend(self.diags_of(r));
        }
        v
    }

    pub fn count(&self, s: Severity) -> usize {
        self.diagnostics().iter().filter(|d| d.severity == s).count()
    }

    pub fn relayout(&mut self, i: usize, text: &str) -> Result<(), String> {
        let path = self.layouts[i].path.clone();
        let file: LayoutFile =
            parse(text).map_err(|(at, m)| format!("{}: {at}: {m}", path.display()))?;
        let layout = &self.layouts[i].item;
        let board =
            self.boards.iter().find(|b| b.name == layout.board).ok_or("board is missing")?;
        let schematic = self
            .schematics
            .iter()
            .find(|s| s.name == layout.schematic)
            .ok_or("the layout's schematic is missing")?;
        let cx = Context {
            dir: path.parent().map(Path::to_path_buf).unwrap_or_default(),
            board: &board.item,
            schematic: &schematic.item,
            footprints: self.footprints.iter().map(|e| (e.name.as_str(), &e.item)).collect(),
        };
        let mut d = Diags::new(&file.name);
        let item = file.resolve(&cx, &mut d);
        self.layouts[i] = Entry { name: item.name.clone(), diags: tag(d, &path), path, item };
        Ok(())
    }
}

fn push<T>(e: &mut Entry<T>, severity: Severity, at: &str, message: String) {
    let item = e.name.clone();
    e.diags.push(Diagnostic {
        severity,
        file: Some(e.path.clone()),
        item,
        at: at.into(),
        message,
        rule: None,
    });
}
