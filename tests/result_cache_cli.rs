use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "soro-result-cache-{prefix}-{}-{nonce}",
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

fn project(temp: &TempDir) -> PathBuf {
    let root = temp.path().join("project");
    fs::create_dir_all(root.join("src")).unwrap();
    let source = fs::read_to_string("fixtures/weak-auth/src/lib.rs").unwrap();
    fs::write(root.join("src/lib.rs"), source).unwrap();
    root
}

fn run_test_with_command(
    root: &Path,
    target: &Path,
    command: &str,
    extra: &[&str],
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test")
        .arg(root)
        .arg("--target-dir")
        .arg(target)
        .arg("--test-command")
        .arg(command)
        .arg("--operator")
        .arg("AUTH-001")
        .args(extra)
        .output()
        .expect("soro-mutants should run")
}

fn run_test(root: &Path, target: &Path, extra: &[&str]) -> std::process::Output {
    run_test_with_command(root, target, "true", extra)
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn cached_outcomes_are_reused_when_identity_matches() {
    let temp = TempDir::new("hit");
    let root = project(&temp);
    let target = temp.path().join("target");

    let first = run_test(&root, &target, &["--cache"]);
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(!stdout(&first).contains("(cache hit)"));
    assert!(target.join("result-cache.json").exists());

    let second = run_test(&root, &target, &["--cache"]);
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(stdout(&second).contains("(cache hit)"));
    assert!(stderr(&second).contains("1 hit(s), 0 miss(es)"));
}

#[test]
fn changing_test_command_invalidates_cached_outcomes() {
    let temp = TempDir::new("test-command");
    let root = project(&temp);
    let target = temp.path().join("target");

    let first = run_test(&root, &target, &["--cache"]);
    assert!(first.status.success(), "{}", stderr(&first));

    let second = run_test_with_command(&root, &target, "echo other", &["--cache"]);
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(stdout(&second).contains("cache miss"));
    assert!(stderr(&second).contains("cache identity does not match"));
}

#[test]
fn changing_source_invalidates_cached_outcomes() {
    let temp = TempDir::new("source");
    let root = project(&temp);
    let target = temp.path().join("target");

    let first = run_test(&root, &target, &["--cache"]);
    assert!(first.status.success(), "{}", stderr(&first));

    let source_path = root.join("src/lib.rs");
    let mut source = fs::read_to_string(&source_path).unwrap();
    source.push_str("\n// cache invalidation regression fixture\n");
    fs::write(&source_path, source).unwrap();

    let second = run_test(&root, &target, &["--cache"]);
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(stdout(&second).contains("cache miss"));
    assert!(stderr(&second).contains("cache identity does not match"));
}

#[test]
fn no_cache_prevents_reading_and_writing_results() {
    let temp = TempDir::new("disabled");
    let root = project(&temp);
    let target = temp.path().join("target");

    let first = run_test(&root, &target, &["--no-cache"]);
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(stderr(&first).contains("persistent result cache disabled"));
    assert!(!target.join("result-cache.json").exists());

    let second = run_test(&root, &target, &["--no-cache"]);
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(!stdout(&second).contains("(cache hit)"));
}

#[test]
fn clear_cache_removes_stored_outcomes() {
    let temp = TempDir::new("clear");
    let root = project(&temp);
    let target = temp.path().join("target");

    let first = run_test(&root, &target, &["--cache"]);
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(target.join("result-cache.json").exists());

    let second = run_test(&root, &target, &["--cache", "--clear-cache"]);
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(stderr(&second).contains("cleared persistent result cache"));
    assert!(!stdout(&second).contains("(cache hit)"));
}

#[test]
fn tampered_cache_entries_are_not_reused() {
    let temp = TempDir::new("tamper");
    let root = project(&temp);
    let target = temp.path().join("target");

    let first = run_test(&root, &target, &["--cache"]);
    assert!(first.status.success(), "{}", stderr(&first));

    let cache_path = target.join("result-cache.json");
    let cache = fs::read_to_string(&cache_path).unwrap();
    let tampered = cache.replacen("\"outcome\": \"SURVIVED\"", "\"outcome\": \"KILLED\"", 1);
    assert_ne!(cache, tampered, "expected a stored SURVIVED outcome");
    fs::write(&cache_path, tampered).unwrap();

    let second = run_test(&root, &target, &["--cache"]);
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(!stdout(&second).contains("(cache hit)"));
    assert!(stderr(&second).contains("integrity check"));
}
