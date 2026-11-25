use std::{fs, path::PathBuf, process::Command, time::{SystemTime, UNIX_EPOCH}};

fn fixture_with(source: &str) -> PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("soro-discovery-{}-{nonce}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), source).unwrap();
    root
}

fn discover(path: &std::path::Path) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", path.to_str().unwrap(), "--json"])
        .output().expect("list should start");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn semantic_fixture_reports_stable_overlap_suppression_and_contiguous_ids() {
    let source = include_str!("../fixtures/semantic-edge-cases/src/lib.rs");
    let root = fixture_with(source);
    let first = discover(&root);
    let second = discover(&root);
    let a = first["mutants"].as_array().unwrap();
    let b = second["mutants"].as_array().unwrap();
    assert_eq!(a, b, "mutant discovery must be deterministic");
    assert!(!a.is_empty());
    for (i, mutant) in a.iter().enumerate() {
        assert_eq!(mutant["id"].as_str().unwrap(), format!("M{:04}", i + 1));
    }
    let suppressions = first["suppression"]["details"].as_array().unwrap();
    assert!(suppressions.iter().any(|s| s["reason"] == "overlapping source span"),
        "expected whole helper call and nested argument recognizers to overlap");
    assert!(a.iter().any(|m| m["operator"] == "AUTH-001"));
    assert!(!a.iter().any(|m| m["function"] == "unrelated"),
        "unrelated library aliases must not be inferred");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unicode_before_and_after_mutations_keeps_byte_column_locations_consistent() {
    let source = "pub use soroban_sdk::Address as Person;\n\
                  fn guard(user: Person) { let café = \"🦀 日本語\"; user.require_auth(); /* 🍀 */ }\n";
    let root = fixture_with(source);
    let mutants = discover(&root);
    let selected = mutants["mutants"].as_array().unwrap();
    let auth = selected.iter().find(|m| m["operator"] == "AUTH-001").unwrap();
    let line = source.lines().nth(auth["span"]["line"].as_u64().unwrap() as usize - 1).unwrap();
    let original = auth["original"].as_str().unwrap();
    assert_eq!(auth["span"]["column"].as_u64().unwrap() as usize,
        line.find(original).unwrap() + 1);
    assert!(original.contains("require_auth"));
    fs::remove_dir_all(root).unwrap();
}
