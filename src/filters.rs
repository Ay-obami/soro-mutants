//! Structured parsers and validators for CLI filters.
//!
//! Handles operator, glob exclusion, shard, file, and function filters with
//! deterministic error reporting and fuzz-safe panic-free guarantees.

use globset::{Glob, GlobBuilder};
use std::fmt;
use std::str::FromStr;

/// All known built-in operator IDs in ascending lexicographic order.
pub const KNOWN_OPERATOR_IDS: &[&str] = &[
    "AUTH-001",
    "AUTH-002",
    "AUTH-003",
    "AUTH-004",
    "AUTH-005",
    "AUTH-006",
    "AUTH-007",
    "CALL-001",
    "CALL-002",
    "CALL-003",
    "EVENT-001",
    "EVENT-002",
    "EVENT-003",
    "EVENT-004",
    "EVENT-005",
    "STORAGE-001",
    "TOKEN-001",
    "TOKEN-002",
    "TOKEN-003",
    "TOKEN-004",
    "TTL-001",
    "TTL-002",
    "TTL-003",
];

/// Structured error returned when CLI filter parsing fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterParseError {
    /// The specified operator ID is not recognized.
    UnknownOperator {
        requested: String,
        valid: Vec<&'static str>,
    },
    /// The exclusion glob is malformed or invalid.
    InvalidExcludeGlob { glob: String, details: String },
    /// The shard specifier is malformed or out of bounds.
    InvalidShard { input: String, reason: String },
    /// The file filter path contains invalid or pathological characters.
    InvalidFileFilter { input: String, reason: String },
    /// The function filter identifier is malformed.
    InvalidFunctionFilter { input: String, reason: String },
}

impl fmt::Display for FilterParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOperator { requested, valid } => {
                let list = valid.join(", ");
                write!(f, "unknown operator ID {requested:?}; valid IDs: {list}")
            }
            Self::InvalidExcludeGlob { glob, details } => {
                write!(f, "invalid exclude-path glob {glob:?}: {details}")
            }
            Self::InvalidShard { input, reason } => {
                write!(
                    f,
                    "invalid shard {input:?}: {reason}; expected INDEX/TOTAL with 1 <= INDEX <= TOTAL (positive integers)"
                )
            }
            Self::InvalidFileFilter { input, reason } => {
                write!(f, "invalid file filter {input:?}: {reason}")
            }
            Self::InvalidFunctionFilter { input, reason } => {
                write!(f, "invalid function filter {input:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for FilterParseError {}

/// Parse and validate an operator ID against known built-in operators.
pub fn parse_operator(value: &str) -> Result<String, String> {
    if KNOWN_OPERATOR_IDS.contains(&value) {
        Ok(value.to_owned())
    } else {
        Err(FilterParseError::UnknownOperator {
            requested: value.to_owned(),
            valid: KNOWN_OPERATOR_IDS.to_vec(),
        }
        .to_string())
    }
}

/// Parse and build an exclusion glob.
pub fn parse_exclude_glob(value: &str) -> Result<Glob, String> {
    if value.is_empty() {
        return Err(FilterParseError::InvalidExcludeGlob {
            glob: value.to_owned(),
            details: "exclude glob pattern cannot be empty".to_owned(),
        }
        .to_string());
    }
    if value.len() > 2048 {
        return Err(FilterParseError::InvalidExcludeGlob {
            glob: value.chars().take(32).collect::<String>() + "...",
            details: "exclude glob pattern exceeds maximum supported length (2048 bytes)"
                .to_owned(),
        }
        .to_string());
    }

    GlobBuilder::new(value)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map_err(|error| {
            FilterParseError::InvalidExcludeGlob {
                glob: value.to_owned(),
                details: error.to_string(),
            }
            .to_string()
        })
}

/// Shard selection specifier (1-based index and total count).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shard {
    pub index: usize,
    pub total: usize,
}

impl FromStr for Shard {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid =
            || "expected INDEX/TOTAL with 1 <= INDEX <= TOTAL (positive integers)".to_owned();
        parse_shard(value).map_err(|_| invalid())
    }
}

/// Parse a shard specifier in the format `INDEX/TOTAL`.
pub fn parse_shard(value: &str) -> Result<Shard, FilterParseError> {
    let invalid = |reason: &str| FilterParseError::InvalidShard {
        input: value.to_owned(),
        reason: reason.to_owned(),
    };

    let (index_str, total_str) = value
        .split_once('/')
        .ok_or_else(|| invalid("missing '/' separator"))?;

    if index_str.is_empty() || total_str.is_empty() {
        return Err(invalid("index or total is empty"));
    }

    if !index_str.bytes().all(|b| b.is_ascii_digit())
        || !total_str.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid("index and total must be positive ASCII digits"));
    }

    let index = index_str
        .parse::<usize>()
        .map_err(|_| invalid("index overflow or invalid integer"))?;
    let total = total_str
        .parse::<usize>()
        .map_err(|_| invalid("total overflow or invalid integer"))?;

    if index == 0 {
        return Err(invalid("index must be >= 1 (1-based)"));
    }
    if total == 0 {
        return Err(invalid("total must be >= 1"));
    }
    if index > total {
        return Err(invalid("index cannot exceed total shard count"));
    }

    Ok(Shard { index, total })
}

/// Parse and validate a `--file` path filter.
pub fn parse_file_filter(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err(FilterParseError::InvalidFileFilter {
            input: value.to_owned(),
            reason: "file filter cannot be empty".to_owned(),
        }
        .to_string());
    }
    if value.contains('\0') {
        return Err(FilterParseError::InvalidFileFilter {
            input: value.to_owned(),
            reason: "file filter cannot contain null bytes".to_owned(),
        }
        .to_string());
    }
    if value.len() > 4096 {
        return Err(FilterParseError::InvalidFileFilter {
            input: value.chars().take(32).collect::<String>() + "...",
            reason: "file filter exceeds maximum length (4096 bytes)".to_owned(),
        }
        .to_string());
    }

    Ok(value.to_owned())
}

/// Parse and validate a `--function` identifier filter.
pub fn parse_function_filter(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err(FilterParseError::InvalidFunctionFilter {
            input: value.to_owned(),
            reason: "function filter cannot be empty".to_owned(),
        }
        .to_string());
    }
    if value.contains('\0') {
        return Err(FilterParseError::InvalidFunctionFilter {
            input: value.to_owned(),
            reason: "function filter cannot contain null bytes".to_owned(),
        }
        .to_string());
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(FilterParseError::InvalidFunctionFilter {
            input: value.to_owned(),
            reason: "function filter cannot contain control characters".to_owned(),
        }
        .to_string());
    }
    if value.len() > 1024 {
        return Err(FilterParseError::InvalidFunctionFilter {
            input: value.chars().take(32).collect::<String>() + "...",
            reason: "function filter exceeds maximum length (1024 bytes)".to_owned(),
        }
        .to_string());
    }

    Ok(value.to_owned())
}

/// Fuzz entry point: executes all filter parsers against arbitrary input.
/// Never panics, regardless of the input.
#[allow(dead_code)]
pub fn fuzz_parse_all_filters(input: &str) {
    let _ = parse_operator(input);
    let _ = parse_exclude_glob(input);
    let _ = parse_shard(input);
    let _ = parse_file_filter(input);
    let _ = parse_function_filter(input);
}
