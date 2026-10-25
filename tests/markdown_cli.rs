use std::{fs, process::Command};

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(args)
        .output()
        .expect("CLI should run")
}

#[test]
fn markdown_report_is_deterministic_and_includes_links_and_score() {
    let args = [
        "test",
        "fixtures/weak-auth",
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
        "--markdown",
    ];
    let first = run(&args);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, run(&args).stdout);
    let report = String::from_utf8(first.stdout).unwrap();
    let expected = concat!(
        "# Soro Mutants Report\n\n",
        "| Metric | Count |\n| --- | ---: |\n",
        "| Generated | 1 |\n| Killed | 0 |\n| Survived | 1 |\n",
        "| Unviable | 0 |\n| Timeout | 0 |\n\n",
        "**Semantic score: 0.0%** (0/1 viable mutants killed).\n\n",
        "## Survived (1)\n\n",
        "- `M0001` **AUTH-001** [src/lib.rs:11](src/lib.rs#L11) — remove current_admin.require_auth()\n\n",
        "## Killed (0)\n\n_None._\n\n",
        "## Unviable (0)\n\n_None._\n\n",
        "## Timeout (0)\n\n_None._\n\n",
    );
    assert_eq!(report, expected, "Markdown snapshot changed unexpectedly");
}

#[test]
fn github_summary_appends_survivors_without_replacing_stdout() {
    let summary_path = std::env::temp_dir().join(format!(
        "soro-mutants-summary-{}-{}.md",
        std::process::id(),
        "survivors"
    ));
    fs::write(&summary_path, "Prior job content\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args([
            "test",
            "fixtures/weak-auth",
            "--operator",
            "AUTH-001",
            "--test-command",
            "true",
            "--github-summary",
        ])
        .env("GITHUB_STEP_SUMMARY", &summary_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Summary"));
    let summary = fs::read_to_string(&summary_path).unwrap();
    assert!(summary.starts_with("Prior job content\n# Soro Mutants Report"));
    assert!(summary.contains("| Survived | 1 |"));
    assert!(summary.contains("**Semantic score: 0.0%**"));
    assert!(summary.contains("[src/lib.rs:11](src/lib.rs#L11)"));
    fs::remove_file(summary_path).unwrap();
}

#[test]
fn github_summary_handles_empty_results_and_missing_environment() {
    let summary_path = std::env::temp_dir().join(format!(
        "soro-mutants-summary-{}-empty.md",
        std::process::id()
    ));
    let empty = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args([
            "test",
            "fixtures/weak-auth",
            "--operator",
            "TOKEN-001",
            "--github-summary",
        ])
        .env("GITHUB_STEP_SUMMARY", &summary_path)
        .output()
        .unwrap();
    assert!(empty.status.success());
    let summary = fs::read_to_string(&summary_path).unwrap();
    assert!(summary.contains("| Generated | 0 |"));
    assert!(summary.contains("## Survived (0)"));
    fs::remove_file(summary_path).unwrap();
    let local = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args([
            "test",
            "fixtures/weak-auth",
            "--operator",
            "TOKEN-001",
            "--github-summary",
        ])
        .env_remove("GITHUB_STEP_SUMMARY")
        .output()
        .unwrap();
    assert!(local.status.success());
    assert!(String::from_utf8_lossy(&local.stdout).contains("No matching"));
}

#[test]
fn markdown_empty_run_has_all_sections_and_conflicts_with_json() {
    let output = run(&[
        "test",
        "fixtures/weak-auth",
        "--operator",
        "TOKEN-001",
        "--markdown",
    ]);
    assert!(output.status.success());
    let report = String::from_utf8(output.stdout).unwrap();
    assert!(report.contains("| Generated | 0 |"));
    assert!(report.contains("## Survived (0)"));
    assert!(report.contains("## Killed (0)"));
    let conflict = run(&["test", "fixtures/weak-auth", "--markdown", "--json"]);
    assert_eq!(conflict.status.code(), Some(2));
}
