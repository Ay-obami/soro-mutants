use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "soro-mutants-output-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/weak-auth")
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(args)
        .output()
        .expect("CLI should run")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn list_text_and_json_files_match_existing_stdout() {
    let temp = TempDir::new();
    let fixture = fixture();
    let project = fixture.to_str().unwrap();

    for extra in [&[][..], &["--json"][..], &["--count-only"][..]] {
        let mut args = vec!["list", project];
        args.extend(extra);
        let stdout_report = run(&args);
        assert_success(&stdout_report);

        let output_path = temp.path().join("report.txt");
        args.extend(["--output", output_path.to_str().unwrap()]);
        let saved_report = run(&args);
        assert_success(&saved_report);
        assert!(saved_report.stdout.is_empty());
        assert_eq!(fs::read(output_path).unwrap(), stdout_report.stdout);
    }
}

#[test]
fn test_reports_are_written_for_empty_and_nonempty_runs() {
    let temp = TempDir::new();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::copy(fixture().join("src/lib.rs"), project.join("src/lib.rs")).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"report-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let project = project.to_str().unwrap();
    let text_file = temp.path().join("results.txt");

    let text_output = run(&[
        "test",
        project,
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
        "--output",
        text_file.to_str().unwrap(),
    ]);
    assert_success(&text_output);
    assert!(text_output.stdout.is_empty());
    let report = fs::read_to_string(&text_file).unwrap();
    assert!(report.contains("Baseline: true"));
    assert!(report.contains("Baseline PASS"));
    assert!(report.contains("SURVIVED"));
    assert!(report.contains("Summary"));

    let json_file = temp.path().join("results.json");
    let json_output = run(&[
        "test",
        project,
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
        "--json",
        "--output",
        json_file.to_str().unwrap(),
    ]);
    assert_success(&json_output);
    assert!(json_output.stdout.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&fs::read(json_file).unwrap()).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["results"].as_array().unwrap().len(), 1);

    let empty_file = temp.path().join("empty.json");
    let empty_output = run(&[
        "test",
        project,
        "--operator",
        "TOKEN-001",
        "--json",
        "--output",
        empty_file.to_str().unwrap(),
    ]);
    assert_success(&empty_output);
    let report: serde_json::Value = serde_json::from_slice(&fs::read(empty_file).unwrap()).unwrap();
    assert_eq!(report["results"], serde_json::json!([]));
}

#[test]
fn unwritable_output_path_reports_a_clear_error() {
    let temp = TempDir::new();
    let invalid = temp.path().join("missing-parent/report.json");
    for command in ["list", "test"] {
        let output = run(&[
            command,
            fixture().to_str().unwrap(),
            "--operator",
            "TOKEN-001",
            "--json",
            "--output",
            invalid.to_str().unwrap(),
        ]);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("cannot create report at"), "{error}");
        assert!(error.contains("report.json"), "{error}");
    }
}
