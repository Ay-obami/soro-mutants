use std::{fs, process::Command, time::{SystemTime, UNIX_EPOCH}};

fn unique_project() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("soro-whilelet-test-{}-{nonce}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    root
}

#[test]
fn handles_while_let_loops_around_supported_calls() {
    let root = unique_project();
    let src = root.join("src/lib.rs");
    fs::write(
        &src,
        r#"
pub fn drain_and_authorize(mut users: soroban_sdk::Vec<Address>, fallback: Address) {
    while let Some(user) = users.pop() {
        user.require_auth();
    }
    fallback.require_auth();
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", root.to_str().unwrap(), "--json"])
        .output()
        .expect("CLI should run");
    assert!(output.status.success(), "while-let loops should parse and discover mutants");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mutants = report["mutants"].as_array().unwrap();
    assert!(!mutants.is_empty(), "mutants in while-let scope must be discovered");
    assert!(mutants.iter().any(|m| m["original"].as_str().unwrap().contains("fallback.require_auth()")));

    let _ = fs::remove_dir_all(&root);
}
