use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn oversized_rust_source_fails_before_reading_contents() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "soro-mutants-oversized-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    let file = fs::File::create(root.join("src/huge.rs")).unwrap();
    file.set_len(17 * 1024 * 1024).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("list")
        .arg(&root)
        .output()
        .expect("CLI should launch");
    let _ = fs::remove_dir_all(&root);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("maximum supported size"), "{stderr}");
    assert!(stderr.contains("huge.rs"), "{stderr}");
}
