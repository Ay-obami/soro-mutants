use std::{fs, process::Command, time::{SystemTime, UNIX_EPOCH}};

fn unique_project() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("soro-corpus-test-{}-{nonce}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    root
}

#[test]
fn parser_corpus_handles_modern_stable_rust_syntax_without_panic() {
    let root = unique_project();
    let src = root.join("src/lib.rs");
    // Corpus exercising modern stable Rust syntax constructs:
    // let-else, const blocks, raw identifiers, destructuring, type aliases, async, macro calls
    fs::write(
        &src,
        r#"
pub type Nonce = u64;

pub const fn const_helper<const N: usize>() -> usize {
    N * 2
}

pub fn test_syntax_corpus(user: Address, opt: Option<u32>) {
    user.require_auth();

    // let-else syntax
    let Some(val) = opt else {
        return;
    };

    // raw identifiers
    let r#type = val;

    // tuple destructuring
    let (a, b) = (1, 2);

    // const block expression
    let _ = const { 1 + 2 };

    let _ = (r#type, a, b);
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", root.to_str().unwrap(), "--json"])
        .output()
        .expect("CLI should run");
    assert!(output.status.success(), "parser should succeed without panicking on stable Rust syntax");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mutants = report["mutants"].as_array().unwrap();
    assert!(!mutants.is_empty(), "mutant in test_syntax_corpus should be discovered");

    let _ = fs::remove_dir_all(&root);
}
