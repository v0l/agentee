use agentee_core::project::Project;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

struct Run {
    handle: JoinHandle<Result<(), String>>,
    cancel: Arc<AtomicBool>,
    started: Instant,
}

#[derive(Default)]
pub struct Runs {
    running: HashMap<String, Run>,
    failed: HashMap<String, String>,
}

impl Runs {
    pub fn start(&mut self, project: &Project, name: &str) {
        self.failed.remove(name);
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, sim, flag) = (project.clone(), name.to_string(), cancel.clone());
        let handle = std::thread::spawn(move || {
            agentee_sim::runner::with_cancel(flag, || {
                agentee_sim::runner::run(&p, &sim, false, &mut |_, _, _| {}).map(|_| ())
            })
        });
        self.running.insert(name.to_string(), Run { handle, cancel, started: Instant::now() });
    }

    pub fn stop(&mut self, name: &str, pid: Option<u32>) {
        if let Some(run) = self.running.get(name) {
            run.cancel.store(true, Ordering::Relaxed);
            return;
        }
        #[cfg(unix)]
        if let Some(pid) = pid.filter(|p| *p != std::process::id()) {
            let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
        }
        #[cfg(not(unix))]
        let _ = pid;
    }

    pub fn poll(&mut self) {
        let done: Vec<String> = self
            .running
            .iter()
            .filter(|(_, r)| r.handle.is_finished())
            .map(|(n, _)| n.clone())
            .collect();
        for name in done {
            let Some(run) = self.running.remove(&name) else { continue };
            let stopped = run.cancel.load(Ordering::Relaxed);
            match run.handle.join() {
                Ok(Err(e)) if !stopped => {
                    self.failed.insert(name, e);
                }
                Err(_) => {
                    self.failed.insert(name, "the run panicked".into());
                }
                _ => {}
            }
        }
    }

    pub fn starting(&self, name: &str) -> bool {
        self.running.contains_key(name)
    }

    pub fn elapsed(&self, name: &str) -> Option<Duration> {
        self.running.get(name).map(|r| r.started.elapsed())
    }

    pub fn any(&self) -> bool {
        !self.running.is_empty()
    }

    pub fn failure(&self, name: &str) -> Option<&str> {
        self.failed.get(name).map(String::as_str)
    }
}
