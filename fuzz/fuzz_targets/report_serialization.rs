#![no_main]

use cargo_soro_mutants::report::{
    fuzz_serialize_all_reports, ReportMutant, ReportOutcome, ReportResult, ReportSpan,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let results = vec![ReportResult {
            mutant: ReportMutant {
                id: "M0001".into(),
                operator: s.chars().take(20).collect(),
                file: s.chars().take(50).collect(),
                function: Some(s.chars().take(30).collect()),
                span: ReportSpan { line: 1, column: 1 },
                original: s.chars().take(100).collect(),
                replacement: s.chars().take(100).collect(),
                description: s.chars().take(100).collect(),
            },
            outcome: ReportOutcome::Killed,
        }];
        fuzz_serialize_all_reports(&results);
    }
});
