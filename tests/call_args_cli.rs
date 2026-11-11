use std::process::Command;
#[test]
fn call_002_lists_typed_argument_substitutions() {
    for fixture in ["weak-call-args", "strong-call-args"] {
        let path = format!("fixtures/{fixture}");
        let out = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args(["list", &path, "--operator", "CALL-002", "--json"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let m = report["mutants"].as_array().unwrap();
        assert_eq!(m.len(), 3, "{fixture}");
        assert!(m
            .iter()
            .any(|v| v["original"] == "&actor" && v["replacement"] == "&alternate"));
        assert!(m
            .iter()
            .any(|v| v["original"] == "&amount" && v["replacement"] == "&other"));
    }
}
