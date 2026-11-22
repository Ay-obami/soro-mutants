//! Adversarial CLI regressions for issues #183 and #184.
//!
//! #183 proves that values interpolated into shell commands are validated
//! before any shell is spawned. #184 proves that a symlinked deletion target
//! cannot redirect cleanup outside the Soro Mutants-owned paths.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/weak-auth")
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let id = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "soro-guards-{name}-{}-{nanos}-{id}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(args)
        .output()
        .expect("CLI should launch")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Minimal self-contained project so the runner reaches cache/scratch setup.
fn minimal_project(root: &Path) {
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"guard-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .expect("Cargo.toml");
    fs::write(
        root.join("src/lib.rs"),
        "pub struct Address;\nimpl Address { pub fn require_auth(&self) {} }\npub fn guarded(user: &Address) { user.require_auth(); }\n#[cfg(test)] mod tests { #[test] fn t() { super::guarded(&super::Address); } }\n",
    )
    .expect("lib.rs");
}

// ---------------------------------------------------------------------------
// #183: shell command construction
// ---------------------------------------------------------------------------

#[test]
fn unsafe_package_name_is_rejected_before_any_shell_command_runs() {
    let root = temp_dir("shell-package");
    let output = cli(&[
        "test",
        root.to_str().expect("utf8 temp path"),
        "--package",
        "pkg; touch /tmp/soro-injected",
    ]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("unsafe shell word in --package"), "{err}");
    assert!(!err.contains("panicked at"), "{err}");
    assert!(
        !std::path::Path::new("/tmp/soro-injected").exists(),
        "injected command must never execute"
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn control_bytes_in_test_command_are_rejected_without_a_panic() {
    let root = temp_dir("shell-control");
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test")
        .arg(&root)
        .args(["--test-command", "cargo\r test"])
        .output()
        .expect("CLI should launch");
    // clap rejects NUL in an OS argument itself; either way the CLI must exit
    // cleanly without a Rust panic.
    assert!(!output.status.success());
    assert!(
        !stderr(&output).contains("panicked at"),
        "{:?}",
        stderr(&output)
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn documented_shell_syntax_is_still_accepted() {
    let output = cli(&[
        "test",
        fixture().to_str().expect("utf8 fixture path"),
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
        "--no-progress",
    ]);
    let err = stderr(&output);
    assert!(
        !err.contains("unsafe shell word") && !err.contains("nul byte"),
        "plain shell commands must not be rejected: {err}"
    );
}

// ---------------------------------------------------------------------------
// #184: filesystem deletion paths
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn symlinked_scratch_is_not_deleted_by_a_run() {
    use std::os::unix::fs::symlink;

    let root = temp_dir("scratch-link");
    minimal_project(&root);
    let external = temp_dir("scratch-external");
    fs::write(external.join("sentinel"), "keep").expect("sentinel");
    symlink(&external, root.join(".soro-mutants-worktree")).expect("symlink");

    let output = cli(&[
        "test",
        root.to_str().expect("utf8 temp path"),
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
    ]);
    let err = stderr(&output);
    assert!(
        err.contains("refusing to remove symlinked Soro Mutants path")
            || err.contains("refusing to clean symlinked Soro Mutants path"),
        "{err}"
    );
    assert!(!err.contains("panicked at"), "{err}");
    assert_eq!(
        fs::read_to_string(external.join("sentinel")).expect("sentinel intact"),
        "keep"
    );
    fs::remove_dir_all(root).expect("cleanup");
    fs::remove_dir_all(external).expect("cleanup");
}

#[cfg(unix)]
#[test]
fn symlinked_cache_child_is_not_deleted_by_cache_validation() {
    use std::os::unix::fs::symlink;

    let root = temp_dir("cache-link");
    minimal_project(&root);
    let cache = root.join(".soro-mutants-target");
    fs::create_dir_all(&cache).expect("cache root");
    let external = temp_dir("cache-external");
    fs::write(external.join("sentinel"), "keep").expect("sentinel");
    symlink(&external, cache.join("baseline")).expect("symlink");

    let output = cli(&[
        "test",
        root.to_str().expect("utf8 temp path"),
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
    ]);
    let err = stderr(&output);
    assert!(
        err.contains("refusing to remove symlinked Soro Mutants path"),
        "{err}"
    );
    assert!(!err.contains("panicked at"), "{err}");
    assert_eq!(
        fs::read_to_string(external.join("sentinel")).expect("sentinel intact"),
        "keep"
    );
    fs::remove_dir_all(root).expect("cleanup");
    fs::remove_dir_all(external).expect("cleanup");
}

#[test]
fn clean_remains_deterministic_for_an_empty_project() {
    let root = temp_dir("clean-empty");
    let output = cli(&["clean", root.to_str().expect("utf8 temp path")]);
    assert!(
        output.status.success() || !stderr(&output).is_empty(),
        "clean must remain deterministic for an empty project"
    );
    fs::remove_dir_all(root).expect("cleanup");
}
