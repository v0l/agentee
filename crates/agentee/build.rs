use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-changed=../../crates");
    let mut watched = vec!["HEAD".to_string(), "index".to_string(), "packed-refs".to_string()];
    if let Some(head) = git(&["rev-parse", "--symbolic-full-name", "HEAD"])
        && head.starts_with("refs/")
    {
        watched.push(head);
    }
    for w in &watched {
        if let Some(path) = git(&["rev-parse", "--git-path", w]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let id = match git(&["rev-parse", "--short", "HEAD"]) {
        Some(hash) if !hash.is_empty() => {
            let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|s| !s.is_empty());
            if dirty { format!("{hash}-dirty") } else { hash }
        }
        _ => "unknown".to_string(),
    };
    println!("cargo:rustc-env=AGENTEE_BUILD_ID={id}");
}
