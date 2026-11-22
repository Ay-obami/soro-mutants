//! Safe construction and validation of process and shell command lines.
//!
//! `--test-command` is intentionally evaluated by a shell, so shell operators
//! inside it are a documented feature, not a bug. The security boundary is
//! narrower and explicit:
//!
//! - values *derived* from CLI flags or benchmark manifests and interpolated
//!   into a shell string must be single, inert words;
//! - every string handed to `bash -lc` must be free of NUL and control bytes
//!   that could truncate or reinterpret the command;
//! - no validation path may panic, and every rejection is a structured,
//!   deterministic error that names the offending field and value.

use std::fmt;

/// Maximum accepted length of a single shell word (e.g. a Cargo package name).
pub const MAX_SHELL_WORD_BYTES: usize = 256;

/// Maximum accepted length of a complete shell command line.
pub const MAX_SHELL_COMMAND_BYTES: usize = 65_536;

/// Structured, actionable error returned when a command cannot be built safely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandConstructionError {
    Empty { field: &'static str },
    NulByte { field: &'static str },
    ControlCharacter { field: &'static str, byte: u8 },
    TooLong { field: &'static str, limit: usize },
    UnsafeShellWord { field: &'static str, value: String },
    LeadingDash { field: &'static str, value: String },
}

impl fmt::Display for CommandConstructionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { field } => {
                write!(f, "empty {field}; pass a non-empty value")
            }
            Self::NulByte { field } => {
                write!(f, "nul byte in {field}; remove control bytes")
            }
            Self::ControlCharacter { field, byte } => {
                write!(
                    f,
                    "control byte 0x{byte:02x} in {field}; remove control bytes"
                )
            }
            Self::TooLong { field, limit } => {
                write!(f, "{field} exceeds the maximum length of {limit} bytes")
            }
            Self::UnsafeShellWord { field, value } => {
                write!(
                    f,
                    "unsafe shell word in {field}: {value:?}; use a plain value made of letters, digits, '-' or '_'"
                )
            }
            Self::LeadingDash { field, value } => {
                write!(
                    f,
                    "option-like value in {field}: {value:?}; a leading '-' can be confused with a command flag"
                )
            }
        }
    }
}

impl std::error::Error for CommandConstructionError {}

fn first_control_byte(value: &str) -> Option<u8> {
    value
        .bytes()
        .find(|byte| *byte == 0 || *byte < 0x20 || *byte == 0x7f)
}

/// Reject a value that will be interpolated as exactly one word into a shell
/// string. The allowlist mirrors `benchmark_id_is_safe`: only ASCII letters,
/// digits, `-`, `_`, `.` and `+` are accepted. This prevents `;`, `&&`, `$()`,
/// backticks, quotes, whitespace and glob characters from changing the command.
pub fn validate_shell_word(
    value: &str,
    field: &'static str,
) -> Result<(), CommandConstructionError> {
    if value.is_empty() {
        return Err(CommandConstructionError::Empty { field });
    }
    if value.len() > MAX_SHELL_WORD_BYTES {
        return Err(CommandConstructionError::TooLong {
            field,
            limit: MAX_SHELL_WORD_BYTES,
        });
    }
    if let Some(byte) = first_control_byte(value) {
        return Err(if byte == 0 {
            CommandConstructionError::NulByte { field }
        } else {
            CommandConstructionError::ControlCharacter { field, byte }
        });
    }
    if value.starts_with('-') {
        return Err(CommandConstructionError::LeadingDash {
            field,
            value: value.to_owned(),
        });
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+'))
    {
        return Err(CommandConstructionError::UnsafeShellWord {
            field,
            value: value.to_owned(),
        });
    }
    Ok(())
}

/// Validate a complete command line just before it is executed by `bash -lc`.
/// Shell syntax is allowed; only bytes that can corrupt or truncate the string
/// are rejected.
pub fn validate_shell_command(
    command: &str,
    field: &'static str,
) -> Result<(), CommandConstructionError> {
    if command.is_empty() {
        return Err(CommandConstructionError::Empty { field });
    }
    if command.len() > MAX_SHELL_COMMAND_BYTES {
        return Err(CommandConstructionError::TooLong {
            field,
            limit: MAX_SHELL_COMMAND_BYTES,
        });
    }
    // Tab and newline are legitimate shell separators; everything else in the
    // C0 range and DEL is a control byte with no place in a command line.
    if let Some(byte) = command.bytes().find(|byte| {
        *byte == 0 || (*byte < 0x20 && *byte != b'\t' && *byte != b'\n') || *byte == 0x7f
    }) {
        return Err(if byte == 0 {
            CommandConstructionError::NulByte { field }
        } else {
            CommandConstructionError::ControlCharacter { field, byte }
        });
    }
    Ok(())
}

/// Build the `-p <package>` fragment used by the default test commands.
/// Each package name is validated before it becomes part of a shell string.
pub fn package_words(packages: &[String]) -> Result<String, CommandConstructionError> {
    let mut words = Vec::with_capacity(packages.len());
    for package in packages {
        validate_shell_word(package, "--package")?;
        words.push(format!("-p {package}"));
    }
    Ok(words.join(" "))
}

/// Panic-free entry point for fuzzing arbitrary CLI/benchmark strings.
/// Reached from the fuzz target through the library crate, not the binary.
#[allow(dead_code)]
pub fn fuzz_command_construction(value: &str) {
    let _ = validate_shell_command(value, "fuzz command");
    let _ = validate_shell_word(value, "fuzz word");
    let _ = package_words(&[value.to_owned()]);
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIELD: &str = "test field";

    #[test]
    fn shell_words_accept_plain_names() {
        for name in ["my-contract", "pool_v2", "pkg.v2", "a+b", "A1"] {
            assert_eq!(validate_shell_word(name, FIELD), Ok(()), "{name}");
        }
    }

    #[test]
    fn shell_words_reject_injection_and_argument_confusion() {
        for value in [
            "pkg; rm -rf /",
            "pkg && touch /tmp/pwn",
            "$(id)",
            "`id`",
            "pkg|cat",
            "a b",
            "a'b",
            "a\"b",
            "-pkg",
            "../etc/passwd",
            "pkg*",
        ] {
            assert!(validate_shell_word(value, FIELD).is_err(), "{value}");
        }
        assert_eq!(
            validate_shell_word("", FIELD),
            Err(CommandConstructionError::Empty { field: FIELD })
        );
        assert_eq!(
            validate_shell_word("a\u{0000}b", FIELD),
            Err(CommandConstructionError::NulByte { field: FIELD })
        );
        assert_eq!(
            validate_shell_word("a\nb", FIELD),
            Err(CommandConstructionError::ControlCharacter {
                field: FIELD,
                byte: 0x0a
            })
        );
        assert_eq!(
            validate_shell_word(&"a".repeat(MAX_SHELL_WORD_BYTES + 1), FIELD),
            Err(CommandConstructionError::TooLong {
                field: FIELD,
                limit: MAX_SHELL_WORD_BYTES
            })
        );
    }

    #[test]
    fn shell_commands_keep_documented_syntax_but_reject_control_bytes() {
        for command in [
            "cargo test -q",
            "cd contracts && cargo +stable test auth",
            "RUSTFLAGS='-Dwarnings' cargo test -q -- --nocapture",
            "cargo test -q -p my-contract",
        ] {
            assert_eq!(validate_shell_command(command, FIELD), Ok(()), "{command}");
        }
        assert_eq!(
            validate_shell_command("cargo\0 test", FIELD),
            Err(CommandConstructionError::NulByte { field: FIELD })
        );
        assert_eq!(
            validate_shell_command("cargo\r test", FIELD),
            Err(CommandConstructionError::ControlCharacter {
                field: FIELD,
                byte: 0x0d
            })
        );
        assert_eq!(
            validate_shell_command("", FIELD),
            Err(CommandConstructionError::Empty { field: FIELD })
        );
    }

    #[test]
    fn package_words_are_validated_before_interpolation() {
        assert_eq!(
            package_words(&["one".to_owned(), "two".to_owned()]),
            Ok("-p one -p two".to_owned())
        );
        assert_eq!(
            package_words(&["one".to_owned(), "two; id".to_owned()]),
            Err(CommandConstructionError::UnsafeShellWord {
                field: "--package",
                value: "two; id".to_owned()
            })
        );
    }

    #[test]
    fn fuzz_entry_point_never_panics() {
        for value in ["", "\0", "\n", "a;b", "-flag", &"x".repeat(300)] {
            fuzz_command_construction(value);
        }
    }
}
