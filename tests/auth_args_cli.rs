use serde_json::Value;
use std::process::Command;

fn mutants(path: &str) -> Vec<Value> {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", path, "--operator", "AUTH-003", "--json"])
        .output()
        .expect("CLI should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["mutants"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn auth_args_operator_targets_only_authorized_argument_span_in_both_fixtures() {
    for fixture in ["fixtures/weak-auth-args", "fixtures/strong-auth-args"] {
        let found = mutants(fixture);
        assert_eq!(found.len(), 1, "{fixture}");
        let mutant = &found[0];
        assert_eq!(mutant["operator"], "AUTH-003");
        assert_eq!(mutant["file"], "src/lib.rs");
        assert_eq!(mutant["function"], "authorize");
        assert_eq!(mutant["original"], "(recipient.clone(),).into_val(&env)");
        assert_eq!(mutant["replacement"], "soroban_sdk::Vec::new(&env)");
        assert_eq!(mutant["span"]["line"], 11);
    }
}
