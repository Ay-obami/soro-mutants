use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("soro-cache-{prefix}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_test(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test")
        .arg(root)
        .args(args)
        .output()
        .expect("soro-mutants should run")
}

#[test]
fn cache_creates_identity_file_and_artifact_markers() {
    let temp = TempDir::new("markers");
    let target = temp.path().join("target");
    let fixture = Path::new("fixtures/weak-auth");

    let output = run_test(fixture, &[
        "--target-dir", target.to_str().unwrap(),
        "--test-command", "true",
        "--operator", "AUTH-001",
    ]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    let identity_path = target.join("cache-identity.json");
    assert!(identity_path.exists(), "cache-identity.json should be created");

    let baseline_marker = target.join("baseline/.soro-baseline-artifact");
    let mutant_marker = target.join("mutants-shared/.soro-mutant-artifact");
    assert!(baseline_marker.exists(), "baseline artifact marker should exist");
    assert!(mutant_marker.exists(), "mutant artifact marker should exist");
}

#[test]
fn cache_invalidates_when_test_command_changes() {
    let temp = TempDir::new("test-cmd");
    let target = temp.path().join("target");
    let fixture = Path::new("fixtures/weak-auth");

    let out1 = run_test(fixture, &[
        "--target-dir", target.to_str().unwrap(),
        "--test-command", "true",
        "--operator", "AUTH-001",
    ]);
    assert!(out1.status.success());

    // Place a sentinel file in mutants-shared
    let sentinel = target.join("mutants-shared/sentinel.txt");
    fs::write(&sentinel, "data").unwrap();
    assert!(sentinel.exists());

    // Run again with different test-command
    let out2 = run_test(fixture, &[
        "--target-dir", target.to_str().unwrap(),
        "--test-command", "echo custom_test",
        "--operator", "AUTH-001",
    ]);
    assert!(out2.status.success());

    // Sentinel should have been removed due to cache invalidation!
    assert!(!sentinel.exists(), "stale build artifacts should be invalidated when test command changes");
}

#[test]
fn cache_invalidates_when_operator_changes() {
    let temp = TempDir::new("op-change");
    let target = temp.path().join("target");
    let fixture = Path::new("fixtures/weak-auth");

    let out1 = run_test(fixture, &[
        "--target-dir", target.to_str().unwrap(),
        "--test-command", "true",
        "--operator", "AUTH-001",
    ]);
    assert!(out1.status.success());

    let sentinel = target.join("mutants-shared/sentinel.txt");
    fs::write(&sentinel, "data").unwrap();
    assert!(sentinel.exists());

    // Run again with different operator
    let out2 = run_test(fixture, &[
        "--target-dir", target.to_str().unwrap(),
        "--test-command", "true",
        "--operator", "AUTH-002",
    ]);
    assert!(out2.status.success());

    assert!(!sentinel.exists(), "stale build artifacts should be invalidated when operator changes");
}

#[test]
fn cache_invalidates_when_workspace_changes() {
    let temp = TempDir::new("ws-change");
    let target = temp.path().join("target");
    let fixture1 = Path::new("fixtures/weak-auth");
    let fixture2 = Path::new("fixtures/weak-ttl-lifecycle");

    let out1 = run_test(fixture1, &[
        "--target-dir", target.to_str().unwrap(),
        "--test-command", "true",
    ]);
    assert!(out1.status.success());

    let sentinel = target.join("mutants-shared/sentinel.txt");
    fs::write(&sentinel, "data").unwrap();
    assert!(sentinel.exists());

    // Run pointing the same target-dir to a different workspace
    let out2 = run_test(fixture2, &[
        "--target-dir", target.to_str().unwrap(),
        "--test-command", "true",
    ]);
    assert!(out2.status.success());

    assert!(!sentinel.exists(), "stale cache should be invalidated when workspace changes");
}

#[test]
fn artifact_marker_contamination_triggers_invalidation() {
    let temp = TempDir::new("contamination");
    let target = temp.path().join("target");
    let fixture = Path::new("fixtures/weak-auth");

    let out1 = run_test(fixture, &[
        "--target-dir", target.to_str().unwrap(),
        "--test-command", "true",
        "--operator", "AUTH-001",
    ]);
    assert!(out1.status.success());

    // Simulate cross-contamination by dropping a mutant marker in baseline
    let contaminated_marker = target.join("baseline/.soro-mutant-artifact");
    fs::write(&contaminated_marker, "leak").unwrap();
    let baseline_canary = target.join("baseline/canary.txt");
    fs::write(&baseline_canary, "canary").unwrap();

    // Re-run
    let out2 = run_test(fixture, &[
        "--target-dir", target.to_str().unwrap(),
        "--test-command", "true",
        "--operator", "AUTH-001",
    ]);
    assert!(out2.status.success());

    assert!(!baseline_canary.exists(), "contaminated baseline should be purged");
}
