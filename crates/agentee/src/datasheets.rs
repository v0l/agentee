use crate::ops;
use agentee_core::project::{ItemRef, Project};
use agentee_core::schematic::Schematic;
use agentee_core::{Diagnostic, Severity};
use agentee_parts::datasheet::{self, Cache, Copy, Entry, Fetcher, Passage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SIDECAR: &str = "datasheets.toml";

#[derive(Clone, Debug, Default, Serialize)]
pub struct Want {
    pub key: String,
    pub manufacturer: Option<String>,
    pub mpn: Option<String>,
    pub refs: Vec<String>,
    pub urls: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Record {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Sidecar {
    #[serde(default)]
    pub parts: Vec<Record>,
}

impl Sidecar {
    pub fn load(root: &Path) -> Sidecar {
        std::fs::read_to_string(root.join(SIDECAR))
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save(&self, root: &Path) -> Result<PathBuf, String> {
        let path = root.join(SIDECAR);
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    fn merge(&mut self, records: Vec<Record>) {
        self.parts.retain(|p| {
            records.iter().any(|r| r.key == p.key)
                || !p.refs.iter().any(|x| records.iter().any(|r| r.refs.contains(x)))
        });
        for r in records {
            match self.parts.iter_mut().find(|p| p.key == r.key) {
                Some(p) => *p = r,
                None => self.parts.push(r),
            }
        }
        self.parts.sort_by(|a, b| a.key.cmp(&b.key));
    }

    pub fn record(&self, key: &str) -> Option<&Record> {
        let k = datasheet::normal(key);
        self.parts.iter().find(|p| datasheet::normal(&p.key) == k)
    }

    fn for_ref(&self, r: &str) -> Option<&Record> {
        self.parts.iter().find(|p| p.refs.iter().any(|x| x == r))
    }
}

pub fn real_mpn(mpn: &str) -> bool {
    let m = mpn.trim();
    datasheet::normal(m).len() >= 4
        && !m.contains(char::is_whitespace)
        && m.chars().any(|c| c.is_ascii_digit())
}

fn url_of(s: &str) -> Option<String> {
    let s = s.trim();
    (s != "~" && datasheet::is_url(s)).then(|| datasheet::with_scheme(s))
}

fn split_mpn(maker: Option<String>, mpn: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(m) = mpn.map(str::trim) else { return (maker, None) };
    if real_mpn(m) {
        return (maker, Some(m.to_string()));
    }
    match m.rsplit_once(char::is_whitespace) {
        Some((head, last))
            if real_mpn(last)
                && last.len() >= 5
                && last.chars().any(|c| c.is_ascii_alphabetic())
                && !head.contains(',') =>
        {
            (maker.or_else(|| Some(head.trim().to_string())), Some(last.to_string()))
        }
        _ => (maker, None),
    }
}

pub fn wants(sch: &Schematic) -> Vec<Want> {
    let mut out: Vec<Want> = Vec::new();
    for line in agentee_parts::bom(sch) {
        let Some(part) = sch.parts.iter().find(|p| p.reference == line.refs[0]) else {
            continue;
        };
        let (maker, mpn) = split_mpn(line.manufacturer.clone(), line.mpn.as_deref());
        let mut urls = Vec::new();
        if let Some(u) = part.fields.get("datasheet").and_then(|s| url_of(s)) {
            urls.push(("part field".to_string(), u));
        }
        if let Some(u) = url_of(&part.symbol.datasheet) {
            urls.push((format!("symbol {}", part.symbol.name), u));
        }
        let key = match &mpn {
            Some(m) => m.clone(),
            None if !urls.is_empty() => part.symbol.name.clone(),
            None => line.mpn.clone().unwrap_or_else(|| line.value.clone()),
        };
        match out.iter_mut().find(|w| w.key == key) {
            Some(w) => w.refs.extend(line.refs.iter().cloned()),
            None => out.push(Want { key, manufacturer: maker, mpn, refs: line.refs.clone(), urls }),
        }
    }
    out
}

fn design(p: &Project, name: &str) -> Option<usize> {
    let r = ops::find(p, &format!("pcb:{name}"))
        .or_else(|_| ops::find(p, &format!("sch:{name}")))
        .or_else(|_| ops::find(p, name))
        .ok()?;
    match r {
        ItemRef::Layout(i) => {
            p.schematics.iter().position(|s| s.name == p.layouts[i].item.schematic)
        }
        ItemRef::Schematic(i) => Some(i),
        _ => None,
    }
}

fn top_schematics(p: &Project) -> impl Iterator<Item = &Schematic> {
    p.schematics.iter().filter(|s| s.item.parent.is_none()).map(|s| &s.item)
}

pub enum Target {
    Design(Vec<Want>),
    One(Want),
}

pub fn target(p: Option<&Project>, name: &str) -> Target {
    if let Some(p) = p {
        if let Some(i) = design(p, name) {
            return Target::Design(wants(&p.schematics[i].item));
        }
        for sch in top_schematics(p) {
            if let Some(w) = wants(sch).into_iter().find(|w| w.refs.iter().any(|r| r == name)) {
                return Target::One(w);
            }
        }
        for sch in top_schematics(p) {
            if let Some(w) = wants(sch).into_iter().find(|w| {
                datasheet::normal(&w.key) == datasheet::normal(name)
                    || w.mpn
                        .as_deref()
                        .is_some_and(|m| datasheet::normal(m) == datasheet::normal(name))
            }) {
                return Target::One(w);
            }
        }
        if let Some(s) = p.symbols.iter().find(|s| s.name == name) {
            return Target::One(Want {
                key: name.to_string(),
                urls: url_of(&s.item.datasheet)
                    .map(|u| vec![(format!("symbol {name}"), u)])
                    .unwrap_or_default(),
                mpn: real_mpn(name).then(|| name.to_string()),
                ..Default::default()
            });
        }
    }
    let (manufacturer, mpn) = match name.trim().split_once(char::is_whitespace) {
        Some((m, n)) if real_mpn(n) => (Some(m.to_string()), n.trim().to_string()),
        _ => (None, name.trim().to_string()),
    };
    Target::One(Want { key: mpn.clone(), manufacturer, mpn: Some(mpn), ..Default::default() })
}

#[derive(Debug, Serialize)]
pub struct Got {
    pub key: String,
    pub refs: Vec<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tried: Vec<String>,
}

impl Got {
    fn found(w: &Want, status: &str, e: &Entry, c: &Copy) -> Got {
        Got {
            key: w.key.clone(),
            refs: w.refs.clone(),
            status: status.into(),
            path: Some(e.pdf(c)),
            url: c.url.clone(),
            source: Some(c.source.clone()),
            revision: c.revision.clone(),
            sha256: Some(c.sha256.clone()),
            tried: Vec::new(),
        }
    }

    fn record(&self, w: &Want) -> Record {
        let ok = self.sha256.is_some();
        Record {
            key: w.key.clone(),
            manufacturer: w.manufacturer.clone(),
            refs: w.refs.clone(),
            url: self.url.clone(),
            source: self.source.clone().filter(|_| ok),
            sha256: self.sha256.clone(),
            revision: self.revision.clone(),
            fetched: ok.then(datasheet::today),
            missing: (!ok)
                .then(|| self.tried.join("; "))
                .filter(|t| !t.is_empty())
                .or_else(|| (!ok).then(|| self.status.clone())),
        }
    }
}

pub struct FetchArgs<'a> {
    pub url: Option<&'a str>,
    pub refresh: bool,
    pub config: Option<&'a Path>,
    pub cache: Option<&'a Path>,
    pub distributors: bool,
}

#[derive(Debug, Default)]
struct Listing {
    maker: String,
    offers: usize,
    urls: Vec<(String, String)>,
}

type Listed = BTreeMap<String, Vec<Listing>>;

fn maker_key(name: &str) -> String {
    const DROP: &[&str] = &[
        "inc",
        "incorporated",
        "corp",
        "corporation",
        "co",
        "ltd",
        "limited",
        "llc",
        "gmbh",
        "ag",
        "sa",
        "bv",
        "plc",
        "semiconductor",
        "semiconductors",
        "electronics",
        "technology",
        "technologies",
        "international",
        "group",
    ];
    let words: Vec<String> = name
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty() && !DROP.contains(w))
        .map(str::to_string)
        .collect();
    let joined = words.join("");
    match joined.as_str() {
        "ti" | "texasinstruments" | "nationalsemiconductor" => "texasinstruments".into(),
        "st" | "stmicro" | "stmicroelectronics" => "stmicroelectronics".into(),
        "on" | "onsemi" | "onsemiconductor" | "fairchild" => "onsemi".into(),
        "adi" | "analogdevices" | "lineartechnology" | "maxim" | "maximintegrated" => {
            "analogdevices".into()
        }
        "microchip" | "atmel" | "microsemi" => "microchip".into(),
        _ => joined,
    }
}

pub(crate) fn same_maker(a: &str, b: &str) -> bool {
    let (a, b) = (maker_key(a), maker_key(b));
    !a.is_empty() && !b.is_empty() && (a == b || a.contains(&b) || b.contains(&a))
}

fn distributor_urls(wants: &[&Want], config: Option<&Path>) -> (Listed, Vec<String>) {
    let mut out = Listed::new();
    let mpns: Vec<String> = wants.iter().filter_map(|w| w.mpn.clone()).collect();
    if mpns.is_empty() {
        return (out, Vec::new());
    }
    let q = ops::PartsQuery {
        boards: 1,
        alternatives: false,
        distributors: &[],
        farnell_store: None,
        refs: &[],
        config,
    };
    let d = match ops::Distributors::open(&q) {
        Ok(d) => d,
        Err(e) => return (out, vec![e]),
    };
    let mut errors = Vec::new();
    for s in d.list() {
        match s.by_mpn(&mpns) {
            Ok(offers) => {
                for o in offers {
                    let groups = out.entry(datasheet::normal(&o.mpn)).or_default();
                    let k = match groups.iter().position(|g| same_maker(&g.maker, &o.manufacturer))
                    {
                        Some(k) => k,
                        None => {
                            groups.push(Listing {
                                maker: o.manufacturer.clone(),
                                ..Default::default()
                            });
                            groups.len() - 1
                        }
                    };
                    let g = &mut groups[k];
                    g.offers += 1;
                    if let Some(u) = o.datasheet.clone()
                        && !g.urls.iter().any(|(_, x)| *x == u)
                    {
                        g.urls.push((o.distributor.clone(), u));
                    }
                }
            }
            Err(e) => errors.push(e),
        }
    }
    (out, errors)
}

fn listing_for(found: &Listed, mpn: &str) -> Vec<Listing> {
    let want = datasheet::normal(mpn);
    let exact = found.get(&want);
    let makers: Vec<&str> = exact.into_iter().flatten().map(|g| g.maker.as_str()).collect();
    let mut out: Vec<Listing> = exact
        .into_iter()
        .flatten()
        .map(|g| Listing { maker: g.maker.clone(), offers: g.offers, urls: g.urls.clone() })
        .collect();
    if out.iter().any(|g| !g.urls.is_empty()) || want.len() < 5 {
        return out;
    }
    for (k, groups) in found {
        if *k == want || !k.starts_with(&want) {
            continue;
        }
        for g in groups {
            if !makers.is_empty() && !makers.iter().any(|m| same_maker(m, &g.maker)) {
                continue;
            }
            match out.iter_mut().find(|o| same_maker(&o.maker, &g.maker)) {
                Some(o) => o.urls.extend(g.urls.iter().cloned()),
                None => out.push(Listing {
                    maker: g.maker.clone(),
                    offers: g.offers,
                    urls: g.urls.clone(),
                }),
            }
        }
    }
    out
}

fn makers_to_try<'a>(
    mpn: &str,
    listed: &'a [Listing],
    wanted: Option<&str>,
) -> (Vec<&'a Listing>, Option<String>) {
    let with_urls: Vec<&Listing> = listed.iter().filter(|g| !g.urls.is_empty()).collect();
    if let Some(m) = wanted {
        let ours: Vec<&Listing> =
            with_urls.iter().copied().filter(|g| same_maker(&g.maker, m)).collect();
        let note = (ours.is_empty() && !with_urls.is_empty()).then(|| {
            let others: Vec<&str> = with_urls.iter().map(|g| g.maker.as_str()).collect();
            format!(
                "{mpn}: the distributors list a datasheet from {} but not from {m}",
                others.join(", ")
            )
        });
        return (ours, note);
    }
    let mut all = with_urls;
    all.sort_by(|a, b| b.offers.cmp(&a.offers));
    let note = (all.len() > 1).then(|| {
        let names: Vec<String> =
            all.iter().map(|g| format!("{} ({} offers)", g.maker, g.offers)).collect();
        format!(
            "{mpn} is listed by {}; took {}, name the maker to pick another (`datasheet fetch \"MAKER {mpn}\"`, or `mfr` on the part)",
            names.join(", "),
            all[0].maker
        )
    });
    (all, note)
}

fn try_urls(
    cache: &Cache,
    fetcher: &Fetcher,
    w: &Want,
    urls: &[(String, String)],
    tried: &mut Vec<String>,
) -> Option<(Entry, Copy)> {
    let name = w.mpn.as_deref().unwrap_or(&w.key);
    for (source, url) in urls {
        if tried.iter().any(|t| t.starts_with(&format!("{url}:"))) {
            continue;
        }
        match fetcher
            .pdf(url)
            .and_then(|b| cache.store(w.manufacturer.as_deref(), name, &b, Some(url), source))
        {
            Ok(got) => return Some(got),
            Err(e) if e.starts_with(url.as_str()) => tried.push(e),
            Err(e) => tried.push(format!("{url}: {e}")),
        }
    }
    None
}

pub fn cached(cache: &Cache, w: &Want, sidecar: Option<&Sidecar>) -> Option<(Entry, Copy)> {
    if let Some(sha) = sidecar.and_then(|s| s.record(&w.key)).and_then(|r| r.sha256.as_deref())
        && let Some(hit) = cache.by_hash(sha)
    {
        return Some(hit);
    }
    let name = w.mpn.as_deref().or((!w.urls.is_empty()).then_some(w.key.as_str()))?;
    let e = cache.find(name).into_iter().next()?;
    let c = e.current()?.clone();
    Some((e, c))
}

#[derive(Debug, Serialize)]
pub struct Fetched {
    pub parts: Vec<Got>,
    pub errors: Vec<String>,
    pub recorded: Option<PathBuf>,
}

pub fn fetch(project: Option<&Project>, name: &str, a: &FetchArgs) -> Result<Fetched, String> {
    let (mut wants, is_design) = match target(project, name) {
        Target::Design(w) => (w, true),
        Target::One(w) => (vec![w], false),
    };
    if let Some(u) = a.url {
        if wants.len() != 1 {
            return Err("--url takes one part, not a whole design".into());
        }
        wants[0].urls = vec![("given".into(), datasheet::with_scheme(u))];
    }
    let sidecar = project.map(|p| Sidecar::load(&p.root));
    let (out, errors) = fetch_wants(&mut wants, a, sidecar.as_ref())?;
    let written = match (project, is_design || out.iter().any(|g| !g.refs.is_empty())) {
        (Some(p), true) => {
            let mut s = sidecar.unwrap_or_default();
            s.merge(
                wants
                    .iter()
                    .zip(&out)
                    .filter(|(w, g)| g.sha256.is_some() || w.mpn.is_some() || !w.urls.is_empty())
                    .map(|(w, g)| g.record(w))
                    .collect(),
            );
            s.parts.retain(|r| {
                r.sha256.is_some()
                    || !r.missing.as_deref().is_some_and(|m| m.starts_with("no part number"))
            });
            Some(s.save(&p.root)?)
        }
        _ => None,
    };
    Ok(Fetched { parts: out, errors, recorded: written })
}

pub fn fetch_wants(
    wants: &mut [Want],
    a: &FetchArgs,
    sidecar: Option<&Sidecar>,
) -> Result<(Vec<Got>, Vec<String>), String> {
    let cache = Cache::open(a.cache)?;
    let fetcher = Fetcher::default();
    let mut got: Vec<Option<Got>> = Vec::new();
    let mut tried: Vec<Vec<String>> = vec![Vec::new(); wants.len()];
    for (i, w) in wants.iter().enumerate() {
        if !a.refresh
            && a.url.is_none()
            && let Some((e, c)) = cached(&cache, w, sidecar)
        {
            got.push(Some(Got::found(w, "cached", &e, &c)));
            continue;
        }
        got.push(
            try_urls(&cache, &fetcher, w, &w.urls, &mut tried[i])
                .map(|(e, c)| Got::found(w, "fetched", &e, &c)),
        );
    }
    let mut errors = Vec::new();
    let pending: Vec<&Want> = wants
        .iter()
        .zip(&got)
        .filter(|(w, g)| g.is_none() && w.mpn.is_some())
        .map(|(w, _)| w)
        .collect();
    if a.distributors && a.url.is_none() && !pending.is_empty() {
        let (found, errs) = distributor_urls(&pending, a.config);
        errors.extend(errs);
        for (i, w) in wants.iter_mut().enumerate() {
            if got[i].is_some() {
                continue;
            }
            let Some(mpn) = w.mpn.clone() else { continue };
            let listed = listing_for(&found, &mpn);
            let given = w.manufacturer.clone();
            let (makers, note) = makers_to_try(&mpn, &listed, given.as_deref());
            errors.extend(note);
            for g in makers {
                if given.is_none() {
                    w.manufacturer = Some(g.maker.clone());
                }
                got[i] = try_urls(&cache, &fetcher, w, &g.urls, &mut tried[i])
                    .map(|(e, c)| Got::found(w, "fetched", &e, &c));
                if got[i].is_some() {
                    break;
                }
            }
            if got[i].is_none() {
                w.manufacturer = given;
            }
        }
    }
    let out: Vec<Got> = wants
        .iter()
        .zip(got)
        .zip(tried)
        .map(|((w, g), t)| {
            g.unwrap_or_else(|| Got {
                key: w.key.clone(),
                refs: w.refs.clone(),
                status: if w.mpn.is_none() && w.urls.is_empty() {
                    "no part number and no datasheet URL".into()
                } else if t.is_empty() {
                    "no datasheet URL found".into()
                } else {
                    "unreachable".into()
                },
                path: None,
                url: None,
                source: None,
                revision: None,
                sha256: None,
                tried: t,
            })
        })
        .collect();
    Ok((out, errors))
}

pub fn fetch_text(f: &Fetched) -> String {
    use std::fmt::Write as _;
    let (got, errors, written) = (&f.parts, &f.errors, f.recorded.as_deref());
    let mut s = String::new();
    let (ok, missing): (Vec<&Got>, Vec<&Got>) = got.iter().partition(|g| g.path.is_some());
    for g in &ok {
        let refs =
            if g.refs.is_empty() { String::new() } else { format!(" ({})", g.refs.join(" ")) };
        let _ = writeln!(
            s,
            "{:<8} {}{}  {}{}",
            g.status,
            g.key,
            refs,
            g.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            g.revision.as_ref().map(|r| format!("  [{r}]")).unwrap_or_default()
        );
    }
    let (bare, missing): (Vec<&Got>, Vec<&Got>) =
        missing.into_iter().partition(|g| g.status.starts_with("no part number"));
    if !missing.is_empty() {
        let _ = writeln!(s, "\nno datasheet, ask the user for these:");
        for g in &missing {
            let _ = writeln!(s, "  {} ({}): {}", g.key, g.refs.join(" "), g.status);
            for t in &g.tried {
                let _ = writeln!(s, "    tried {t}");
            }
        }
    }
    if !bare.is_empty() {
        let refs: Vec<&str> = bare.iter().flat_map(|g| g.refs.iter().map(String::as_str)).collect();
        let _ = writeln!(
            s,
            "\nno part number and no datasheet URL, give them an `mpn` field to fetch one: {}",
            refs.join(" ")
        );
    }
    for e in errors {
        let _ = writeln!(s, "distributors: {e}");
    }
    if let Some(w) = written {
        let _ = writeln!(s, "recorded in {}", w.display());
    }
    let _ = writeln!(
        s,
        "{} cached, {} unreachable, {} without a part number",
        ok.len(),
        missing.len(),
        bare.len()
    );
    s
}

pub struct TextArgs<'a> {
    pub find: Option<&'a str>,
    pub pages: Option<&'a str>,
    pub context: usize,
    pub cache: Option<&'a Path>,
    pub config: Option<&'a Path>,
    pub offline: bool,
}

fn page_range(spec: &str, count: usize) -> Result<(usize, usize), String> {
    let bad = || format!("pages `{spec}`: give N or N-M");
    let (a, b) = match spec.split_once('-') {
        Some((a, b)) => {
            (a.trim().parse().map_err(|_| bad())?, b.trim().parse().map_err(|_| bad())?)
        }
        None => {
            let n = spec.trim().parse().map_err(|_| bad())?;
            (n, n)
        }
    };
    if a == 0 || a > b {
        return Err(bad());
    }
    Ok((a, b.min(count)))
}

pub fn locate(
    project: Option<&Project>,
    name: &str,
    a: &TextArgs,
) -> Result<(Want, Entry, Copy), String> {
    let cache = Cache::open(a.cache)?;
    if let Some(sha) = crate::library::datasheet_of(project.map(|p| p.root.as_path()), name)
        && let Some((e, c)) = cache.by_hash(&sha)
    {
        return Ok((Want { key: name.to_string(), ..Default::default() }, e, c));
    }
    let w = match target(project, name) {
        Target::One(w) => w,
        Target::Design(_) => {
            return Err(format!("`{name}` is a design; give one part, reference or part number"));
        }
    };
    let sidecar = project.map(|p| Sidecar::load(&p.root));
    let by_ref = sidecar
        .as_ref()
        .and_then(|s| s.for_ref(name))
        .and_then(|r| r.sha256.as_deref())
        .and_then(|sha| cache.by_hash(sha));
    if let Some((e, c)) = by_ref.or_else(|| cached(&cache, &w, sidecar.as_ref())) {
        return Ok((w, e, c));
    }
    if a.offline {
        return Err(format!(
            "no cached datasheet for `{name}`: run agentee datasheet fetch {name}, or add one with agentee datasheet add"
        ));
    }
    let args = FetchArgs {
        url: None,
        refresh: false,
        config: a.config,
        cache: a.cache,
        distributors: true,
    };
    let Fetched { parts: got, errors, .. } = fetch(project, name, &args)?;
    let g = got.first().ok_or("nothing to fetch")?;
    let Some(sha) = &g.sha256 else {
        let mut why = vec![g.status.clone()];
        why.extend(g.tried.iter().cloned());
        why.extend(errors);
        return Err(format!(
            "no datasheet for `{name}`: {}. Ask the user for it and add it with agentee datasheet add {} FILE.pdf",
            why.join("; "),
            w.mpn.as_deref().unwrap_or(&w.key)
        ));
    };
    let (e, c) = cache.by_hash(sha).ok_or("the fetched copy is missing from the cache")?;
    Ok((w, e, c))
}

pub fn text(project: Option<&Project>, name: &str, a: &TextArgs) -> Result<Value, String> {
    let (w, e, c) = locate(project, name, a)?;
    let pdf = e.pdf(&c);
    let pages = datasheet::text(&pdf)?;
    let head = json!({
        "part": w.key,
        "refs": w.refs,
        "mpn": e.meta.mpn,
        "manufacturer": e.meta.manufacturer,
        "file": pdf,
        "url": c.url,
        "revision": c.revision,
        "sha256": c.sha256,
        "pages": pages.len(),
    });
    let mut v = head;
    if let Some(q) = a.find {
        let mut hits: Vec<Passage> = datasheet::find(&pages, q, a.context);
        if let Some(r) = a.pages {
            let (lo, hi) = page_range(r, pages.len())?;
            hits.retain(|h| (lo..=hi).contains(&h.page));
        }
        v["find"] = json!(q);
        v["passages"] = json!(hits);
    } else {
        let (lo, hi) = match a.pages {
            Some(r) => page_range(r, pages.len())?,
            None => (1, pages.len()),
        };
        v["text"] = json!(
            pages
                .iter()
                .enumerate()
                .skip(lo - 1)
                .take(hi + 1 - lo)
                .map(|(i, t)| json!({ "page": i + 1, "text": t }))
                .collect::<Vec<_>>()
        );
    }
    Ok(v)
}

pub fn text_render(v: &Value) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{} {}, {} pages, {}{}",
        v["manufacturer"].as_str().unwrap_or(""),
        v["mpn"].as_str().unwrap_or(""),
        v["pages"],
        v["file"].as_str().unwrap_or(""),
        v["revision"].as_str().map(|r| format!(" [{r}]")).unwrap_or_default()
    );
    if let Some(ps) = v["passages"].as_array() {
        if ps.is_empty() {
            let _ = writeln!(s, "no match for `{}`", v["find"].as_str().unwrap_or(""));
        }
        for p in ps {
            let _ = writeln!(s, "\n--- page {}, line {}", p["page"], p["line"]);
            let _ = writeln!(s, "{}", p["text"].as_str().unwrap_or(""));
        }
    }
    if let Some(ps) = v["text"].as_array() {
        for p in ps {
            let _ = writeln!(s, "\n--- page {}", p["page"]);
            let _ = write!(s, "{}", p["text"].as_str().unwrap_or(""));
        }
    }
    s
}

pub fn add(
    mpn: &str,
    file: &Path,
    manufacturer: Option<&str>,
    url: Option<&str>,
    cache: Option<&Path>,
) -> Result<Value, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let cache = Cache::open(cache)?;
    let (e, c) = cache
        .store(manufacturer, mpn, &bytes, url, "user")
        .map_err(|x| format!("{}: {x}", file.display()))?;
    Ok(json!({ "mpn": e.meta.mpn, "file": e.pdf(&c), "sha256": c.sha256, "revision": c.revision }))
}

pub fn list(cache: Option<&Path>) -> Result<Value, String> {
    let cache = Cache::open(cache)?;
    let rows: Vec<Value> = cache
        .entries()
        .iter()
        .flat_map(|e| {
            e.meta.copies.iter().map(move |c| {
                json!({
                    "manufacturer": e.meta.manufacturer,
                    "mpn": e.meta.mpn,
                    "file": e.pdf(c),
                    "sha256": c.sha256,
                    "source": c.source,
                    "url": c.url,
                    "fetched": c.fetched,
                    "revision": c.revision,
                    "pinned": c.pinned,
                })
            })
        })
        .collect();
    Ok(json!({ "cache": cache.dir, "datasheets": rows }))
}

pub fn purge(cache: Option<&Path>) -> Result<Value, String> {
    let cache = Cache::open(cache)?;
    let r = cache.purge()?;
    Ok(
        json!({ "cache": cache.dir, "removed": r.removed, "kept_pinned": r.kept, "bytes_freed": r.bytes }),
    )
}

pub fn apply_checks(p: &mut Project) {
    let sidecar = Sidecar::load(&p.root);
    let mut seen = std::collections::HashSet::new();
    let mut order: Vec<usize> = (0..p.schematics.len()).collect();
    order.sort_by_key(|&i| {
        (p.schematics[i].item.parent.is_some(), std::cmp::Reverse(p.schematics[i].item.parts.len()))
    });
    for i in order {
        let entry = &p.schematics[i];
        let mut found: Vec<Diagnostic> = Vec::new();
        for w in wants(&entry.item) {
            let Some(mpn) = &w.mpn else { continue };
            if !seen.insert(datasheet::normal(mpn)) {
                continue;
            }
            let refs = w.refs.join(" ");
            let record = sidecar.record(&w.key);
            let message = match record {
                Some(r) if r.sha256.is_none() => Some(format!(
                    "{refs} ({mpn}): no datasheet could be fetched ({}); ask the user for it and add it with agentee datasheet add",
                    r.missing.as_deref().unwrap_or("unreachable")
                )),
                Some(_) => None,
                None if w.urls.is_empty() => Some(format!(
                    "{refs} ({mpn}) has no datasheet: set the symbol's `datasheet` URL, or run agentee datasheet fetch {}",
                    entry.name
                )),
                None => None,
            };
            if let Some(message) = message {
                found.push(Diagnostic {
                    severity: Severity::Warning,
                    file: Some(entry.path.clone()),
                    item: entry.name.clone(),
                    at: w.refs.first().cloned().unwrap_or_default(),
                    message,
                    rule: Some("datasheet".into()),
                });
            }
        }
        p.schematics[i].diags.extend(found);
    }
}

pub fn project_for(path: &Path) -> Option<Project> {
    ops::load(path).ok().filter(|p| !p.all_refs().is_empty())
}

#[cfg(test)]
mod tests {

    #[test]
    fn makers_match_across_spellings() {
        assert!(same_maker("Texas Instruments", "TI"));
        assert!(same_maker("STMicroelectronics", "ST Microelectronics"));
        assert!(same_maker("ON Semiconductor", "onsemi"));
        assert!(same_maker("Comchip Technology", "COMCHIP"));
        assert!(!same_maker("onsemi", "Comchip Technology"));
        assert!(!same_maker("Texas Instruments", "STMicroelectronics"));
    }

    #[test]
    fn a_longer_part_number_lends_a_datasheet_only_from_the_same_maker() {
        let g = |maker: &str, url: Option<&str>| Listing {
            maker: maker.into(),
            offers: 1,
            urls: url.map(|u| ("Farnell".to_string(), u.to_string())).into_iter().collect(),
        };
        let mut found = Listed::new();
        found.insert("LM358P".into(), vec![g("Texas Instruments", None)]);
        found.insert("LM358PT".into(), vec![g("STMICROELECTRONICS", Some("st.pdf"))]);
        assert!(listing_for(&found, "LM358P").iter().all(|l| l.urls.is_empty()));
        found.insert("LM358PE4".into(), vec![g("TEXAS INSTRUMENTS", Some("ti.pdf"))]);
        let l = listing_for(&found, "LM358P");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].urls[0].1, "ti.pdf");
        found.remove("LM358P");
        assert_eq!(listing_for(&found, "LM358P").len(), 2);
    }

    #[test]
    fn a_named_maker_takes_only_its_own_datasheets() {
        let listed = vec![
            Listing {
                maker: "STMicroelectronics".into(),
                offers: 3,
                urls: vec![("Farnell".into(), "st.pdf".into())],
            },
            Listing {
                maker: "Texas Instruments".into(),
                offers: 2,
                urls: vec![("Mouser".into(), "ti.pdf".into())],
            },
        ];
        let (picked, note) = makers_to_try("LM358P", &listed, Some("TI"));
        assert_eq!(picked.iter().map(|g| g.urls[0].1.as_str()).collect::<Vec<_>>(), ["ti.pdf"]);
        assert!(note.is_none());
        let (picked, note) = makers_to_try("LM358P", &listed, None);
        assert_eq!(picked[0].maker, "STMicroelectronics");
        assert!(note.unwrap().contains("Texas Instruments (2 offers)"));
        let (picked, note) = makers_to_try("LM358P", &listed, Some("onsemi"));
        assert!(picked.is_empty());
        assert!(note.unwrap().contains("not from onsemi"));
    }

    use super::*;

    #[test]
    fn a_maker_and_part_number_split_but_a_description_does_not() {
        let split = |m: &str| split_mpn(None, Some(m));
        assert_eq!(split("TPS7A9101DSKR"), (None, Some("TPS7A9101DSKR".into())));
        assert_eq!(
            split("Analog Devices AD9364BBCZ"),
            (Some("Analog Devices".into()), Some("AD9364BBCZ".into()))
        );
        assert_eq!(split("10 uF X5R 10 V 0402"), (None, None));
        assert_eq!(split("Abracon ABM8-24.000MHZ-10-1-U-T, 10 pF"), (None, None));
        assert_eq!(split("C0G"), (None, None));
    }

    #[test]
    fn check_warns_for_a_part_number_without_a_datasheet() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logic");
        let mut p = ops::load(&root).unwrap();
        apply_checks(&mut p);
        let refs: Vec<String> = p
            .diagnostics()
            .into_iter()
            .filter(|d| d.rule.as_deref() == Some("datasheet"))
            .map(|d| d.at.clone())
            .collect();
        assert_eq!(refs, ["U1", "U2"]);
    }
}
