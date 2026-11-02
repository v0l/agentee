use agentee_core::project::Project;
use agentee_view::revision::Frozen;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?}");
}

fn board(name: &str) -> String {
    let demo =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo/sensor-node.board.toml");
    std::fs::read_to_string(demo).unwrap().replacen("sensor-node", name, 1)
}

#[test]
fn a_project_loads_as_committed_while_the_files_on_disk_have_moved_on() {
    let dir = std::env::temp_dir().join(format!("agentee_revision_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    git(&dir, &["init", "-q"]);
    std::fs::write(dir.join("old.board.toml"), board("ordered")).unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-qm", "ordered"]);
    std::fs::remove_file(dir.join("old.board.toml")).unwrap();
    std::fs::write(dir.join("new.board.toml"), board("reworked")).unwrap();

    let names = |p: &Project| p.boards.iter().map(|b| b.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&Project::load(&dir).unwrap()), ["reworked"]);

    let revs = egui_bench::revision::list(&dir, 5).unwrap();
    let snapshot = egui_bench::revision::Snapshot::open(&dir, &revs[0].sha).unwrap();
    agentee_core::vfs::set(Some(Arc::new(Frozen(snapshot))));
    let at_commit = Project::load(&dir).unwrap();
    let write = agentee_core::vfs::write(&dir.join("old.board.toml"), "x");
    agentee_core::vfs::set(None);

    assert_eq!(names(&at_commit), ["ordered"]);
    assert!(write.is_err());
    assert!(!dir.join("old.board.toml").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
