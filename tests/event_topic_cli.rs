use std::process::Command;
#[test]
fn event_005_lists_one_topic_substitution_in_fixtures() {
    for fixture in ["weak-event-topic", "strong-event-topic"] {
        let path = format!("fixtures/{fixture}");
        let out = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args(["list", &path, "--operator", "EVENT-005", "--json"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let m = report["mutants"].as_array().unwrap();
        assert_eq!(m.len(), 1, "{fixture}");
        assert_eq!(m[0]["original"], "actor");
        assert_eq!(m[0]["replacement"], "actor: alternate");
    }
}
