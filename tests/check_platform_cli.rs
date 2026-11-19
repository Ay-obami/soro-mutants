use serde_json::Value;
use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["check-platform"])
        .args(args)
        .output()
        .expect("soro-mutants should run")
}

#[test]
fn platform_json_report_is_structured_and_versioned() {
    let output = run(&["--json"]);
    // An unsupported environment is reported as structured JSON with exit 1,
    // so only assert success for supported/best-effort hosts.
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        output.status.success() || report["status"] == "unsupported",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["policy_version"], "1.0");
    assert_eq!(report["msrv"], "1.85.0");
    assert!(matches!(
        report["status"].as_str().unwrap(),
        "supported" | "best-effort" | "unsupported"
    ));
    assert!(report["summary"].as_str().unwrap().contains("environment"));

    let ids = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|check| check["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    for required in [
        "operating-system",
        "architecture",
        "rustc-msrv",
        "cargo-version",
        "shell",
        "git",
    ] {
        assert!(ids.contains(&required), "missing check {required}");
    }
}

#[test]
fn platform_text_report_shows_remediation_and_policy_link() {
    let output = run(&[]);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() || text.contains("does not meet"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("Soro Mutants v1.0 platform check"));
    assert!(text.contains("MSRV: 1.85.0"));
    assert!(text.contains("docs/supported-platforms.md"));
}
