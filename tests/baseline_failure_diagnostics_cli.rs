use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str, source: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "soro-baseline-diagnostics-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n"
            ),
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), source).unwrap();
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn run(&self, command: &str) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .arg("test")
            .arg(self.path())
            .args([
                "--operator",
                "AUTH-001",
                "--test-command",
                command,
                "--timeout",
                "10",
                "--json",
            ])
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const ADDRESS: &str = r#"
pub struct Address;
impl Address { pub fn require_auth(&self) {} }
"#;

#[test]
fn baseline_compile_failure_is_classified_with_bounded_context() {
    let source = format!(
        "{ADDRESS}\npub fn check(user: Address) {{ user.require_auth(); let _ = missing_symbol; }}\n"
    );
    let fixture = Fixture::new("compile_failure_fixture", &source);
    let output = fixture.run("cargo test -q");
    assert_eq!(output.status.code(), Some(3));
    assert!(
        output.stdout.is_empty(),
        "baseline failure must not emit a JSON result"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("baseline tests failed (compile_failure)"),
        "{stderr}"
    );
    assert!(
        stderr.contains(&fixture.path().display().to_string()),
        "{stderr}"
    );
    assert!(stderr.contains("cargo test -q"), "{stderr}");
    assert!(stderr.contains("could not compile"), "{stderr}");
    assert!(!stderr.contains("SURVIVED") && !stderr.contains("KILLED"));
}

#[test]
fn baseline_test_failure_is_classified_and_includes_output_excerpt() {
    let source = format!(
        "{ADDRESS}\npub fn check(user: Address) {{ user.require_auth(); }}\n#[cfg(test)] mod tests {{ #[test] fn fails() {{ eprintln!(\"diagnostic-marker\"); assert!(false); }} }}\n"
    );
    let fixture = Fixture::new("test_failure_fixture", &source);
    let output = fixture.run("cargo test -q");
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("baseline tests failed (test_failure)"),
        "{stderr}"
    );
    assert!(stderr.contains("diagnostic-marker"), "{stderr}");
    assert!(stderr.contains("exit code:"), "{stderr}");
}

#[test]
fn missing_baseline_command_is_not_misclassified_as_a_test_failure() {
    let source = format!("{ADDRESS}\npub fn check(user: Address) {{ user.require_auth(); }}\n");
    let fixture = Fixture::new("missing_command_fixture", &source);
    let output = fixture.run("soro-mutants-definitely-missing-command");
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("baseline tests failed (command_not_found)"),
        "{stderr}"
    );
    assert!(
        stderr.contains("soro-mutants-definitely-missing-command"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("baseline tests failed (test_failure)"),
        "{stderr}"
    );
}
