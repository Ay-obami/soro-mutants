use std::process::Command;

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
    let output = run(&["test", "fixtures/weak-auth", "--test-timeout", "not-a-number"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("non-negative integer"));
}
