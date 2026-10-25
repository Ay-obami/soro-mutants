use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct GitProject(PathBuf);
impl GitProject {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("soro-changed-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let project = Self(root);
        project.git(&["init", "-q"]);
        project.git(&["config", "user.email", "test@example.com"]);
        project.git(&["config", "user.name", "Test"]);
        project.write("src/modified.rs", "fn edited(a: Address) { a.require_auth(); }\nfn untouched(a: Address) { a.require_auth(); }\n");
        project.write(
            "src/renamed.rs",
            "fn renamed(a: Address) { a.require_auth(); }\n",
        );
        project.write(
            "src/unrelated.rs",
            "fn unrelated(a: Address) { a.require_auth(); }\n",
        );
        project.git(&["add", "."]);
        project.git(&["commit", "-qm", "base"]);
        project
    }
    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn write(&self, relative: &str, content: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .arg("list")
            .arg(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
    fn mutants(&self, args: &[&str]) -> Vec<Value> {
        let mut args = args.to_vec();
        args.push("--json");
        let output = self.run(&args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["mutants"]
            .as_array()
            .unwrap()
            .clone()
    }
}
impl Drop for GitProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn explicit_changed_file_set_intersects_existing_filters() {
    let project = GitProject::new();
    let mutants = project.mutants(&[
        "--changed-file",
        "src/modified.rs",
        "--changed-file",
        "src/unrelated.rs",
        "--file",
        "modified.rs",
        "--function",
        "edited",
        "--operator",
        "AUTH-001",
    ]);
    assert_eq!(mutants.len(), 1);
    assert_eq!(mutants[0]["file"], "src/modified.rs");
    assert_eq!(mutants[0]["function"], "edited");
}

#[test]
fn git_diff_includes_add_modify_and_rename_but_excludes_unrelated() {
    let project = GitProject::new();
    project.write("src/modified.rs", "fn edited(a: Address) { a.require_auth(); } // changed\nfn untouched(a: Address) { a.require_auth(); }\n");
    project.write(
        "src/added.rs",
        "fn added(a: Address) { a.require_auth(); }\n",
    );
    project.git(&["mv", "src/renamed.rs", "src/new_name.rs"]);
    project.git(&["add", "src/added.rs"]);
    let mutants = project.mutants(&["--diff-base", "HEAD"]);
    let files: std::collections::BTreeSet<_> = mutants
        .iter()
        .map(|m| m["file"].as_str().unwrap())
        .collect();
    assert_eq!(
        files,
        ["src/added.rs", "src/modified.rs", "src/new_name.rs"]
            .into_iter()
            .collect()
    );
}

#[test]
fn changed_functions_select_all_mutants_in_touched_function() {
    let project = GitProject::new();
    project.write("src/modified.rs", "fn edited(a: Address) { a.require_auth(); } // changed\nfn untouched(a: Address) { a.require_auth(); }\n");
    let mutants = project.mutants(&["--diff-base", "HEAD", "--changed-functions"]);
    assert_eq!(mutants.len(), 1);
    assert_eq!(mutants[0]["function"], "edited");
}

#[test]
fn changed_file_paths_accept_dot_prefix_and_empty_run_skips_baseline() {
    let project = GitProject::new();
    let mutants = project.mutants(&["--changed-file", "./src/modified.rs"]);
    assert_eq!(mutants.len(), 2);
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test")
        .arg(&project.0)
        .args([
            "--changed-file",
            "src/not-changed.rs",
            "--test-command",
            "false",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["results"].as_array().unwrap().len(), 0);
}

#[test]
fn git_missing_and_conflicting_selection_report_clear_errors() {
    let project = GitProject::new();
    let invalid = project.run(&["--changed-file", "src/modified.rs", "--diff-base", "HEAD"]);
    assert_eq!(invalid.status.code(), Some(2));
    let missing = project.run(&["--diff-base", "revision-that-does-not-exist"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("git diff"));
    let no_git = std::env::temp_dir().join(format!("soro-no-git-{}", std::process::id()));
    fs::create_dir_all(&no_git).unwrap();
    fs::write(
        no_git.join("lib.rs"),
        "fn sample(a: Address) { a.require_auth(); }",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list", no_git.to_str().unwrap(), "--diff-base", "HEAD"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Git repository"));
    let _ = fs::remove_dir_all(no_git);
}
