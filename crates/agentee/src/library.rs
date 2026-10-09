use crate::datasheets::{self, FetchArgs, Want};
use crate::ops;
use agentee_core::project::Project;
use agentee_parts::datasheet::{self, Cache};
use agentee_parts::library::{self, Entry, Library, PartFile, Provenance, Stock};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn open(project: Option<&Path>) -> Library {
    Library::open(project, None)
}

fn kicad_version(lib: &Path) -> Option<String> {
    let text = std::fs::read_to_string(lib).ok()?;
    let head = &text[..text.len().min(400)];
    let at = head.find("(generator_version")?;
    let rest = &head[at + "(generator_version".len()..];
    Some(rest.split(')').next()?.trim().trim_matches('"').to_string())
}

struct Sourced {
    text: String,
    source: String,
    file: String,
}

fn stem(name: &str) -> String {
    agentee_kicad::file_stem(name)
}

fn source_symbol(spec: &str, project: Option<&Project>) -> Result<Sourced, String> {
    let path = Path::new(spec);
    if spec.ends_with(".sym.toml") && path.is_file() {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{spec}: {e}"))?;
        let file = path.file_name().and_then(|f| f.to_str()).unwrap_or(spec).to_string();
        let abs = std::fs::canonicalize(path).unwrap_or(path.to_path_buf());
        return Ok(Sourced { text, source: format!("file {}", abs.display()), file });
    }
    if let Some(p) = project
        && let Some(s) = p.symbols.iter().find(|s| s.name == spec)
    {
        let text =
            std::fs::read_to_string(&s.path).map_err(|e| format!("{}: {e}", s.path.display()))?;
        let file = s.path.file_name().and_then(|f| f.to_str()).unwrap_or("").to_string();
        return Ok(Sourced { text, source: format!("project {}", s.path.display()), file });
    }
    if spec.contains(':') {
        let s = agentee_kicad::import_symbol(spec)?;
        let lib = spec.rsplit_once(':').map(|(l, _)| l).unwrap_or(spec);
        let version = agentee_kicad::symbol_library(lib)
            .ok()
            .and_then(|p| kicad_version(&p).map(|v| format!(", KiCad {v}")))
            .unwrap_or_default();
        let text = format!(
            "# imported from KiCad {spec}\n{}",
            toml::to_string(&s).map_err(|e| e.to_string())?
        );
        return Ok(Sourced {
            text,
            source: format!("kicad {spec}{version}"),
            file: format!("{}.sym.toml", stem(&s.name)),
        });
    }
    Err(format!(
        "no symbol `{spec}`: give a .sym.toml file, a project symbol or a KiCad Library:Name"
    ))
}

fn source_footprint(spec: &str, project: Option<&Project>) -> Result<Sourced, String> {
    let path = Path::new(spec);
    if spec.ends_with(".fp.toml") && path.is_file() {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{spec}: {e}"))?;
        let file = path.file_name().and_then(|f| f.to_str()).unwrap_or(spec).to_string();
        let abs = std::fs::canonicalize(path).unwrap_or(path.to_path_buf());
        return Ok(Sourced { text, source: format!("file {}", abs.display()), file });
    }
    let bare = spec.rsplit(':').next().unwrap_or(spec);
    if let Some(p) = project
        && let Some(f) =
            p.footprints.iter().find(|f| f.name == spec || (!spec.contains(':') && f.name == bare))
    {
        let text =
            std::fs::read_to_string(&f.path).map_err(|e| format!("{}: {e}", f.path.display()))?;
        let file = f.path.file_name().and_then(|x| x.to_str()).unwrap_or("").to_string();
        return Ok(Sourced { text, source: format!("project {}", f.path.display()), file });
    }
    if spec.contains(':') || spec.ends_with(".kicad_mod") {
        let f = agentee_kicad::import_footprint(spec)?;
        let text = format!(
            "# imported from KiCad {spec}\n{}",
            toml::to_string(&f).map_err(|e| e.to_string())?
        );
        return Ok(Sourced {
            text,
            source: format!("kicad {spec}"),
            file: format!("{}.fp.toml", stem(&f.name)),
        });
    }
    Err(format!(
        "no footprint `{spec}`: give a .fp.toml file, a project footprint or a KiCad Library:Name"
    ))
}

fn set_key(text: &str, key: &str, value: &str) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    doc[key] = toml_edit::value(value);
    Ok(doc.to_string())
}

fn get_key(text: &str, key: &str) -> Option<String> {
    let doc: toml_edit::DocumentMut = text.parse().ok()?;
    doc.get(key)?.as_str().map(str::to_string)
}

pub struct AddArgs<'a> {
    pub mpn: &'a str,
    pub name: Option<&'a str>,
    pub role: &'a str,
    pub manufacturer: Option<&'a str>,
    pub symbol: &'a str,
    pub footprint: Option<&'a str>,
    pub model: Option<&'a str>,
    pub datasheet: Option<&'a str>,
    pub value: Option<&'a str>,
    pub rating: Option<&'a str>,
    pub pins: Option<u32>,
    pub tags: &'a [String],
    pub notes: Option<&'a str>,
    pub preferred: bool,
    pub generic: bool,
    pub local: bool,
    pub force: bool,
    pub distributors: bool,
    pub config: Option<&'a Path>,
}

fn target_dir(lib: &Library, local: bool) -> Result<PathBuf, String> {
    let scope = if local { "project" } else { "global" };
    lib.dirs
        .iter()
        .find(|(s, _)| s == scope)
        .map(|(_, d)| d.clone())
        .ok_or_else(|| format!("no {scope} library directory"))
}

pub fn offers_for(
    mpns: &[String],
    config: Option<&Path>,
) -> (Vec<agentee_parts::offer::Offer>, Vec<String>) {
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
        Err(e) => return (Vec::new(), vec![e]),
    };
    let mut offers = Vec::new();
    let mut errors = Vec::new();
    for s in d.list() {
        match s.by_mpn(mpns) {
            Ok(o) => offers.extend(o),
            Err(e) => errors.push(e),
        }
    }
    (offers, errors)
}

pub fn same_part(listed: &str, mpn: &str) -> bool {
    if datasheet::normal(listed) == datasheet::normal(mpn) {
        return true;
    }
    let (l, m) = (listed.to_uppercase(), mpn.trim().to_uppercase());
    l.starts_with(&m) && !l[m.len()..].starts_with(|c: char| c.is_ascii_alphanumeric())
}

fn stock_of(mpn: &str, offers: &[agentee_parts::offer::Offer], today: &str) -> Vec<Stock> {
    let mut v: Vec<Stock> = offers
        .iter()
        .filter(|o| same_part(&o.mpn, mpn))
        .map(|o| Stock::from_offer(o, today))
        .collect();
    v.sort_by(|a, b| a.distributor.cmp(&b.distributor).then(a.sku.cmp(&b.sku)));
    v
}

fn pin_datasheet(name: &str, sha: &str) -> Result<(), String> {
    Cache::open(None)?.pin(sha, &format!("library:{name}"), true)
}

pub fn add(project: Option<&Project>, a: &AddArgs) -> Result<Value, String> {
    let lib = open(project.map(|p| p.root.as_path()));
    let name = stem(a.name.unwrap_or(a.mpn));
    let dir = target_dir(&lib, a.local)?.join(&name);
    if dir.join(library::PART_FILE).exists() && !a.force {
        return Err(format!("{} exists, pass force to replace it", dir.display()));
    }
    let mut notes: Vec<String> = Vec::new();
    let sym = source_symbol(a.symbol, project)?;
    let fp_spec = a.footprint.map(str::to_string).or_else(|| get_key(&sym.text, "footprint"));
    let fp = match fp_spec {
        Some(f) => Some(source_footprint(&f, project)?),
        None => {
            notes.push("no footprint: give --footprint, or the symbol a `footprint`".into());
            None
        }
    };
    let mut sym_text = set_key(&sym.text, "name", &name)?;
    let sym = Sourced { file: format!("{name}.sym.toml"), ..sym };
    let mut fp_text = fp.as_ref().map(|f| f.text.clone());
    if let Some(ft) = &fp_text
        && let Some(fname) = get_key(ft, "name")
    {
        sym_text = set_key(&sym_text, "footprint", &fname)?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files = Vec::new();
    let mut model_rel = None;
    let root = project.map(|p| p.root.clone()).unwrap_or_else(|| PathBuf::from("."));
    let model_spec = a
        .model
        .map(str::to_string)
        .or_else(|| fp_text.as_deref().and_then(|t| get_key(t, "model")));
    if let (Some(m), Some(ft)) = (model_spec, fp_text.as_mut()) {
        let found = if Path::new(&m).is_file() {
            Ok(PathBuf::from(&m))
        } else {
            agentee_3d::locate(&m, &root).map(Ok).unwrap_or_else(|| agentee_3d::fetch(&m))
        };
        match found {
            Ok(path) => {
                let file =
                    path.file_name().and_then(|f| f.to_str()).unwrap_or("model.step").to_string();
                let rel = format!("3dmodels/{file}");
                std::fs::create_dir_all(dir.join("3dmodels")).map_err(|e| e.to_string())?;
                let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                std::fs::write(dir.join(&rel), &bytes).map_err(|e| e.to_string())?;
                *ft = set_key(ft, "model", &rel)?;
                if get_key(ft, "height").is_none()
                    && let Ok(file) = toml::from_str::<agentee_core::footprint::FootprintFile>(ft)
                    && let Ok(mesh) = agentee_3d::load(&path)
                {
                    let f = file.resolve(&mut agentee_core::diag::Diags::new(&file.name));
                    if let Some(h) = agentee_3d::model_height(&f, &mesh) {
                        *ft = set_key(ft, "height", &format!("{h:.2}mm"))?;
                    }
                }
                files.push(Provenance {
                    path: rel.clone(),
                    source: m.clone(),
                    sha256: datasheet::sha256(&bytes),
                });
                model_rel = Some(rel);
            }
            Err(e) => notes.push(format!("model {m} not stored: {e}")),
        }
    }
    std::fs::write(dir.join(&sym.file), &sym_text).map_err(|e| e.to_string())?;
    files.insert(
        0,
        Provenance {
            path: sym.file.clone(),
            source: sym.source.clone(),
            sha256: datasheet::sha256(sym_text.as_bytes()),
        },
    );
    if let (Some(f), Some(t)) = (&fp, &fp_text) {
        std::fs::write(dir.join(&f.file), t).map_err(|e| e.to_string())?;
        files.insert(
            1,
            Provenance {
                path: f.file.clone(),
                source: f.source.clone(),
                sha256: datasheet::sha256(t.as_bytes()),
            },
        );
    }
    let today = datasheet::today();
    let mpn = a.mpn.trim().to_string();
    let (offers, errors) = if a.distributors {
        offers_for(std::slice::from_ref(&mpn), a.config)
    } else {
        (Vec::new(), Vec::new())
    };
    notes.extend(errors.into_iter().map(|e| format!("distributors: {e}")));
    let stock = stock_of(&mpn, &offers, &today);
    let manufacturer = a.manufacturer.map(str::to_string).or_else(|| {
        offers.iter().find(|o| same_part(&o.mpn, &mpn)).map(|o| o.manufacturer.clone())
    });
    let datasheet = match a.datasheet {
        Some(f) if Path::new(f).is_file() => {
            let v = datasheets::add(&mpn, Path::new(f), manufacturer.as_deref(), None, None)?;
            v["sha256"].as_str().map(|sha| library::Datasheet {
                url: None,
                sha256: sha.to_string(),
                revision: v["revision"].as_str().map(str::to_string),
            })
        }
        given => {
            let sym_url = get_key(&sym_text, "datasheet").filter(|u| datasheet::is_url(u));
            let urls: Vec<(String, String)> = given
                .map(|u| ("given".to_string(), u.to_string()))
                .or(sym_url.map(|u| ("symbol".to_string(), u)))
                .into_iter()
                .collect();
            let mut want = vec![Want {
                key: mpn.clone(),
                manufacturer: manufacturer.clone(),
                mpn: Some(mpn.clone()),
                refs: Vec::new(),
                urls,
            }];
            let args = FetchArgs {
                url: given,
                refresh: false,
                config: a.config,
                cache: None,
                distributors: a.distributors,
            };
            let (got, _) = datasheets::fetch_wants(&mut want, &args, None)?;
            match got.into_iter().next() {
                Some(g) if g.sha256.is_some() => Some(library::Datasheet {
                    url: g.url,
                    sha256: g.sha256.unwrap_or_default(),
                    revision: g.revision,
                }),
                Some(g) => {
                    let mut why = vec![g.status];
                    why.extend(g.tried);
                    notes.push(format!(
                        "no datasheet ({}): ask the user for it, then agentee datasheet add {mpn} FILE.pdf and add the part again",
                        why.join("; ")
                    ));
                    None
                }
                None => None,
            }
        }
    };
    if let Some(d) = &datasheet {
        pin_datasheet(&name, &d.sha256)?;
    }
    let part = PartFile {
        role: a.role.trim().to_lowercase(),
        mpn: Some(mpn.clone()),
        manufacturer,
        value: a.value.map(str::to_string),
        rating: a.rating.map(str::to_string),
        pins: a.pins,
        preferred: a.preferred,
        generic: a.generic,
        tags: a.tags.to_vec(),
        notes: a.notes.unwrap_or_default().to_string(),
        symbol: sym.file.clone(),
        footprint: fp.as_ref().map(|f| f.file.clone()),
        model: model_rel,
        added: Some(today),
        datasheet,
        files,
        stock,
    };
    library::write_entry(&dir, &part)?;
    let entry = library::read_entry(&dir, if a.local { "project" } else { "global" })?;
    Ok(json!({ "added": entry, "notes": notes }))
}

pub fn list_text(entries: &[&Entry]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let mut role = String::new();
    for e in entries {
        if e.part.role != role {
            role = e.part.role.clone();
            let _ = writeln!(s, "{role}");
        }
        let _ = writeln!(s, "  {:<24} {}", e.name, e.summary());
        match e.best_stock() {
            Some(st) => {
                let _ = writeln!(s, "  {:<24} {}", "", st.line());
            }
            None if e.part.stock.is_empty() => {
                let _ = writeln!(s, "  {:<24} stock not checked, run agentee library refresh", "");
            }
            None => {
                let _ = writeln!(
                    s,
                    "  {:<24} out of stock everywhere as of {}",
                    "", e.part.stock[0].checked
                );
            }
        }
    }
    if entries.is_empty() {
        s.push_str("no library parts match\n");
    }
    s
}

pub fn show(lib: &Library, name: &str) -> Result<Value, String> {
    let e = lib.get(name).ok_or_else(|| no_part(lib, name))?;
    let cache = Cache::open(None)?;
    let pdf =
        e.part.datasheet.as_ref().and_then(|d| cache.by_hash(&d.sha256)).map(|(en, c)| en.pdf(&c));
    Ok(json!({
        "entry": e,
        "datasheet_file": pdf,
        "stock_lines": e.part.stock.iter().map(Stock::line).collect::<Vec<_>>(),
    }))
}

fn no_part(lib: &Library, name: &str) -> String {
    let names: Vec<&str> = lib.entries.iter().map(|e| e.name.as_str()).collect();
    format!(
        "no library part `{name}`; there is {}",
        if names.is_empty() { "none".to_string() } else { names.join(", ") }
    )
}

fn copy_into(src: &Path, dst: &Path, force: bool, files: &mut Vec<String>) -> Result<(), String> {
    let bytes = std::fs::read(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if let Ok(old) = std::fs::read(dst) {
        if old == bytes {
            files.push(format!("{} (unchanged)", dst.display()));
            return Ok(());
        }
        if !force {
            return Err(format!(
                "{} exists and differs from the library copy, pass force to replace it",
                dst.display()
            ));
        }
    }
    if let Some(d) = dst.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(dst, bytes).map_err(|e| format!("{}: {e}", dst.display()))?;
    files.push(dst.display().to_string());
    Ok(())
}

pub fn use_part(lib: &Library, name: &str, root: &Path, force: bool) -> Result<Value, String> {
    let e = lib.get(name).ok_or_else(|| no_part(lib, name))?;
    let mut files = Vec::new();
    copy_into(
        &e.file(&e.part.symbol),
        &root.join("symbols").join(&e.part.symbol),
        force,
        &mut files,
    )?;
    if let Some(f) = &e.part.footprint {
        copy_into(&e.file(f), &root.join("footprints").join(f), force, &mut files)?;
    }
    if let Some(m) = &e.part.model {
        copy_into(&e.file(m), &root.join(m), force, &mut files)?;
    }
    let mut fields = serde_json::Map::new();
    if let Some(m) = &e.part.mpn {
        fields.insert("mpn".into(), json!(m));
    }
    if let Some(m) = &e.part.manufacturer {
        fields.insert("mfr".into(), json!(m));
    }
    fields.insert("library".into(), json!(e.name));
    let symbol = e.symbol_name.clone().unwrap_or_default();
    let value = e.part.value.clone().or(e.part.mpn.clone()).unwrap_or_default();
    let mut cmd = format!("agentee edit sch SCHEMATIC add REF {symbol} \"{value}\"");
    if let Some(f) = &e.footprint_name {
        cmd += &format!(" --footprint {f}");
    }
    for (k, v) in &fields {
        cmd += &format!(" --field \"{k}={}\"", v.as_str().unwrap_or(""));
    }
    Ok(json!({
        "part": e.name,
        "symbol": symbol,
        "footprint": e.footprint_name,
        "fields": fields,
        "files": files,
        "add_with": cmd,
        "stock": e.part.stock.iter().map(Stock::line).collect::<Vec<_>>(),
    }))
}

pub fn refresh(lib: &Library, name: Option<&str>, config: Option<&Path>) -> Result<Value, String> {
    let targets: Vec<&Entry> = match name {
        Some(n) => vec![lib.get(n).ok_or_else(|| no_part(lib, n))?],
        None => lib.entries.iter().collect(),
    };
    let mut mpns: Vec<String> = targets.iter().filter_map(|e| e.part.mpn.clone()).collect();
    mpns.sort();
    mpns.dedup();
    if mpns.is_empty() {
        return Ok(json!({ "refreshed": [] }));
    }
    let (offers, errors) = offers_for(&mpns, config);
    if offers.is_empty() && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    let today = datasheet::today();
    let mut done = Vec::new();
    for e in targets {
        let Some(mpn) = &e.part.mpn else { continue };
        let fresh = stock_of(mpn, &offers, &today);
        let mut part = e.part.clone();
        let failed: Vec<&str> = errors.iter().map(|x| x.split(':').next().unwrap_or("")).collect();
        part.stock.retain(|s| {
            failed.contains(&s.distributor.as_str())
                && !fresh.iter().any(|f| f.distributor == s.distributor)
        });
        part.stock.extend(fresh);
        part.stock.sort_by(|a, b| a.distributor.cmp(&b.distributor).then(a.sku.cmp(&b.sku)));
        library::write_entry(&e.dir, &part)?;
        done.push(json!({ "part": e.name, "stock": part.stock.iter().map(Stock::line).collect::<Vec<_>>() }));
    }
    Ok(json!({ "refreshed": done, "errors": errors }))
}

#[derive(serde::Serialize)]
struct Finding {
    ok: bool,
    what: String,
}

fn words_in(text: &str) -> std::collections::HashSet<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '#'))
        .filter(|w| !w.is_empty())
        .map(str::to_uppercase)
        .collect()
}

fn package_tokens(fp_name: &str) -> Vec<String> {
    fp_name
        .split('_')
        .filter(|t| {
            t.chars().any(|c| c.is_ascii_alphabetic()) && t.chars().any(|c| c.is_ascii_digit())
        })
        .filter(|t| !t.contains("Metric") && !t.ends_with("mm"))
        .filter(|t| !(t.starts_with('P') && t[1..].starts_with(|c: char| c.is_ascii_digit())))
        .filter(|t| !t.split('x').all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit())))
        .map(|t| t.to_uppercase())
        .collect()
}

fn verify_one(e: &Entry, cache: &Cache) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut push = |ok: bool, what: String| out.push(Finding { ok, what });
    let sym_path = e.file(&e.part.symbol);
    let sym = std::fs::read_to_string(&sym_path)
        .ok()
        .and_then(|t| toml::from_str::<agentee_core::symbol::SymbolFile>(&t).ok())
        .map(|f| f.resolve(&mut agentee_core::diag::Diags::new(&f.name)));
    let Some(sym) = sym else {
        push(false, format!("symbol {} does not load", sym_path.display()));
        return out;
    };
    let mut numbers: Vec<String> = sym.pins.iter().map(|p| p.number.clone()).collect();
    numbers.sort();
    numbers.dedup();
    let fp =
        e.part.footprint.as_ref().and_then(|f| std::fs::read_to_string(e.file(f)).ok()).and_then(
            |t| {
                toml::from_str::<agentee_core::footprint::FootprintFile>(&t)
                    .ok()
                    .map(|f| f.resolve(&mut agentee_core::diag::Diags::new(&f.name)))
            },
        );
    if let Some(fp) = &fp {
        let pads: std::collections::BTreeSet<String> =
            fp.pad_numbers().into_iter().filter(|n| !n.is_empty()).map(str::to_string).collect();
        let missing: Vec<&String> = numbers.iter().filter(|n| !pads.contains(*n)).collect();
        let extra: Vec<&String> = pads.iter().filter(|n| !numbers.contains(n)).collect();
        push(
            missing.is_empty(),
            if missing.is_empty() {
                format!("every symbol pin ({}) has a pad on {}", numbers.len(), fp.name)
            } else {
                format!(
                    "symbol pins with no pad on {}: {}",
                    fp.name,
                    missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ")
                )
            },
        );
        if !extra.is_empty() {
            push(
                extra.iter().all(|n| n.starts_with("MP") || n.chars().all(|c| !c.is_ascii_digit())),
                format!(
                    "pads with no symbol pin: {}",
                    extra.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ")
                ),
            );
        }
    } else if e.part.footprint.is_some() {
        push(false, "footprint does not load".into());
    }
    if let Some(n) = e.part.pins
        && n as usize != numbers.len()
    {
        push(false, format!("part.toml says {n} pins, the symbol has {}", numbers.len()));
    }
    let ds = e.part.datasheet.as_ref().and_then(|d| cache.by_hash(&d.sha256));
    match ds {
        None => push(false, "no pinned datasheet in the cache".into()),
        Some((en, c)) => match datasheet::text(&en.pdf(&c)) {
            Err(err) => push(false, format!("datasheet unreadable: {err}")),
            Ok(pages) => {
                let all = pages.join("\n");
                let words = words_in(&all);
                let flat: String = all
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_uppercase();
                if let Some(m) = &e.part.mpn {
                    let n = datasheet::normal(m);
                    let mut k = n.len();
                    while k > 5 && !flat.contains(&n[..k]) {
                        k -= 1;
                    }
                    let full = flat.contains(&n);
                    push(
                        full || k > 5,
                        if full {
                            format!("the datasheet names {m}")
                        } else if k > 5 {
                            format!(
                                "the datasheet names {} but not the full {m} (ordering suffix?)",
                                &n[..k]
                            )
                        } else {
                            format!("the datasheet never names {m}: is it the right document?")
                        },
                    );
                }
                let named: Vec<&str> = sym
                    .pins
                    .iter()
                    .map(|p| p.name.as_str())
                    .filter(|n| !n.is_empty() && *n != "~" && n.chars().any(|c| c.is_alphabetic()))
                    .filter(|n| {
                        !n.strip_prefix("Pin_")
                            .is_some_and(|d| d.chars().all(|c| c.is_ascii_digit()))
                    })
                    .collect();
                if !named.is_empty() {
                    let mut uniq: Vec<&str> = named.clone();
                    uniq.sort();
                    uniq.dedup();
                    let missing: Vec<&str> = uniq
                        .iter()
                        .copied()
                        .filter(|n| !words.contains(&n.trim_start_matches('~').to_uppercase()))
                        .collect();
                    let found = uniq.len() - missing.len();
                    push(
                        missing.len() * 4 <= uniq.len(),
                        if missing.is_empty() {
                            format!("all {} pin names appear in the datasheet", uniq.len())
                        } else {
                            format!(
                                "{found} of {} pin names appear in the datasheet; not found: {}",
                                uniq.len(),
                                missing.join(" ")
                            )
                        },
                    );
                }
                let n = numbers.len();
                let says = [
                    format!("{n}-PIN"),
                    format!("{n} PIN"),
                    format!("{n}-LEAD"),
                    format!("{n} LEAD"),
                    format!("{n}-POSITION"),
                    format!("{n} POSITION"),
                    format!("{n}-WAY"),
                    format!("{n}P"),
                ]
                .iter()
                .any(|p| all.to_uppercase().contains(p.as_str()));
                if let Some(fp) = &fp {
                    let tokens = package_tokens(&fp.name);
                    let hit = tokens.iter().find(|t| {
                        let flat_t: String =
                            t.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
                        flat_t.len() >= 3 && flat.contains(&flat_t)
                    });
                    push(
                        hit.is_some() || says,
                        match hit {
                            Some(t) => format!("the datasheet names the package {t}"),
                            None if says => format!("the datasheet mentions a {n} pin package"),
                            None => format!(
                                "the datasheet names none of {} and no {n} pin package: check the land pattern by hand",
                                tokens.join(" ")
                            ),
                        },
                    );
                }
            }
        },
    }
    if let (Some(m), Some(fp)) = (&e.part.model, &fp) {
        match agentee_3d::load(&e.file(m)) {
            Err(err) => push(false, format!("model {m}: {err}")),
            Ok(mesh) => {
                let (lo, hi) = mesh.bounds();
                let mw = f64::from(hi[0] - lo[0]) * fp.model_scale[0];
                let mh = f64::from(hi[1] - lo[1]) * fp.model_scale[1];
                let cy = fp.courtyard("F");
                let b = if cy.is_empty() { fp.bounds() } else { cy };
                let (fw, fh) = (b.max[0] - b.min[0], b.max[1] - b.min[1]);
                let fits =
                    |w: f64, h: f64| w <= fw + 0.5 && h <= fh + 0.5 && w * h >= 0.25 * fw * fh;
                let ok = fits(mw, mh) || fits(mh, mw);
                push(
                    ok,
                    format!(
                        "model {:.2} x {:.2} mm against the {} {:.2} x {:.2} mm{}",
                        mw,
                        mh,
                        if cy.is_empty() { "footprint" } else { "courtyard" },
                        fw,
                        fh,
                        if ok { "" } else { ": wrong model or a model_offset/scale problem" }
                    ),
                );
            }
        }
    }
    out
}

pub fn verify(lib: &Library, name: Option<&str>) -> Result<(String, Value, bool), String> {
    use std::fmt::Write as _;
    let cache = Cache::open(None)?;
    let targets: Vec<&Entry> = match name {
        Some(n) => vec![lib.get(n).ok_or_else(|| no_part(lib, n))?],
        None => lib.entries.iter().collect(),
    };
    let mut text = String::new();
    let mut rows = Vec::new();
    let mut all_ok = true;
    for e in targets {
        let mut f = verify_one(e, &cache);
        for p in &e.part.files {
            let now = std::fs::read(e.file(&p.path)).map(|b| datasheet::sha256(&b)).ok();
            if now.as_deref() != Some(p.sha256.as_str()) {
                f.push(Finding {
                    ok: false,
                    what: format!("{} changed since it was added from {}", p.path, p.source),
                });
            }
        }
        let ok = f.iter().all(|x| x.ok);
        all_ok &= ok;
        let _ = writeln!(text, "{} {}", if ok { "ok  " } else { "FAIL" }, e.name);
        for x in &f {
            let _ = writeln!(text, "  {} {}", if x.ok { "ok  " } else { "warn" }, x.what);
        }
        rows.push(json!({ "part": e.name, "ok": ok, "findings": f }));
    }
    Ok((text, json!({ "parts": rows }), all_ok))
}

pub fn apply_checks(p: &mut Project) {
    let lib = open(Some(&p.root));
    if lib.entries.is_empty() {
        return;
    }
    for i in 0..p.schematics.len() {
        if p.schematics[i].item.parent.is_some() {
            continue;
        }
        let found = library::check(&lib, &p.schematics[i].item, &p.schematics[i].path);
        p.schematics[i].diags.extend(found);
    }
}

pub fn datasheet_of(project_root: Option<&Path>, name: &str) -> Option<String> {
    let lib = open(project_root);
    lib.get(name)?.part.datasheet.as_ref().map(|d| d.sha256.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_with_an_ordering_suffix_is_the_same_part() {
        assert!(same_part("B4B-XH-A (LF)(SN)", "B4B-XH-A"));
        assert!(same_part("b4bxha", "B4B-XH-A"));
        assert!(!same_part("B4B-XH-AM", "B4B-XH-A"));
    }

    #[test]
    fn package_words_come_from_the_footprint_name() {
        assert_eq!(package_tokens("JST_PH_S2B-PH-K_1x02_P2.00mm_Horizontal"), ["S2B-PH-K"]);
        assert_eq!(package_tokens("SOT-23-5"), ["SOT-23-5"]);
        assert_eq!(package_tokens("C_0402_1005Metric"), Vec::<String>::new());
    }
}
