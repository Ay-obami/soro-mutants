use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("operators")
        .args(args)
        .output()
        .expect("operators command should run")
}

#[test]
fn operators_exits_successfully() {
    let output = run(&[]);
    assert!(
        output.status.success(),
        "operators should exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn operators_lists_all_known_ids() {
    let output = run(&[]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    for id in ["AUTH-001", "AUTH-002", "EVENT-001", "TOKEN-001", "TTL-001"] {
        assert!(
            stdout.contains(id),
            "expected operator id {id} in output:\n{stdout}"
        );
    }
}

#[test]
fn operators_output_is_deterministic_and_sorted() {
    let first = run(&[]);
    let second = run(&[]);
    assert!(first.status.success());
    assert!(second.status.success());
    // Identical on every invocation.
    assert_eq!(first.stdout, second.stdout);

    // IDs appear in ascending lexicographic order.
    let stdout = String::from_utf8(first.stdout).unwrap();
    let ids: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|token| token.contains('-'))
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "operator IDs should appear in sorted order");
}

#[test]
fn operators_includes_one_line_description_per_id() {
    let output = run(&[]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let id_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| {
            line.split_whitespace()
                .next()
                .map(|t| t.contains('-'))
                .unwrap_or(false)
        })
        .collect();
    // Every ID line must contain a non-empty description after the ID.
    for line in &id_lines {
        let parts: Vec<&str> = line.splitn(2, "  ").collect();
        assert_eq!(
            parts.len(),
            2,
            "expected ID and description separated by two spaces: {line}"
        );
        assert!(
            !parts[1].trim().is_empty(),
            "description for '{}' should not be empty",
            parts[0].trim()
        );
    }
}

#[test]
fn operators_help_flag_succeeds() {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["operators", "--help"])
        .output()
        .expect("operators --help should run");
    assert!(
        output.status.success(),
        "operators --help should exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("operator") || text.contains("Operator"),
        "--help output should mention operators"
    );
}
