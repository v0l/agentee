use crate::datasheet::normal;
use crate::offer::{Break, Offer};
use agentee_core::diag::{Diagnostic, Diags, Severity};
use agentee_core::schematic::{Part, Schematic};
use agentee_core::symbol::SymbolFile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PART_FILE: &str = "part.toml";
pub const CONFIG_FILE: &str = "library.toml";

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Datasheet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Provenance {
    pub path: String,
    pub source: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Stock {
    pub distributor: String,
    pub sku: String,
    pub stock: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub currency: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breaks: Vec<[f64; 2]>,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub min: u32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub mult: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub checked: String,
}

fn one() -> u32 {
    1
}

fn is_one(n: &u32) -> bool {
    *n == 1
}

impl Stock {
    pub fn from_offer(o: &Offer, checked: &str) -> Stock {
        Stock {
            distributor: o.distributor.clone(),
            sku: o.sku.clone(),
            stock: o.stock,
            currency: o.currency.clone(),
            breaks: o.breaks.iter().map(|b| [f64::from(b.qty), b.price]).collect(),
            min: o.min.max(1),
            mult: o.mult.max(1),
            lifecycle: o.lifecycle.clone(),
            url: o.url.clone(),
            checked: checked.to_string(),
        }
    }

    pub fn offer(&self) -> Offer {
        Offer {
            distributor: self.distributor.clone(),
            sku: self.sku.clone(),
            stock: self.stock,
            currency: self.currency.clone(),
            breaks: self.breaks.iter().map(|b| Break { qty: b[0] as u32, price: b[1] }).collect(),
            min: self.min,
            mult: self.mult,
            lifecycle: self.lifecycle.clone(),
            url: self.url.clone(),
            ..Default::default()
        }
    }

    pub fn unit_at(&self, qty: u32) -> Option<f64> {
        self.offer().cost(qty).map(|c| c.unit)
    }

    pub fn line(&self) -> String {
        let price = self
            .unit_at(1)
            .map(|u| match self.offer().order_qty(1) {
                1 => format!(", {u:.4} {} each", self.currency),
                q => format!(", {u:.4} {} each, min {q}", self.currency),
            })
            .unwrap_or_default();
        let status = self.lifecycle.as_ref().map(|l| format!(", {l}")).unwrap_or_default();
        format!(
            "{} {}: {} in stock{price}{status}, checked {}",
            self.distributor, self.sku, self.stock, self.checked
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PartFile {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lcsc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pins: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preferred: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub generic: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datasheet: Option<Datasheet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<Provenance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stock: Vec<Stock>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub dir: PathBuf,
    pub scope: String,
    #[serde(flatten)]
    pub part: PartFile,
    pub symbol_name: Option<String>,
    pub footprint_name: Option<String>,
    pub pin_count: Option<u32>,
}

impl Entry {
    pub fn file(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    pub fn summary(&self) -> String {
        let mut bits: Vec<String> = Vec::new();
        if let Some(m) = &self.part.mpn {
            bits.push(match &self.part.manufacturer {
                Some(maker) => format!("{maker} {m}"),
                None => m.clone(),
            });
        }
        for v in [&self.part.value, &self.part.rating].into_iter().flatten() {
            bits.push(v.clone());
        }
        if let Some(n) = self.pin_count {
            bits.push(format!("{n} pins"));
        }
        if self.part.preferred {
            bits.push("preferred".into());
        }
        if !self.part.notes.is_empty() {
            bits.push(self.part.notes.clone());
        }
        bits.join(", ")
    }

    pub fn best_stock(&self) -> Option<&Stock> {
        self.part.stock.iter().filter(|s| s.stock > 0 && s.lifecycle.is_none()).min_by(|a, b| {
            a.unit_at(1).unwrap_or(f64::MAX).total_cmp(&b.unit_at(1).unwrap_or(f64::MAX))
        })
    }

    fn words(&self) -> String {
        let p = &self.part;
        let mut t = vec![self.name.clone(), p.role.clone(), p.notes.clone()];
        t.extend(p.tags.iter().cloned());
        t.extend([&p.mpn, &p.manufacturer, &p.value, &p.rating].into_iter().flatten().cloned());
        t.extend(self.footprint_name.iter().cloned());
        if let Some(n) = self.pin_count {
            t.push(format!("{n}-pin {n}pin {n} pins"));
        }
        t.join(" ").to_lowercase()
    }

    pub fn matches(&self, words: &[String]) -> bool {
        let hay = self.words();
        words.iter().all(|w| hay.contains(&w.to_lowercase()))
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Enforce {
    #[default]
    Warning,
    Info,
    Off,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub check: Enforce,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
}

pub fn global_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AGENTEE_LIBRARY") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("agentee").join("library"))
}

pub fn project_dir(root: &Path) -> PathBuf {
    root.join(".agentee").join("library")
}

pub struct Library {
    pub dirs: Vec<(String, PathBuf)>,
    pub entries: Vec<Entry>,
    pub config: Config,
    pub errors: Vec<String>,
}

fn symbol_info(path: &Path) -> Option<(String, u32, Option<String>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let f: SymbolFile = toml::from_str(&text).ok()?;
    let s = f.resolve(&mut Diags::new(&f.name));
    let mut numbers: Vec<&str> = s.pins.iter().map(|p| p.number.as_str()).collect();
    numbers.sort();
    numbers.dedup();
    Some((s.name.clone(), numbers.len() as u32, s.footprint.clone()))
}

fn footprint_name(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: toml::Table = toml::from_str(&text).ok()?;
    v.get("name")?.as_str().map(str::to_string)
}

pub fn read_entry(dir: &Path, scope: &str) -> Result<Entry, String> {
    let path = dir.join(PART_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let part: PartFile = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
    let sym = symbol_info(&dir.join(&part.symbol));
    let footprint_name = part.footprint.as_ref().and_then(|f| footprint_name(&dir.join(f)));
    Ok(Entry {
        name,
        dir: dir.to_path_buf(),
        scope: scope.to_string(),
        pin_count: part.pins.or(sym.as_ref().map(|s| s.1)),
        symbol_name: sym.map(|s| s.0),
        footprint_name,
        part,
    })
}

pub fn write_entry(dir: &Path, part: &PartFile) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(PART_FILE);
    let text = toml::to_string(part).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

impl Library {
    pub fn open(project: Option<&Path>, global: Option<&Path>) -> Library {
        let mut dirs = Vec::new();
        if let Some(g) = global.map(Path::to_path_buf).or_else(global_dir) {
            dirs.push(("global".to_string(), g));
        }
        if let Some(root) = project {
            dirs.push(("project".to_string(), project_dir(root)));
        }
        let mut lib =
            Library { dirs, entries: Vec::new(), config: Config::default(), errors: Vec::new() };
        for (scope, dir) in lib.dirs.clone() {
            if let Ok(text) = std::fs::read_to_string(dir.join(CONFIG_FILE)) {
                match toml::from_str::<Config>(&text) {
                    Ok(c) => lib.config = c,
                    Err(e) => lib.errors.push(format!("{}: {e}", dir.join(CONFIG_FILE).display())),
                }
            }
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut subs: Vec<PathBuf> = rd
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.join(PART_FILE).is_file())
                .collect();
            subs.sort();
            for d in subs {
                match read_entry(&d, &scope) {
                    Ok(e) => {
                        lib.entries.retain(|x| x.name != e.name);
                        lib.entries.push(e);
                    }
                    Err(e) => lib.errors.push(e),
                }
            }
        }
        lib.entries.sort_by(|a, b| {
            a.part
                .role
                .cmp(&b.part.role)
                .then(b.part.preferred.cmp(&a.part.preferred))
                .then(a.name.cmp(&b.name))
        });
        lib
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name).or_else(|| {
            let n = normal(name);
            self.entries.iter().find(|e| e.part.mpn.as_deref().is_some_and(|m| normal(m) == n))
        })
    }

    pub fn filter(&self, role: Option<&str>, words: &[String]) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|e| role.is_none_or(|r| same_role(&e.part.role, r)))
            .filter(|e| e.matches(words))
            .collect()
    }

    pub fn roles(&self) -> Vec<String> {
        let mut v: Vec<String> = self.entries.iter().map(|e| e.part.role.to_lowercase()).collect();
        v.sort();
        v.dedup();
        v
    }
}

pub fn same_role(a: &str, b: &str) -> bool {
    let n = |s: &str| s.trim().trim_end_matches('s').to_lowercase();
    n(a) == n(b)
}

pub fn role_of(p: &Part) -> Option<String> {
    if let Some(r) = p.fields.get("role").filter(|r| !r.trim().is_empty()) {
        return Some(r.trim().to_lowercase());
    }
    let prefix: String = p.reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let fp = p.footprint.as_deref().unwrap_or("").to_lowercase();
    let sym = p.symbol.name.to_lowercase();
    let role = match prefix.to_uppercase().as_str() {
        "J" | "P" | "CN" | "CON" | "X" if connector_kind(&fp, &sym).is_some() => {
            connector_kind(&fp, &sym).unwrap_or_default()
        }
        "J" | "P" | "CN" | "CON" => "connector",
        "R" | "RN" => "resistor",
        "C" => "capacitor",
        "L" => "inductor",
        "FB" => "ferrite",
        "D" if fp.starts_with("led") || sym.starts_with("led") => "led",
        "LED" => "led",
        "D" => "diode",
        "Q" => "transistor",
        "Y" => "crystal",
        "F" => "fuse",
        "SW" | "S" => "switch",
        "BT" => "battery",
        "K" => "relay",
        _ => return None,
    };
    Some(role.into())
}

fn connector_kind(fp: &str, sym: &str) -> Option<&'static str> {
    let has = |words: &[&str]| words.iter().any(|w| fp.contains(w) || sym.contains(w));
    if has(&["coax", "sma", "u.fl", "ufl", "bnc", "mcx", "mmcx", "ipex", "smb_", "n_type"]) {
        Some("rf connector")
    } else if has(&["usb"]) {
        Some("usb connector")
    } else if has(&["rj45", "rj11", "8p8c"]) {
        Some("modular connector")
    } else if has(&["card", "sd_"]) {
        Some("card socket")
    } else {
        None
    }
}

fn pin_count(p: &Part) -> u32 {
    let mut n: Vec<&str> = p.symbol.pins.iter().map(|x| x.number.as_str()).collect();
    n.sort();
    n.dedup();
    n.len() as u32
}

pub fn holds(e: &Entry, p: &Part) -> bool {
    if p.fields.get("library").is_some_and(|l| l == &e.name) {
        return true;
    }
    let full = p.fields.get("mpn").map(|m| m.trim()).unwrap_or("");
    let last = full.rsplit(char::is_whitespace).next().unwrap_or(full);
    if let Some(m) = &e.part.mpn
        && !full.is_empty()
        && (normal(m) == normal(full) || normal(m) == normal(last))
    {
        return true;
    }
    e.part.generic
        && e.footprint_name.is_some()
        && p.footprint.as_deref() == e.footprint_name.as_deref()
}

const VALUED: [&str; 4] = ["resistor", "capacitor", "inductor", "ferrite"];

fn same_value(e: &Entry, p: &Part, role: &str) -> bool {
    if e.part.generic || !VALUED.iter().any(|r| same_role(r, role)) {
        return true;
    }
    let (Some(want), Some(have)) = (crate::spec::si(&p.value), e.part.value.as_deref()) else {
        return true;
    };
    crate::spec::si(have).is_none_or(|v| (v - want).abs() <= want.abs() * 1e-6)
}

pub fn suggest<'a>(lib: &'a Library, p: &Part, role: &str) -> Vec<&'a Entry> {
    let pins = pin_count(p);
    let mut v: Vec<&Entry> = lib
        .entries
        .iter()
        .filter(|e| same_role(&e.part.role, role) && same_value(e, p, role))
        .collect();
    v.sort_by_key(|e| (e.pin_count != Some(pins), !e.part.preferred, e.name.clone()));
    v
}

pub fn check(lib: &Library, sch: &Schematic, path: &Path) -> Vec<Diagnostic> {
    let severity = match lib.config.check {
        Enforce::Off => return Vec::new(),
        Enforce::Info => Severity::Info,
        Enforce::Warning => Severity::Warning,
    };
    let covered = lib.roles();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for p in &sch.parts {
        if !seen.insert(p.reference.as_str())
            || p.dnp
            || p.symbol.power
            || p.reference.starts_with('#')
            || p.fields.get("library").is_some_and(|l| l == "no")
        {
            continue;
        }
        let Some(role) = role_of(p) else { continue };
        if !covered.iter().any(|r| same_role(r, &role)) {
            continue;
        }
        if !lib.config.roles.is_empty() && !lib.config.roles.iter().any(|r| same_role(r, &role)) {
            continue;
        }
        if lib.entries.iter().any(|e| same_role(&e.part.role, &role) && holds(e, p)) {
            continue;
        }
        let options = suggest(lib, p, &role);
        let Some(best) = options.first() else {
            if VALUED.iter().any(|r| same_role(r, &role)) && !p.value.trim().is_empty() {
                out.push(Diagnostic {
                    severity,
                    file: Some(path.to_path_buf()),
                    item: sch.name.clone(),
                    at: p.reference.clone(),
                    message: format!(
                        "{} is a {} {role} and the parts library has no {role} of that value: add one with agentee library add, or set the part's `library = \"no\"` field",
                        p.reference, p.value
                    ),
                    rule: Some("library".into()),
                });
            }
            continue;
        };
        let pins = pin_count(p);
        let message = if best.pin_count.is_some_and(|n| n != pins) {
            format!(
                "{} is a {pins} pin {role} outside the parts library, and no library {role} has {pins} pins: find a current one in stock with agentee library find, add it with agentee library add, or set the part's `library = \"no\"` field",
                p.reference
            )
        } else {
            let others = if options.len() > 1 {
                format!(" ({} more: agentee library list --role \"{role}\")", options.len() - 1)
            } else {
                String::new()
            };
            format!(
                "{} is a {role} outside the parts library: use {} ({}) with agentee library use {}{others}, or set the part's `library = \"no\"` field",
                p.reference,
                best.name,
                best.summary(),
                best.name
            )
        };
        out.push(Diagnostic {
            severity,
            file: Some(path.to_path_buf()),
            item: sch.name.clone(),
            at: p.reference.clone(),
            message,
            rule: Some("library".into()),
        });
    }
    out
}
