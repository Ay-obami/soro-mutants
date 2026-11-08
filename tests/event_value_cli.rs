use std::process::Command;
#[test]
fn event_004_lists_one_numeric_substitution_in_weak_and_strong_fixtures() {
    for fixture in ["weak-event-value", "strong-event-value"] {
        let path = format!("fixtures/{fixture}");
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args(["list", &path, "--operator", "EVENT-004", "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let mutants = report["mutants"].as_array().unwrap();
        assert_eq!(mutants.len(), 1, "{fixture}");
        assert_eq!(mutants[0]["original"], "amount");
        assert_eq!(mutants[0]["replacement"], "amount: 0");
    }
}
