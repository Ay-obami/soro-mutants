use std::{fs, process::Command};

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(args)
        .output()
        .expect("CLI should run")
}

fn valid_entry(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "operator": "AUTH-001",
        "source_pattern": {"language":"rust", "code":"user.require_auth();"},
        "mutation": {"original":"user.require_auth()", "replacement":"()"},
        "expectations": {"weak":"SURVIVED", "strong":"KILLED"},
        "provenance": {"kind":"fixture", "reference":"fixtures/weak-auth/src/lib.rs"}
    })
}

fn validate_document(name: &str, value: &serde_json::Value) -> std::process::Output {
    let path = std::env::temp_dir().join(format!("soro-corpus-{}-{name}.json", std::process::id()));
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("validate-corpus")
        .arg(&path)
        .output()
        .unwrap();
    fs::remove_file(path).unwrap();
    output
}

#[test]
fn json_schema_declares_v1_and_all_required_corpus_fields() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../docs/corpus-manifest.schema.json")).unwrap();
    assert_eq!(schema["properties"]["schema_version"]["const"], 1);
    for field in ["schema_version", "entries"] {
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&field.into()));
    }
    for field in [
        "id",
        "operator",
        "source_pattern",
        "mutation",
        "expectations",
        "provenance",
    ] {
        assert!(schema["$defs"]["entry"]["required"]
            .as_array()
            .unwrap()
            .contains(&field.into()));
    }
}

#[test]
fn bundled_corpus_example_validates_and_has_two_entries() {
    let output = run(&["validate-corpus", "docs/corpus-example.json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Validated 2 corpus entries"));
}

#[test]
fn manifest_validation_rejects_unsupported_version_and_duplicate_ids() {
    let unsupported =
        serde_json::json!({"schema_version": 2, "entries": [valid_entry("auth-removal") ]});
    let output = validate_document("version", &unsupported);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("schema_version"));
    let duplicate = serde_json::json!({"schema_version": 1, "entries": [valid_entry("same"), valid_entry("same")]});
    let output = validate_document("duplicate", &duplicate);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate"));
}

#[test]
fn manifest_requires_patterns_expectations_and_provenance() {
    let mut entry = valid_entry("missing");
    entry.as_object_mut().unwrap().remove("source_pattern");
    let output = validate_document(
        "missing",
        &serde_json::json!({"schema_version": 1, "entries": [entry]}),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("source_pattern"));
    let mut entry = valid_entry("empty");
    entry["provenance"]["reference"] = "".into();
    let output = validate_document(
        "empty",
        &serde_json::json!({"schema_version": 1, "entries": [entry]}),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("reference"));
}

#[test]
fn manifest_accepts_additive_unknown_fields_in_version_one() {
    let mut entry = valid_entry("future-metadata");
    entry["future_tag"] = "optional".into();
    let document =
        serde_json::json!({"schema_version": 1, "entries": [entry], "future_metadata": true});
    let output = validate_document("extension", &document);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
