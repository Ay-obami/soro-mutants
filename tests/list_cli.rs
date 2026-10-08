use std::process::Command;

fn run(extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", "fixtures/weak-auth"])
        .args(extra)
        .output()
        .expect("CLI should run")
}

#[test]
fn count_only_prints_matching_mutant_count() {
    let output = run(&["--count-only"]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "2");
}

#[test]
fn count_only_respects_existing_filters_and_zero_matches() {
    let auth = run(&["--operator", "AUTH-001", "--count-only"]);
    assert!(auth.status.success());
    assert_eq!(String::from_utf8(auth.stdout).unwrap().trim(), "1");

    let none = run(&["--operator", "TOKEN-001", "--count-only"]);
    assert!(none.status.success());
    assert_eq!(String::from_utf8(none.stdout).unwrap().trim(), "0");

    let file = run(&["--file", "src/lib.rs", "--count-only"]);
    assert!(file.status.success());
    assert_eq!(String::from_utf8(file.stdout).unwrap().trim(), "2");

    let function = run(&["--function", "change_admin", "--count-only"]);
    assert!(function.status.success());
    assert_eq!(String::from_utf8(function.stdout).unwrap().trim(), "2");

    let missing_function = run(&["--function", "missing", "--count-only"]);
    assert!(missing_function.status.success());
    assert_eq!(
        String::from_utf8(missing_function.stdout).unwrap().trim(),
        "0"
    );
}
