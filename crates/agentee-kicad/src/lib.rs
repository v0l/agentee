pub mod footprint;
pub mod sexpr;
pub mod symbol;

use agentee_core::footprint::FootprintFile;
use agentee_core::symbol::SymbolFile;
use std::path::{Path, PathBuf};

const SHARE: &[&str] = &[
    "/usr/share/kicad",
    "/usr/local/share/kicad",
    "/Applications/KiCad/KiCad.app/Contents/SharedSupport",
];

fn dirs(envs: &[&str], sub: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = envs.iter().filter_map(std::env::var_os).map(PathBuf::from).collect();
    v.extend(SHARE.iter().map(|s| Path::new(s).join(sub)));
    v.retain(|p| p.is_dir());
    v
}

pub fn symbol_dirs() -> Vec<PathBuf> {
    dirs(&["KICAD9_SYMBOL_DIR", "KICAD8_SYMBOL_DIR", "KICAD_SYMBOL_DIR"], "symbols")
}

pub fn footprint_dirs() -> Vec<PathBuf> {
    dirs(&["KICAD9_FOOTPRINT_DIR", "KICAD8_FOOTPRINT_DIR", "KICAD_FOOTPRINT_DIR"], "footprints")
}

fn split_spec(spec: &str, ext: &str) -> (String, Option<String>) {
    if let Some(i) = spec.find(ext) {
        let (path, rest) = spec.split_at(i + ext.len());
        return (
            path.to_string(),
            rest.strip_prefix(':').map(str::to_string).filter(|s| !s.is_empty()),
        );
    }
    match spec.split_once(':') {
        Some((lib, name)) => (lib.to_string(), Some(name.to_string())),
        None => (spec.to_string(), None),
    }
}

pub fn symbol_library(lib: &str) -> Result<PathBuf, String> {
    let direct = Path::new(lib);
    if direct.extension().is_some_and(|e| e == "kicad_sym") {
        return direct
            .is_file()
            .then(|| direct.to_path_buf())
            .ok_or_else(|| format!("{lib} not found"));
    }
    symbol_dirs()
        .into_iter()
        .map(|d| d.join(format!("{lib}.kicad_sym")))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            format!("no KiCad symbol library `{lib}` (searched {})", searched(symbol_dirs()))
        })
}

pub fn footprint_library(lib: &str) -> Result<PathBuf, String> {
    let direct = Path::new(lib);
    if direct.extension().is_some_and(|e| e == "pretty") {
        return direct
            .is_dir()
            .then(|| direct.to_path_buf())
            .ok_or_else(|| format!("{lib} not found"));
    }
    footprint_dirs()
        .into_iter()
        .map(|d| d.join(format!("{lib}.pretty")))
        .find(|p| p.is_dir())
        .ok_or_else(|| {
            format!("no KiCad footprint library `{lib}` (searched {})", searched(footprint_dirs()))
        })
}

fn searched(v: Vec<PathBuf>) -> String {
    if v.is_empty() {
        "nothing, set KICAD9_SYMBOL_DIR / KICAD9_FOOTPRINT_DIR".into()
    } else {
        v.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    }
}

fn read_tree(path: &Path) -> Result<sexpr::Node, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    sexpr::parse(&src).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn import_symbol(spec: &str) -> Result<SymbolFile, String> {
    let (lib, name) = split_spec(spec, ".kicad_sym");
    let name = name.ok_or("give the symbol as Library:Name")?;
    let tree = read_tree(&symbol_library(&lib)?)?;
    symbol::convert(&tree, &name)
}

pub fn import_footprint(spec: &str) -> Result<FootprintFile, String> {
    let direct = Path::new(spec);
    let path = if direct.extension().is_some_and(|e| e == "kicad_mod") {
        direct.to_path_buf()
    } else {
        let (lib, name) = split_spec(spec, ".pretty");
        let name = name.ok_or("give the footprint as Library:Name")?;
        footprint_library(&lib)?.join(format!("{name}.kicad_mod"))
    };
    if !path.is_file() {
        return Err(format!("{} not found", path.display()));
    }
    footprint::convert(&read_tree(&path)?)
}

pub fn list_symbol_libraries() -> Vec<String> {
    list(symbol_dirs(), ".kicad_sym")
}

pub fn list_footprint_libraries() -> Vec<String> {
    list(footprint_dirs(), ".pretty")
}

fn list(dirs: Vec<PathBuf>, ext: &str) -> Vec<String> {
    let mut v: Vec<String> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(ext).map(str::to_string))
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn list_symbols(lib: &str) -> Result<Vec<String>, String> {
    Ok(symbol::names(&read_tree(&symbol_library(lib)?)?))
}

pub fn list_footprints(lib: &str) -> Result<Vec<String>, String> {
    let dir = footprint_library(lib)?;
    let mut v: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".kicad_mod").map(str::to_string))
        .collect();
    v.sort();
    Ok(v)
}

pub fn file_stem(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || "-_.+".contains(c) { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentee_core::diag::Diags;

    #[test]
    fn specs_split() {
        assert_eq!(split_spec("Device:R", ".kicad_sym"), ("Device".into(), Some("R".into())));
        assert_eq!(
            split_spec("/x/y.kicad_sym:R", ".kicad_sym"),
            ("/x/y.kicad_sym".into(), Some("R".into()))
        );
    }

    #[test]
    fn installed_libraries_round_trip() {
        if symbol_dirs().is_empty() || footprint_dirs().is_empty() {
            return;
        }
        let rules = agentee_core::board::fab_rules("generic").unwrap();
        for spec in
            ["Amplifier_Operational:LM358", "Device:R", "MCU_ST_STM32F1:STM32F103C8Tx", "power:GND"]
        {
            let s = import_symbol(spec).unwrap();
            let text = toml::to_string(&s).unwrap();
            let back: SymbolFile = toml::from_str(&text).unwrap();
            let mut d = Diags::new(spec);
            back.resolve(&mut d).check(&mut d);
            assert!(!d.has_errors(), "{spec}: {:?}", d.list);
        }
        for spec in [
            "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm",
            "Package_QFP:LQFP-48_7x7mm_P0.5mm",
            "Resistor_SMD:R_0603_1608Metric",
            "Connector_PinHeader_2.54mm:PinHeader_1x04_P2.54mm_Vertical",
            "Package_DFN_QFN:QFN-32-1EP_5x5mm_P0.5mm_EP3.45x3.45mm",
        ] {
            let f = import_footprint(spec).unwrap();
            let text = toml::to_string(&f).unwrap();
            let back: FootprintFile = toml::from_str(&text).unwrap();
            let mut d = Diags::new(spec);
            back.resolve(&mut d).check(&rules, &mut d);
            assert!(!d.has_errors(), "{spec}: {:?}", d.list);
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Hit {
    pub library: String,
    pub name: String,
}

fn matches(terms: &[String], lib: &str, name: &str) -> bool {
    let hay = format!("{lib}:{name}").to_lowercase();
    terms.iter().all(|t| hay.contains(t.as_str()))
}

pub fn search_symbols(query: &str, limit: usize) -> Vec<Hit> {
    let terms: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    let mut out = Vec::new();
    for lib in list_symbol_libraries() {
        let Ok(path) = symbol_library(&lib) else { continue };
        let Ok(src) = std::fs::read_to_string(path) else { continue };
        for line in src.lines() {
            let Some(rest) =
                line.strip_prefix("\t(symbol \"").or_else(|| line.strip_prefix("  (symbol \""))
            else {
                continue;
            };
            let Some(name) = rest.split('"').next() else { continue };
            if matches(&terms, &lib, name) {
                out.push(Hit { library: lib.clone(), name: name.to_string() });
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

pub fn search_footprints(query: &str, limit: usize) -> Vec<Hit> {
    let terms: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    let mut out = Vec::new();
    for lib in list_footprint_libraries() {
        for name in list_footprints(&lib).unwrap_or_default() {
            if matches(&terms, &lib, &name) {
                out.push(Hit { library: lib.clone(), name });
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}
