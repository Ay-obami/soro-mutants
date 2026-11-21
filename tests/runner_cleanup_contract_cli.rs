use std::{fs, path::{Path, PathBuf}, process::Command, time::{SystemTime, UNIX_EPOCH}};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("soro-runner-cleanup-{}-{nonce}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "fn check(user: Address) { user.require_auth(); }\n").unwrap();
        fs::write(root.join("source-sentinel"), "unchanged").unwrap();
        Self(root)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

fn run(root: &Path, test_command: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test").arg(root)
        .args(["--operator", "AUTH-001", "--test-command", test_command, "--timeout", "3"])
        .output().expect("test runner should launch")
}

fn verify_cleanup(root: &Path) {
    assert!(!root.join(".soro-mutants-worktree").exists(), "scratch worktree must be removed");
    assert_eq!(fs::read_to_string(root.join("source-sentinel")).unwrap(), "unchanged");
    assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), "fn check(user: Address) { user.require_auth(); }\n");
}

#[test]
fn success_removes_scratch_but_keeps_reusable_cache() {
    let fixture = Fixture::new();
    let output = run(fixture.path(), "true");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    verify_cleanup(fixture.path());
    assert!(fixture.path().join(".soro-mutants-target").exists(), "reusable cache should remain");
}

#[test]
fn baseline_failure_removes_scratch_and_preserves_source() {
    let fixture = Fixture::new();
    let output = run(fixture.path(), "false");
    assert_eq!(output.status.code(), Some(3), "{}", String::from_utf8_lossy(&output.stderr));
    verify_cleanup(fixture.path());
}

#[cfg(unix)]
#[test]
fn baseline_timeout_removes_scratch_and_preserves_source() {
    let fixture = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test").arg(fixture.path())
        .args(["--operator", "AUTH-001", "--test-command", "sleep 2", "--test-timeout", "0"])
        .output().unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", String::from_utf8_lossy(&output.stderr));
    verify_cleanup(fixture.path());
}
