use std::{fs, process::Command, time::{SystemTime, UNIX_EPOCH}};

fn unique_project() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("soro-destruct-test-{}-{nonce}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    root
}

#[test]
fn handles_tuple_and_struct_destructuring_bindings_safely() {
    let root = unique_project();
    let src = root.join("src/lib.rs");
    fs::write(
        &src,
        r#"
pub struct AuthPair {
    pub primary: Address,
    pub secondary: Address,
}

pub fn check_destructuring(pair: AuthPair, addresses: (Address, Address)) {
    // struct destructuring
    let AuthPair { primary, secondary } = pair;
    primary.require_auth();
    secondary.require_auth();

    // tuple destructuring
    let (addr1, addr2) = addresses;
    addr1.require_auth();
    addr2.require_auth();
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", root.to_str().unwrap(), "--json"])
        .output()
        .expect("CLI should run");
    assert!(output.status.success(), "destructuring patterns must parse without panic");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mutants = report["mutants"].as_array().unwrap();
    // Destructured variables with require_auth calls should parse cleanly
    assert!(output.status.success());
    let _ = fs::remove_dir_all(&root);
}
