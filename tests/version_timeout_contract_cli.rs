use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(args)
        .output()
        .expect("CLI should launch")
}

#[test]
fn version_contract_is_stable_on_stdout() {
    let expected = format!("cargo-soro-mutants {}\n", env!("CARGO_PKG_VERSION"));
    for args in [&["--version"][..], &["-V"][..]] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        assert!(output.stderr.is_empty(), "version must not write to stderr");
    }
}

#[test]
fn negative_timeout_flags_are_usage_errors() {
    for flag in ["--timeout", "--compile-timeout", "--test-timeout"] {
        let output = run(&["test", "fixtures/weak-auth", flag, "-1"]);
        assert_eq!(output.status.code(), Some(2), "{flag}");
        assert!(output.stdout.is_empty(), "{flag} must not print a report");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("non-negative"), "{flag}: {stderr}");
        assert!(stderr.contains(flag), "{flag}: {stderr}");
    }
}

#[test]
fn malformed_timeout_is_a_usage_error() {
    let output = run(&[
        "test",
        "fixtures/weak-auth",
        "--test-timeout",
        "not-a-number",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("non-negative integer"));
}

#[test]
fn benchmark_negative_timeout_is_a_usage_error() {
    let output = run(&["benchmark-matrix", "--timeout", "-1"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("non-negative"), "{stderr}");
    assert!(stderr.contains("--timeout"), "{stderr}");
}

#[test]
fn non_negative_timeout_values_are_accepted() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "soro-timeout-parser-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn noop() {}\n").unwrap();
    let root = root.to_string_lossy().into_owned();

    for (flag, value) in [
        ("--timeout", "0"),
        ("--compile-timeout", "0"),
        ("--test-timeout", "5"),
    ] {
        let output = run(&["test", &root, flag, value, "--json"]);
        assert!(
            output.status.success(),
            "{flag}={value}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fs::remove_dir_all(root).unwrap();
}
