use std::{fs, process::Command};

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(args)
        .output()
        .expect("CLI should run")
}

#[test]
fn success_returns_zero() {
    assert_eq!(cli(&["operators"]).status.code(), Some(0));
}

#[test]
fn malformed_arguments_return_two() {
    assert_eq!(
        cli(&["list", "--operator", "UNKNOWN"]).status.code(),
        Some(2)
    );
}

#[test]
fn baseline_failure_returns_three() {
    let root = std::env::temp_dir().join(format!("soro-exit-{}", std::process::id()));
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("lib.rs"),
        "fn check(user: Address) { user.require_auth(); }\n",
    )
    .unwrap();
    let output = cli(&["test", root.to_str().unwrap(), "--test-command", "false"]);
    let _ = fs::remove_dir_all(&root);
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("baseline tests failed"));
}

#[test]
fn ordinary_runtime_failure_returns_one() {
    let output = cli(&["list", "/this-directory-does-not-exist-soro-mutants"]);
    assert_eq!(output.status.code(), Some(1));
}
