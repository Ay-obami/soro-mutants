use std::{path::Path, process::Command};

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("list")
        .arg(root)
        .args(args)
        .output()
        .expect("cargo-soro-mutants should run")
}

#[test]
fn workspace_without_package_filter_discovers_all_packages() {
    let root = Path::new("fixtures/workspace-multi-package");
    let output = run(root, &["--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let val: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let mutants = val["mutants"].as_array().unwrap();
    assert_eq!(mutants.len(), 2);
    let files: Vec<&str> = mutants
        .iter()
        .map(|m| m["file"].as_str().unwrap())
        .collect();
    assert!(files.iter().any(|f| f.contains("package-a")));
    assert!(files.iter().any(|f| f.contains("package-b")));
}

#[test]
fn workspace_with_package_a_discovers_only_package_a() {
    let root = Path::new("fixtures/workspace-multi-package");
    let output = run(root, &["-p", "package-a", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let val: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let mutants = val["mutants"].as_array().unwrap();
    assert_eq!(mutants.len(), 1);
    let file = mutants[0]["file"].as_str().unwrap();
    assert!(file.contains("package-a"));
    assert!(!file.contains("package-b"));
}

#[test]
fn workspace_with_package_b_discovers_only_package_b() {
    let root = Path::new("fixtures/workspace-multi-package");
    let output = run(root, &["--package", "package-b", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let val: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let mutants = val["mutants"].as_array().unwrap();
    assert_eq!(mutants.len(), 1);
    let file = mutants[0]["file"].as_str().unwrap();
    assert!(file.contains("package-b"));
    assert!(!file.contains("package-a"));
}

#[test]
fn workspace_with_both_packages_discovers_both() {
    let root = Path::new("fixtures/workspace-multi-package");
    let output = run(root, &["-p", "package-a", "-p", "package-b", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let val: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let mutants = val["mutants"].as_array().unwrap();
    assert_eq!(mutants.len(), 2);
}

#[test]
fn workspace_with_nonexistent_package_fails() {
    let root = Path::new("fixtures/workspace-multi-package");
    let output = run(root, &["-p", "nonexistent-package"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("package `nonexistent-package` not found in workspace"));
}
