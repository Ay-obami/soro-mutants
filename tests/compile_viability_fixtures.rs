use std::fs;
use std::process::Command;

fn list_mutants(path: &str) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", path, "--json"])
        .output()
        .expect("list CLI should launch");
    assert!(
        output.status.success(),
        "listing {} failed: {}",
        path,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid JSON")
}

#[test]
fn compile_only_auth_fixture_mutants_are_syntactically_viable() {
    let report = list_mutants("fixtures/compile-only/auth");
    let mutants = report["mutants"].as_array().expect("mutants array");
    assert!(
        !mutants.is_empty(),
        "expected auth mutants to be discovered"
    );

    let source =
        fs::read_to_string("fixtures/compile-only/auth/src/lib.rs").expect("read fixture source");

    for mutant in mutants {
        let start = mutant["span"]["byte_start"].as_u64().unwrap() as usize;
        let end = mutant["span"]["byte_end"].as_u64().unwrap() as usize;
        let replacement = mutant["replacement"].as_str().unwrap();

        let mut mutated = source.clone();
        mutated.replace_range(start..end, replacement);

        let parse_result = syn::parse_file(&mutated);
        assert!(
            parse_result.is_ok(),
            "operator {} emitted invalid Rust syntax: {:?}",
            mutant["operator"],
            parse_result.err()
        );
    }
}

#[test]
fn compile_only_token_fixture_mutants_are_syntactically_viable() {
    let report = list_mutants("fixtures/compile-only/token");
    let mutants = report["mutants"].as_array().expect("mutants array");
    assert!(
        !mutants.is_empty(),
        "expected token mutants to be discovered"
    );

    let source =
        fs::read_to_string("fixtures/compile-only/token/src/lib.rs").expect("read fixture source");

    for mutant in mutants {
        let start = mutant["span"]["byte_start"].as_u64().unwrap() as usize;
        let end = mutant["span"]["byte_end"].as_u64().unwrap() as usize;
        let replacement = mutant["replacement"].as_str().unwrap();

        let mut mutated = source.clone();
        mutated.replace_range(start..end, replacement);

        let parse_result = syn::parse_file(&mutated);
        assert!(
            parse_result.is_ok(),
            "operator {} emitted invalid Rust syntax: {:?}",
            mutant["operator"],
            parse_result.err()
        );
    }
}

#[test]
fn compile_only_event_fixture_mutants_are_syntactically_viable() {
    let report = list_mutants("fixtures/compile-only/event");
    let mutants = report["mutants"].as_array().expect("mutants array");
    assert!(
        !mutants.is_empty(),
        "expected event mutants to be discovered"
    );

    let source =
        fs::read_to_string("fixtures/compile-only/event/src/lib.rs").expect("read fixture source");

    for mutant in mutants {
        let start = mutant["span"]["byte_start"].as_u64().unwrap() as usize;
        let end = mutant["span"]["byte_end"].as_u64().unwrap() as usize;
        let replacement = mutant["replacement"].as_str().unwrap();

        let mut mutated = source.clone();
        mutated.replace_range(start..end, replacement);

        let parse_result = syn::parse_file(&mutated);
        assert!(
            parse_result.is_ok(),
            "operator {} emitted invalid Rust syntax: {:?}",
            mutant["operator"],
            parse_result.err()
        );
    }
}

#[test]
fn compile_only_storage_fixture_mutants_are_syntactically_viable() {
    let report = list_mutants("fixtures/compile-only/storage");
    let mutants = report["mutants"].as_array().expect("mutants array");
    assert!(
        !mutants.is_empty(),
        "expected storage mutants to be discovered"
    );

    let source = fs::read_to_string("fixtures/compile-only/storage/src/lib.rs")
        .expect("read fixture source");

    for mutant in mutants {
        let start = mutant["span"]["byte_start"].as_u64().unwrap() as usize;
        let end = mutant["span"]["byte_end"].as_u64().unwrap() as usize;
        let replacement = mutant["replacement"].as_str().unwrap();

        let mut mutated = source.clone();
        mutated.replace_range(start..end, replacement);

        let parse_result = syn::parse_file(&mutated);
        assert!(
            parse_result.is_ok(),
            "operator {} emitted invalid Rust syntax: {:?}",
            mutant["operator"],
            parse_result.err()
        );
    }
}

#[test]
fn compile_viability_regression_detects_malformed_syntax() {
    let corrupted_source = "fn invalid() { let x = ; }";
    assert!(syn::parse_file(corrupted_source).is_err());
}
