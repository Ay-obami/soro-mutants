use std::process::Command;

#[test]
fn runtime_budget_script_self_test_and_regression_verification() {
    let script_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github/scripts/check-runtime-budget.sh");

    assert!(
        script_path.exists(),
        "runtime budget script must exist at {:?}",
        script_path
    );

    let output = Command::new(&script_path)
        .arg("--self-test")
        .output()
        .expect("check-runtime-budget.sh should execute successfully");

    assert!(
        output.status.success(),
        "self-test failed with status {:?}:\nSTDOUT:\n{}\nSTDERR:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("All self-tests passed"),
        "expected success confirmation in output: {stdout}"
    );
}

#[test]
fn runtime_budget_script_validates_arguments() {
    let script_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github/scripts/check-runtime-budget.sh");

    // Missing arguments
    let output = Command::new(&script_path)
        .output()
        .expect("script executes");
    assert_eq!(output.status.code(), Some(2));

    // Invalid budget value
    let output2 = Command::new(&script_path)
        .args(["invalid_budget", "echo", "hi"])
        .output()
        .expect("script executes");
    assert_eq!(output2.status.code(), Some(2));
}
