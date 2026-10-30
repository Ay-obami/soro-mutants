use std::process::Command;

#[test]
fn call_target_operator_lists_the_explicitly_substituted_address() {
    for fixture in ["weak-call-target", "strong-call-target"] {
        let path = format!("fixtures/{fixture}");
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args(["list", &path, "--operator", "CALL-001", "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let mutants = result["mutants"].as_array().unwrap();
        assert_eq!(mutants.len(), 1, "{fixture}");
        assert_eq!(mutants[0]["original"], "&primary");
        assert_eq!(mutants[0]["replacement"], "&alternate");
        assert!(mutants[0]["description"]
            .as_str()
            .unwrap()
            .contains("alternate instead of primary"));
    }
}
