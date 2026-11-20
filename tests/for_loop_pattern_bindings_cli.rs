use std::{fs, process::Command, time::{SystemTime, UNIX_EPOCH}};

fn unique_project() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("soro-forloop-test-{}-{nonce}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    root
}

#[test]
fn respects_for_loop_pattern_bindings_in_address_scope() {
    let root = unique_project();
    let src = root.join("src/lib.rs");
    fs::write(
        &src,
        r#"
pub fn loop_process(admin: Address, items: soroban_sdk::Vec<Address>) {
    admin.require_auth();

    // for-loop binding an Address item
    for item in items.iter() {
        item.require_auth();
    }
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", root.to_str().unwrap(), "--json"])
        .output()
        .expect("CLI should run");
    assert!(output.status.success(), "for loops should parse and respect loop bindings");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mutants = report["mutants"].as_array().unwrap();
    assert!(!mutants.is_empty());
    assert!(mutants.iter().any(|m| m["original"].as_str().unwrap().contains("admin.require_auth()")));

    let _ = fs::remove_dir_all(&root);
}
