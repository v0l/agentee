pub mod step;
pub mod wrl;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Debug, Default)]
pub struct Part {
    pub colour: [f32; 3],
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub parts: Vec<Part>,
}

impl Mesh {
    pub fn triangles(&self) -> usize {
        self.parts.iter().map(|p| p.positions.len() / 3).sum()
    }

    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in self.parts.iter().flat_map(|p| &p.positions) {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        (lo, hi)
    }
}

#[derive(Default)]
pub struct MeshBuilder {
    parts: Vec<Part>,
    index: HashMap<[u16; 3], usize>,
}

impl MeshBuilder {
    fn part(&mut self, colour: [f32; 3]) -> &mut Part {
        let key = colour.map(|c| (c.clamp(0.0, 1.0) * 1000.0) as u16);
        let n = self.parts.len();
        let i = *self.index.entry(key).or_insert(n);
        if i == n {
            self.parts.push(Part { colour, ..Default::default() });
        }
        &mut self.parts[i]
    }

    pub fn push(&mut self, colour: [f32; 3], tri: [([f32; 3], [f32; 3]); 3]) {
        let part = self.part(colour);
        for (p, n) in tri {
            part.positions.push(p);
            part.normals.push(n);
        }
    }

    pub fn push_flat(&mut self, colour: [f32; 3], p: [[f32; 3]; 3]) {
        let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
        let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if l < 1e-12 {
            return;
        }
        let n = n.map(|v| v / l);
        self.push(colour, p.map(|q| (q, n)));
    }

    pub fn finish(self) -> Mesh {
        Mesh { parts: self.parts.into_iter().filter(|p| !p.positions.is_empty()).collect() }
    }
}

pub fn load(path: &Path) -> Result<Mesh, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let mesh = if text.starts_with("ISO-10303-21") {
        step::parse(&text)?
    } else if text.starts_with("#VRML") {
        wrl::parse(&text)
    } else {
        return Err(format!("{}: not a STEP or VRML file", path.display()));
    };
    if mesh.parts.is_empty() {
        return Err(format!("{}: no geometry", path.display()));
    }
    Ok(mesh)
}

pub fn relative(model: &str) -> String {
    let rest = match model.find("}/") {
        Some(i) if model.starts_with("${") => &model[i + 2..],
        _ => model,
    };
    rest.trim_start_matches('/').to_string()
}

fn with_ext(rel: &str, ext: &str) -> String {
    let (dir, file) = rel.rsplit_once('/').unwrap_or(("", rel));
    let stem = file.rsplit_once('.').map(|a| a.0).unwrap_or(file);
    if dir.is_empty() { format!("{stem}.{ext}") } else { format!("{dir}/{stem}.{ext}") }
}

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("agentee").join("3dmodels")
}

fn roots(project: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for var in ["KICAD10_3DMODEL_DIR", "KICAD9_3DMODEL_DIR", "KICAD8_3DMODEL_DIR", "KISYS3DMOD"] {
        if let Some(v) = std::env::var_os(var) {
            out.push(PathBuf::from(v));
        }
    }
    out.push(project.join("3dmodels"));
    out.push(cache_dir());
    out.push(PathBuf::from("/usr/share/kicad/3dmodels"));
    out
}

pub fn locate(model: &str, project: &Path) -> Option<PathBuf> {
    let direct = project.join(model);
    if !model.starts_with("${") && direct.is_file() {
        return Some(direct);
    }
    let rel = relative(model);
    let names = [rel.clone(), with_ext(&rel, "step"), with_ext(&rel, "stp"), with_ext(&rel, "wrl")];
    for root in roots(project) {
        for n in &names {
            let p = root.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

const MIRRORS: [&str; 2] = [
    "https://gitlab.com/kicad/libraries/kicad-packages3D/-/raw/master/",
    "https://raw.githubusercontent.com/KiCad/kicad-packages3D/master/",
];

pub fn fetch(model: &str) -> Result<PathBuf, String> {
    let rel = relative(model);
    if rel.starts_with('/') || rel.contains("..") || !rel.contains(".3dshapes/") {
        return Err(format!("{model}: not a KiCad library model"));
    }
    for name in [with_ext(&rel, "step"), with_ext(&rel, "wrl")] {
        for base in MIRRORS {
            let Ok(resp) = ureq::get(&format!("{base}{name}")).call() else { continue };
            if resp.status() != 200 {
                continue;
            }
            let Ok(bytes) = resp.into_body().with_config().limit(64 << 20).read_to_vec() else {
                continue;
            };
            if !(bytes.starts_with(b"ISO-10303-21") || bytes.starts_with(b"#VRML")) {
                continue;
            }
            let path = cache_dir().join(&name);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            return Ok(path);
        }
    }
    Err(format!("{rel}: not found in the KiCad 3D library"))
}

#[derive(Clone)]
pub enum Status {
    Ready(Arc<Mesh>),
    Pending,
    Missing(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fetch {
    Never,
    Background,
    Blocking,
}

#[derive(Default)]
struct Library {
    loaded: HashMap<PathBuf, Result<Arc<Mesh>, String>>,
    fetching: HashMap<String, Option<String>>,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
}

static LIBRARY: OnceLock<Mutex<Library>> = OnceLock::new();
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn library() -> std::sync::MutexGuard<'static, Library> {
    LIBRARY.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

pub fn set_waker(f: impl Fn() + Send + Sync + 'static) {
    library().waker = Some(Arc::new(f));
}

fn load_cached(path: PathBuf) -> Status {
    if let Some(r) = library().loaded.get(&path) {
        return match r {
            Ok(m) => Status::Ready(m.clone()),
            Err(e) => Status::Missing(e.clone()),
        };
    }
    let r = load(&path).map(Arc::new);
    library().loaded.insert(path, r.clone());
    match r {
        Ok(m) => Status::Ready(m),
        Err(e) => Status::Missing(e),
    }
}

pub fn get(model: &str, project: &Path, fetch_mode: Fetch) -> Status {
    if let Some(path) = locate(model, project) {
        return load_cached(path);
    }
    let rel = relative(model);
    if let Some(state) = library().fetching.get(&rel) {
        return match state {
            None => Status::Pending,
            Some(e) => Status::Missing(e.clone()),
        };
    }
    match fetch_mode {
        Fetch::Never => Status::Missing(format!("{rel}: not found locally")),
        Fetch::Blocking => match fetch(model) {
            Ok(path) => load_cached(path),
            Err(e) => {
                library().fetching.insert(rel, Some(e.clone()));
                Status::Missing(e)
            }
        },
        Fetch::Background => {
            library().fetching.insert(rel.clone(), None);
            let model = model.to_string();
            std::thread::spawn(move || {
                let state = match fetch(&model) {
                    Ok(path) => {
                        let _ = load_cached(path);
                        library().fetching.remove(&rel);
                        None
                    }
                    Err(e) => Some(e),
                };
                let waker = {
                    let mut lib = library();
                    if let Some(e) = state {
                        lib.fetching.insert(rel, Some(e));
                    }
                    lib.waker.clone()
                };
                GENERATION.fetch_add(1, Ordering::Relaxed);
                if let Some(w) = waker {
                    w();
                }
            });
            Status::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_paths_drop_the_variable_and_swap_extensions() {
        let m = "${KICAD9_3DMODEL_DIR}/Package_TO_SOT_SMD.3dshapes/SOT-89-3.step";
        assert_eq!(relative(m), "Package_TO_SOT_SMD.3dshapes/SOT-89-3.step");
        assert_eq!(with_ext(&relative(m), "wrl"), "Package_TO_SOT_SMD.3dshapes/SOT-89-3.wrl");
        assert!(fetch("/etc/passwd").is_err());
    }
}
