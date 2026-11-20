use cargo_soro_mutants::parser::{
    fuzz_parse_source, parse_and_discover_source, validate_nesting, MAX_RUST_SYNTAX_NESTING,
};
use std::{fs, path::PathBuf, process::Command};

#[test]
fn parser_entry_point_handles_valid_soroban_contract() {
    let source = r#"
        #![no_std]
        use soroban_sdk::{contract, contractimpl, Address, Env};

        #[contract]
        pub struct Contract;

        #[contractimpl]
        impl Contract {
            pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
                from.require_auth();
            }
        }
    "#;

    let dummy_path = PathBuf::from("src/lib.rs");
    let res = parse_and_discover_source(source, &dummy_path);
    assert!(res.is_ok());
    let summary = res.unwrap();
    assert_eq!(summary.function_count, 1);
    assert!(summary.has_soroban_patterns);
}

#[test]
fn parser_entry_point_rejects_pathological_nesting_panic_free() {
    // Deliberate nesting exceeding MAX_RUST_SYNTAX_NESTING
    let deep_nested = "{".repeat(MAX_RUST_SYNTAX_NESTING + 5) + &"}".repeat(MAX_RUST_SYNTAX_NESTING + 5);
    let res = validate_nesting(&deep_nested);
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.contains("exceeds maximum syntax nesting depth"));

    // Ensure parse_and_discover_source also fails gracefully without panicking
    let dummy_path = PathBuf::from("src/deep.rs");
    let res2 = parse_and_discover_source(&deep_nested, &dummy_path);
    assert!(res2.is_err());
}

#[test]
fn parser_entry_point_handles_pathological_inputs_panic_free() {
    let seeds = [
        "",
        "   \n\t  ",
        "\0",
        "fn main() {\0}",
        "/* unclosed comment",
        "/* /* recursive unclosed comment *",
        "\"unclosed string literal",
        "\"escaped \\\" string",
        "fn f() { [ [ [ ( ( { } ) ) ] ] ] }",
        "struct Foo<T: 'static> where T: std::fmt::Debug { x: i32 }",
        "macro_rules! evil { () => { evil!() }; }",
        &"let x = 1;\n".repeat(1000),
        "fn unclosed_brace() {",
        "fn unexpected_tokens() { @#$%^&*! }",
        "fn null_in_ident_ab\0cd() {}",
        "🦀🦀🦀 soroban contract syntax error 🦀🦀🦀",
    ];

    for seed in seeds {
        fuzz_parse_source(seed);
    }
}

#[test]
fn pseudo_random_fuzz_simulation_for_parser() {
    let mut state: u64 = 0x9e3779b97f4a7c15;
    let mut next_u32 = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (state >> 32) as u32
    };

    let alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789{}()[];:,.<>/?!@#$%^&*-_=+\"'\\\n\t ";

    for _ in 0..1000 {
        let len = (next_u32() % 128) as usize;
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            let idx = (next_u32() as usize) % alphabet.len();
            bytes.push(alphabet[idx]);
        }

        if let Ok(s) = std::str::from_utf8(&bytes) {
            fuzz_parse_source(s);
        }
    }
}

#[test]
fn cli_list_survives_unparseable_and_malformed_source_panic_free() {
    let root = std::env::temp_dir().join(format!("soro-fuzz-cli-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).unwrap();

    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "fuzz-test"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    // Write a completely broken source file
    fs::write(
        root.join("src/lib.rs"),
        "this is not valid rust code @#$%%^&*() { unclosed\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("list")
        .arg(&root)
        .output()
        .expect("cargo-soro-mutants binary should execute");

    let _ = fs::remove_dir_all(&root);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked at"),
        "CLI panicked on unparseable input: {stderr}"
    );
}
