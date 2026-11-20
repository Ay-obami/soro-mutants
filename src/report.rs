//! Robust serialization entry points for structured reports (JSON, Markdown, HTML).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as FmtWrite;

pub const REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReportOutcome {
    Killed,
    Survived,
    Unviable,
    Timeout,
}

impl ReportOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Killed => "KILLED",
            Self::Survived => "SURVIVED",
            Self::Unviable => "UNVIABLE",
            Self::Timeout => "TIMEOUT",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportSpan {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportMutant {
    pub id: String,
    pub operator: String,
    pub file: String,
    pub function: Option<String>,
    pub span: ReportSpan,
    pub original: String,
    pub replacement: String,
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportResult {
    pub mutant: ReportMutant,
    pub outcome: ReportOutcome,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportMetadata {
    pub rustc_version: Option<String>,
    pub cargo_version: Option<String>,
    pub test_command: String,
    pub compile_timeout_seconds: u64,
    pub test_timeout_seconds: u64,
}

#[derive(Serialize)]
pub struct JsonTestReport<'a> {
    pub schema_version: u32,
    pub results: &'a [ReportResult],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<&'a ReportMetadata>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeJsonTestReport {
    pub schema_version: u32,
    pub results: Vec<ReportResult>,
    pub metadata: Option<ReportMetadata>,
}

pub fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

pub fn escape_markdown_table_cell(input: &str) -> String {
    input
        .replace('|', "\\|")
        .replace('\n', " ")
        .replace('\r', "")
}

pub fn serialize_json(
    results: &[ReportResult],
    metadata: Option<&ReportMetadata>,
) -> Result<String, String> {
    let report = JsonTestReport {
        schema_version: REPORT_SCHEMA_VERSION,
        results,
        metadata,
    };
    serde_json::to_string_pretty(&report).map_err(|e| format!("JSON serialization error: {e}"))
}

pub fn serialize_markdown(results: &[ReportResult]) -> Result<String, String> {
    let mut out = String::new();
    let killed = results
        .iter()
        .filter(|r| r.outcome == ReportOutcome::Killed)
        .count();
    let survived = results
        .iter()
        .filter(|r| r.outcome == ReportOutcome::Survived)
        .count();
    let unviable = results
        .iter()
        .filter(|r| r.outcome == ReportOutcome::Unviable)
        .count();
    let timeout = results
        .iter()
        .filter(|r| r.outcome == ReportOutcome::Timeout)
        .count();
    let viable = killed + survived;
    let score = if viable == 0 {
        0.0
    } else {
        100.0 * (killed as f64) / (viable as f64)
    };

    writeln!(out, "# Soro Mutants Report\n").map_err(|e| e.to_string())?;
    writeln!(out, "| Metric | Count |").map_err(|e| e.to_string())?;
    writeln!(out, "| --- | ---: |").map_err(|e| e.to_string())?;
    for (name, count) in [
        ("Generated", results.len()),
        ("Killed", killed),
        ("Survived", survived),
        ("Unviable", unviable),
        ("Timeout", timeout),
    ] {
        writeln!(out, "| {name} | {count} |").map_err(|e| e.to_string())?;
    }

    writeln!(
        out,
        "\n**Semantic score: {score:.1}%** ({killed}/{viable} viable mutants killed).\n"
    )
    .map_err(|e| e.to_string())?;

    for (heading, label, count) in [
        ("Survived", "SURVIVED", survived),
        ("Killed", "KILLED", killed),
        ("Unviable", "UNVIABLE", unviable),
        ("Timeout", "TIMEOUT", timeout),
    ] {
        writeln!(out, "## {heading} ({count})\n").map_err(|e| e.to_string())?;
        if count == 0 {
            writeln!(out, "_None._\n").map_err(|e| e.to_string())?;
            continue;
        }
        for result in results.iter().filter(|r| r.outcome.as_str() == label) {
            let m = &result.mutant;
            writeln!(
                out,
                "- **{}** `{}` ({}:{}) — `{}`",
                escape_markdown_table_cell(&m.id),
                escape_markdown_table_cell(&m.operator),
                escape_markdown_table_cell(&m.file),
                m.span.line,
                escape_markdown_table_cell(m.function.as_deref().unwrap_or("unknown"))
            )
            .map_err(|e| e.to_string())?;
            writeln!(
                out,
                "  - Description: {}",
                escape_markdown_table_cell(&m.description)
            )
            .map_err(|e| e.to_string())?;
        }
        writeln!(out).map_err(|e| e.to_string())?;
    }

    Ok(out)
}

pub fn serialize_html(results: &[ReportResult]) -> Result<String, String> {
    let mut out = String::new();
    let mut counts = [0usize; 4];
    let mut by_operator: BTreeMap<&str, [usize; 4]> = BTreeMap::new();

    for r in results {
        let idx = match r.outcome {
            ReportOutcome::Killed => 0,
            ReportOutcome::Survived => 1,
            ReportOutcome::Unviable => 2,
            ReportOutcome::Timeout => 3,
        };
        counts[idx] += 1;
        by_operator.entry(&r.mutant.operator).or_default()[idx] += 1;
    }

    let viable = counts[0] + counts[1];
    let score = if viable == 0 {
        0.0
    } else {
        counts[0] as f64 * 100.0 / viable as f64
    };

    writeln!(
        out,
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Soro Mutants Report</title>
</head>
<body>
<h1>Soro Mutants Report</h1>
<section><h2>Summary</h2>
<p>Semantic score: {score:.1}%</p>
</section>
<section><h2>Mutant details</h2>"#
    )
    .map_err(|e| e.to_string())?;

    for r in results {
        let m = &r.mutant;
        writeln!(
            out,
            "<article><h3>{} &mdash; {} &mdash; {}</h3><small>{}:{} (function: {})</small><p>{}</p><pre><code>{}</code></pre></article>",
            escape_html(&m.id),
            escape_html(&m.operator),
            escape_html(r.outcome.as_str()),
            escape_html(&m.file),
            m.span.line,
            escape_html(m.function.as_deref().unwrap_or("unknown")),
            escape_html(&m.description),
            escape_html(&m.replacement)
        )
        .map_err(|e| e.to_string())?;
    }

    writeln!(out, "</section></body></html>").map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn fuzz_serialize_all_reports(results: &[ReportResult]) {
    let _ = serialize_json(results, None);
    let _ = serialize_markdown(results);
    let _ = serialize_html(results);
}
