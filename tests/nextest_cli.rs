use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(args)
        .output()
        .expect("CLI should run")
}

#[test]
fn default_runner_and_explicit_cargo_test_preserve_existing_output() {
    let base = [
        "test",
        "fixtures/weak-auth",
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
        "--json",
    ];
    let existing = run(&base);
    assert!(existing.status.success());
    let explicit = run(&[
        "test",
        "fixtures/weak-auth",
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
        "--json",
        "--runner",
        "cargo-test",
    ]);
    assert!(
        explicit.status.success(),
        "{}",
        String::from_utf8_lossy(&explicit.stderr)
    );
    assert_eq!(existing.stdout, explicit.stdout);
}

#[test]
fn nextest_rejects_custom_test_commands_and_invalid_runner_values() {
    let custom = run(&[
        "test",
        "fixtures/weak-auth",
        "--operator",
        "AUTH-001",
        "--runner",
        "nextest",
        "--test-command",
        "true",
    ]);
    assert!(!custom.status.success());
    assert!(String::from_utf8_lossy(&custom.stderr).contains("cannot combine"));
    let invalid = run(&["test", "--runner", "unknown"]);
    assert_eq!(invalid.status.code(), Some(2));
}

#[test]
fn missing_nextest_reports_clear_installation_error() {
    let nextest_exists = Command::new("cargo")
        .args(["nextest", "--version"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if nextest_exists {
        return; // Installed runner is exercised by normal CLI use.
    }
    let output = run(&[
        "test",
        "fixtures/weak-auth",
        "--operator",
        "AUTH-001",
        "--runner",
        "nextest",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cargo-nextest is not installed"),
        "{stderr}"
    );
}
