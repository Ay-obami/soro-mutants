use std::process::Command;

fn list(path: &str) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", path, "--operator", "TOKEN-001", "--json"])
        .output()
        .expect("list CLI should launch");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid JSON")
}

#[test]
fn identical_sender_and_recipient_do_not_generate_noop_transfer_mutant() {
    let report = list("fixtures/equivalence-positive");
    assert_eq!(report["mutants"].as_array().unwrap().len(), 0);
}

#[test]
fn different_sender_and_recipient_keep_meaningful_transfer_mutant() {
    let report = list("fixtures/equivalence-negative");
    let mutants = report["mutants"].as_array().unwrap();
    assert_eq!(mutants.len(), 1);
    assert_eq!(mutants[0]["operator"], "TOKEN-001");
    assert!(mutants[0]["replacement"]
        .as_str()
        .unwrap()
        .contains("transfer(&recipient, &owner, &amount)"));
}
