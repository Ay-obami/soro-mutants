use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn specific_test_timeout_overrides_legacy_timeout_for_baseline() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("soro-timeout-{nonce}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "fn check(user: Address) { user.require_auth(); }\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test")
        .arg(&root)
        .args([
            "--test-command",
            "true",
            "--timeout",
            "0",
            "--test-timeout",
            "5",
            "--compile-timeout",
            "1",
        ])
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("SURVIVED"));
}

#[test]
fn baseline_test_timeout_is_reported_as_baseline_failure() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("soro-timeout-fail-{nonce}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "fn check(user: Address) { user.require_auth(); }\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test")
        .arg(&root)
        .args([
            "--test-command",
            "sleep 2",
            "--timeout",
            "5",
            "--test-timeout",
            "0",
        ])
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(root);
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("baseline tests timed out"));
}
