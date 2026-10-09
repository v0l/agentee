use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

const BROWSER: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";
const META: &str = "meta.json";

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Copy {
    pub sha256: String,
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub source: String,
    pub fetched: String,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pinned: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    pub mpn: String,
    pub copies: Vec<Copy>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub dir: PathBuf,
    #[serde(flatten)]
    pub meta: Meta,
}

impl Entry {
    pub fn current(&self) -> Option<&Copy> {
        self.meta.copies.last()
    }

    pub fn by_hash(&self, sha: &str) -> Option<&Copy> {
        self.meta.copies.iter().find(|c| c.sha256.starts_with(sha))
    }

    pub fn pdf(&self, c: &Copy) -> PathBuf {
        self.dir.join(&c.file)
    }
}

pub struct Cache {
    pub dir: PathBuf,
}

pub fn default_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AGENTEE_DATASHEETS") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("agentee").join("datasheets"))
}

pub fn normal(mpn: &str) -> String {
    mpn.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase()
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '+') {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() || out.starts_with('.') { format!("_{out}") } else { out }
}

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

impl Cache {
    pub fn open(dir: Option<&Path>) -> Result<Cache, String> {
        let dir = match dir {
            Some(d) => d.to_path_buf(),
            None => default_dir().ok_or("no home directory for the datasheet cache")?,
        };
        Ok(Cache { dir })
    }

    fn read(dir: &Path) -> Option<Entry> {
        let text = std::fs::read_to_string(dir.join(META)).ok()?;
        let meta: Meta = serde_json::from_str(&text).ok()?;
        Some(Entry { dir: dir.to_path_buf(), meta })
    }

    fn write(e: &Entry) -> Result<(), String> {
        let path = e.dir.join(META);
        let text = serde_json::to_string_pretty(&e.meta).map_err(|x| x.to_string())?;
        std::fs::write(&path, text).map_err(|x| format!("{}: {x}", path.display()))
    }

    pub fn entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        let Ok(makers) = std::fs::read_dir(&self.dir) else { return out };
        let mut makers: Vec<_> = makers.filter_map(Result::ok).map(|e| e.path()).collect();
        makers.sort();
        for m in makers.into_iter().filter(|m| m.is_dir()) {
            let Ok(parts) = std::fs::read_dir(&m) else { continue };
            let mut parts: Vec<_> = parts.filter_map(Result::ok).map(|e| e.path()).collect();
            parts.sort();
            out.extend(parts.iter().filter_map(|p| Self::read(p)));
        }
        out
    }

    pub fn find(&self, mpn: &str) -> Vec<Entry> {
        let want = normal(mpn);
        if want.is_empty() {
            return Vec::new();
        }
        let all: Vec<Entry> =
            self.entries().into_iter().filter(|e| !e.meta.copies.is_empty()).collect();
        let exact: Vec<Entry> =
            all.iter().filter(|e| normal(&e.meta.mpn) == want).cloned().collect();
        if !exact.is_empty() {
            return exact;
        }
        let mut near: Vec<Entry> = all
            .into_iter()
            .filter(|e| {
                let have = normal(&e.meta.mpn);
                have.len().min(want.len()) >= 5
                    && (want.starts_with(&have) || have.starts_with(&want))
            })
            .collect();
        near.sort_by_key(|e| std::cmp::Reverse(normal(&e.meta.mpn).len()));
        near
    }

    pub fn by_hash(&self, sha: &str) -> Option<(Entry, Copy)> {
        if sha.len() < 8 {
            return None;
        }
        self.entries().into_iter().find_map(|e| {
            let c = e.by_hash(sha)?.clone();
            Some((e, c))
        })
    }

    pub fn store(
        &self,
        manufacturer: Option<&str>,
        mpn: &str,
        bytes: &[u8],
        url: Option<&str>,
        source: &str,
    ) -> Result<(Entry, Copy), String> {
        if !bytes.starts_with(b"%PDF") {
            return Err("not a PDF".into());
        }
        let maker = manufacturer.map(slug).unwrap_or_else(|| "_".into());
        let dir = self.dir.join(maker).join(slug(mpn));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut e = Self::read(&dir).unwrap_or_else(|| Entry {
            dir: dir.clone(),
            meta: Meta {
                manufacturer: manufacturer.map(str::to_string),
                mpn: mpn.to_string(),
                copies: Vec::new(),
            },
        });
        let sha = sha256(bytes);
        let file = format!("{}.pdf", &sha[..16]);
        let path = dir.join(&file);
        if !path.exists() {
            std::fs::write(&path, bytes).map_err(|x| format!("{}: {x}", path.display()))?;
        }
        let mut copy = match e.meta.copies.iter().position(|c| c.sha256 == sha) {
            Some(i) => e.meta.copies.remove(i),
            None => Copy { sha256: sha.clone(), file, ..Default::default() },
        };
        copy.url = url.map(str::to_string).or(copy.url);
        copy.source = source.to_string();
        copy.fetched = today();
        copy.bytes = bytes.len() as u64;
        if copy.revision.is_none() {
            copy.revision = text(&path).ok().and_then(|p| revision(&p));
        }
        e.meta.copies.push(copy.clone());
        Self::write(&e)?;
        Ok((e, copy))
    }

    pub fn pin(&self, sha: &str, by: &str, pinned: bool) -> Result<(), String> {
        let (mut e, _) = self.by_hash(sha).ok_or_else(|| format!("no cached datasheet {sha}"))?;
        for c in e.meta.copies.iter_mut().filter(|c| c.sha256.starts_with(sha)) {
            c.pinned.retain(|p| p != by);
            if pinned {
                c.pinned.push(by.to_string());
            }
        }
        Self::write(&e)
    }

    pub fn purge(&self) -> Result<Purged, String> {
        let mut out = Purged::default();
        for mut e in self.entries() {
            let (keep, drop): (Vec<Copy>, Vec<Copy>) =
                e.meta.copies.drain(..).partition(|c| !c.pinned.is_empty());
            for c in &drop {
                for f in [e.pdf(c), e.pdf(c).with_extension("txt")] {
                    if let Ok(m) = std::fs::metadata(&f) {
                        out.bytes += m.len();
                        let _ = std::fs::remove_file(&f);
                    }
                }
                out.removed += 1;
            }
            out.kept += keep.len();
            e.meta.copies = keep;
            if e.meta.copies.is_empty() {
                let _ = std::fs::remove_dir_all(&e.dir);
            } else {
                Self::write(&e)?;
            }
        }
        Ok(out)
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Purged {
    pub removed: usize,
    pub kept: usize,
    pub bytes: u64,
}

pub fn pdftotext(pdf: &Path) -> Result<String, String> {
    let out = std::process::Command::new("pdftotext")
        .args(["-layout", "-enc", "UTF-8"])
        .arg(pdf)
        .arg("-")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "pdftotext is not installed: install poppler-utils (apt, dnf) or poppler (brew)"
                    .to_string()
            } else {
                format!("pdftotext: {e}")
            }
        })?;
    if !out.status.success() {
        return Err(format!(
            "pdftotext {}: {}",
            pdf.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn text(pdf: &Path) -> Result<Vec<String>, String> {
    let cached = pdf.with_extension("txt");
    let raw = match std::fs::read_to_string(&cached) {
        Ok(t) => t,
        Err(_) => {
            let t = pdftotext(pdf)?;
            let _ = std::fs::write(&cached, &t);
            t
        }
    };
    let mut pages: Vec<String> = raw.split('\u{c}').map(str::to_string).collect();
    while pages.last().is_some_and(|p| p.trim().is_empty()) {
        pages.pop();
    }
    Ok(pages)
}

pub fn revision(pages: &[String]) -> Option<String> {
    let first = pages.iter().take(2);
    let last = pages.iter().rev().take(2);
    for page in first.chain(last) {
        for line in page.lines() {
            let l = line.to_lowercase();
            if l.contains("history") || l.contains(". . .") || l.contains("....") {
                continue;
            }
            let hit = l.contains("revised")
                || l.contains("revision")
                || l.split(|c: char| !c.is_alphanumeric()).any(|w| w == "rev");
            if hit && line.chars().any(|c| c.is_ascii_digit()) {
                let t = line.split_whitespace().collect::<Vec<_>>().join(" ");
                if t.len() <= 120 {
                    return Some(t);
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Passage {
    pub page: usize,
    pub line: usize,
    pub text: String,
}

fn hits(line: &str, needle: &str, fold: bool, word: bool) -> bool {
    let (hay, needle) = if fold {
        (line.to_lowercase(), needle.to_lowercase())
    } else {
        (line.to_string(), needle.to_string())
    };
    if !word {
        return hay.contains(&needle);
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    hay.match_indices(&needle).any(|(i, m)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + m.len()..].chars().next();
        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    })
}

pub fn find(pages: &[String], needle: &str, context: usize) -> Vec<Passage> {
    let needle = needle.trim();
    if needle.is_empty() {
        return Vec::new();
    }
    let fold = !needle.chars().any(char::is_uppercase);
    let word = needle.chars().all(|c| c.is_alphanumeric() || c == '_');
    let mut out = Vec::new();
    for (p, page) in pages.iter().enumerate() {
        let lines: Vec<&str> = page.lines().collect();
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if !hits(l, needle, fold, word) {
                continue;
            }
            let (a, b) = (i.saturating_sub(context), (i + context + 1).min(lines.len()));
            match spans.last_mut() {
                Some(last) if a <= last.1 => last.1 = b,
                _ => spans.push((a, b)),
            }
        }
        for (a, b) in spans {
            let mut text: Vec<&str> = lines[a..b].to_vec();
            while text.first().is_some_and(|l| l.trim().is_empty()) {
                text.remove(0);
            }
            while text.last().is_some_and(|l| l.trim().is_empty()) {
                text.pop();
            }
            out.push(Passage { page: p + 1, line: a + 1, text: text.join("\n") });
        }
    }
    out
}

fn title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let a = lower.find("<title")?;
    let a = a + lower[a..].find('>')? + 1;
    let b = a + lower[a..].find("</title")?;
    let t = html[a..b].split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then_some(t)
}

pub fn with_scheme(url: &str) -> String {
    let u = url.trim().replace("&amp;", "&");
    let u = u.as_str();
    if u.starts_with("http://") || u.starts_with("https://") {
        u.to_string()
    } else if let Some(rest) = u.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        format!("https://{u}")
    }
}

pub fn is_url(s: &str) -> bool {
    let s = s.trim();
    if s.starts_with("http://") || s.starts_with("https://") {
        return true;
    }
    let host = s.split('/').next().unwrap_or("");
    s.contains('/') && host.contains('.') && !host.contains(' ') && !s.starts_with('.')
}

pub struct Fetcher {
    agent: ureq::Agent,
    pub tries: u32,
    stalled: std::sync::Mutex<std::collections::HashSet<String>>,
}

fn host(url: &str) -> &str {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or("")
}

impl Default for Fetcher {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(90)))
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .max_redirects(10)
            .build()
            .into();
        Fetcher { agent, tries: 3, stalled: Default::default() }
    }
}

fn pdf_link(html: &str, base: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find("href=") {
        let start = at + i + 5;
        let quote = html[start..].chars().next()?;
        let body = if quote == '"' || quote == '\'' { &html[start + 1..] } else { &html[start..] };
        let end = body.find(|c: char| c == quote || c == '>' || c.is_whitespace())?;
        let href = body[..end].replace("&amp;", "&");
        at = start + end;
        if href.to_lowercase().split(['?', '#']).next().is_some_and(|p| p.ends_with(".pdf")) {
            if href.starts_with("http") {
                return Some(href);
            }
            let origin: String = base.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
            return Some(if href.starts_with("//") {
                format!("https:{href}")
            } else if href.starts_with('/') {
                format!("{origin}{href}")
            } else {
                let dir = base.rsplit_once('/').map(|(d, _)| d).unwrap_or(base);
                format!("{dir}/{href}")
            });
        }
    }
    None
}

impl Fetcher {
    fn once(&self, url: &str) -> Result<(u16, String, Vec<u8>, Option<u64>), String> {
        let mut resp = self
            .agent
            .get(url)
            .header("User-Agent", BROWSER)
            .header("Accept", "application/pdf,text/html;q=0.9,*/*;q=0.8")
            .header("Accept-Language", "en-US,en;q=0.9")
            .call()
            .map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let kind = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let retry = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok());
        let body = resp
            .body_mut()
            .with_config()
            .limit(200 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| e.to_string())?;
        Ok((status, kind, body, retry))
    }

    pub fn pdf(&self, url: &str) -> Result<Vec<u8>, String> {
        let url = with_scheme(url);
        if self.stalled.lock().unwrap().contains(host(&url)) {
            return Err(format!("{url}: skipped, {} timed out earlier", host(&url)));
        }
        let mut last = String::new();
        for attempt in 0..self.tries.max(1) {
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs(2 * u64::from(attempt)));
            }
            match self.once(&url) {
                Ok((200, _, body, _)) if body.starts_with(b"%PDF") => return Ok(body),
                Ok((200, kind, body, _)) => {
                    let html = String::from_utf8_lossy(&body);
                    if kind.contains("html") || html.trim_start().starts_with('<') {
                        if let Some(link) = pdf_link(&html, &url)
                            && let Ok((200, _, b, _)) = self.once(&link)
                            && b.starts_with(b"%PDF")
                        {
                            return Ok(b);
                        }
                        return Err(match title(&html) {
                            Some(t) => format!("{url}: an HTML page, not a PDF ({t})"),
                            None => format!("{url}: an HTML page, not a PDF"),
                        });
                    }
                    let what = if body.starts_with(b"PK") { "a zip" } else { "not a PDF" };
                    return Err(format!("{url}: {what}"));
                }
                Ok((status @ (429 | 500..=599), _, _, retry)) => {
                    last = format!("{url}: HTTP {status}");
                    if let Some(s) = retry.filter(|&s| s <= 20) {
                        std::thread::sleep(Duration::from_secs(s));
                    }
                }
                Ok((status, _, _, _)) => return Err(format!("{url}: HTTP {status}")),
                Err(e) if e.contains("timeout") => {
                    self.stalled.lock().unwrap().insert(host(&url).to_string());
                    return Err(format!("{url}: {e}"));
                }
                Err(e) => last = format!("{url}: {e}"),
            }
        }
        Err(last)
    }
}
