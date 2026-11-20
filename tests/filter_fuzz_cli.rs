use cargo_soro_mutants::filters::{
    fuzz_parse_all_filters, parse_exclude_glob, parse_file_filter, parse_function_filter,
    parse_operator, parse_shard, Shard, KNOWN_OPERATOR_IDS,
};
use std::process::Command;

#[test]
fn operator_filter_parsing_deterministic_and_panic_free() {
    for id in KNOWN_OPERATOR_IDS {
        let result = parse_operator(id);
        assert_eq!(result, Ok((*id).to_string()));
    }

    let pathological_inputs = [
        "",
        "   ",
        "\0",
        "AUTH-001\0",
        "auth-001",
        "AUTH_001",
        "INVALID-999",
        "TOKEN-999",
        "🦀",
        "'; DROP TABLE mutants;--",
        &"A".repeat(5000),
    ];

    for input in pathological_inputs {
        let result = parse_operator(input);
        assert!(result.is_err(), "input {input:?} should fail gracefully");
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("unknown operator ID"),
            "expected structured unknown operator message, got: {err_msg}"
        );
    }
}

#[test]
fn filter_operator_registry_matches_real_cli_registry() {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("operators")
        .output()
        .expect("operators command should run");
    assert!(
        output.status.success(),
        "operators command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("operators output is UTF-8");
    let cli_ids: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|token| token.contains('-'))
        .collect();
    assert_eq!(cli_ids, KNOWN_OPERATOR_IDS);
}

#[test]
fn shard_filter_parsing_deterministic_and_panic_free() {
    let valid_shards = [
        ("1/1", 1, 1),
        ("1/2", 1, 2),
        ("2/2", 2, 2),
        ("1/100", 1, 100),
        ("50/100", 50, 100),
        ("100/100", 100, 100),
    ];

    for (input, expected_idx, expected_total) in valid_shards {
        let shard = parse_shard(input).expect("valid shard should parse");
        assert_eq!(
            shard,
            Shard {
                index: expected_idx,
                total: expected_total
            }
        );
    }

    let pathological_shards = [
        "",
        "/",
        "//",
        "///",
        "1",
        "1/",
        "/1",
        "0/0",
        "0/1",
        "1/0",
        "2/1",
        "100/50",
        "-1/2",
        "1/-2",
        "+1/2",
        "1/+2",
        "1.0/2.0",
        "1 / 2",
        " 1/2",
        "1/2 ",
        "a/b",
        "1/b",
        "a/2",
        "\0/1",
        "1/\0",
        "1/999999999999999999999999999999999999999999999999999999999999999999999999999",
        "999999999999999999999999999999999999999999999999999999999999999999999999999/1",
        "NaN/NaN",
        "Infinity/1",
    ];

    for input in pathological_shards {
        let result = parse_shard(input);
        assert!(result.is_err(), "shard {input:?} must fail validation");
    }
}

#[test]
fn exclude_glob_filter_parsing_panic_free() {
    assert!(parse_exclude_glob("src/**/*.rs").is_ok());
    assert!(parse_exclude_glob("tests/*").is_ok());
    assert!(parse_exclude_glob("target").is_ok());

    let pathological_globs = ["", "[", "[]", "[a-", "**/**/**/[", &"a/".repeat(2000)];

    for input in pathological_globs {
        let result = parse_exclude_glob(input);
        assert!(result.is_err(), "glob {input:?} should fail gracefully");
    }
}

#[test]
fn file_and_function_filter_parsing_panic_free() {
    assert!(parse_file_filter("src/contract.rs").is_ok());
    assert!(parse_file_filter("tests/lib.rs").is_ok());
    assert!(parse_function_filter("transfer").is_ok());
    assert!(parse_function_filter("require_auth").is_ok());

    assert!(parse_file_filter("").is_err());
    assert!(parse_file_filter("contract\0.rs").is_err());
    assert!(parse_file_filter(&"x".repeat(5000)).is_err());

    assert!(parse_function_filter("").is_err());
    assert!(parse_function_filter("fn\0name").is_err());
    assert!(parse_function_filter("fn\nname").is_err());
    assert!(parse_function_filter(&"x".repeat(2000)).is_err());
}

#[test]
fn pseudo_random_fuzz_simulation() {
    let mut state: u64 = 0x853c49e6748fea9b;
    let mut next_u32 = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (state >> 32) as u32
    };

    let alphabet =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-/_.*[]\\ \t\n\0:";

    for _ in 0..5000 {
        let len = (next_u32() % 64) as usize;
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            let idx = (next_u32() as usize) % alphabet.len();
            bytes.push(alphabet[idx]);
        }

        if let Ok(s) = std::str::from_utf8(&bytes) {
            fuzz_parse_all_filters(s);
        }
    }
}

#[test]
fn cli_rejects_malformed_filters_with_exit_code_2() {
    let binary = env!("CARGO_BIN_EXE_cargo-soro-mutants");

    let bad_invocations: &[&[&str]] = &[
        &["list", "--operator", "NOT-A-REAL-OP"],
        &["list", "--shard", "9/2"],
        &["list", "--shard", "0/1"],
        &["list", "--exclude-path", "["],
        &["list", "--file", ""],
        &["list", "--function", ""],
    ];

    for args in bad_invocations {
        let output = Command::new(binary)
            .args(*args)
            .output()
            .expect("binary should execute");

        assert_eq!(
            output.status.code(),
            Some(2),
            "command {:?} should exit with code 2 on invalid filter",
            args
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.is_empty(),
            "command {:?} should output stderr",
            args
        );
    }
}
