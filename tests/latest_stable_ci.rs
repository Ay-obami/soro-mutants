//! Guards the latest-stable Rust CI contract for issue #181.
//!
//! The stable check lives in the `stable` job of `.github/workflows/msrv.yml`.
//! This test makes that contract explicit and catches silent regressions such
//! as pinning the job back to the MSRV toolchain, dropping its runtime bound,
//! or removing one of its verification steps. No workflow change is required
//! to run these tests locally.

use std::fs;
use std::path::Path;

fn repo_file(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// Extract the `stable` job block from a workflow, stopping at the next job.
fn stable_job(workflow: &str) -> Option<&str> {
    let start = workflow.find("\n  stable:")? + 1;
    let tail = &workflow[start..];
    let end = tail
        .lines()
        .enumerate()
        .skip(1)
        .find(|(_, line)| {
            line.starts_with("  ") && !line.starts_with("    ") && line.trim_end().ends_with(':')
        })
        .map(|(offset, _)| {
            tail.lines()
                .take(offset)
                .map(|line| line.len() + 1)
                .sum::<usize>()
        })
        .unwrap_or(tail.len());
    Some(&tail[..end])
}

/// Deterministic validation of the stable job. Returns every violation so a
/// contributor can fix them all in one pass.
fn validate_stable_job(workflow: &str) -> Vec<&'static str> {
    let Some(job) = stable_job(workflow) else {
        return vec!["missing 'stable' job in .github/workflows/msrv.yml"];
    };
    validate_job_body(job, workflow)
}

/// Validate an already-extracted stable job body. Taking the body directly
/// lets the regression seeds mutate one property at a time.
fn validate_job_body(job: &str, workflow: &str) -> Vec<&'static str> {
    let mut problems = Vec::new();
    if !job.contains("dtolnay/rust-toolchain@stable") {
        problems.push("stable job must use dtolnay/rust-toolchain@stable");
    }
    if job
        .lines()
        .any(|line| line.trim_start().starts_with("toolchain:"))
    {
        problems.push("stable job must not pin a toolchain version");
    }
    let bounded = job.lines().find_map(|line| {
        line.trim_start()
            .strip_prefix("timeout-minutes:")
            .and_then(|value| value.trim().parse::<u32>().ok())
    });
    match bounded {
        None => problems.push("stable job must declare timeout-minutes"),
        Some(minutes) if minutes > 15 => {
            problems.push("stable job runtime bound must be <= 15 minutes")
        }
        Some(_) => {}
    }
    for (needle, description) in [
        (
            "cargo check --locked --all-targets --all-features",
            "locked all-target compilation",
        ),
        (
            "cargo test --locked --test check_platform_cli",
            "platform policy regression test",
        ),
        (
            "cargo run --locked -- check-platform --json",
            "platform policy verification",
        ),
        (
            "cargo test --locked --test latest_stable_ci",
            "latest-stable CI contract test",
        ),
    ] {
        if !job.contains(needle) {
            problems.push(description);
        }
    }
    if !job.contains("GITHUB_STEP_SUMMARY") {
        problems.push("stable job must report toolchain versions to the step summary");
    }
    if !workflow.contains("workflow_dispatch:") || !workflow.contains("pull_request:") {
        problems.push("stable workflow must run on pull_request and workflow_dispatch");
    }
    problems
}

#[test]
fn stable_job_contract_is_satisfied() {
    let workflow = repo_file(".github/workflows/msrv.yml");
    let problems = validate_stable_job(&workflow);
    assert!(
        problems.is_empty(),
        "stable CI contract violations: {}",
        problems.join("; ")
    );
}

#[test]
fn regression_seed_pinned_and_unbounded_stable_jobs_are_rejected() {
    let workflow = repo_file(".github/workflows/msrv.yml");
    let job = stable_job(&workflow).expect("stable job");

    // Failure class 1: the stable check silently becomes another MSRV check.
    let pinned = job.replace(
        "- uses: dtolnay/rust-toolchain@stable",
        "- uses: dtolnay/rust-toolchain@master\n        with:\n          toolchain: \"1.85.0\"",
    );
    let problems = validate_job_body(&pinned, &workflow);
    assert!(
        problems.iter().any(|p| p.contains("must not pin")),
        "a pinned stable job must be rejected: {problems:?}"
    );

    // Failure class 2: the job loses its bounded runtime.
    let unbounded = job.replace("timeout-minutes: 15", "timeout-minutes: 90");
    let problems = validate_job_body(&unbounded, &workflow);
    assert!(
        problems.iter().any(|p| p.contains("runtime bound")),
        "an unbounded stable job must be rejected: {problems:?}"
    );

    // Failure class 3: the dedicated contract test is dropped from CI.
    let missing_test = job.replace(
        "        run: .github/scripts/check-runtime-budget.sh 30 cargo test --locked --test latest_stable_ci\n",
        "",
    );
    let problems = validate_job_body(&missing_test, &workflow);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("latest-stable CI contract test")),
        "dropping the regression target must be rejected: {problems:?}"
    );
}

#[test]
fn latest_stable_check_is_documented_for_contributors() {
    let platforms = repo_file("docs/supported-platforms.md");
    let ci = repo_file("docs/ci-integration.md");
    let contributing = repo_file("CONTRIBUTING.md");
    assert!(
        platforms.contains("latest stable"),
        "supported-platforms.md should name the latest-stable gate"
    );
    assert!(
        platforms.contains("cargo +stable"),
        "local reproduction must use +stable"
    );
    assert!(
        platforms.contains("latest_stable_ci"),
        "local reproduction must include the CI contract test"
    );
    assert!(
        ci.contains("Latest stable Rust check"),
        "ci-integration.md should describe the gate"
    );
    assert!(
        contributing.contains("latest_stable_ci"),
        "CONTRIBUTING.md should tell contributors how to run the test"
    );
}
