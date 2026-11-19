use std::{path::PathBuf, process::Command};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/weak-auth")
}

#[test]
fn invalid_diff_base_is_an_error_not_a_panic() {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list"])
        .arg(fixture())
        .arg("--diff-base=-bad")
        .output()
        .expect("CLI should launch");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid Git diff base revision"),
        "{stderr}"
    );
    assert!(!stderr.contains("panicked at"), "{stderr}");
}

#[test]
fn production_code_has_no_direct_unwrap_or_expect_calls() {
    let source = include_str!("../src/main.rs");
    let production = source.split("#[cfg(test)]").next().unwrap_or(source);
    assert!(
        !production.contains(".unwrap()"),
        "production .unwrap() found"
    );
    assert!(
        !production.contains(".expect("),
        "production .expect(...) found"
    );
}
