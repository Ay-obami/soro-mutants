#![cfg(unix)]

use std::{
    ffi::OsString,
    fs,
    os::unix::ffi::OsStringExt,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn list_handles_non_unicode_project_root_without_panicking() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut name = b"soro-mutants-nonunicode-".to_vec();
    name.extend_from_slice(std::process::id().to_string().as_bytes());
    name.push(b'-');
    name.extend_from_slice(nonce.to_string().as_bytes());
    name.push(0xff);
    let root = std::env::temp_dir().join(OsString::from_vec(name));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "use soroban_sdk::Address; fn f(user: Address) { user.require_auth(); }",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("list")
        .arg(&root)
        .arg("--json")
        .output()
        .expect("CLI should launch");
    let _ = fs::remove_dir_all(&root);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked at"));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["mutants"].as_array().unwrap().len(), 1);
}
