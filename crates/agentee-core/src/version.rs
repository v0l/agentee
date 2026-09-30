use std::sync::OnceLock;

static BUILD: OnceLock<(String, String)> = OnceLock::new();

pub fn set_build(version: &str, id: &str) {
    let _ = BUILD.set((version.to_string(), id.to_string()));
}

pub fn watermark() -> String {
    match BUILD.get() {
        Some((version, id)) => format!("agentee v{version}-{id}"),
        None => format!("agentee v{}-unknown", env!("CARGO_PKG_VERSION")),
    }
}
