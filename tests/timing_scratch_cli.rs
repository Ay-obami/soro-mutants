use std::{fs, path::PathBuf, process::Command, time::{SystemTime, UNIX_EPOCH}};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "soro-timing-scratch-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn make_project(root: &std::path::Path) -> String {
    let project = root.join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"timing-scratch-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    let source = "pub struct Address;\nimpl Address { pub fn require_auth(&self) {} }\npub fn check(user: Address) { user.require_auth(); }\n";
    fs::write(project.join("src/lib.rs"), source).unwrap();
    source.to_owned()
}

fn run(root: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"));
    command.arg("test").arg(root);
    command.args(extra);
    command.output().expect("CLI should launch")
}

#[test]
fn timing_is_opt_in_and_records_real_baseline_and_mutant_phases() {
    let temp = TempDir::new();
    let project = temp.0.join("project");
    let original = make_project(&temp.0);
    let scratch_base = temp.0.join("scratch-disk");
    fs::create_dir_all(&scratch_base).unwrap();

    let output = run(
        &project,
        &[
            "--operator",
            "AUTH-001",
            "--timing",
            "--scratch-dir",
            scratch_base.to_str().unwrap(),
            "--json",
            "--no-progress",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_timing = &report["metadata"]["timing"];
    for field in [
        "discovery_ms",
        "baseline_ms",
        "mutant_wall_ms",
        "compile_ms",
        "test_ms",
    ] {
        assert!(run_timing[field].is_u64(), "missing {field}: {run_timing}");
    }
    let results = report["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "{report}");
    let mutant_timing = &results[0]["timing"];
    assert!(mutant_timing["total_ms"].is_u64(), "{mutant_timing}");
    assert!(mutant_timing["compile_ms"].is_u64(), "{mutant_timing}");
    assert!(mutant_timing["test_ms"].is_u64(), "{mutant_timing}");
    assert_eq!(fs::read_to_string(project.join("src/lib.rs")).unwrap(), original);
    assert!(
        !scratch_base.join(".soro-mutants-worktree").exists(),
        "managed scratch must be removed after the run"
    );

    let untimed = run(&project, &["--operator", "AUTH-001", "--json", "--no-progress"]);
    assert!(
        untimed.status.success(),
        "{}",
        String::from_utf8_lossy(&untimed.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&untimed.stdout).unwrap();
    assert!(report["metadata"].get("timing").is_none(), "{report}");
    assert!(report["results"][0].get("timing").is_none(), "{report}");
}

#[test]
fn custom_scratch_root_may_be_inside_project_but_not_project_root_or_parent() {
    let temp = TempDir::new();
    let project = temp.0.join("project");
    make_project(&temp.0);

    let nested = project.join("local-scratch");
    let good = run(
        &project,
        &[
            "--operator",
            "AUTH-001",
            "--scratch-dir",
            nested.to_str().unwrap(),
            "--test-command",
            "true",
            "--no-progress",
        ],
    );
    assert!(
        good.status.success(),
        "{}",
        String::from_utf8_lossy(&good.stderr)
    );
    assert!(!nested.join(".soro-mutants-worktree").exists());

    let bad = run(
        &project,
        &[
            "--operator",
            "AUTH-001",
            "--scratch-dir",
            project.to_str().unwrap(),
            "--test-command",
            "true",
            "--no-progress",
        ],
    );
    assert!(!bad.status.success());
    let stderr = String::from_utf8_lossy(&bad.stderr);
    assert!(
        stderr.contains("--scratch-dir must not be the project root or one of its parents"),
        "{stderr}"
    );
}
