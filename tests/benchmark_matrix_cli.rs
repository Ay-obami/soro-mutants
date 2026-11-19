use serde_json::{json, Value};
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
            "soro-benchmark-matrix-{prefix}-{}-{nonce}",
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

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["benchmark-matrix"])
        .args(args)
        .output()
        .expect("soro-mutants should run")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn manifest(temp: &TempDir, test_command: &str) -> PathBuf {
    let path = temp.path().join("targets.json");
    let document = json!({
        "schema_version": 1,
        "description": "regression fixture",
        "targets": [{
            "id": "local-auth",
            "kind": "local",
            "path": "fixtures/weak-auth",
            "test_command": test_command,
            "operator": "AUTH-001"
        }]
    });
    fs::write(&path, serde_json::to_string_pretty(&document).unwrap()).unwrap();
    path
}

#[test]
fn external_targets_require_explicit_opt_in() {
    let output = run(&[
        "docs/benchmark-matrix.json",
        "--target",
        "external-phoenix-pool-propose-admin",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("--allow-external"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn local_only_and_allow_external_conflict() {
    let output = run(&[
        "docs/benchmark-matrix.json",
        "--local-only",
        "--allow-external",
    ]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("mutually exclusive"));
}

#[test]
fn unknown_target_is_a_structured_error() {
    let output = run(&["docs/benchmark-matrix.json", "--target", "not-a-target"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("no benchmark target with id not-a-target"));
}

#[test]
fn invalid_pinned_revision_is_rejected_before_execution() {
    let temp = TempDir::new("revision");
    let path = temp.path().join("invalid.json");
    let document = json!({
        "schema_version": 1,
        "targets": [{
            "id": "external-bad",
            "kind": "external",
            "repository": "https://github.com/example/project",
            "revision": "HEAD",
            "test_command": "true"
        }]
    });
    fs::write(&path, serde_json::to_string_pretty(&document).unwrap()).unwrap();

    let output = run(&[path.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("hexadecimal characters"));
}

#[test]
fn local_benchmark_records_baseline_and_scored_outcomes() {
    let temp = TempDir::new("local");
    let manifest = manifest(&temp, "true");
    let checkouts = temp.path().join("checkouts");
    let output = run(&[
        manifest.to_str().unwrap(),
        "--local-only",
        "--checkout-dir",
        checkouts.to_str().unwrap(),
        "--json",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));

    let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["schema_version"], 1);
    assert!(summary["notice"]
        .as_str()
        .unwrap()
        .contains("not a security finding"));
    assert_eq!(summary["targets"].as_array().unwrap().len(), 1);
    let target = &summary["targets"][0];
    assert_eq!(target["status"], "completed");
    assert_eq!(target["baseline"]["outcome"], "passed");
    assert_eq!(target["baseline"]["exit_code"], 0);
    assert!(target["counts"]["generated"].as_u64().unwrap() > 0);
    assert_eq!(target["counts"]["survived"], target["counts"]["generated"]);
    assert_eq!(target["mutants"][0]["outcome"], "SURVIVED");
}

#[test]
fn failed_baseline_is_never_scored() {
    let temp = TempDir::new("baseline");
    let manifest = manifest(&temp, "false");
    let checkouts = temp.path().join("checkouts");
    let output = run(&[
        manifest.to_str().unwrap(),
        "--local-only",
        "--checkout-dir",
        checkouts.to_str().unwrap(),
        "--json",
    ]);
    assert!(!output.status.success());

    let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
    let target = &summary["targets"][0];
    assert_eq!(target["status"], "failed");
    assert_eq!(target["baseline"]["outcome"], "failed");
    assert!(target["counts"].is_null());
    assert!(target["mutants"].as_array().unwrap().is_empty());
    assert!(target["reason"]
        .as_str()
        .unwrap()
        .contains("were not scored"));
}
