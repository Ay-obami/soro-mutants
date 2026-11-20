//! Regression tests for environment-derived credential leakage.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const ENV_NAME: &str = "SORO_MUTANTS_REGRESSION_SECRET";
const SECRET: &str = "redaction-regression-secret-191-never-print";

struct TemporaryProject(PathBuf);

impl TemporaryProject {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("soro-redaction-{}-{nonce}", std::process::id()));
        fs::create_dir_all(root.join("src")).expect("create isolated test project");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/weak-auth/src/lib.rs"),
            root.join("src/lib.rs"),
        )
        .expect("copy fixture source");
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"secret-redaction-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("create manifest");
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn list_error_does_not_echo_environment_secret() {
    let project = TemporaryProject::new();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", project.path().to_str().unwrap(), "--output"])
        .arg(project.path().join(SECRET).join("missing/report.json"))
        .env(ENV_NAME, SECRET)
        .output()
        .expect("run CLI");
    assert!(!output.status.success());
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    assert!(!diagnostics.contains(SECRET), "{diagnostics}");
    assert!(diagnostics.contains("[REDACTED]"), "{diagnostics}");
    assert!(
        !project.path().join(SECRET).exists(),
        "must not create directories"
    );
}

#[test]
fn test_json_metadata_redacts_secret_in_custom_command() {
    let project = TemporaryProject::new();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args([
            "test",
            project.path().to_str().unwrap(),
            "--operator",
            "TOKEN-001",
            "--json",
            "--test-command",
            SECRET,
        ])
        .env(ENV_NAME, SECRET)
        .output()
        .expect("run CLI");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(SECRET), "{stdout}");
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["metadata"]["test_command"], "[REDACTED]");
    assert_eq!(document["results"], serde_json::json!([]));
}

#[test]
fn baseline_stderr_and_text_report_never_echo_environment_secret() {
    let project = TemporaryProject::new();
    let original_source = fs::read(project.path().join("src/lib.rs")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args([
            "test",
            project.path().to_str().unwrap(),
            "--operator",
            "AUTH-001",
            "--test-command",
            "printf '%s\\n' \"$SORO_MUTANTS_REGRESSION_SECRET\" >&2; exit 13",
        ])
        .env(ENV_NAME, SECRET)
        .output()
        .expect("run CLI");
    assert!(!output.status.success(), "expected failing baseline");
    for output in [&output.stdout, &output.stderr] {
        let rendered = String::from_utf8_lossy(output);
        assert!(!rendered.contains(SECRET), "{rendered}");
    }
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("[REDACTED]"),
        "stderr should display a useful redaction marker"
    );
    assert_eq!(
        fs::read(project.path().join("src/lib.rs")).unwrap(),
        original_source
    );
    assert!(!project.path().join(".soro-mutants-worktree").exists());
}
