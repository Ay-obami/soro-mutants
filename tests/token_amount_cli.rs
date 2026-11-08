use std::process::Command;
#[test]
fn token_003_lists_zero_and_one_mutants_in_weak_and_strong_fixtures() {
    for fixture in ["weak-token-amount", "strong-token-amount"] {
        let path = format!("fixtures/{fixture}");
        let out = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args(["list", &path, "--operator", "TOKEN-003", "--json"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let m = report["mutants"].as_array().unwrap();
        assert_eq!(m.len(), 2, "{fixture}");
        assert!(m.iter().any(|v| v["replacement"] == "&0"));
        assert!(m.iter().any(|v| v["replacement"] == "&1"));
    }
}
