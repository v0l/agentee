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

fn find_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for e in read.flatten() {
        let p = e.path();
        if p.is_dir() {
            find_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn unzip(zip: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let dir =
        std::env::temp_dir().join(format!("agentee-model-{}", &datasheet::sha256(&bytes)[..16]));
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("{}: {e}", zip.display()))?;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = f.enclosed_name() else { continue };
        if f.is_dir() {
            continue;
        }
        let out = dir.join(name);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut w = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut f, &mut w).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

enum Vendor {
    KiCad(PathBuf),
    Placed(Box<agentee_core::footprint::Footprint>, String),
}

struct Model {
    path: PathBuf,
    source: String,
    vendor: Option<Vendor>,
    note: Option<String>,
}

fn local_model(m: &str, root: &Path) -> Result<Model, String> {
    if let Some((path, kicad)) = vendor_bundle(m)? {
        return Ok(Model {
            path,
            source: m.to_string(),
            vendor: kicad.map(Vendor::KiCad),
            note: None,
        });
    }
    if Path::new(m).is_file() {
        return Ok(Model {
            path: PathBuf::from(m),
            source: m.to_string(),
            vendor: None,
            note: None,
        });
    }
    let (path, near) = agentee_3d::resolve(m, root)?;
    let note = near.as_ref().map(|n| {
        format!(
            "KiCad has no model {}; stored the nearest body {n}, check it with agentee library verify",
            agentee_3d::relative(m)
        )
    });
    Ok(Model { path, source: near.unwrap_or_else(|| m.to_string()), vendor: None, note })
}

fn vendor_bundle(spec: &str) -> Result<Option<(PathBuf, Option<PathBuf>)>, String> {
    let path = Path::new(spec);
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else if path.is_file() && spec.to_ascii_lowercase().ends_with(".zip") {
        unzip(path)?
    } else {
        return Ok(None);
    };
    let mut files = Vec::new();
    find_files(&dir, &mut files);
    files.sort();
    let ext = |p: &PathBuf| {
        p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default()
    };
    let model = ["step", "stp", "wrl"]
        .iter()
        .find_map(|want| files.iter().find(|p| ext(p) == *want))
        .cloned()
        .ok_or(format!("{spec} holds no STEP or VRML model"))?;
    let footprint = files.iter().find(|p| ext(p) == "kicad_mod").cloned();
    Ok(Some((model, footprint)))
}

fn fit_to(ft: &str, vendor: &Path) -> Result<(String, String), String> {
    let theirs = agentee_kicad::import_footprint(&vendor.to_string_lossy())?;
    let theirs = theirs.resolve(&mut agentee_core::diag::Diags::new(&theirs.name));
    let name = vendor.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    fit_footprint(ft, &theirs, &format!("the vendor footprint {name}"))
}

fn fit_footprint(
    ft: &str,
    theirs: &agentee_core::footprint::Footprint,
    what: &str,
) -> Result<(String, String), String> {
    let ours: agentee_core::footprint::FootprintFile =
        toml::from_str(ft).map_err(|e| e.to_string())?;
    let ours = ours.resolve(&mut agentee_core::diag::Diags::new(&ours.name));
    let f = agentee_3d::fit::fit(theirs, &ours)?;
    let mut doc: toml_edit::DocumentMut = ft.parse().map_err(|e| format!("{e}"))?;
    let mut offset = toml_edit::Array::new();
    for v in f.model_offset {
        offset.push(format!("{}mm", agentee_core::units::trim(v, 4)));
    }
    doc["model_offset"] = toml_edit::value(offset);
    let mut rotate = toml_edit::Array::new();
    for v in f.model_rotate {
        rotate.push(v);
    }
    doc["model_rotate"] = toml_edit::value(rotate);
    if f.model_scale != [1.0; 3] {
        let mut scale = toml_edit::Array::new();
        for v in f.model_scale {
            scale.push(v);
        }
        doc["model_scale"] = toml_edit::value(scale);
    }
    let note = format!(
        "model placed by {what}: {} pads matched within {:.3} mm, turned {} degrees",
        f.pads, f.rms, f.turn
    );
    Ok((doc.to_string(), note))
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

struct Stored {
    rel: Option<String>,
    file: Option<Provenance>,
    lcsc: Option<String>,
    notes: Vec<String>,
}

fn store_model(
    ft: &mut String,
    dir: &Path,
    model_spec: Option<&str>,
    mpn: &str,
    easyeda: bool,
    root: &Path,
) -> Result<Stored, String> {
    let mut out = Stored { rel: None, file: None, lcsc: None, notes: Vec::new() };
    let mut tried = Vec::new();
    let mut got: Option<Model> = None;
    if let Some(m) = model_spec {
        match local_model(m, root) {
            Ok(found) => got = Some(found),
            Err(e) => tried.push(format!("model {m}: {e}")),
        }
    }
    if got.is_none() && easyeda {
        match agentee_3d::easyeda::find(mpn) {
            Ok(f) => {
                out.lcsc = Some(f.lcsc.clone());
                got = Some(Model {
                    path: f.model,
                    source: format!("EasyEDA {} (easyeda.com)", f.lcsc),
                    vendor: Some(Vendor::Placed(
                        Box::new(f.footprint),
                        format!("the EasyEDA footprint of {}", f.lcsc),
                    )),
                    note: None,
                })
            }
            Err(e) => tried.push(format!("EasyEDA: {e}")),
        }
    }
    let fitted = got.as_ref().and_then(|model| match &model.vendor {
        Some(Vendor::KiCad(path)) => Some(fit_to(ft, path)),
        Some(Vendor::Placed(fp, what)) => Some(fit_footprint(ft, fp, what)),
        None => None,
    });
    match fitted {
        Some(Ok((text, note))) => {
            *ft = text;
            out.notes.push(note);
        }
        Some(Err(e)) => {
            let source = got.take().map(|m| m.source).unwrap_or_default();
            tried.push(format!("{source}: {e}"));
        }
        None => {}
    }
    let Some(model) = got else {
        if !tried.is_empty() {
            out.notes.push(format!("no 3D model stored: {}", tried.join("; ")));
        }
        return Ok(out);
    };
    out.notes.extend(model.note.clone());
    let path = &model.path;
    let file = path.file_name().and_then(|f| f.to_str()).unwrap_or("model.step").to_string();
    let rel = format!("3dmodels/{file}");
    std::fs::create_dir_all(dir.join("3dmodels")).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::write(dir.join(&rel), &bytes).map_err(|e| e.to_string())?;
    *ft = set_key(ft, "model", &rel)?;
    if get_key(ft, "height").is_none()
        && let Ok(file) = toml::from_str::<agentee_core::footprint::FootprintFile>(ft)
        && let Ok(mesh) = agentee_3d::load(path)
    {
        let f = file.resolve(&mut agentee_core::diag::Diags::new(&file.name));
        if let Some(h) = agentee_3d::model_height(&f, &mesh) {
            *ft = set_key(ft, "height", &format!("{h:.2}mm"))?;
        }
    }
    out.file = Some(Provenance {
        path: rel.clone(),
        source: model.source.clone(),
        sha256: datasheet::sha256(&bytes),
    });
    out.rel = Some(rel);
    Ok(out)
}

fn find_datasheet(
    mpn: &str,
    manufacturer: Option<&str>,
    given: Option<&str>,
    symbol_url: Option<String>,
    distributors: bool,
    config: Option<&Path>,
    notes: &mut Vec<String>,
) -> Result<Option<library::Datasheet>, String> {
    if let Some(f) = given.filter(|f| Path::new(f).is_file()) {
        let v = datasheets::add(mpn, Path::new(f), manufacturer, None, None)?;
        return Ok(v["sha256"].as_str().map(|sha| library::Datasheet {
            url: None,
            sha256: sha.to_string(),
            revision: v["revision"].as_str().map(str::to_string),
        }));
    }
    let sym_url = symbol_url.filter(|u| datasheet::is_url(u));
    let urls: Vec<(String, String)> = given
        .map(|u| ("given".to_string(), u.to_string()))
        .or(sym_url.map(|u| ("symbol".to_string(), u)))
        .into_iter()
        .collect();
    let mut want = vec![Want {
        key: mpn.to_string(),
        manufacturer: manufacturer.map(str::to_string),
        mpn: Some(mpn.to_string()),
        refs: Vec::new(),
        urls,
    }];
    let args = FetchArgs { url: given, refresh: false, config, cache: None, distributors };
    let (got, _) = datasheets::fetch_wants(&mut want, &args, None)?;
    Ok(match got.into_iter().next() {
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
    })
}

pub struct FillArgs<'a> {
    pub name: Option<&'a str>,
    pub datasheet: Option<&'a str>,
    pub model: Option<&'a str>,
    pub like: Option<&'a str>,
    pub distributors: bool,
    pub config: Option<&'a Path>,
}

pub fn fill(lib: &Library, a: &FillArgs) -> Result<Value, String> {
    let (name, distributors, config) = (a.name, a.distributors, a.config);
    if name.is_none() && (a.datasheet.is_some() || a.model.is_some() || a.like.is_some()) {
        return Err("--datasheet, --model and --like fill one part: name it".into());
    }
    let targets: Vec<&Entry> = match name {
        Some(n) => vec![lib.get(n).ok_or_else(|| no_part(lib, n))?],
        None => lib.entries.iter().collect(),
    };
    let mut done = Vec::new();
    let mut open_items = Vec::new();
    for e in targets {
        let mut part = e.part.clone();
        let mut filled = Vec::new();
        let mut notes = Vec::new();
        let mpn = part.mpn.clone().unwrap_or_else(|| e.name.clone());
        let sym_path = e.file(&part.symbol);
        if part.datasheet.is_none() || a.datasheet.is_some() {
            let sym_text = std::fs::read_to_string(&sym_path).unwrap_or_default();
            let found = find_datasheet(
                &mpn,
                part.manufacturer.as_deref(),
                a.datasheet,
                get_key(&sym_text, "datasheet"),
                distributors,
                config,
                &mut notes,
            );
            match found {
                Ok(Some(d)) => {
                    pin_datasheet(&e.name, &d.sha256)?;
                    if let Some(url) = &d.url
                        && !sym_text.is_empty()
                        && get_key(&sym_text, "datasheet").as_deref() != Some(url.as_str())
                    {
                        let text = set_key(&sym_text, "datasheet", url)?;
                        std::fs::write(&sym_path, &text).map_err(|e| e.to_string())?;
                        let sym = part.symbol.clone();
                        rehash(&mut part, &sym, text.as_bytes());
                    }
                    part.datasheet = Some(d);
                    filled.push("datasheet");
                }
                Ok(None) => {}
                Err(err) => notes.push(format!("datasheet: {err}")),
            }
        }
        let want_model = part.model.is_none() || a.model.is_some() || a.like.is_some();
        match (want_model, part.footprint.clone()) {
            (true, Some(f)) => {
                let fp_path = e.file(&f);
                let mut ft = std::fs::read_to_string(&fp_path)
                    .map_err(|err| format!("{}: {err}", fp_path.display()))?;
                let spec = a.model.map(str::to_string).or_else(|| get_key(&ft, "model"));
                let easyeda = distributors && !part.generic;
                let spec = spec.filter(|_| a.model.is_some() || a.like.is_none());
                let look_up = a.like.unwrap_or(&mpn);
                let stored =
                    store_model(&mut ft, &e.dir, spec.as_deref(), look_up, easyeda, &e.dir)?;
                notes.extend(stored.notes);
                if let Some(rel) = stored.rel {
                    std::fs::write(&fp_path, &ft).map_err(|err| err.to_string())?;
                    rehash(&mut part, &f, ft.as_bytes());
                    part.files.retain(|p| p.path != rel);
                    part.files.extend(stored.file);
                    part.model = Some(rel);
                    if part.lcsc.is_none() {
                        part.lcsc = stored.lcsc;
                    }
                    filled.push("model");
                }
            }
            (_, None) => notes.push(
                "no footprint: give one with agentee library set NAME --footprint ...".into(),
            ),
            _ => {}
        }
        if !filled.is_empty() {
            library::write_entry(&e.dir, &part)?;
            done.push(json!({ "part": e.name, "filled": filled, "notes": notes }));
        } else if part.datasheet.is_none() || part.model.is_none() {
            let mut missing = Vec::new();
            if part.datasheet.is_none() {
                missing.push("datasheet");
            }
            if part.model.is_none() {
                missing.push("model");
            }
            open_items.push(json!({ "part": e.name, "missing": missing, "notes": notes }));
        }
    }
    Ok(json!({ "filled": done, "still_missing": open_items }))
}

fn rehash(part: &mut PartFile, path: &str, bytes: &[u8]) {
    for p in part.files.iter_mut().filter(|p| p.path == path) {
        p.sha256 = datasheet::sha256(bytes);
    }
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
    let mut lcsc = None;
    let root = project.map(|p| p.root.clone()).unwrap_or_else(|| PathBuf::from("."));
    let model_spec = a
        .model
        .map(str::to_string)
        .or_else(|| fp_text.as_deref().and_then(|t| get_key(t, "model")));
    if let Some(ft) = fp_text.as_mut() {
        let easyeda = a.distributors && !a.generic;
        let stored = store_model(ft, &dir, model_spec.as_deref(), a.mpn.trim(), easyeda, &root)?;
        notes.extend(stored.notes);
        files.extend(stored.file);
        lcsc = stored.lcsc;
        model_rel = stored.rel;
    }
    if let (Some(f), Some(t)) = (&fp, &fp_text) {
        std::fs::write(dir.join(&f.file), t).map_err(|e| e.to_string())?;
        files.insert(
            0,
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
    if a.distributors && !offers.is_empty() {
        let live = stock.iter().any(|st| st.stock > 0 && st.offer().active());
        if !live {
            notes.push(format!(
                "{mpn} is not in stock as an active part at any distributor: find a current one with agentee library find, or ask the user"
            ));
        }
    }
    let manufacturer = a.manufacturer.map(str::to_string).or_else(|| {
        offers.iter().find(|o| same_part(&o.mpn, &mpn)).map(|o| o.manufacturer.clone())
    });
    let datasheet = find_datasheet(
        &mpn,
        manufacturer.as_deref(),
        a.datasheet,
        get_key(&sym_text, "datasheet"),
        a.distributors,
        a.config,
        &mut notes,
    )?;
    if let Some(d) = &datasheet {
        pin_datasheet(&name, &d.sha256)?;
    }
    let sym_text = match datasheet.as_ref().and_then(|d| d.url.as_deref()) {
        Some(url) if get_key(&sym_text, "datasheet").as_deref() != Some(url) => {
            notes.push(format!("the symbol's datasheet now points at {url}"));
            set_key(&sym_text, "datasheet", url)?
        }
        _ => sym_text,
    };
    std::fs::write(dir.join(&sym.file), &sym_text).map_err(|e| e.to_string())?;
    files.insert(
        0,
        Provenance {
            path: sym.file.clone(),
            source: sym.source.clone(),
            sha256: datasheet::sha256(sym_text.as_bytes()),
        },
    );
    if lcsc.is_none() && a.distributors {
        lcsc = agentee_3d::easyeda::lcsc_code(&mpn).ok();
    }
    let part = PartFile {
        role: a.role.trim().to_lowercase(),
        mpn: Some(mpn.clone()),
        manufacturer,
        lcsc,
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

pub struct FindArgs<'a> {
    pub words: &'a [String],
    pub role: Option<&'a str>,
    pub need: u32,
    pub distributors: &'a [String],
    pub config: Option<&'a Path>,
    pub limit: usize,
    pub search: bool,
}

#[derive(serde::Serialize)]
struct Candidate {
    mpn: String,
    manufacturer: String,
    description: String,
    stock: u64,
    unit: Option<(f64, String)>,
    lifecycle: Option<String>,
    datasheet: Option<String>,
    at: Vec<String>,
    in_library: Option<String>,
}

pub fn find(lib: &Library, a: &FindArgs) -> Result<Value, String> {
    let ours: Vec<&Entry> = lib.filter(a.role, a.words);
    let q = ops::PartsQuery {
        boards: 1,
        alternatives: false,
        distributors: a.distributors,
        farnell_store: None,
        refs: &[],
        config: a.config,
    };
    let keyword = a.words.join(" ");
    let mut offers = Vec::new();
    let mut errors = Vec::new();
    if !keyword.trim().is_empty() && (ours.is_empty() || a.search) {
        match ops::Distributors::open(&q) {
            Ok(d) => {
                for s in d.list() {
                    match s.search(&keyword) {
                        Ok(o) => offers.extend(o),
                        Err(e) => errors.push(e),
                    }
                }
            }
            Err(e) => errors.push(e),
        }
    }
    let by = candidates(&offers, lib, a.need, a.limit);
    Ok(json!({
        "library": ours.iter().map(|e| json!({
            "name": e.name,
            "summary": e.summary(),
            "own_stock": e.self_stock(),
            "bin": e.bin(),
            "stock": e.best_stock().map(Stock::line),
        })).collect::<Vec<_>>(),
        "candidates": by,
        "errors": errors,
        "next": if ours.is_empty() {
            "pick a candidate from its datasheet, then agentee library add MPN --role ROLE --symbol ... --footprint ... and agentee library use NAME"
        } else {
            "use the library part: agentee library use NAME; add --search to look at the distributors too"
        },
    }))
}

fn candidates(
    offers: &[agentee_parts::offer::Offer],
    lib: &Library,
    need: u32,
    limit: usize,
) -> Vec<Candidate> {
    let mut by: Vec<Candidate> = Vec::new();
    for o in offers.iter().filter(|o| o.active() && o.stock >= u64::from(need.max(1))) {
        if o.mpn.trim().is_empty() {
            continue;
        }
        let unit = o.cost(need.max(1)).map(|c| (c.unit, o.currency.clone()));
        match by.iter_mut().find(|c| {
            datasheet::normal(&c.mpn) == datasheet::normal(&o.mpn)
                && (c.manufacturer.is_empty()
                    || o.manufacturer.is_empty()
                    || datasheets::same_maker(&c.manufacturer, &o.manufacturer))
        }) {
            Some(c) => {
                c.stock += o.stock;
                if !c.at.contains(&o.distributor) {
                    c.at.push(o.distributor.clone());
                }
                if c.datasheet.is_none() {
                    c.datasheet = o.datasheet.clone();
                }
                if let (Some(u), Some(b)) = (&unit, &c.unit)
                    && u.1 == b.1
                    && u.0 < b.0
                {
                    c.unit = unit.clone();
                }
                if c.unit.is_none() {
                    c.unit = unit;
                }
            }
            None => by.push(Candidate {
                mpn: o.mpn.clone(),
                manufacturer: o.manufacturer.clone(),
                description: o.description.clone(),
                stock: o.stock,
                unit,
                lifecycle: o.lifecycle.clone(),
                datasheet: o.datasheet.clone(),
                at: vec![o.distributor.clone()],
                in_library: lib.get(&o.mpn).map(|e| e.name.clone()),
            }),
        }
    }
    by.sort_by(|x, y| {
        y.in_library
            .is_some()
            .cmp(&x.in_library.is_some())
            .then(y.at.len().cmp(&x.at.len()))
            .then(y.stock.cmp(&x.stock))
    });
    by.truncate(limit.max(1));
    by
}

pub fn find_text(v: &Value) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let ours = v["library"].as_array().cloned().unwrap_or_default();
    if ours.is_empty() {
        s.push_str("in the library: none\n");
    } else {
        s.push_str("in the library, use these first:\n");
        for e in &ours {
            let _ = writeln!(
                s,
                "  {:<24} {}",
                e["name"].as_str().unwrap_or(""),
                e["summary"].as_str().unwrap_or("")
            );
            if let Some(st) = e["stock"].as_str() {
                let _ = writeln!(s, "  {:<24} {st}", "");
            }
        }
    }
    let found = v["candidates"].as_array().cloned().unwrap_or_default();
    if found.is_empty() {
        s.push_str("in stock and active at the distributors: none\n");
    } else {
        s.push_str("in stock and active at the distributors:\n");
        for c in &found {
            let price = c["unit"]
                .as_array()
                .map(|u| {
                    format!(
                        ", {:.4} {} each",
                        u[0].as_f64().unwrap_or(0.0),
                        u[1].as_str().unwrap_or("")
                    )
                })
                .unwrap_or_default();
            let lib =
                c["in_library"].as_str().map(|n| format!(" (library: {n})")).unwrap_or_default();
            let at: Vec<&str> = c["at"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            let _ = writeln!(
                s,
                "  {} {}{lib}: {} in stock at {}{price}",
                c["manufacturer"].as_str().unwrap_or(""),
                c["mpn"].as_str().unwrap_or(""),
                c["stock"],
                at.join(", ")
            );
            let _ = writeln!(s, "    {}", c["description"].as_str().unwrap_or(""));
        }
    }
    for e in v["errors"].as_array().into_iter().flatten() {
        let _ = writeln!(s, "error: {}", e.as_str().unwrap_or(""));
    }
    let _ = writeln!(s, "next: {}", v["next"].as_str().unwrap_or(""));
    s
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
        if let Some(n) = e.self_stock() {
            let bin = e.bin().map(|b| format!(" in bin {b}")).unwrap_or_default();
            let _ = writeln!(s, "  {:<24} {n} in your own stock{bin}", "");
        }
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

enum Copy {
    Write(PathBuf, Vec<u8>),
    Same(PathBuf),
    Kept(PathBuf),
}

fn without_model(bytes: &[u8]) -> Option<toml::Table> {
    let mut t: toml::Table = std::str::from_utf8(bytes).ok()?.parse().ok()?;
    t.remove("model");
    t.remove("height");
    Some(t)
}

fn plan_copy(src: &Path, dst: &Path, force: bool) -> Result<Copy, String> {
    let bytes = std::fs::read(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let Ok(old) = std::fs::read(dst) else {
        return Ok(Copy::Write(dst.to_path_buf(), bytes));
    };
    if old == bytes {
        return Ok(Copy::Same(dst.to_path_buf()));
    }
    if force {
        return Ok(Copy::Write(dst.to_path_buf(), bytes));
    }
    let footprint = dst.to_string_lossy().ends_with(".fp.toml");
    if footprint && without_model(&old).is_some_and(|o| without_model(&bytes) == Some(o)) {
        return Ok(Copy::Kept(dst.to_path_buf()));
    }
    Err(format!(
        "{} exists and differs from the library copy, pass force to replace it; nothing was written",
        dst.display()
    ))
}

fn apply(plan: Vec<Copy>) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    for c in plan {
        match c {
            Copy::Same(p) => files.push(format!("{} (unchanged)", p.display())),
            Copy::Kept(p) => files.push(format!(
                "{} (kept, differs from the library only in model and height)",
                p.display()
            )),
            Copy::Write(p, bytes) => {
                if let Some(d) = p.parent() {
                    std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
                }
                std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display()))?;
                files.push(p.display().to_string());
            }
        }
    }
    Ok(files)
}

pub fn use_part(lib: &Library, name: &str, root: &Path, force: bool) -> Result<Value, String> {
    let e = lib.get(name).ok_or_else(|| no_part(lib, name))?;
    let mut plan = vec![plan_copy(
        &e.file(&e.part.symbol),
        &root.join("symbols").join(&e.part.symbol),
        force,
    )?];
    let mut footprint_kept = false;
    if let Some(f) = &e.part.footprint {
        let c = plan_copy(&e.file(f), &root.join("footprints").join(f), force)?;
        footprint_kept = matches!(c, Copy::Kept(_));
        plan.push(c);
    }
    if let Some(m) = e.part.model.as_ref().filter(|_| !footprint_kept) {
        plan.push(plan_copy(&e.file(m), &root.join(m), force)?);
    }
    let files = apply(plan)?;
    let mut fields = serde_json::Map::new();
    if let Some(m) = &e.part.mpn {
        fields.insert("mpn".into(), json!(m));
    }
    if let Some(m) = &e.part.manufacturer {
        fields.insert("mfr".into(), json!(m));
    }
    if let Some(c) = &e.part.lcsc {
        fields.insert("lcsc".into(), json!(c));
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
            s.own > 0
                || failed.contains(&s.distributor.as_str())
                    && !fresh.iter().any(|f| f.distributor == s.distributor)
        });
        part.stock.extend(fresh);
        part.stock.sort_by(|a, b| a.distributor.cmp(&b.distributor).then(a.sku.cmp(&b.sku)));
        if part.lcsc.is_none() && !part.generic {
            part.lcsc = agentee_3d::easyeda::lcsc_code(mpn).ok();
        }
        library::write_entry(&e.dir, &part)?;
        done.push(json!({ "part": e.name, "lcsc": part.lcsc, "stock": part.stock.iter().map(Stock::line).collect::<Vec<_>>() }));
    }
    Ok(json!({ "refreshed": done, "errors": errors }))
}

pub struct Fields {
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub lcsc: Option<String>,
    pub value: Option<String>,
    pub rating: Option<String>,
    pub role: Option<String>,
    pub notes: Option<String>,
    pub footprint: Option<String>,
    pub symbol: Option<String>,
}

fn swap_symbol(
    e: &Entry,
    part: &mut PartFile,
    spec: &str,
    project: Option<&Project>,
) -> Result<(), String> {
    let sym = source_symbol(spec, project)?;
    let old_path = e.file(&part.symbol);
    let old = std::fs::read_to_string(&old_path).unwrap_or_default();
    let mut text = set_key(&sym.text, "name", &get_key(&old, "name").unwrap_or(e.name.clone()))?;
    for key in ["footprint", "datasheet"] {
        if let Some(v) = get_key(&old, key) {
            text = set_key(&text, key, &v)?;
        }
    }
    std::fs::write(&old_path, &text).map_err(|err| err.to_string())?;
    let file = part.symbol.clone();
    part.files.retain(|p| p.path != file);
    part.files.insert(
        0,
        Provenance { path: file, source: sym.source, sha256: datasheet::sha256(text.as_bytes()) },
    );
    Ok(())
}

fn swap_footprint(
    e: &Entry,
    part: &mut PartFile,
    spec: &str,
    project: Option<&Project>,
) -> Result<String, String> {
    let fp = source_footprint(spec, project)?;
    let fname = get_key(&fp.text, "name").unwrap_or_else(|| stem(&fp.file));
    let mut gone: Vec<String> = part.model.take().into_iter().collect();
    if let Some(old) = part.footprint.take().filter(|o| *o != fp.file) {
        gone.push(old);
    }
    for g in &gone {
        let _ = std::fs::remove_file(e.file(g));
    }
    part.files.retain(|p| !gone.contains(&p.path) && p.path != fp.file);
    std::fs::write(e.file(&fp.file), &fp.text).map_err(|err| err.to_string())?;
    part.files.push(Provenance {
        path: fp.file.clone(),
        source: fp.source,
        sha256: datasheet::sha256(fp.text.as_bytes()),
    });
    let sym_path = e.file(&part.symbol);
    let sym = std::fs::read_to_string(&sym_path)
        .map_err(|err| format!("{}: {err}", sym_path.display()))?;
    let sym = set_key(&sym, "footprint", &fname)?;
    std::fs::write(&sym_path, &sym).map_err(|err| err.to_string())?;
    let symbol = part.symbol.clone();
    rehash(part, &symbol, sym.as_bytes());
    part.footprint = Some(fp.file);
    Ok(fname)
}

pub fn set_fields(
    lib: &Library,
    name: &str,
    f: &Fields,
    project: Option<&Project>,
) -> Result<Value, String> {
    let e = lib.get(name).ok_or_else(|| no_part(lib, name))?;
    let mut part = e.part.clone();
    if let Some(spec) = &f.symbol {
        swap_symbol(e, &mut part, spec, project)?;
    }
    let footprint = match &f.footprint {
        Some(spec) => Some(swap_footprint(e, &mut part, spec, project)?),
        None => None,
    };
    let text = |v: &Option<String>| v.as_ref().map(|s| s.trim().to_string());
    let some = |v: Option<String>| v.filter(|s| !s.is_empty());
    if let Some(m) = text(&f.mpn) {
        if part.mpn.as_deref() != Some(m.as_str()) {
            let old = part.mpn.clone();
            for s in part.stock.iter_mut().filter(|s| s.own > 0) {
                if old.as_deref() == Some(s.sku.as_str()) || s.sku == e.name {
                    s.sku = m.clone();
                }
            }
            part.stock.retain(|s| s.own > 0 || s.distributor == "home");
            part.datasheet = None;
            if f.lcsc.is_none() {
                part.lcsc = None;
            }
        }
        part.mpn = some(Some(m));
    }
    if f.manufacturer.is_some() {
        part.manufacturer = some(text(&f.manufacturer));
    }
    if f.lcsc.is_some() {
        part.lcsc = some(text(&f.lcsc));
    }
    if f.value.is_some() {
        part.value = some(text(&f.value));
    }
    if f.rating.is_some() {
        part.rating = some(text(&f.rating));
    }
    if let Some(r) = text(&f.role).filter(|r| !r.is_empty()) {
        part.role = r.to_lowercase();
    }
    if let Some(n) = text(&f.notes) {
        part.notes = n;
    }
    library::write_entry(&e.dir, &part)?;
    let mut out = json!({ "part": e.name, "mpn": part.mpn, "manufacturer": part.manufacturer });
    if let Some(fp) = footprint {
        out["footprint"] = json!(fp);
        out["next"] = json!(format!(
            "the model went with the old footprint: agentee library fill {} finds one for the new",
            e.name
        ));
    }
    Ok(out)
}

pub fn set_stock(
    lib: &Library,
    name: &str,
    count: u32,
    bin: Option<&str>,
) -> Result<String, String> {
    let e = lib.get(name).ok_or_else(|| no_part(lib, name))?;
    let part = library::set_own(e, u64::from(count), bin)?;
    let n: u64 = part.stock.iter().map(|s| s.own).sum();
    let bin = part.stock.iter().find_map(|s| s.bin.clone());
    let place = bin.filter(|_| n > 0).map(|b| format!(" in bin {b}")).unwrap_or_default();
    Ok(format!("{}: {n} in your own stock{place}", e.name))
}

fn bag_entry<'a>(lib: &'a Library, bag: &agentee_parts::intake::Bag) -> Option<&'a Entry> {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b.trim());
    bag.mpn.as_deref().and_then(|m| lib.get(m)).or_else(|| {
        let sku = bag.sku.as_deref()?;
        lib.entries.iter().find(|e| {
            e.part.lcsc.as_deref().is_some_and(|c| same(c, sku))
                || e.part.stock.iter().any(|s| same(&s.sku, sku))
        })
    })
}

pub fn intake(project: &Path, codes: &[String], bin: Option<&str>) -> (String, bool) {
    let mut out = String::new();
    let mut ok = true;
    for code in codes.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
        let Some(bag) = agentee_parts::intake::parse(code) else {
            out += &format!("could not read a part number and quantity from `{code}`\n");
            ok = false;
            continue;
        };
        let lib = open(Some(project));
        let what = bag.mpn.clone().or(bag.sku.clone()).unwrap_or_default();
        let Some(e) = bag_entry(&lib, &bag) else {
            out += &format!(
                "{what} ({} {}): not in the library; `agentee library add {what}`, then scan it again\n",
                bag.distributor, bag.qty
            );
            ok = false;
            continue;
        };
        let have = e.self_stock().unwrap_or(0);
        let total = have + bag.qty;
        match set_stock(&lib, &e.name, total.min(u64::from(u32::MAX)) as u32, bin) {
            Ok(line) => out += &format!("+{} from {}, {line}\n", bag.qty, bag.distributor),
            Err(err) => {
                out += &format!("{}: {err}\n", e.name);
                ok = false;
            }
        }
    }
    (out, ok)
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
    if let Some(fp) = &fp
        && let Some(rules) = agentee_core::board::fab_rules("jlcpcb")
    {
        let mut d = agentee_core::diag::Diags::new(&fp.name);
        fp.check(&rules, &mut d);
        let mut seen = std::collections::BTreeMap::new();
        for x in d.list.iter().filter(|x| x.severity == agentee_core::Severity::Error) {
            *seen.entry(x.message.clone()).or_insert(0) += 1;
        }
        for (m, n) in seen {
            let times = if n > 1 { format!(", {n} pads") } else { String::new() };
            push(false, format!("{}: {m}{times} (jlcpcb rules)", fp.name));
        }
    }
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
                let frame = agentee_3d::export::model_frame(fp);
                let mut placed = agentee_core::graphic::Bounds::EMPTY;
                for k in 0..8 {
                    let c =
                        [0, 1, 2].map(|i| f64::from(if k >> i & 1 == 0 { lo[i] } else { hi[i] }));
                    let q = [0, 1]
                        .map(|i| (0..3).map(|j| frame[i][j] * c[j]).sum::<f64>() + frame[i][3]);
                    placed.add([q[0], -q[1]]);
                }
                let (mw, mh) = (placed.max[0] - placed.min[0], placed.max[1] - placed.min[1]);
                let cy = fp.courtyard("F");
                let b = if cy.is_empty() { fp.bounds() } else { cy };
                let (fw, fh) = (b.max[0] - b.min[0], b.max[1] - b.min[1]);
                let centre = placed.center();
                let inside = centre[0] >= b.min[0]
                    && centre[0] <= b.max[0]
                    && centre[1] >= b.min[1]
                    && centre[1] <= b.max[1];
                let ok = mw <= fw + 0.5 && mh <= fh + 0.5 && mw * mh >= 0.25 * fw * fh && inside;
                push(
                    ok,
                    format!(
                        "model {:.2} x {:.2} mm as placed, centred at [{:.2}, {:.2}], against the {} {:.2} x {:.2} mm{}",
                        mw,
                        mh,
                        centre[0],
                        centre[1],
                        if cy.is_empty() { "footprint" } else { "courtyard" },
                        fw,
                        fh,
                        if ok {
                            ""
                        } else {
                            ": wrong model or a model_offset, model_rotate or model_scale problem"
                        }
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
    fn one_part_number_from_two_makers_stays_two_candidates() {
        let offer = |at: &str, maker: &str, url: &str| agentee_parts::offer::Offer {
            distributor: at.into(),
            manufacturer: maker.into(),
            mpn: "1N5817G".into(),
            stock: 100,
            currency: "EUR".into(),
            datasheet: Some(url.into()),
            ..Default::default()
        };
        let offers = [
            offer("Mouser", "Comchip Technology", "comchip.pdf"),
            offer("Farnell", "onsemi", "onsemi.pdf"),
            offer("Mouser", "ON Semiconductor", "onsemi-2.pdf"),
        ];
        let lib = Library {
            dirs: Vec::new(),
            entries: Vec::new(),
            config: Default::default(),
            errors: Vec::new(),
        };
        let got = candidates(&offers, &lib, 1, 10);
        let row = |m: &str| got.iter().find(|c| c.manufacturer == m).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(row("Comchip Technology").datasheet.as_deref(), Some("comchip.pdf"));
        assert_eq!(row("onsemi").datasheet.as_deref(), Some("onsemi.pdf"));
        assert_eq!(row("onsemi").stock, 200);
    }

    #[test]
    fn a_vendor_bundle_places_its_model_by_the_pads() {
        let dir = std::env::temp_dir().join(format!("agentee-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("KiCad")).unwrap();
        std::fs::create_dir_all(dir.join("3D")).unwrap();
        std::fs::write(dir.join("3D/part.stp"), "ISO-10303-21;\n").unwrap();
        std::fs::write(
            dir.join("KiCad/part.kicad_mod"),
            "(footprint \"V\" (pad \"1\" smd rect (at -1 0) (size 0.5 0.5) (layers \"F.Cu\")) (pad \"2\" smd rect (at 1 0) (size 0.5 0.5) (layers \"F.Cu\")) (pad \"3\" smd rect (at 0 2) (size 0.5 0.5) (layers \"F.Cu\")) (model \"part.stp\" (offset (xyz 0.5 0 0)) (rotate (xyz 0 0 90))))",
        )
        .unwrap();
        let (model, vendor) = vendor_bundle(dir.to_str().unwrap()).unwrap().unwrap();
        assert!(model.ends_with("3D/part.stp"));
        let zip_path = dir.with_extension("zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        for name in ["3D/part.stp", "KiCad/part.kicad_mod"] {
            w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut w, &std::fs::read(dir.join(name)).unwrap()).unwrap();
        }
        w.finish().unwrap();
        let (zm, zv) = vendor_bundle(zip_path.to_str().unwrap()).unwrap().unwrap();
        assert!(
            zm.ends_with("3D/part.stp") && zv.is_some_and(|v| v.ends_with("KiCad/part.kicad_mod"))
        );
        let _ = std::fs::remove_file(&zip_path);
        let vendor = vendor.unwrap();
        let pad = |n: &str, at: [f64; 2]| {
            let q = agentee_core::geom::rotate(at, 90.0);
            format!(
                "\n[[pads]]\nnumber = \"{n}\"\nkind = \"smd\"\nshape = \"rect\"\nat = [{}, {}]\nsize = [0.5, 0.5]\n",
                q[0] + 2.0,
                q[1] + 3.0
            )
        };
        let ours = format!(
            "name = \"P\"\nmodel = \"x.step\"\n{}{}{}",
            pad("1", [-1.0, 0.0]),
            pad("2", [1.0, 0.0]),
            pad("3", [0.0, 2.0])
        );
        let (text, note) = fit_to(&ours, &vendor).unwrap();
        assert!(note.contains("3 pads matched") && note.contains("turned 90 degrees"), "{note}");
        let fp: agentee_core::footprint::FootprintFile = toml::from_str(&text).unwrap();
        let fp = fp.resolve(&mut agentee_core::diag::Diags::new("P"));
        let theirs = agentee_kicad::import_footprint(&vendor.to_string_lossy()).unwrap();
        let theirs = theirs.resolve(&mut agentee_core::diag::Diags::new("V"));
        let at = |f: &agentee_core::footprint::Footprint| {
            let r = agentee_3d::export::rotation(f.model_rotate);
            let m = [1.0, 0.5, 0.0];
            let q = [0, 1].map(|i| (0..3).map(|k| r[i][k] * m[k]).sum::<f64>() + f.model_offset[i]);
            [q[0], -q[1]]
        };
        let want = agentee_core::geom::rotate(at(&theirs), 90.0);
        let got = at(&fp);
        assert!(
            agentee_core::geom::dist([want[0] + 2.0, want[1] + 3.0], got) < 1e-3,
            "{want:?} {got:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_keeps_current_stocked_parts_merged_by_part_number() {
        use agentee_parts::offer::{Break, Offer};
        let offer = |d: &str, mpn: &str, stock: u64, price: f64, life: Option<&str>| Offer {
            distributor: d.into(),
            mpn: mpn.into(),
            manufacturer: "Diodes".into(),
            stock,
            currency: "EUR".into(),
            breaks: vec![Break { qty: 1, price }],
            lifecycle: life.map(str::to_string),
            ..Default::default()
        };
        let offers = [
            offer("Mouser", "AP63203WU-7", 900, 0.60, None),
            offer("Farnell", "AP63203WU-7", 100, 0.55, None),
            offer("Mouser", "AP63200WU-7", 5, 0.50, None),
            offer("Mouser", "OLD-PART", 5000, 0.10, Some("Obsolete")),
            offer("Farnell", "TPS62162DSGR", 3000, 1.10, None),
        ];
        let lib = Library::open(None, Some(Path::new("/nonexistent")));
        let c = candidates(&offers, &lib, 10, 10);
        let mpns: Vec<&str> = c.iter().map(|c| c.mpn.as_str()).collect();
        assert_eq!(mpns, ["AP63203WU-7", "TPS62162DSGR"]);
        assert_eq!(c[0].stock, 1000);
        assert_eq!(c[0].at, ["Mouser", "Farnell"]);
        assert_eq!(c[0].unit.as_ref().map(|u| u.0), Some(0.55));
    }

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
