use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

struct Run {
    child: Child,
    log: PathBuf,
    started: std::time::Instant,
}

#[derive(Default)]
pub struct Runs {
    running: HashMap<String, Run>,
    failed: HashMap<String, String>,
}

fn log_path(name: &str) -> PathBuf {
    let safe: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    std::env::temp_dir().join(format!("agentee-sim-{safe}.log"))
}

impl Runs {
    pub fn start(&mut self, project: &Path, name: &str) {
        self.failed.remove(name);
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => {
                self.failed.insert(name.to_string(), e.to_string());
                return;
            }
        };
        let log = log_path(name);
        let stderr = match std::fs::File::create(&log) {
            Ok(f) => Stdio::from(f),
            Err(_) => Stdio::null(),
        };
        let spawned = Command::new(exe)
            .arg("sim")
            .arg(name)
            .arg("--project")
            .arg(project)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn();
        match spawned {
            Ok(child) => {
                self.running.insert(
                    name.to_string(),
                    Run { child, log, started: std::time::Instant::now() },
                );
            }
            Err(e) => {
                self.failed.insert(name.to_string(), e.to_string());
            }
        }
    }

    pub fn stop(&mut self, name: &str, pid: Option<u32>) {
        if let Some(mut run) = self.running.remove(name) {
            let _ = run.child.kill();
            let _ = run.child.wait();
            return;
        }
        #[cfg(unix)]
        if let Some(pid) = pid {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        #[cfg(not(unix))]
        let _ = pid;
    }

    pub fn poll(&mut self) {
        let mut done = Vec::new();
        for (name, run) in self.running.iter_mut() {
            if let Ok(Some(status)) = run.child.try_wait() {
                done.push((name.clone(), status.success(), run.log.clone()));
            }
        }
        for (name, ok, log) in done {
            self.running.remove(&name);
            if !ok {
                let text = std::fs::read_to_string(&log).unwrap_or_default();
                let last = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed");
                self.failed.insert(name, last.to_string());
            }
        }
    }

    pub fn starting(&self, name: &str) -> bool {
        self.running.contains_key(name)
    }

    pub fn elapsed(&self, name: &str) -> Option<std::time::Duration> {
        self.running.get(name).map(|r| r.started.elapsed())
    }

    pub fn any(&self) -> bool {
        !self.running.is_empty()
    }

    pub fn failure(&self, name: &str) -> Option<&str> {
        self.failed.get(name).map(String::as_str)
    }
}
