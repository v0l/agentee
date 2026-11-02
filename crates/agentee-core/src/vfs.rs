use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

pub trait Overlay: Send + Sync {
    fn covers(&self, path: &Path) -> bool;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn is_file(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
    fn read_dir(&self, path: &Path) -> io::Result<Vec<(PathBuf, bool)>>;
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    fn id(&self) -> u64;
}

static ACTIVE: RwLock<Option<Arc<dyn Overlay>>> = RwLock::new(None);

pub fn set(overlay: Option<Arc<dyn Overlay>>) {
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = overlay;
}

fn covering(path: &Path) -> Option<Arc<dyn Overlay>> {
    ACTIVE.read().unwrap_or_else(|e| e.into_inner()).clone().filter(|o| o.covers(path))
}

pub fn frozen(path: &Path) -> bool {
    covering(path).is_some()
}

pub fn read_to_string(path: &Path) -> io::Result<String> {
    match covering(path) {
        Some(o) => String::from_utf8(o.read(path)?)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        None => std::fs::read_to_string(path),
    }
}

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    match covering(path) {
        Some(o) => o.read(path),
        None => std::fs::read(path),
    }
}

pub fn is_file(path: &Path) -> bool {
    match covering(path) {
        Some(o) => o.is_file(path),
        None => path.is_file(),
    }
}

pub fn is_dir(path: &Path) -> bool {
    match covering(path) {
        Some(o) => o.is_dir(path),
        None => path.is_dir(),
    }
}

pub fn exists(path: &Path) -> bool {
    is_file(path) || is_dir(path)
}

pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    match covering(path) {
        Some(o) => o.canonicalize(path),
        None => std::fs::canonicalize(path),
    }
}

pub fn read_dir(path: &Path) -> io::Result<Vec<(PathBuf, bool)>> {
    match covering(path) {
        Some(o) => o.read_dir(path),
        None => Ok(std::fs::read_dir(path)?
            .filter_map(Result::ok)
            .map(|e| {
                let p = e.path();
                let dir = p.is_dir();
                (p, dir)
            })
            .collect()),
    }
}

pub fn stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    match covering(path) {
        Some(o) => {
            let bytes = o.read(path).ok()?;
            Some((
                std::time::UNIX_EPOCH + std::time::Duration::from_nanos(o.id()),
                bytes.len() as u64,
            ))
        }
        None => {
            let meta = std::fs::metadata(path).ok()?;
            Some((meta.modified().ok()?, meta.len()))
        }
    }
}

pub fn write(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    if frozen(path) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "an earlier revision is read only",
        ));
    }
    std::fs::write(path, contents)
}
