use cargo_soro_mutants::report::{
    escape_html, escape_markdown_table_cell, fuzz_serialize_all_reports, serialize_html,
    serialize_json, serialize_markdown, ReportMutant, ReportOutcome, ReportResult, ReportSpan,
    REPORT_SCHEMA_VERSION,
};

#[test]
fn report_serialization_deterministic_and_valid() {
    let result = ReportResult {
        timing: None,
        mutant: ReportMutant {
            id: "M0001".into(),
            operator: "AUTH-001".into(),
            file: "src/lib.rs".into(),
            function: Some("transfer".into()),
            span: ReportSpan {
                line: 12,
                column: 5,
            },
            original: "user.require_auth()".into(),
            replacement: "()".into(),
            description: "Remove require_auth()".into(),
        },
        outcome: ReportOutcome::Killed,
    };

    let json = serialize_json(&[result.clone()], None).expect("json should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("json must be valid");
    assert_eq!(parsed["schema_version"], REPORT_SCHEMA_VERSION);
    assert_eq!(parsed["results"][0]["mutant"]["id"], "M0001");
    assert_eq!(parsed["results"][0]["outcome"], "KILLED");

    let md = serialize_markdown(&[result.clone()]).expect("markdown should serialize");
    assert!(md.contains("Semantic score: 100.0%"));
    assert!(md.contains("AUTH-001"));

    let html = serialize_html(&[result]).expect("html should serialize");
    assert!(html.contains("<!doctype html>"));
    assert!(html.contains("M0001"));
}

#[test]
fn report_serialization_handles_pathological_strings_panic_free() {
    let pathological_strings = [
        "",
        "   ",
        "\0",
        "<script>alert('xss')</script>",
        "\" ' & < >",
        "| piped | table |",
        "newline\nand\r\ncarriage\treturn",
        "🦀 Soroban Contract Fault Injection 🦀",
        "%s%d%n",
        &"A".repeat(5000),
    ];

    for (idx, str_val) in pathological_strings.iter().enumerate() {
        let result = ReportResult {
            timing: None,
            mutant: ReportMutant {
                id: format!("M{:04}", idx + 1),
                operator: (*str_val).to_string(),
                file: (*str_val).to_string(),
                function: Some((*str_val).to_string()),
                span: ReportSpan {
                    line: idx,
                    column: idx,
                },
                original: (*str_val).to_string(),
                replacement: (*str_val).to_string(),
                description: (*str_val).to_string(),
            },
            outcome: match idx % 4 {
                0 => ReportOutcome::Killed,
                1 => ReportOutcome::Survived,
                2 => ReportOutcome::Unviable,
                _ => ReportOutcome::Timeout,
            },
        };

        // Ensure all serialization formats handle arbitrary strings gracefully
        let json = serialize_json(&[result.clone()], None).expect("JSON must never fail");
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("JSON must be valid syntax");
        assert_eq!(parsed["schema_version"], REPORT_SCHEMA_VERSION);

        let html = serialize_html(&[result.clone()]).expect("HTML must never fail");
        assert!(!html.contains("<script>"), "HTML must escape script tags");

        let md = serialize_markdown(&[result.clone()]).expect("Markdown must never fail");
        assert!(!md.is_empty());
    }
}

#[test]
fn pseudo_random_fuzz_simulation_for_reports() {
    let mut state: u64 = 0x123456789abcdef0;
    let mut next_u32 = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (state >> 32) as u32
    };

    let alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_/<>\"'&|\n\0 ";

    let outcomes = [
        ReportOutcome::Killed,
        ReportOutcome::Survived,
        ReportOutcome::Unviable,
        ReportOutcome::Timeout,
    ];

    for _ in 0..1000 {
        let num_results = (next_u32() % 5) as usize;
        let mut results = Vec::with_capacity(num_results);

        for i in 0..num_results {
            let len = (next_u32() % 32) as usize;
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                let idx = (next_u32() as usize) % alphabet.len();
                bytes.push(alphabet[idx]);
            }
            let s = String::from_utf8_lossy(&bytes).into_owned();

            results.push(ReportResult {
                timing: None,
                mutant: ReportMutant {
                    id: format!("M{:04}", i + 1),
                    operator: s.clone(),
                    file: s.clone(),
                    function: Some(s.clone()),
                    span: ReportSpan { line: 1, column: 1 },
                    original: s.clone(),
                    replacement: s.clone(),
                    description: s,
                },
                outcome: outcomes[(next_u32() as usize) % outcomes.len()],
            });
        }

        fuzz_serialize_all_reports(&results);
    }
}

#[test]
fn html_and_markdown_escaping_correctness() {
    let raw = "<script>alert(\"hello & welcome\")</script>";
    let escaped = escape_html(raw);
    assert_eq!(
        escaped,
        "&lt;script&gt;alert(&quot;hello &amp; welcome&quot;)&lt;/script&gt;"
    );

    let md_raw = "pipe | cell \n newline";
    let md_escaped = escape_markdown_table_cell(md_raw);
    assert_eq!(md_escaped, "pipe \\| cell   newline");
}
