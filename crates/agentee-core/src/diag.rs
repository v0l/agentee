use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    pub item: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub at: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        write!(f, "{sev}: ")?;
        if let Some(p) = &self.file {
            write!(f, "{}: ", p.display())?;
        }
        write!(f, "{}", self.item)?;
        if !self.at.is_empty() {
            write!(f, " {}", self.at)?;
        }
        match &self.rule {
            Some(r) => write!(f, ": [{r}] {}", self.message),
            None => write!(f, ": {}", self.message),
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct Diags {
    item: String,
    pub list: Vec<Diagnostic>,
}

impl Diags {
    pub fn new(item: impl Into<String>) -> Self {
        Diags { item: item.into(), list: Vec::new() }
    }

    pub fn push(&mut self, severity: Severity, at: impl Into<String>, message: impl Into<String>) {
        self.list.push(Diagnostic {
            severity,
            file: None,
            item: self.item.clone(),
            at: at.into(),
            message: message.into(),
            rule: None,
        });
    }

    pub fn push_rule(
        &mut self,
        severity: Severity,
        rule: &str,
        at: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.list.push(Diagnostic {
            severity,
            file: None,
            item: self.item.clone(),
            at: at.into(),
            message: message.into(),
            rule: Some(rule.to_string()),
        });
    }

    pub fn error(&mut self, at: impl Into<String>, message: impl Into<String>) {
        self.push(Severity::Error, at, message);
    }

    pub fn warn(&mut self, at: impl Into<String>, message: impl Into<String>) {
        self.push(Severity::Warning, at, message);
    }

    pub fn info(&mut self, at: impl Into<String>, message: impl Into<String>) {
        self.push(Severity::Info, at, message);
    }

    pub fn has_errors(&self) -> bool {
        self.list.iter().any(|d| d.severity == Severity::Error)
    }
}
