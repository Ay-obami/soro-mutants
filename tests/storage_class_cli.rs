use std::process::Command;
#[test]
fn storage_001_lists_two_direct_class_substitutions() {
    for fixture in ["weak-storage-class", "strong-storage-class"] {
        let path = format!("fixtures/{fixture}");
        let out = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args([
                "list",
                &path,
                "--operator",
                "STORAGE-001",
                "--function",
                "write",
                "--json",
            ])
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
