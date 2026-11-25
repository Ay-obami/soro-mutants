use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_project() -> std::path::PathBuf {
    static SEQUENCE: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "soro-match-scope-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"match-scope-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
    root
}

#[test]
fn option_address_bindings_are_scoped_to_each_match_arm_guard_and_nested_match() {
    let root = unique_project();
    fs::write(
        root.join("src/lib.rs"),
        r#"
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Address(u32);
impl Address { pub fn require_auth(&self) {} }

pub fn choose(maybe: Option<Address>, nested: Option<Address>, fallback: Address, other: Address) {
    match maybe {
        Some(candidate) if candidate == fallback => {
            fallback.require_auth();
            candidate.require_auth();
            match nested {
                Some(inner) => { fallback.require_auth(); inner.require_auth(); }
                None => candidate.require_auth(),
            }
        }
        Some(second) => { other.require_auth(); second.require_auth(); }
        None => fallback.require_auth(),
    }
    fallback.require_auth();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smoke() { choose(Some(Address(1)), Some(Address(2)), Address(1), Address(3)); }
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args([
            "list",
            root.to_str().unwrap(),
            "--json",
            "--operator",
            "AUTH-002",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mutants = report["mutants"].as_array().unwrap();
    assert!(mutants
        .iter()
        .any(|m| m["original"] == "fallback.require_auth()"
            && m["replacement"] == "candidate.require_auth()"));
    assert!(mutants
        .iter()
        .any(|m| m["original"] == "other.require_auth()"
            && m["replacement"] == "second.require_auth()"));
    assert!(mutants
        .iter()
        .any(|m| m["original"] == "fallback.require_auth()"
            && m["replacement"] == "inner.require_auth()"));
    assert!(!mutants
        .iter()
        .any(|m| m["replacement"] == "inner.require_auth()"
            && m["span"]["line"].as_u64().is_some_and(|line| line > 13)));
    assert!(!mutants
        .iter()
        .any(|m| m["replacement"] == "second.require_auth()"
            && m["span"]["line"].as_u64().is_some_and(|line| line > 16)));

    let execution = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args([
            "test",
            root.to_str().unwrap(),
            "--operator",
            "AUTH-002",
            "--test-command",
            "cargo test -q",
            "--timeout",
            "30",
            "--no-progress",
        ])
        .output()
        .unwrap();
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
    assert!(!String::from_utf8_lossy(&execution.stdout).contains("Unviable  : 1"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn non_address_match_bindings_do_not_become_auth_candidates() {
    let root = unique_project();
    fs::write(
        root.join("src/lib.rs"),
        r#"
pub struct Address;
impl Address { pub fn require_auth(&self) {} }
pub fn choose(maybe: Option<u32>, fallback: Address) {
    match maybe { Some(number) => fallback.require_auth(), None => fallback.require_auth() }
}
"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args([
            "list",
            root.to_str().unwrap(),
            "--json",
            "--operator",
            "AUTH-002",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!report["mutants"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["replacement"] == "number.require_auth()"));
    let _ = fs::remove_dir_all(root);
}
