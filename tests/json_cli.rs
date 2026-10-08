use std::process::Command;

fn run(command: &str, extra: &[&str]) -> std::process::Output {
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
    assert_eq!(mutants.len(), 2);
    assert_eq!(mutants[0]["id"], "M0001");
    assert_eq!(mutants[0]["operator"], "AUTH-001");
    assert_eq!(mutants[1]["id"], "M0002");
    assert_eq!(mutants[1]["operator"], "AUTH-002");
    for mutant in mutants {
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
        assert_eq!(report.as_object().unwrap().len(), 2);
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report[field], serde_json::json!([]));
    }
}
