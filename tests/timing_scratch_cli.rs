use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    scratch_base: PathBuf,
    source: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "soro-timing-scratch-{}-{nonce}",
            std::process::id()
        ));
        let project = root.join("project");
        let scratch_base = root.join("scratch");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::create_dir_all(&scratch_base).unwrap();
        fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"timing-scratch-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        )
        .unwrap();
        let source = br#"pub struct Address;
impl Address {
    pub fn require_auth(&self) {}
}
pub fn guarded(user: Address) {
    user.require_auth();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smoke() {
        guarded(Address);
    }
}
"#
        .to_vec();
        fs::write(project.join("src/lib.rs"), &source).unwrap();
        Self {
            root,
            project,
            scratch_base,
            source,
        }
    }

    fn run(&self, extra: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .arg("test")
            .arg(&self.project)
            .args(["--operator", "AUTH-001", "--no-cache"])
            .args(extra)
            .output()
            .expect("CLI should run")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn as_utf8(path: &Path) -> &str {
    path.to_str().expect("temporary path must be UTF-8")
}

#[test]
fn timing_reports_real_baseline_compile_test_and_mutant_durations() {
    let fixture = Fixture::new();
    let output = fixture.run(&[
        "--json",
        "--timing",
        "--scratch-dir",
        as_utf8(&fixture.scratch_base),
        "--test-command",
        "cargo test -q",
        "--timeout",
        "30",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let timing = &report["metadata"]["timing"];
    for field in [
        "discovery_ms",
        "baseline_ms",
        "mutant_wall_ms",
        "compile_ms",
        "test_ms",
    ] {
        assert!(
            timing[field].is_u64(),
            "missing timing field {field}: {timing}"
        );
    }

    let results = report["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    let mutant_timing = &results[0]["timing"];
    assert!(mutant_timing["total_ms"].is_u64());
    assert!(mutant_timing["compile_ms"].is_u64());
    assert!(mutant_timing["test_ms"].is_u64());

    let scratch_root = fixture.scratch_base.join(".soro-mutants-worktree");
    assert_eq!(
        report["metadata"]["scratch_dir"],
        scratch_root.display().to_string()
    );
    assert!(
        !scratch_root.exists(),
        "custom scratch tree must be cleaned"
    );
    assert_eq!(
        fs::read(fixture.project.join("src/lib.rs")).unwrap(),
        fixture.source
    );
}

#[test]
fn timing_is_opt_in_and_default_json_shape_has_no_timing_fields() {
    let fixture = Fixture::new();
    let output = fixture.run(&["--json", "--test-command", "true", "--timeout", "10"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report["metadata"].get("timing").is_none());
    for result in report["results"].as_array().unwrap() {
        assert!(result.get("timing").is_none());
    }
}

#[test]
fn scratch_dir_rejects_project_root_and_parent() {
    let fixture = Fixture::new();
    for invalid in [&fixture.project, &fixture.root] {
        let output = fixture.run(&[
            "--scratch-dir",
            as_utf8(invalid),
            "--test-command",
            "true",
            "--timeout",
            "10",
        ]);
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("must not be the project root or one of its parents"),
            "{stderr}"
        );
    }
}
