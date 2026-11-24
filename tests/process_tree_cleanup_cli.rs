use std::{
    fs,
    process::Command,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
#[test]
fn timeout_terminates_spawned_child_process_tree() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "soro-process-tree-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "pub struct Address; impl Address { pub fn require_auth(&self) {} } pub fn check(user: Address) { user.require_auth(); }\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test")
        .arg(&root)
        .args([
            "--operator",
            "AUTH-001",
            "--test-command",
            "sh -c 'sleep 30 & echo $! > child.pid; wait'",
            "--test-timeout",
            "1s",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let pid_file = root.join("child.pid");
    assert!(pid_file.exists(), "child pid should be written before timeout");
    let pid = fs::read_to_string(&pid_file).unwrap();
    let pid = pid.trim();
    thread::sleep(Duration::from_millis(100));
    let alive = Command::new("kill")
        .args(["-0", pid])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    assert!(!alive, "child process {pid} survived timeout cleanup");

    let _ = fs::remove_dir_all(root);
}

#[cfg(not(unix))]
#[test]
fn process_tree_cleanup_platform_limitation_is_explicit() {
    let docs = fs::read_to_string("docs/process-execution.md").unwrap();
    assert!(docs.contains("Windows"));
}
