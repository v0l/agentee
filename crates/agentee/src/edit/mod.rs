pub mod board;
pub mod pcb;
pub mod sch;

use agentee_core::Severity;
use agentee_core::project::{Kind, Project};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value as TValue};

pub fn tv<T: Into<TValue>>(v: T) -> TValue {
    v.into()
}

#[derive(Default)]
pub struct Report {
    pub log: Vec<String>,
    pub facts: Vec<Value>,
}

impl Report {
    pub fn log(s: impl Into<String>) -> Self {
        Report { log: vec![s.into()], facts: Vec::new() }
    }
}

pub type Cmd = fn(&mut Session, &Path, Opts) -> Result<Report, String>;

const LENGTH_KEYS: &[&str] = &[
    "track_width",
    "clearance",
    "drill",
    "diameter",
    "width",
    "corner_radius",
    "relief_gap",
    "spoke_width",
    "min_width",
    "max_skew",
    "max_uncoupled",
    "neckdown",
    "diff_gap",
    "coplanar_gap",
    "offset",
    "margin",
    "pitch",
];

const BOOL_KEYS: &[&str] =
    &["mirror", "dnp", "locked", "hide", "stacked", "skip", "relief_tht_only"];

const F64_KEYS: &[&str] = &["min_island_area", "cost", "er", "loss_tangent", "rotation"];
const INT_KEYS: &[&str] = &["priority"];

const ENUMS: &[(&str, &[&str])] = &[
    ("style", &["wire", "label", "power"]),
    ("type", &["through", "blind", "buried", "microvia"]),
    (
        "fill",
        &[
            "tented",
            "tented_covered",
            "plugged",
            "plugged_covered",
            "filled",
            "filled_covered",
            "filled_capped",
        ],
    ),
    ("solver", &["formula", "field"]),
    ("side", &["top", "bottom", "f", "b"]),
    ("pad_connection", &["solid", "relief", "none"]),
    ("drill_kind", &["mechanical", "laser", "controlled_depth"]),
];

fn quantity(key: &str) -> Option<fn(&str) -> Result<(), String>> {
    use agentee_core::units::{Amps, Kelvin, Ohms, Percent};
    Some(match key {
        "current" => |s: &str| Amps::parse(s).map(|_| ()),
        "impedance" => |s: &str| Ohms::parse(s).map(|_| ()),
        "max_temp_rise" => |s: &str| Kelvin::parse(s).map(|_| ()),
        "impedance_tolerance" => |s: &str| Percent::parse(s).map(|_| ()),
        _ => return None,
    })
}

pub fn yes_no(tok: &str, what: &str) -> Result<bool, String> {
    match tok.to_ascii_lowercase().as_str() {
        "" | "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err(format!("`{tok}` is not a yes or no for {what}")),
    }
}

pub fn length(tok: &str, what: &str) -> Result<TValue, String> {
    if let Ok(n) = tok.parse::<f64>() {
        return Ok(TValue::from(n));
    }
    Ok(TValue::from(
        agentee_core::units::Length::parse(tok).map_err(|e| format!("{what}: {e}"))?.to_string(),
    ))
}

pub fn point(tok: &str, what: &str) -> Result<TValue, String> {
    let Some((x, y)) = tok.split_once(',') else {
        return Err(format!("{what} is X,Y in mm, like 12.7,20.32"));
    };
    let (x, y) = (length(x.trim(), what)?, length(y.trim(), what)?);
    let mut a = Array::new();
    a.push(x);
    a.push(y);
    Ok(TValue::Array(a))
}

pub fn points(words: &[String], what: &str) -> Result<TValue, String> {
    let mut a = Array::new();
    for w in words {
        a.push(point(w, what)?);
    }
    Ok(tv(a))
}

pub fn value(key: &str, tok: &str) -> Result<TValue, String> {
    if BOOL_KEYS.contains(&key) {
        return Ok(tv(yes_no(tok, key)?));
    }
    for (name, set) in ENUMS {
        if *name == key {
            let low = tok.to_ascii_lowercase();
            if !set.contains(&low.as_str()) {
                return Err(format!("`{tok}` is not {key}: use {}", set.join(", ")));
            }
            return Ok(tv(low));
        }
    }
    if LENGTH_KEYS.contains(&key) {
        return length(tok, key);
    }
    if let Some(parse) = quantity(key) {
        parse(tok).map_err(|e| format!("{key}: {e}"))?;
        return Ok(tv(tok));
    }
    if F64_KEYS.contains(&key) {
        return tok.parse::<f64>().map(tv).map_err(|_| format!("{key}: `{tok}` is not a number"));
    }
    if INT_KEYS.contains(&key) {
        return tok
            .parse::<i64>()
            .map(tv)
            .map_err(|_| format!("{key}: `{tok}` is not a whole number"));
    }
    match tok.to_ascii_lowercase().as_str() {
        "true" => return Ok(tv(true)),
        "false" => return Ok(tv(false)),
        _ => {}
    }
    Ok(tv(tok))
}

pub fn set(t: &mut Table, key: &str, v: impl Into<Item>) {
    t[key] = v.into();
}

pub fn text(t: &Table, key: &str) -> Option<String> {
    t.get(key).and_then(|v| v.as_str()).map(String::from)
}

pub fn texts(t: &Table, key: &str) -> Vec<String> {
    t.get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

pub fn table_point(t: &Table, key: &str) -> Option<[f64; 2]> {
    let a = t.get(key)?.as_array()?;
    let n =
        |i: usize| a.get(i).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)));
    Some([n(0)?, n(1)?])
}

pub fn table_of(item: Option<&Item>) -> Result<Table, String> {
    match item {
        None => Ok(Table::new()),
        Some(Item::Table(t)) => Ok(t.clone()),
        Some(Item::Value(TValue::InlineTable(t))) => Ok(t.clone().into_table()),
        Some(_) => Err("that key is not a table".into()),
    }
}

pub fn insert_into(doc: &mut DocumentMut, key: &str, t: Table) {
    match doc.get_mut(key).and_then(|v| v.as_array_of_tables_mut()) {
        Some(a) => a.push(t),
        None => {
            let mut a = ArrayOfTables::new();
            a.push(t);
            doc[key] = Item::ArrayOfTables(a);
        }
    }
}

pub fn insert(doc: &mut DocumentMut, key: &str, t: Table) {
    insert_into(doc, key, t)
}

pub fn arr<'a>(doc: &'a mut DocumentMut, key: &str) -> Result<&'a mut Array, String> {
    if doc.get(key).is_none() {
        doc[key] = Item::Value(TValue::Array(Array::new()));
    }
    doc.get_mut(key).and_then(|v| v.as_array_mut()).ok_or_else(|| format!("`{key}` is not a list"))
}

pub fn push_new(a: &mut Array, what: &str, words: &[String]) -> Result<(), String> {
    for w in words {
        if a.iter().any(|v| v.as_str() == Some(w.as_str())) {
            return Err(format!("{what} `{w}` is already there"));
        }
        a.push(w.as_str());
    }
    Ok(())
}

pub fn tokens(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut quoted = false;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None if c == '"' || c == '\'' => {
                if !quoted && !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
                quoted = true;
                quote = Some(c);
            }
            None if c.is_whitespace() => {
                if !word.is_empty() || quoted {
                    out.push(std::mem::take(&mut word));
                    quoted = false;
                }
            }
            None if c == '#' => break,
            None => word.push(c),
        }
    }
    if quote.is_some() {
        return Err(format!("unclosed quote in `{line}`"));
    }
    if !word.is_empty() || quoted {
        out.push(word);
    }
    Ok(out)
}

pub struct Opts {
    pub pos: Vec<String>,
    pub flags: BTreeMap<String, String>,
}

impl Opts {
    pub fn split(words: &[String], bools: &'static [&'static str]) -> Result<Self, String> {
        let mut pos = Vec::new();
        let mut flags = BTreeMap::new();
        let mut i = 0;
        while i < words.len() {
            let w = &words[i];
            if let Some(name) = w.strip_prefix("--") {
                if name.is_empty() {
                    return Err("-- wants a name".into());
                }
                if let Some((k, v)) = name.split_once('=') {
                    flags.insert(k.to_string(), v.to_string());
                    i += 1;
                    continue;
                }
                if bools.contains(&name) {
                    flags.insert(name.to_string(), String::new());
                    i += 1;
                    continue;
                }
                let v = words
                    .get(i + 1)
                    .filter(|v| !v.starts_with("--"))
                    .ok_or(format!("--{name} wants a value"))?;
                flags.insert(name.to_string(), v.clone());
                i += 2;
            } else {
                pos.push(w.clone());
                i += 1;
            }
        }
        Ok(Opts { pos, flags })
    }

    pub fn word(&mut self, what: &str) -> Result<String, String> {
        if self.pos.is_empty() {
            return Err(format!("{what} is missing"));
        }
        Ok(self.pos.remove(0))
    }

    pub fn rest(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pos)
    }

    pub fn take(&mut self, key: &str) -> Option<String> {
        self.flags.remove(key)
    }

    pub fn flag(&mut self, key: &str) -> bool {
        self.flags.remove(key).is_some()
    }

    pub fn typed(&mut self, key: &str) -> Result<Option<TValue>, String> {
        match self.flags.remove(key) {
            Some(v) => Ok(Some(value(key, &v)?)),
            None => Ok(None),
        }
    }

    pub fn at(&mut self, key: &str) -> Result<Option<TValue>, String> {
        match self.flags.remove(key) {
            Some(v) => Ok(Some(point(&v, &format!("--{key} X,Y"))?)),
            None => Ok(None),
        }
    }

    pub fn words(&mut self, key: &str) -> Result<Vec<String>, String> {
        Ok(self.flags.remove(key).map(comma).unwrap_or_default())
    }

    pub fn done(&self, cmd: &str, positional: usize) -> Result<(), String> {
        if let Some(k) = self.flags.keys().next() {
            return Err(format!("{cmd}: no `--{k}` here"));
        }
        if self.pos.len() > positional {
            return Err(format!(
                "{cmd}: `{}` is not an argument here, it may want a --flag",
                self.pos[positional]
            ));
        }
        Ok(())
    }
}

pub fn comma(list: String) -> Vec<String> {
    list.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                walk(&p, out);
            }
        } else if Kind::of(&p).is_some() {
            out.push(p);
        }
    }
}

pub struct Session {
    pub root: PathBuf,
    pub project: Project,
    pub docs: BTreeMap<PathBuf, DocumentMut>,
    broken: Vec<String>,
    target: Option<(PathBuf, Kind)>,
}

impl Session {
    pub fn open(root: &Path) -> Result<Self, String> {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let project = Project::load(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        let mut files = Vec::new();
        walk(&root, &mut files);
        let mut docs = BTreeMap::new();
        let mut broken = Vec::new();
        for f in files {
            match std::fs::read_to_string(&f) {
                Ok(text) => match text.parse::<DocumentMut>() {
                    Ok(doc) => {
                        docs.insert(f, doc);
                    }
                    Err(e) => broken.push(format!("{}: {e}", f.display())),
                },
                Err(e) => broken.push(format!("{}: {e}", f.display())),
            }
        }
        Ok(Session { root, project, docs, broken, target: None })
    }

    pub fn item(&self, kind: Kind, name: &str) -> Option<(PathBuf, Kind)> {
        let name = name.rsplit(':').next().unwrap_or(name);
        let stem = |p: &Path| {
            p.file_name().map(|n| n.to_string_lossy().replace(kind.ext(), "")).unwrap_or_default()
        };
        let docs: Vec<(&PathBuf, &DocumentMut)> =
            self.docs.iter().filter(|(p, _)| Kind::of(p) == Some(kind)).collect();
        for (path, doc) in &docs {
            let named = doc.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            if named == name || (named.is_empty() && stem(path) == name) {
                return Some(((*path).clone(), kind));
            }
        }
        docs.iter()
            .find(|(p, _)| stem(p).eq_ignore_ascii_case(name))
            .map(|(p, _)| ((*p).clone(), kind))
    }

    pub fn target(&mut self, kind: Kind, name: &str) -> Result<PathBuf, String> {
        if let Some(found) = self.item(kind, name) {
            self.target = Some(found.clone());
            return Ok(found.0);
        }
        let mut of: Vec<String> = self
            .docs
            .keys()
            .filter(|p| Kind::of(p) == Some(kind))
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().replace(kind.ext(), ""))
                    .unwrap_or_default()
            })
            .collect();
        of.sort();
        of.dedup();
        Err(format!(
            "no {} named `{name}` in {}{}{}",
            kind_name(kind),
            self.root.display(),
            if of.is_empty() { String::new() } else { format!("; there is {}", of.join(", ")) },
            if self.broken.is_empty() {
                String::new()
            } else {
                format!("; unparsable: {}", self.broken.join("; "))
            },
        ))
    }

    pub fn doc(&mut self, path: &Path) -> Result<&mut DocumentMut, String> {
        if let Some(b) = self.broken.iter().find(|b| b.starts_with(&format!("{}:", path.display())))
        {
            return Err(b.clone());
        }
        self.docs.get_mut(path).ok_or_else(|| format!("{} is not in the project", path.display()))
    }

    pub fn read(&self, path: &Path) -> Result<&DocumentMut, String> {
        self.docs.get(path).ok_or_else(|| format!("{} is not in the project", path.display()))
    }

    pub fn name_of(&self, path: &Path) -> String {
        let kind = Kind::of(path).unwrap_or(Kind::Board);
        self.docs
            .get(path)
            .and_then(|d| d.get("name"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                path.file_name()
                    .map(|n| n.to_string_lossy().replace(kind.ext(), ""))
                    .unwrap_or_default()
            })
    }

    pub fn symbol(&self, name: &str) -> Result<&agentee_core::symbol::Symbol, String> {
        fn short(s: &str) -> &str {
            s.rsplit(':').next().unwrap_or(s)
        }
        if let Some(s) = self.project.symbols.iter().find(|s| s.item.name == name) {
            return Ok(&s.item);
        }
        if let Some(s) = self.project.symbols.iter().find(|s| s.item.name == short(name)) {
            return Ok(&s.item);
        }
        let mut names: Vec<&str> =
            self.project.symbols.iter().map(|s| s.item.name.as_str()).collect();
        names.sort();
        Err(format!(
            "no symbol `{name}` in the project; import it with `agentee import symbol Library:NAME` ({} symbols here)",
            names.len()
        ))
    }

    pub fn board(&self, name: Option<&str>) -> Result<&agentee_core::board::Board, String> {
        let boards = &self.project.boards;
        match name {
            Some(n) => boards.iter().find(|b| b.name == n).map(|b| &b.item),
            None => boards.first().map(|b| &b.item),
        }
        .ok_or_else(|| match name {
            Some(n) => format!("no board named `{n}`"),
            None => "this project has no board, so a netclass or a layer cannot be checked".into(),
        })
    }

    pub fn classes(&self, board: Option<&str>) -> Result<Vec<String>, String> {
        Ok(self.board(board)?.netclasses.iter().map(|c| c.name.clone()).collect())
    }

    pub fn check_class(&self, board: Option<&str>, class: &str) -> Result<(), String> {
        let Ok(mut names) = self.classes(board) else {
            return Ok(());
        };
        if !names.iter().any(|c| c == class) {
            names.sort();
            return Err(format!(
                "the board has no netclass `{class}`; it has {}",
                names.join(", ")
            ));
        }
        Ok(())
    }

    pub fn layers(&self, board: Option<&str>) -> Result<Vec<String>, String> {
        Ok(self.board(board)?.stackup.copper_names())
    }

    pub fn check_layer(&self, board: Option<&str>, layer: &str) -> Result<(), String> {
        let names = self.layers(board)?;
        if !names.iter().any(|l| l == layer) {
            return Err(format!("`{layer}` is not a copper layer; it has {}", names.join(", ")));
        }
        Ok(())
    }

    pub fn vias(&self, board: Option<&str>) -> Result<Vec<String>, String> {
        Ok(self.board(board)?.vias.iter().map(|v| v.name.clone()).collect())
    }

    pub fn schematic_board(&self, path: &Path) -> Option<String> {
        self.docs.get(path).and_then(|d| d.get("board")).and_then(|v| v.as_str()).map(String::from)
    }

    fn commit(&mut self) -> Result<Vec<PathBuf>, String> {
        let mut changed = Vec::new();
        for (path, doc) in &self.docs {
            let before = std::fs::read_to_string(path).unwrap_or_default();
            let after = doc.to_string();
            if before != after {
                std::fs::write(path, &after).map_err(|e| format!("{}: {e}", path.display()))?;
                changed.push(path.clone());
            }
        }
        Ok(changed)
    }

    pub fn finish(
        &mut self,
        log: Vec<String>,
        facts: Vec<Value>,
    ) -> Result<(String, Value, bool), String> {
        let target = self.target.clone();
        let changed = self.commit()?;
        let mut refill = Vec::new();
        let stored: Vec<String> = changed
            .iter()
            .filter(|p| Kind::of(p) == Some(Kind::Layout))
            .filter(|p| {
                self.docs
                    .get(p.as_path())
                    .and_then(|d| d.get("fills"))
                    .and_then(|f| f.as_array())
                    .is_some_and(|a| !a.is_empty())
            })
            .map(|p| self.name_of(p))
            .collect();
        for name in stored {
            let p = crate::ops::load(&self.root)?;
            let r = crate::ops::write_fills(&p, &name)?;
            refill.push((name, r));
        }
        let p = crate::ops::load_checked(&self.root)?;
        let touched: BTreeSet<PathBuf> = changed.iter().cloned().collect();
        let of =
            |d: &agentee_core::Diagnostic| d.file.as_ref().is_some_and(|f| touched.contains(f));
        let diags: Vec<&agentee_core::Diagnostic> = p
            .diagnostics()
            .into_iter()
            .filter(|d| d.severity >= Severity::Warning && of(d))
            .collect();
        let elsewhere: Vec<&agentee_core::Diagnostic> = p
            .diagnostics()
            .into_iter()
            .filter(|d| d.severity == Severity::Error && !of(d))
            .collect();
        let ok = diags.iter().all(|d| d.severity != Severity::Error);
        let mut text = String::new();
        for l in &log {
            text += &format!("{l}\n");
        }
        for (name, r) in &refill {
            text += &format!("{name}: refilled {} zones\n", r["fills"]);
        }
        if !diags.is_empty() {
            for d in &diags {
                text += &format!("{d}\n");
            }
            let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
            let warnings = diags.len() - errors;
            text += &format!("{errors} errors, {warnings} warnings in the files this touched\n");
        } else if !changed.is_empty() {
            text += &format!("wrote {} files, nothing flagged in them\n", changed.len());
        } else {
            text.push_str("nothing changed\n");
        }
        if !elsewhere.is_empty() {
            text += &format!(
                "{} errors elsewhere in the project, in files this did not touch\n",
                elsewhere.len()
            );
        }
        let mut out = json!({
            "written": !changed.is_empty(),
            "files": changed.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "applied": log,
            "facts": facts,
            "refilled": refill.iter().map(|(n, r)| json!({ "layout": n, "fills": r["fills"] })).collect::<Vec<_>>(),
            "check": {
                "ok": ok,
                "errors": diags.iter().filter(|d| d.severity == Severity::Error).count(),
                "warnings": diags.len(),
                "errors_elsewhere": elsewhere.len(),
            },
            "diagnostics": diags,
            "elsewhere": elsewhere,
        });
        if let Some((path, kind)) = target {
            out["target"] = json!({
                "kind": kind_name(kind),
                "name": self.name_of(&path),
                "file": path.display().to_string(),
            });
        }
        Ok((text, out, ok))
    }
}

pub fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Board => "board",
        Kind::Symbol => "symbol",
        Kind::Footprint => "footprint",
        Kind::Schematic => "schematic",
        Kind::Layout => "layout",
        Kind::Sim => "sim",
    }
}

fn handler<'a>(kind: Kind, cmd: &str) -> Option<Cmd> {
    match kind {
        Kind::Schematic => sch::command(cmd),
        Kind::Layout => pcb::command(cmd),
        _ => board::command(cmd),
    }
}

fn bools(kind: Kind, cmd: &str) -> &'static [&'static str] {
    match kind {
        Kind::Schematic => sch::bool_flags(cmd),
        Kind::Layout => pcb::bool_flags(cmd),
        _ => board::bool_flags(cmd),
    }
}

pub fn usage(kind: Kind, cmd: &str) -> String {
    match kind {
        Kind::Schematic => sch::usage(cmd).to_string(),
        Kind::Layout => pcb::usage(cmd).to_string(),
        _ => board::usage(cmd).to_string(),
    }
}

pub fn commands(kind: Kind) -> Vec<&'static str> {
    match kind {
        Kind::Schematic => sch::COMMANDS.to_vec(),
        Kind::Layout => pcb::COMMANDS.to_vec(),
        _ => board::COMMANDS.to_vec(),
    }
}

pub fn run_lines(
    root: &Path,
    kind: Kind,
    name: &str,
    lines: &[String],
) -> Result<(String, Value, bool), String> {
    let mut s = Session::open(root)?;
    let path = match name.is_empty() {
        true => {
            let item = only_item(root, kind)?;
            s.target(kind, &item)?
        }
        false => s.target(kind, name)?,
    };
    let mut log = Vec::new();
    let mut facts = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let words = tokens(line)?;
        if words.is_empty() {
            continue;
        }
        let cmd = words[0].clone();
        let Some(handler) = handler(kind, &cmd) else {
            return Err(format!(
                "line {}: no `{cmd}` command for a {}, there is {}",
                n + 1,
                kind_name(kind),
                commands(kind).join(", ")
            ));
        };
        let opts = Opts::split(&words[1..], bools(kind, &cmd))?;
        let report = handler(&mut s, &path, opts)?;
        log.extend(report.log);
        facts.extend(report.facts);
    }
    s.finish(log, facts)
}

pub fn only_item(root: &Path, kind: Kind) -> Result<String, String> {
    let mut s = Session::open(root)?;
    let names: Vec<(PathBuf, String)> = s
        .docs
        .keys()
        .filter(|p| Kind::of(p) == Some(kind))
        .map(|p| {
            let n = s.name_of(p);
            (p.clone(), n)
        })
        .collect();
    match names.as_slice() {
        [(path, name)] => {
            s.target = Some((path.clone(), kind));
            Ok(name.clone())
        }
        [] => Err(format!("the project has no {}", kind_name(kind))),
        many => {
            let mut names: Vec<String> = many.iter().map(|(_, n)| n.clone()).collect();
            names.sort();
            names.dedup();
            Err(format!("the project has {} {}, name one", many.len(), kind_name(kind)))
        }
    }
}

pub fn run_one(
    root: &Path,
    kind: Kind,
    name: &str,
    args: &[String],
) -> Result<(String, Value, bool), String> {
    let mut s = Session::open(root)?;
    let path = s.target(kind, name)?;
    let Some((cmd, args)) = args.split_first() else {
        return Err(format!("give a command: {}", commands(kind).join(", ")));
    };
    let cmd = cmd.as_str();
    let handler = handler(kind, cmd)
        .ok_or_else(|| format!("no `{cmd}` command for a {}", kind_name(kind)))?;
    let opts = Opts::split(args, bools(kind, cmd))?;
    let report = handler(&mut s, &path, opts)?;
    s.finish(report.log, report.facts)
}
