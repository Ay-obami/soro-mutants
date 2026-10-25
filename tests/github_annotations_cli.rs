use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(args)
        .output()
        .expect("CLI should run")
}

#[test]
fn annotations_for_surviving_mutants_have_github_source_locations() {
    let output = run(&[
        "test",
        "fixtures/weak-auth",
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
        "--json",
        "--github-annotations",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.is_object(), "annotations must not corrupt JSON stdout");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let annotations: Vec<&str> = stderr
        .lines()
        .filter(|line| line.starts_with("::warning "))
        .collect();
    assert_eq!(
        annotations.len(),
        1,
        "one warning per surviving mutant: {stderr}"
    );
    assert!(annotations[0].starts_with("::warning file=src/lib.rs,line=11,col=9,"));
    assert!(annotations[0].contains("AUTH-001"));
    assert!(annotations[0].contains("test-quality"));
}

#[test]
fn empty_or_killed_runs_do_not_emit_annotations() {
    let empty = run(&[
        "test",
        "fixtures/weak-auth",
        "--operator",
        "TOKEN-001",
        "--github-annotations",
    ]);
    assert!(empty.status.success());
    assert!(!String::from_utf8_lossy(&empty.stderr).contains("::warning"));
    let killed = run(&[
        "test",
        "fixtures/weak-auth",
        "--operator",
        "AUTH-001",
        "--test-command",
        "false",
        "--github-annotations",
    ]);
    // The clean baseline fails, so no mutant annotation should be emitted.
    assert!(!killed.status.success());
    assert!(!String::from_utf8_lossy(&killed.stderr).contains("::warning"));
}
