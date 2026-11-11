use std::process::Command;
#[test]
fn call_003_lists_one_return_check_mutant() {
    for fixture in ["weak-call-return", "strong-call-return"] {
        let path = format!("fixtures/{fixture}");
        let out = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args(["list", &path, "--operator", "CALL-003", "--json"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let r: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let m = r["mutants"].as_array().unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0]["replacement"], "false");
    }
}
