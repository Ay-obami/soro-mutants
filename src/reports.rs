//! Report/tool version context for pasted text and Markdown reports.
//!
//! Deterministic: the header depends only on compile-time constants, never on
//! time, host, paths or environment. Std-only; drop into `src/report_meta.rs`.

use std::fmt;

/// Bump on any breaking change to the text or Markdown report layout.
pub const REPORT_FORMAT_VERSION: u32 = 1;
const TOOL_NAME: &str = "soro-mutants";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportMeta {
    pub tool: String,
    pub tool_version: String,
    pub format_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaParseError {
    NotAReportHeader,
    MissingField(&'static str),
    InvalidFormatVersion(String),
}

impl fmt::Display for MetaParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAReportHeader => write!(
                f,
                "line is not a soro-mutants report header; expected `soro-mutants <version> (report format <n>)`"
            ),
            Self::MissingField(n) => write!(f, "report header is missing the {n}"),
            Self::InvalidFormatVersion(v) => {
                write!(f, "report format version `{v}` is not a non-negative integer")
            }
        }
    }
}
impl std::error::Error for MetaParseError {}

impl ReportMeta {
    /// Metadata of the running binary (version comes from Cargo at build time).
    pub fn current() -> Self {
        Self {
            tool: TOOL_NAME.into(),
            tool_version: env!("CARGO_PKG_VERSION").into(),
            format_version: REPORT_FORMAT_VERSION,
        }
    }

    /// Plain-text header: `soro-mutants 0.1.5 (report format 1)`.
    pub fn text_line(&self) -> String {
        format!("{} {} (report format {})", self.tool, self.tool_version, self.format_version)
    }

    /// Markdown header: the same line in italics so it survives rendering.
    pub fn markdown_line(&self) -> String {
        format!("_{}_", self.text_line())
    }

    /// Parse a header copied from a text or Markdown report (for diagnostics/tests).
    pub fn parse(line: &str) -> Result<Self, MetaParseError> {
        let s = line.trim().trim_matches('_');
        let (tool, rest) = s.split_once(' ').ok_or(MetaParseError::NotAReportHeader)?;
        if tool != TOOL_NAME {
            return Err(MetaParseError::NotAReportHeader);
        }
        let (ver, rest) = rest.split_once(' ').ok_or(MetaParseError::MissingField("report format"))?;
        if ver.is_empty() {
            return Err(MetaParseError::MissingField("tool version"));
        }
        let n = rest
            .strip_prefix("(report format ")
            .and_then(|r| r.strip_suffix(')'))
            .ok_or(MetaParseError::MissingField("report format"))?;
        let format_version =
            n.parse().map_err(|_| MetaParseError::InvalidFormatVersion(n.to_string()))?;
        Ok(Self { tool: tool.into(), tool_version: ver.into(), format_version })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        assert_eq!(ReportMeta::current().text_line(), ReportMeta::current().text_line());
    }

    #[test]
    fn roundtrips_text_and_markdown() {
        let m = ReportMeta::current();
        assert_eq!(ReportMeta::parse(&m.text_line()).unwrap(), m);
        assert_eq!(ReportMeta::parse(&m.markdown_line()).unwrap(), m);
    }

    #[test]
    fn structured_errors() {
        assert_eq!(ReportMeta::parse("hello"), Err(MetaParseError::NotAReportHeader));
        assert_eq!(
            ReportMeta::parse("soro-mutants 0.1.5 (report format x)"),
            Err(MetaParseError::InvalidFormatVersion("x".into()))
        );
        assert!(matches!(
            ReportMeta::parse("soro-mutants 0.1.5"),
            Err(MetaParseError::MissingField(_))
        ));
    }
}
