//! Redact secrets read from the environment in diagnostic and metadata strings.
//!
//! Source excerpts are not passed through this path: they are explicit user input.
//! This is a best-effort boundary for environment-derived values, not an excuse
//! to print process environments or raw subprocess stderr elsewhere.

/// Credential-shaped variable names only, to avoid replacing harmless
/// environment values such as PATH, HOME, or short common substrings.
fn is_sensitive_name(name: &str) -> bool {
    let key = name.to_ascii_uppercase();
    key.contains("SECRET")
        || key.contains("TOKEN")
        || key.contains("PASSWORD")
        || key.contains("PASSWD")
        || key.contains("CREDENTIAL")
        || key.contains("PRIVATE_KEY")
        || key.contains("API_KEY")
        || key.contains("ACCESS_KEY")
        || key.contains("AUTHORIZATION")
}

/// Replace non-empty values of credential-shaped environment variables,
/// longest first. Even short credential values are redacted: diagnostics may
/// become less readable in that unusual case, but the security invariant is
/// that configured secrets are never emitted.
pub(crate) fn redact_environment_secrets(message: &str) -> String {
    let mut secrets: Vec<String> = std::env::vars_os()
        .filter_map(|(name, value)| {
            let name = name.into_string().ok()?;
            let value = value.into_string().ok()?;
            (is_sensitive_name(&name) && !value.is_empty()).then_some(value)
        })
        .collect();
    secrets.sort_unstable_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    secrets.dedup();

    let mut sanitized = message.to_owned();
    for secret in secrets {
        sanitized = sanitized.replace(&secret, "[REDACTED]");
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_credential_keys_without_matching_harmless_variables() {
        for key in [
            "GITHUB_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "DB_PASSWORD",
            "API_KEY",
        ] {
            assert!(is_sensitive_name(key), "{key}");
        }
        for key in ["PATH", "HOME", "CARGO_TARGET_DIR", "RUST_LOG"] {
            assert!(!is_sensitive_name(key), "{key}");
        }
    }

    #[test]
    fn retains_unrelated_text() {
        assert_eq!(
            redact_environment_secrets("diagnostic without a credential"),
            "diagnostic without a credential"
        );
    }

    #[test]
    fn redacts_short_sensitive_values_too() {
        const KEY: &str = "SORO_MUTANTS_SHORT_SECRET";
        std::env::set_var(KEY, "z7");
        let redacted = redact_environment_secrets("credential=z7");
        std::env::remove_var(KEY);
        assert_eq!(redacted, "credential=[REDACTED]");
    }
}
