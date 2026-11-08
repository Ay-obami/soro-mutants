use std::process::Command;

#[test]
fn event_003_lists_one_actor_substitution_in_weak_and_strong_fixtures() {
    for fixture in ["weak-event-actor", "strong-event-actor"] {
        let path = format!("fixtures/{fixture}");
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args(["list", &path, "--operator", "EVENT-003", "--json"])
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
        assert_eq!(mutants[0]["original"], "actor");
        assert_eq!(mutants[0]["replacement"], "actor: alternate");
    }
}
