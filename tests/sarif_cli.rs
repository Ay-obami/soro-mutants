use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(args)
        .output()
        .expect("CLI should run")
}

#[test]
fn sarif_contains_surviving_mutant_as_test_quality_finding() {
    let args = [
        "test",
        "fixtures/weak-auth",
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
        "--sarif",
    ];
    let first = run(&args);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, run(&args).stdout);
    assert_eq!(
        first.stdout,
        include_bytes!("../docs/sarif-report-example.json")
    );
    let sarif: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(sarif["version"], "2.1.0");
    assert_eq!(sarif["runs"][0]["tool"]["driver"]["name"], "Soro Mutants");
    assert_eq!(
        sarif["runs"][0]["tool"]["driver"]["rules"][0]["id"],
        "AUTH-001"
    );
    let result = &sarif["runs"][0]["results"][0];
    assert_eq!(result["ruleId"], "AUTH-001");
    assert_eq!(result["properties"]["outcome"], "SURVIVED");
    assert_eq!(
        result["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        "src/lib.rs"
    );
    assert_eq!(
        result["locations"][0]["physicalLocation"]["region"]["startLine"],
        11
    );
    assert!(result["message"]["text"]
        .as_str()
        .unwrap()
        .contains("tests did not detect"));
    assert!(!String::from_utf8(first.stdout)
        .unwrap()
        .contains("confirmed vulnerability"));
}

#[test]
fn sarif_empty_run_is_valid_and_conflicts_with_other_formats() {
    let output = run(&[
        "test",
        "fixtures/weak-auth",
        "--operator",
        "TOKEN-001",
        "--sarif",
    ]);
    assert!(output.status.success());
    let sarif: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sarif["runs"][0]["results"].as_array().unwrap().len(), 0);
    assert_eq!(
        sarif["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(run(&["test", "--sarif", "--json"]).status.code(), Some(2));
    assert_eq!(
        run(&["test", "--sarif", "--markdown"]).status.code(),
        Some(2)
    );
    assert_eq!(run(&["test", "--sarif", "--html"]).status.code(), Some(2));
}
