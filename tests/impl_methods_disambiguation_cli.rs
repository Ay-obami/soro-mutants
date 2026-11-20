use std::{fs, process::Command, time::{SystemTime, UNIX_EPOCH}};

fn unique_project() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("soro-impl-test-{}-{nonce}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    root
}

#[test]
fn disambiguates_methods_across_different_impl_blocks() {
    let root = unique_project();
    let src = root.join("src/lib.rs");
    fs::write(
        &src,
        r#"
pub struct AdminController;
pub struct TokenController;

impl AdminController {
    pub fn process(admin: Address) {
        admin.require_auth();
    }
}

impl TokenController {
    pub fn process(sender: Address) {
        sender.require_auth();
    }
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", root.to_str().unwrap(), "--json"])
        .output()
        .expect("CLI should run");
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mutants = report["mutants"].as_array().unwrap();
    assert_eq!(mutants.len(), 2);
    // Both methods are discovered with distinct deterministic IDs and line locations
    assert_ne!(mutants[0]["id"], mutants[1]["id"]);
    assert_ne!(mutants[0]["span"]["line"], mutants[1]["span"]["line"]);

    let _ = fs::remove_dir_all(&root);
}
