use std::{fs, process::Command};

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(args)
        .output()
        .expect("CLI should run")
}

#[test]
fn html_report_is_portable_deterministic_and_contains_mutant_details() {
    let args = [
        "test",
        "fixtures/weak-auth",
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
        "--html",
    ];
    let first = run(&args);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, run(&args).stdout);
    let html = String::from_utf8(first.stdout).unwrap();
    assert!(html.starts_with("<!doctype html>\n<html lang=\"en\">"));
    assert!(html.contains("<title>Soro Mutants Report</title>"));
    assert!(html.contains("<style>"));
    assert!(!html.contains("<script"));
    assert!(!html.contains("https://"));
    assert!(html.contains("Semantic score"));
    assert!(html.contains("0.0%"));
    assert!(html.contains("AUTH-001"));
    assert!(html.contains("Survived"));
    assert!(html.contains("src/lib.rs"));
    assert!(html.contains("require_auth()"));
}

#[test]
fn html_empty_report_can_be_written_as_one_file() {
    let path = std::env::temp_dir().join(format!("soro-mutants-html-{}.html", std::process::id()));
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args([
            "test",
            "fixtures/weak-auth",
            "--operator",
            "TOKEN-001",
            "--html",
            "--output",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let html = fs::read_to_string(&path).unwrap();
    assert!(html.contains("No matching mutants"));
    assert!(html.contains("Generated"));
    assert!(html.contains("0.0%"));
    fs::remove_file(path).unwrap();
    let conflict = run(&["test", "fixtures/weak-auth", "--html", "--markdown"]);
    assert_eq!(conflict.status.code(), Some(2));
}
