use crate::board::{Board, BoardFile, Rules, fab_rules};
use crate::diag::{Diagnostic, Diags, Severity};
use crate::footprint::{Footprint, FootprintFile, natural_cmp};
use crate::symbol::{Symbol, SymbolFile};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const BOARD_EXT: &str = ".board.toml";
pub const SYMBOL_EXT: &str = ".sym.toml";
pub const FOOTPRINT_EXT: &str = ".fp.toml";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Board,
    Symbol,
    Footprint,
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
        } else {
            None
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Kind::Board => BOARD_EXT,
            Kind::Symbol => SYMBOL_EXT,
            Kind::Footprint => FOOTPRINT_EXT,
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
}

#[derive(Clone, Debug, Default)]
pub struct Project {
    pub root: PathBuf,
    pub boards: Vec<Entry<Board>>,
    pub symbols: Vec<Entry<Symbol>>,
    pub footprints: Vec<Entry<Footprint>>,
    pub failures: Vec<Diagnostic>,
}

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

fn tag(mut d: Diags, path: &Path) -> Vec<Diagnostic> {
    for x in &mut d.list {
        x.file = Some(path.to_path_buf());
    }
    d.list
}

impl Project {
    pub fn load(path: &Path) -> std::io::Result<Project> {
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
        let mut p = Project { root, ..Default::default() };
        let mut sym_files = Vec::new();
        let mut fp_files = Vec::new();
        for f in files {
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
        Ok(p)
    }

    fn fail(&mut self, path: &Path, at: &str, message: String) {
        let item = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.failures.push(Diagnostic {
            severity: Severity::Error,
            file: Some(path.to_path_buf()),
            item,
            at: at.to_string(),
            message,
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
        let exact = |f: &dyn Fn(&str) -> bool| {
            self.boards
                .iter()
                .position(|b| f(&b.item.name))
                .map(ItemRef::Board)
                .or_else(|| self.symbols.iter().position(|s| f(&s.item.name)).map(ItemRef::Symbol))
                .or_else(|| {
                    self.footprints.iter().position(|s| f(&s.item.name)).map(ItemRef::Footprint)
                })
        };
        let short = name.rsplit(':').next().unwrap_or(name);
        exact(&|n| n == name || n == short).or_else(|| exact(&|n| n.eq_ignore_ascii_case(short)))
    }

    pub fn name_of(&self, r: ItemRef) -> &str {
        match r {
            ItemRef::Board(i) => &self.boards[i].item.name,
            ItemRef::Symbol(i) => &self.symbols[i].item.name,
            ItemRef::Footprint(i) => &self.footprints[i].item.name,
        }
    }

    pub fn diags_of(&self, r: ItemRef) -> &[Diagnostic] {
        match r {
            ItemRef::Board(i) => &self.boards[i].diags,
            ItemRef::Symbol(i) => &self.symbols[i].diags,
            ItemRef::Footprint(i) => &self.footprints[i].diags,
        }
    }

    pub fn path_of(&self, r: ItemRef) -> &Path {
        match r {
            ItemRef::Board(i) => &self.boards[i].path,
            ItemRef::Symbol(i) => &self.symbols[i].path,
            ItemRef::Footprint(i) => &self.footprints[i].path,
        }
    }

    pub fn all_refs(&self) -> Vec<ItemRef> {
        (0..self.boards.len())
            .map(ItemRef::Board)
            .chain((0..self.symbols.len()).map(ItemRef::Symbol))
            .chain((0..self.footprints.len()).map(ItemRef::Footprint))
            .collect()
    }

    pub fn diagnostics(&self) -> Vec<&Diagnostic> {
        let mut v: Vec<&Diagnostic> = self.failures.iter().collect();
        v.extend(self.boards.iter().flat_map(|e| &e.diags));
        v.extend(self.footprints.iter().flat_map(|e| &e.diags));
        v.extend(self.symbols.iter().flat_map(|e| &e.diags));
        v
    }

    pub fn count(&self, s: Severity) -> usize {
        self.diagnostics().iter().filter(|d| d.severity == s).count()
    }
}

fn push<T>(e: &mut Entry<T>, severity: Severity, at: &str, message: String) {
    let item = e.name.clone();
    e.diags.push(Diagnostic { severity, file: Some(e.path.clone()), item, at: at.into(), message });
}
