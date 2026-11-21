use std::process::Command;

#[test]
fn generates_completions_for_supported_shells() {
    for shell in ["bash", "zsh", "fish"] {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args(["completions", shell])
            .output()
            .expect("CLI should launch");
        assert!(
            output.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let script = String::from_utf8(output.stdout).expect("completion output should be UTF-8");
        assert!(
            script.contains("soro-mutants"),
            "{shell} completion should mention the command"
        );
        assert!(script.len() > 100, "{shell} completion should not be empty");
    }
}

#[test]
fn rejects_unsupported_completion_shell() {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["completions", "powershell"])
        .output()
        .expect("CLI should launch");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid value"));
}

#[test]
fn operator_completion_lists_operator_ids() {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["completions", "bash"])
        .output()
        .expect("CLI should launch");
    assert!(output.status.success());
    let script = String::from_utf8_lossy(&output.stdout);
    assert!(script.contains("AUTH-001"));
    assert!(script.contains("TTL-003"));
}

#[test]
fn bash_path_completion_does_not_split_paths_with_spaces() {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["completions", "bash"])
        .output()
        .expect("CLI should launch");
    assert!(output.status.success());
    let script = String::from_utf8(output.stdout).unwrap();

    assert!(script.contains("--file)"));
    assert!(script.contains("--changed-file)"));
    assert!(
        script.contains(r#"while IFS= read -r completion"#),
        "Bash file completions must read one completion per line"
    );
    assert!(
        script.contains(r#"COMPREPLY+=("$completion")"#),
        "Bash file completions must append each path as one array element"
    );
    assert!(
        !script.contains(r#"COMPREPLY=($(compgen -f "${cur}"))"#),
        "unquoted command substitution splits paths containing spaces"
    );
}
