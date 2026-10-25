use std::process::Command;

#[test]
fn event_002_lists_exactly_one_standalone_typed_event_mutant_per_fixture() {
    for fixture in ["weak-typed-events", "strong-typed-events"] {
        let path = format!("fixtures/{fixture}");
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args(["list", &path, "--operator", "EVENT-002", "--json"])
            .output()
            .expect("list command should start");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let mutants = report["mutants"].as_array().unwrap();
        assert_eq!(mutants.len(), 1, "{fixture}");
        assert_eq!(mutants[0]["operator"], "EVENT-002");
        assert_eq!(mutants[0]["replacement"], "()");
        assert_eq!(mutants[0]["file"], "src/lib.rs");
        assert!(mutants[0]["original"]
            .as_str()
            .unwrap()
            .contains("TransferNotice { recipient, amount }.publish(&env)"));
    }
}
