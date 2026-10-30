use std::process::Command;

#[test]
fn ttl_lifecycle_operator_discovery_in_both_fixtures() {
    for fixture in ["weak-ttl-lifecycle", "strong-ttl-lifecycle"] {
        let path = format!("fixtures/{fixture}");
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args(["list", &path, "--json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let mutants = report["mutants"].as_array().unwrap();
        for operator in ["TTL-002", "TTL-003"] {
            assert_eq!(
                mutants.iter().filter(|m| m["operator"] == operator).count(),
                1,
                "{fixture}: {operator}"
            );
        }
        let original = mutants.iter().find(|m| m["operator"] == "TTL-002").unwrap();
        assert_eq!(original["original"], "100");
        assert_eq!(original["replacement"], "0");
        let target = mutants.iter().find(|m| m["operator"] == "TTL-003").unwrap();
        assert_eq!(target["original"], "1000");
        assert_eq!(target["replacement"], "100");
    }
}
