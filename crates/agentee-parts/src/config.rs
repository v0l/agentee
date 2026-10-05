use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    pub mouser: Option<MouserConfig>,
    pub farnell: Option<FarnellConfig>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MouserConfig {
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct FarnellConfig {
    #[serde(default)]
    pub api_key: String,
    pub store: Option<String>,
    pub currency: Option<String>,
}

pub fn default_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AGENTEE_DISTRIBUTORS") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("agentee").join("distributors.toml"))
}

pub fn load(path: Option<&Path>) -> Result<Config, String> {
    let path = match path {
        Some(p) => p.to_path_buf(),
        None => default_path().ok_or("no home directory to find distributors.toml in")?,
    };
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "{}: {e}. Write it with [mouser] api_key = \"...\" and/or [farnell] api_key = \"...\", store = \"uk.farnell.com\"",
            path.display()
        )
    })?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

impl Config {
    pub fn mouser_key(&self) -> Option<&str> {
        self.mouser.as_ref().map(|m| m.api_key.trim()).filter(|k| !k.is_empty())
    }

    pub fn farnell_key(&self) -> Option<&str> {
        self.farnell.as_ref().map(|f| f.api_key.trim()).filter(|k| !k.is_empty())
    }

    pub fn farnell_store(&self) -> &str {
        self.farnell.as_ref().and_then(|f| f.store.as_deref()).unwrap_or("uk.farnell.com")
    }
}
