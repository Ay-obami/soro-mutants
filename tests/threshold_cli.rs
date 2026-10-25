use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(args)
        .output()
        .expect("CLI should start")
}

#[test]
fn threshold_is_opt_in_and_uses_inclusive_score_boundary() {
    let args = [
        "test",
        "fixtures/weak-auth",
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
    ];
    assert!(run(&args).status.success());
    let equal = run(&[
        "test",
        "fixtures/weak-auth",
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
        "--min-score",
        "0",
    ]);
    assert!(
        equal.status.success(),
        "{}",
        String::from_utf8_lossy(&equal.stderr)
    );
    let below = run(&[
        "test",
        "fixtures/weak-auth",
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
        "--min-score",
        "0.1",
    ]);
    assert_eq!(below.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&below.stdout).contains("Score     : 0.0%"));
    assert!(String::from_utf8_lossy(&below.stderr).contains("below minimum"));
}

#[test]
fn empty_viable_set_scores_zero_and_honors_threshold() {
    let args = ["test", "fixtures/weak-auth", "--operator", "TOKEN-001"];
    assert!(
        run(&[args[0], args[1], args[2], args[3], "--min-score", "0"])
            .status
            .success()
    );
    let failure = run(&[args[0], args[1], args[2], args[3], "--min-score", "1"]);
    assert_eq!(failure.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&failure.stderr).contains("below minimum"));
}

#[test]
fn threshold_rejects_invalid_values_with_argument_error() {
    for value in ["-1", "100.01", "NaN", "inf", "nope"] {
        let output = run(&["test", "--min-score", value]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{value}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let upper = run(&[
        "test",
        "fixtures/weak-auth",
        "--operator",
        "TOKEN-001",
        "--min-score",
        "100",
    ]);
    assert_eq!(upper.status.code(), Some(4));
}
