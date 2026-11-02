use std::path::PathBuf;

pub struct Frozen(pub egui_bench::revision::Snapshot);

impl agentee_core::vfs::Overlay for Frozen {
    fn covers(&self, path: &std::path::Path) -> bool {
        self.0.covers(path)
    }
    fn read(&self, path: &std::path::Path) -> std::io::Result<Vec<u8>> {
        self.0.read(path)
    }
    fn is_file(&self, path: &std::path::Path) -> bool {
        self.0.is_file(path)
    }
    fn is_dir(&self, path: &std::path::Path) -> bool {
        self.0.is_dir(path)
    }
    fn read_dir(&self, path: &std::path::Path) -> std::io::Result<Vec<(PathBuf, bool)>> {
        self.0.read_dir(path)
    }
    fn canonicalize(&self, path: &std::path::Path) -> std::io::Result<PathBuf> {
        self.0.canonicalize(path)
    }
    fn id(&self) -> u64 {
        self.0.short().bytes().fold(0, |h, b| h * 31 + b as u64)
    }
}
