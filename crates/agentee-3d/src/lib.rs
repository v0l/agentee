pub mod export;
pub mod parametric;
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

fn gunzip(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(bytes);
    }
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&bytes[..]), &mut out)
        .map_err(|e| format!("gzip: {e}"))?;
    Ok(out)
}

pub fn load(path: &Path) -> Result<Mesh, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = gunzip(bytes).map_err(|e| format!("{}: {e}", path.display()))?;
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

pub fn is_url(model: &str) -> bool {
    model.starts_with("https://") || model.starts_with("http://")
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn url_dir(url: &str, cache: &Path) -> PathBuf {
    cache.join("url").join(format!("{:016x}", fnv(url)))
}

fn model_kind(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    let lower = lower.strip_suffix(".gz").unwrap_or(&lower);
    ["step", "stp", "wrl"].into_iter().find(|e| lower.ends_with(&format!(".{e}")))
}

fn url_cached(url: &str, cache: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(url_dir(url, cache))
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file() && p.file_name().and_then(|n| n.to_str()).and_then(model_kind).is_some()
        })
        .collect();
    files.sort();
    files.into_iter().next()
}

fn fetch_url(url: &str, cache: &Path) -> Result<PathBuf, String> {
    let resp = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
    if resp.status() != 200 {
        return Err(format!("{url}: HTTP {}", resp.status()));
    }
    let raw = resp
        .into_body()
        .with_config()
        .limit(256 << 20)
        .read_to_vec()
        .map_err(|e| format!("{url}: {e}"))?;
    let plain = gunzip(raw.clone()).map_err(|e| format!("{url}: {e}"))?;
    let kind = if plain.starts_with(b"ISO-10303-21") {
        "step"
    } else if plain.starts_with(b"#VRML") {
        "wrl"
    } else {
        return Err(format!("{url}: not a STEP or VRML file"));
    };
    let base = url.split(['?', '#']).next().unwrap_or(url);
    let stem: String = base
        .rsplit('/')
        .next()
        .unwrap_or("")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' })
        .collect();
    let name = match model_kind(&stem) {
        Some(_) if !stem.starts_with('.') => stem,
        _ => format!("model.{kind}"),
    };
    let dir = url_dir(url, cache);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(&name);
    let part = dir.join(format!("{name}.part"));
    std::fs::write(&part, &raw).map_err(|e| format!("{}: {e}", part.display()))?;
    std::fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

pub fn in_project(model: &str, project: &Path) -> Option<PathBuf> {
    if is_url(model) {
        return Some(
            url_cached(model, &cache_dir()).unwrap_or_else(|| url_dir(model, &cache_dir())),
        );
    }
    let direct = project.join(model);
    if !model.starts_with("${") && direct.is_file() {
        return Some(direct);
    }
    let rel = relative(model);
    [rel.clone(), with_ext(&rel, "step"), with_ext(&rel, "stp"), with_ext(&rel, "wrl")]
        .into_iter()
        .map(|n| project.join("3dmodels").join(n))
        .find(|p| p.is_file())
}

pub fn locate(model: &str, project: &Path) -> Option<PathBuf> {
    if is_url(model) {
        return url_cached(model, &cache_dir());
    }
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
    if is_url(model) {
        return fetch_url(model, &cache_dir());
    }
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
    loaded: HashMap<PathBuf, (Stamp, Result<Arc<Mesh>, String>)>,
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

type Stamp = Option<(std::time::SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn load_cached(path: PathBuf) -> Status {
    let now = stamp(&path);
    if let Some((was, r)) = library().loaded.get(&path)
        && *was == now
    {
        return match r {
            Ok(m) => Status::Ready(m.clone()),
            Err(e) => Status::Missing(e.clone()),
        };
    }
    let r = load(&path).map(Arc::new);
    library().loaded.insert(path, (now, r.clone()));
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

    fn quad_wrl(quads: usize) -> String {
        let mut s = String::from("#VRML V2.0 utf8\n");
        for i in 0..quads {
            let z = i as f32;
            s += &format!(
                "Shape {{ geometry IndexedFaceSet {{ coord Coordinate {{ point [ 0 0 {z}, 1 0 {z}, 1 1 {z}, 0 1 {z} ] }} coordIndex [ 0 1 2 3 -1 ] }} }}\n"
            );
        }
        s
    }

    fn serve_once(body: Vec<u8>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            s.write_all(head.as_bytes()).unwrap();
            s.write_all(&body).unwrap();
        });
        format!("http://127.0.0.1:{port}/models/Part%20One.wrl?raw=1")
    }

    #[test]
    fn a_url_model_is_downloaded_once_into_the_cache_and_loads() {
        let cache = std::env::temp_dir().join(format!("agentee-3d-url-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        let url = serve_once(quad_wrl(2).into_bytes());
        assert!(url_cached(&url, &cache).is_none());
        let path = fetch_url(&url, &cache).unwrap();
        assert_eq!(url_cached(&url, &cache), Some(path.clone()));
        assert_eq!(path.file_name().unwrap(), "Part_20One.wrl");
        let tris = load(&path).unwrap().triangles();
        let _ = std::fs::remove_dir_all(&cache);
        assert_eq!(tris, 4);
    }

    #[test]
    fn a_url_that_is_not_a_model_is_refused() {
        let cache = std::env::temp_dir().join(format!("agentee-3d-url-bad-{}", std::process::id()));
        let url = serve_once(b"<html>Not Found</html>".to_vec());
        let err = fetch_url(&url, &cache).unwrap_err();
        let _ = std::fs::remove_dir_all(&cache);
        assert!(err.contains("not a STEP or VRML"), "{err}");
        assert!(url_cached(&url, &cache).is_none());
    }

    #[test]
    fn a_gzipped_vrml_loads() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("agentee-3d-gz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("part.wrl");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(quad_wrl(1).as_bytes()).unwrap();
        std::fs::write(&path, gz.finish().unwrap()).unwrap();
        let tris = load(&path).map(|m| m.triangles());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(tris, Ok(2));
    }

    #[test]
    fn a_model_rewritten_on_disk_is_loaded_again() {
        let dir = std::env::temp_dir().join(format!("agentee-3d-reload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("part.wrl");
        let triangles = || match load_cached(path.clone()) {
            Status::Ready(m) => m.triangles(),
            _ => panic!("model did not load"),
        };
        std::fs::write(&path, quad_wrl(1)).unwrap();
        let first = triangles();
        std::fs::write(&path, quad_wrl(3)).unwrap();
        let second = triangles();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(second > first, "{first} then {second} triangles");
    }
}
