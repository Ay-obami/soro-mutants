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
        let path = std::env::temp_dir().join(format!(
            "soro-resume-{prefix}-{}-{nonce}",
            std::process::id()
        ));
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
    use std::sync::{Mutex, OnceLock};

    static COMMAND_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = COMMAND_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("resumable-run command lock");

    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test")
        .arg(root)
        .args(args)
        .output()
        .expect("soro-mutants test should run")
}

#[test]
fn test_creates_progress_file_and_updates_completed_mutants() {
    let temp = TempDir::new("progress");
    let target = temp.path().join("target");
    let progress_file = target.join("progress.json");
    let fixture = Path::new("fixtures/weak-auth");

    let out = run_test(
        fixture,
        &[
            "--target-dir",
            target.to_str().unwrap(),
            "--test-command",
            "true",
            "--operator",
            "AUTH-001",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(progress_file.exists(), "progress file should be created");

    let content = fs::read_to_string(&progress_file).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(val["version"], 1);
    assert!(!val["completed"].as_array().unwrap().is_empty());
}

#[test]
fn resume_skips_already_completed_mutants() {
    let temp = TempDir::new("resume");
    let target = temp.path().join("target");
    let fixture = Path::new("fixtures/weak-auth");

    // First run creates progress
    let out1 = run_test(
        fixture,
        &[
            "--target-dir",
            target.to_str().unwrap(),
            "--test-command",
            "true",
            "--operator",
            "AUTH-001",
        ],
    );
    assert!(out1.status.success());

    // Second run with --resume
    let out2 = run_test(
        fixture,
        &[
            "--target-dir",
            target.to_str().unwrap(),
            "--test-command",
            "true",
            "--operator",
            "AUTH-001",
            "--resume",
        ],
    );
    assert!(
        out2.status.success(),
        "{}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let stdout = String::from_utf8(out2.stdout).unwrap();
    assert!(
        stdout.contains("(resumed)"),
        "completed mutants should be reported as resumed"
    );
}

#[test]
fn resume_refuses_when_configuration_changes() {
    let temp = TempDir::new("incompatible");
    let target = temp.path().join("target");
    let fixture = Path::new("fixtures/weak-auth");

    let out1 = run_test(
        fixture,
        &[
            "--target-dir",
            target.to_str().unwrap(),
            "--test-command",
            "true",
            "--operator",
            "AUTH-001",
        ],
    );
    assert!(out1.status.success());

    // Try to resume with a different test-command
    let out2 = run_test(
        fixture,
        &[
            "--target-dir",
            target.to_str().unwrap(),
            "--test-command",
            "echo changed",
            "--operator",
            "AUTH-001",
            "--resume",
        ],
    );
    assert_eq!(out2.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out2.stderr);
    assert!(
        stderr.contains("test command changed"),
        "incompatible test command should refuse resume"
    );
}

#[test]
fn resume_refuses_when_operator_changes() {
    let temp = TempDir::new("incompat-op");
    let target = temp.path().join("target");
    let fixture = Path::new("fixtures/weak-auth");

    let out1 = run_test(
        fixture,
        &[
            "--target-dir",
            target.to_str().unwrap(),
            "--test-command",
            "true",
            "--operator",
            "AUTH-001",
        ],
    );
    assert!(out1.status.success());

    let out2 = run_test(
        fixture,
        &[
            "--target-dir",
            target.to_str().unwrap(),
            "--test-command",
            "true",
            "--operator",
            "AUTH-002",
            "--resume",
        ],
    );
    assert_eq!(out2.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out2.stderr);
    assert!(
        stderr.contains("operator configuration changed"),
        "incompatible operator should refuse resume"
    );
}

#[test]
fn resume_handles_malformed_progress_file() {
    let temp = TempDir::new("malformed");
    let target = temp.path().join("target");
    let progress_file = target.join("progress.json");
    fs::create_dir_all(&target).unwrap();
    fs::write(&progress_file, "this is not valid json").unwrap();
    let fixture = Path::new("fixtures/weak-auth");

    let out = run_test(
        fixture,
        &[
            "--target-dir",
            target.to_str().unwrap(),
            "--test-command",
            "true",
            "--operator",
            "AUTH-001",
            "--resume",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("malformed progress file"),
        "malformed progress file should be rejected cleanly"
    );
}
