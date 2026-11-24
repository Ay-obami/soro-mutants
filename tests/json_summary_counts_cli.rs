use cargo_soro_mutants::report::{
    serialize_json_with_breakdowns, summary_by_file, summary_by_operator, OutcomeCounts,
    ReportMutant, ReportOutcome, ReportResult, ReportSpan,
};

#[test]
fn test_per_operator_and_per_file_summary_counts() {
    let results = vec![
        ReportResult {
            timing: None,
            mutant: ReportMutant {
                id: "M0001".into(),
                operator: "AUTH-001".into(),
                file: "src/admin.rs".into(),
                function: Some("check".into()),
                span: ReportSpan {
                    line: 10,
                    column: 5,
                },
                original: "orig".into(),
                replacement: "repl".into(),
                description: "desc".into(),
            },
            outcome: ReportOutcome::Killed,
        },
        ReportResult {
            timing: None,
            mutant: ReportMutant {
                id: "M0002".into(),
                operator: "AUTH-001".into(),
                file: "src/admin.rs".into(),
                function: Some("check".into()),
                span: ReportSpan {
                    line: 12,
                    column: 5,
                },
                original: "orig".into(),
                replacement: "repl".into(),
                description: "desc".into(),
            },
            outcome: ReportOutcome::Survived,
        },
        ReportResult {
            timing: None,
            mutant: ReportMutant {
                id: "M0003".into(),
                operator: "TOKEN-001".into(),
                file: "src/token.rs".into(),
                function: Some("transfer".into()),
                span: ReportSpan {
                    line: 20,
                    column: 5,
                },
                original: "orig".into(),
                replacement: "repl".into(),
                description: "desc".into(),
            },
            outcome: ReportOutcome::Unviable,
        },
    ];

    let by_op = summary_by_operator(&results);
    assert_eq!(by_op.len(), 2);
    assert_eq!(
        by_op["AUTH-001"],
        OutcomeCounts {
            generated: 2,
            killed: 1,
            survived: 1,
            unviable: 0,
            timeout: 0,
        }
    );
    assert_eq!(
        by_op["TOKEN-001"],
        OutcomeCounts {
            generated: 1,
            killed: 0,
            survived: 0,
            unviable: 1,
            timeout: 0,
        }
    );

    let by_file = summary_by_file(&results);
    assert_eq!(by_file.len(), 2);
    assert_eq!(by_file["src/admin.rs"].generated, 2);
    assert_eq!(by_file["src/token.rs"].generated, 1);

    let json = serialize_json_with_breakdowns(&results, None).expect("serialization succeeds");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["by_operator"]["AUTH-001"]["killed"], 1);
    assert_eq!(parsed["by_operator"]["AUTH-001"]["survived"], 1);
    assert_eq!(parsed["by_file"]["src/admin.rs"]["generated"], 2);
}

#[test]
fn cli_json_includes_operator_and_file_breakdowns() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args([
            "test",
            "fixtures/weak-auth",
            "--operator",
            "AUTH-001",
            "--test-command",
            "true",
            "--json",
        ])
        .output()
        .expect("CLI should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON report");
    let operator = &report["by_operator"]["AUTH-001"];
    assert!(operator["generated"].as_u64().unwrap_or(0) > 0);
    assert_eq!(operator["survived"], operator["generated"]);
    assert!(report["by_file"]
        .as_object()
        .is_some_and(|map| !map.is_empty()));
    assert!(report["by_function"].is_object());
}
