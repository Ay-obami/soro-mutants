use serde_json::Value;
use std::process::Command;

fn list(fixture: &str, operator: &str) -> Vec<Value> {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", fixture, "--operator", operator, "--json"])
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
fn initialization_operator_targets_only_recognized_initializers() {
    for fixture in ["fixtures/weak-init-auth", "fixtures/strong-init-auth"] {
        let found = list(fixture, "AUTH-006");
        assert_eq!(found.len(), 2, "{fixture}");
        for mutant in &found {
            assert_eq!(mutant["operator"], "AUTH-006");
            assert_eq!(mutant["file"], "src/lib.rs");
            assert_eq!(mutant["function"], "initialize");
            assert_eq!(mutant["original"], "admin.require_auth()");
        }
        assert_eq!(found[0]["replacement"], "()");
        assert_eq!(found[1]["replacement"], "proposer.require_auth()");
        assert!(
            list(fixture, "AUTH-007").is_empty(),
            "initializers must not produce upgrade mutants in {fixture}"
        );
    }
}

#[test]
fn upgrade_operator_targets_only_recognized_upgrade_entrypoints() {
    for fixture in ["fixtures/weak-upgrade-auth", "fixtures/strong-upgrade-auth"] {
        let found = list(fixture, "AUTH-007");
        assert_eq!(found.len(), 2, "{fixture}");
        for mutant in &found {
            assert_eq!(mutant["operator"], "AUTH-007");
            assert_eq!(mutant["file"], "src/lib.rs");
            assert_eq!(mutant["function"], "upgrade");
            assert_eq!(mutant["original"], "admin.require_auth()");
        }
        assert_eq!(found[0]["replacement"], "()");
        assert_eq!(found[1]["replacement"], "proposer.require_auth()");
        assert!(
            list(fixture, "AUTH-006").is_empty(),
            "upgrades must not produce initialization mutants in {fixture}"
        );
    }
}

#[test]
fn lifecycle_operators_ignore_generic_authorization_fixtures() {
    for fixture in [
        "fixtures/weak-auth",
        "fixtures/strong-auth",
        "fixtures/weak-auth-admin",
    ] {
        assert!(
            list(fixture, "AUTH-006").is_empty(),
            "{fixture} is not a lifecycle fixture"
        );
        assert!(
            list(fixture, "AUTH-007").is_empty(),
            "{fixture} is not a lifecycle fixture"
        );
    }
}
