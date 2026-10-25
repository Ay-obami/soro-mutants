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
