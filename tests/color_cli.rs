use std::process::{Command, Output};

fn run(args: &[&str], no_color: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"));
    command
        .args(args)
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        // Exercise color even though the test captures output rather than using a TTY.
        .env("CLICOLOR_FORCE", "1")
        .env("TERM", "xterm-256color");
    if let Some(value) = no_color {
        command.env("NO_COLOR", value);
    }
    command.output().expect("CLI should run")
}

fn has_ansi(bytes: &[u8]) -> bool {
    bytes.windows(2).any(|window| window == b"\x1b[")
}

#[test]
fn no_color_disables_help_color_including_subcommands() {
    for args in [&["--help"][..], &["list", "--help"], &["test", "--help"]] {
        let colored = run(args, None);
        assert!(colored.status.success());
        assert!(has_ansi(&colored.stdout));

        for value in ["1", "", "0"] {
            let plain = run(args, Some(value));
            assert!(plain.status.success());
            assert!(!has_ansi(&plain.stdout));
            assert!(plain.stderr.is_empty());
            assert!(String::from_utf8_lossy(&plain.stdout).contains("Usage:"));
        }
    }
}

#[test]
fn no_color_disables_argument_error_color() {
    let args = ["list", "--not-an-option"];
    let colored = run(&args, None);
    assert!(!colored.status.success());
    assert!(has_ansi(&colored.stderr));

    for value in ["1", "", "0"] {
        let plain = run(&args, Some(value));
        assert_eq!(plain.status.code(), colored.status.code());
        assert!(plain.stdout.is_empty());
        assert!(!has_ansi(&plain.stderr));
        assert!(String::from_utf8_lossy(&plain.stderr).contains("unexpected argument"));
    }
}

#[test]
fn no_color_keeps_human_reports_unchanged() {
    for args in [
        vec!["list", "fixtures/weak-auth"],
        vec!["list", "fixtures/weak-auth", "--count-only"],
        vec!["test", "fixtures/weak-auth", "--operator", "TOKEN-001"],
    ] {
        let original = run(&args, None);
        assert!(original.status.success());
        assert!(!has_ansi(&original.stdout));
        let plain = run(&args, Some("1"));
        assert!(plain.status.success());
        assert_eq!(plain.stdout, original.stdout);
        assert_eq!(plain.stderr, original.stderr);
    }
}

#[test]
fn no_color_keeps_json_output_unchanged() {
    for args in [
        vec!["list", "fixtures/weak-auth", "--json"],
        vec![
            "test",
            "fixtures/weak-auth",
            "--operator",
            "TOKEN-001",
            "--json",
        ],
    ] {
        let original = run(&args, None);
        assert!(original.status.success());
        serde_json::from_slice::<serde_json::Value>(&original.stdout)
            .expect("stdout should contain a JSON report");
        for value in ["1", "", "0"] {
            let plain = run(&args, Some(value));
            assert!(plain.status.success());
            assert_eq!(plain.stdout, original.stdout);
            assert_eq!(plain.stderr, original.stderr);
            assert!(!has_ansi(&plain.stdout));
        }
    }
}
