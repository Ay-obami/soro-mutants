use cargo_soro_mutants::report::{
    serialize_html, serialize_markdown, ReportMutant, ReportOutcome, ReportResult, ReportSpan,
};

#[test]
fn semantic_score_precision_and_rounding_contract() {
    let make_res = |id: &str, outcome: ReportOutcome| ReportResult {
        timing: None,
        mutant: ReportMutant {
            id: id.into(),
            operator: "AUTH-001".into(),
            file: "src/lib.rs".into(),
            function: Some("test".into()),
            span: ReportSpan { line: 1, column: 1 },
            original: "a".into(),
            replacement: "b".into(),
            description: "c".into(),
        },
        outcome,
    };

    // 1. Zero viable mutants: score must be exactly 0.0% in both Markdown and HTML
    let md_zero = serialize_markdown(&[]).unwrap();
    assert!(md_zero.contains("Semantic score: 0.0%"));
    let html_zero = serialize_html(&[]).unwrap();
    assert!(html_zero.contains("<p>Semantic score: 0.0%</p>"));

    // 2. 1 killed out of 3 viable (1 killed, 2 survived) -> 33.333% -> rounds to 33.3%
    let res_one_third = vec![
        make_res("M1", ReportOutcome::Killed),
        make_res("M2", ReportOutcome::Survived),
        make_res("M3", ReportOutcome::Survived),
    ];
    let md_one_third = serialize_markdown(&res_one_third).unwrap();
    assert!(md_one_third.contains("Semantic score: 33.3%"));
    let html_one_third = serialize_html(&res_one_third).unwrap();
    assert!(html_one_third.contains("<p>Semantic score: 33.3%</p>"));

    // 3. 2 killed out of 3 viable (2 killed, 1 survived) -> 66.666% -> rounds to 66.7%
    let res_two_thirds = vec![
        make_res("M1", ReportOutcome::Killed),
        make_res("M2", ReportOutcome::Killed),
        make_res("M3", ReportOutcome::Survived),
    ];
    let md_two_thirds = serialize_markdown(&res_two_thirds).unwrap();
    assert!(md_two_thirds.contains("Semantic score: 66.7%"));
    let html_two_thirds = serialize_html(&res_two_thirds).unwrap();
    assert!(html_two_thirds.contains("<p>Semantic score: 66.7%</p>"));

    // 4. 3 killed out of 3 viable -> exactly 100.0%
    let res_full = vec![
        make_res("M1", ReportOutcome::Killed),
        make_res("M2", ReportOutcome::Killed),
        make_res("M3", ReportOutcome::Killed),
    ];
    let md_full = serialize_markdown(&res_full).unwrap();
    assert!(md_full.contains("Semantic score: 100.0%"));
    let html_full = serialize_html(&res_full).unwrap();
    assert!(html_full.contains("<p>Semantic score: 100.0%</p>"));

    // 5. Timeouts and unviable are excluded from viable count:
    // 1 killed, 1 survived, 1 unviable, 1 timeout -> viable = 2, score = 50.0%
    let res_mixed = vec![
        make_res("M1", ReportOutcome::Killed),
        make_res("M2", ReportOutcome::Survived),
        make_res("M3", ReportOutcome::Unviable),
        make_res("M4", ReportOutcome::Timeout),
    ];
    let md_mixed = serialize_markdown(&res_mixed).unwrap();
    assert!(md_mixed.contains("Semantic score: 50.0%"));
    assert!(md_mixed.contains("1/2 viable mutants killed"));
    let html_mixed = serialize_html(&res_mixed).unwrap();
    assert!(html_mixed.contains("<p>Semantic score: 50.0%</p>"));
}
