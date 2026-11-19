use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn deeply_nested_source_is_rejected_before_syn_parsing() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("soro-mutants-depth-{}-{nonce}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    let nested = format!(
        "fn f() {{ let _ = {}0{}; }}",
        "(".repeat(300),
        ")".repeat(300)
    );
    fs::write(root.join("src/lib.rs"), nested).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("list")
        .arg(&root)
        .output()
        .expect("CLI should launch");
    let _ = fs::remove_dir_all(&root);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("exceeds supported depth 256"), "{stderr}");
    assert!(!stderr.contains("panicked at"), "{stderr}");
}

#[test]
fn delimiters_in_strings_and_comments_do_not_consume_depth_budget() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "soro-mutants-depth-ok-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    let source = format!(
        "// {}\nfn f() {{ let _ = \"{}\"; }}",
        "(".repeat(400),
        "{".repeat(400)
    );
    fs::write(root.join("src/lib.rs"), source).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("list")
        .arg(&root)
        .output()
        .expect("CLI should launch");
    let _ = fs::remove_dir_all(&root);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
