use std::{path::PathBuf, process::Command};

fn shared_target_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/compile-only-fixtures")
}

fn run_fixture(path: &str, operator: &str) -> serde_json::Value {
    let target = shared_target_dir();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["test", path, "--json", "--operator", operator])
        .arg("--target-dir")
        .arg(&target)
        .args([
            "--test-command",
            "cargo +1.91.0 test -q",
            "--compile-timeout",
            "300",
            "--test-timeout",
            "120",
        ])
        .output()
        .expect("test CLI should launch");

    assert!(
        output.status.success(),
        "mutation run for {path} ({operator}) failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid JSON report")
}

fn assert_fixture_reaches_compile_gate(path: &str, operator: &str) {
    let report = run_fixture(path, operator);
    let results = report["results"].as_array().expect("results array");
    assert!(
        !results.is_empty(),
        "expected {operator} mutants for {path}"
    );

    let unviable: Vec<_> = results
        .iter()
        .filter(|result| result["outcome"] == "UNVIABLE")
        .collect();
    assert!(
        unviable.is_empty(),
        "{path} emitted {operator} mutants that failed the compile viability gate: {unviable:#?}"
    );
}

#[test]
#[ignore = "requires Soroban SDK Rust 1.91; CI runs this target explicitly"]
fn compile_only_auth_fixture_mutants_reach_compile_gate() {
    assert_fixture_reaches_compile_gate("fixtures/compile-only/auth", "AUTH-001");
}

#[test]
#[ignore = "requires Soroban SDK Rust 1.91; CI runs this target explicitly"]
fn compile_only_token_fixture_mutants_reach_compile_gate() {
    assert_fixture_reaches_compile_gate("fixtures/compile-only/token", "TOKEN-001");
}

#[test]
#[ignore = "requires Soroban SDK Rust 1.91; CI runs this target explicitly"]
fn compile_only_event_fixture_mutants_reach_compile_gate() {
    assert_fixture_reaches_compile_gate("fixtures/compile-only/event", "EVENT-001");
}

#[test]
#[ignore = "requires Soroban SDK Rust 1.91; CI runs this target explicitly"]
fn compile_only_storage_fixture_mutants_reach_compile_gate() {
    assert_fixture_reaches_compile_gate("fixtures/compile-only/storage", "STORAGE-001");
}

#[test]
fn compile_viability_regression_detects_malformed_rust() {
    assert!(syn::parse_file("fn broken() { let value = ; }").is_err());
}
