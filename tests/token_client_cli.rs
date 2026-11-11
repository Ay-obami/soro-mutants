use std::process::Command;
#[test]
fn token_004_lists_one_deterministic_client_substitution() {
    for fixture in ["weak-token-client", "strong-token-client"] {
        let path = format!("fixtures/{fixture}");
        let out = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args(["list", &path, "--operator", "TOKEN-004", "--json"])
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
        assert_eq!(m[0]["original"], "primary");
        assert_eq!(m[0]["replacement"], "alternate");
    }
}
