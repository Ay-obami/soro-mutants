use std::process::Command;

#[test]
fn every_subcommand_exposes_help() {
    for command in [
        "list", "test", "clean", "validate-corpus", "benchmark-matrix", "check-platform", "operators", "completions",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .args([command, "--help"])
            .output()
            .expect("CLI should launch");
        assert!(output.status.success(), "{command}: {}", String::from_utf8_lossy(&output.stderr));
        let help = String::from_utf8_lossy(&output.stdout);
        assert!(help.contains("Usage:"), "{command} help should include usage");
        assert!(help.contains(command), "{command} help should mention the subcommand");
    }
}
