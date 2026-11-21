use std::{
    process::Command,
    sync::{Mutex, OnceLock},
};

fn fixture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn run(command: &str, extra: &[&str]) -> std::process::Output {
    let _guard = fixture_lock().lock().unwrap();
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args([command, "fixtures/weak-auth"])
        .args(extra)
        .output()
        .expect("CLI should run")
}

#[test]
fn list_json_preserves_discovered_fields_and_ids() {
    let output = run("list", &["--json"]);
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    let mutants = report["mutants"].as_array().unwrap();
    let auth_mutants: Vec<_> = mutants
        .iter()
        .filter(|mutant| mutant["operator"] == "AUTH-001" || mutant["operator"] == "AUTH-002")
        .collect();
    assert_eq!(auth_mutants.len(), 2);
    assert_eq!(auth_mutants[0]["id"], "M0001");
    assert_eq!(auth_mutants[0]["operator"], "AUTH-001");
    assert_eq!(auth_mutants[1]["id"], "M0002");
    assert_eq!(auth_mutants[1]["operator"], "AUTH-002");
    for mutant in auth_mutants {
        assert_eq!(mutant.as_object().unwrap().len(), 8);
        assert_eq!(mutant["file"], "src/lib.rs");
        assert_eq!(mutant["function"], "change_admin");
        assert_eq!(mutant["span"]["line"], 11);
        assert_eq!(mutant["original"], "current_admin.require_auth()");
    }
}

#[test]
fn empty_list_and_test_json_use_versioned_envelopes() {
    for (command, field) in [("list", "mutants"), ("test", "results")] {
        let output = run(command, &["--operator", "TOKEN-001", "--json"]);
        assert!(output.status.success());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if command == "list" {
            assert_eq!(report.as_object().unwrap().len(), 3);
            assert_eq!(report["metadata"]["operator_filter"], "TOKEN-001");
            assert!(report["metadata"]["function_filter"].is_null());
        } else {
            for field in [
                "schema_version",
                "results",
                "metadata",
                "by_operator",
                "by_file",
                "by_function",
            ] {
                assert!(
                    report.get(field).is_some(),
                    "test report must include {field}"
                );
            }
        }
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report[field], serde_json::json!([]));
        if command == "test" {
            assert_eq!(report["metadata"]["test_command"], "cargo test -q");
        }
    }
}

#[test]
fn test_json_records_execution_context_without_host_inventory() {
    let output = run(
        "test",
        &[
            "--json",
            "--test-command",
            "true",
            "--timeout",
            "10",
            "--compile-timeout",
            "15",
            "--test-timeout",
            "20",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let metadata = &report["metadata"];
    assert_eq!(metadata["test_command"], "true");
    assert_eq!(metadata["compile_timeout_seconds"], 15);
    assert_eq!(metadata["test_timeout_seconds"], 20);
    assert!(metadata["operator_filter"].is_null());
    assert!(metadata["file_filter"].is_null());
    assert!(metadata["function_filter"].is_null());
    assert!(metadata["shard_index"].is_null());
    assert!(metadata["shard_total"].is_null());
    assert!(metadata["rustc_version"].is_string() || metadata["rustc_version"].is_null());
    assert!(metadata["cargo_version"].is_string() || metadata["cargo_version"].is_null());
    assert!(metadata.get("environment").is_none());
    assert!(metadata.get("hostname").is_none());
    assert!(metadata.get("target_dir").is_none());
    assert_eq!(report["results"][0]["mutant"]["id"], "M0001");
}

#[test]
fn test_json_records_active_operator_and_function_filters() {
    let output = run(
        "test",
        &[
            "--json",
            "--test-command",
            "true",
            "--operator",
            "AUTH-001",
            "--function",
            "change_admin",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["metadata"]["operator_filter"], "AUTH-001");
    assert_eq!(report["metadata"]["function_filter"], "change_admin");
}

#[test]
fn list_json_records_active_operator_and_function_filters() {
    let output = run(
        "list",
        &[
            "--json",
            "--operator",
            "AUTH-001",
            "--function",
            "change_admin",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["metadata"]["operator_filter"], "AUTH-001");
    assert_eq!(report["metadata"]["function_filter"], "change_admin");
}

#[test]
fn list_and_test_json_record_file_and_shard_selection() {
    let selection_args = ["--file", "src/not-selected.rs", "--shard", "2/3", "--json"];

    let list = run("list", &selection_args);
    assert!(list.status.success());
    let list_report: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(
        list_report["metadata"]["file_filter"],
        "src/not-selected.rs"
    );
    assert_eq!(list_report["metadata"]["shard_index"], 2);
    assert_eq!(list_report["metadata"]["shard_total"], 3);

    let test = run(
        "test",
        &[
            "--file",
            "src/not-selected.rs",
            "--shard",
            "2/3",
            "--test-command",
            "true",
            "--json",
        ],
    );
    assert!(test.status.success());
    let test_report: serde_json::Value = serde_json::from_slice(&test.stdout).unwrap();
    assert_eq!(
        test_report["metadata"]["file_filter"],
        "src/not-selected.rs"
    );
    assert_eq!(test_report["metadata"]["shard_index"], 2);
    assert_eq!(test_report["metadata"]["shard_total"], 3);
    assert_eq!(test_report["results"], serde_json::json!([]));
}
