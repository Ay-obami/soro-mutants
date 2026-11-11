use std::process::Command;
#[test]
fn token_002_lists_owner_and_spender_substitutions() {
    for fixture in ["weak-token-allowance", "strong-token-allowance"] {
        let path = format!("fixtures/{fixture}");
        let out = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args(["list", &path, "--operator", "TOKEN-002", "--json"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let r: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let m = r["mutants"].as_array().unwrap();
        assert_eq!(m.len(), 2);
    }
}
