//! Dependency graph hygiene regressions for issue #182.
//!
//! Complements `.github/scripts/check-dependency-graph.sh` with stable Rust
//! assertions over the committed `Cargo.lock`, `Cargo.toml` and policy file.
//! No new TOML parsing dependency is introduced.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_file(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Map crate name -> distinct versions from a Cargo.lock text.
fn lockfile_duplicate_names(lock_text: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut current_name: Option<String> = None;
    let mut versions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in lock_text.lines() {
        if line.starts_with("[[package]]") {
            current_name = None;
            continue;
        }
        if let Some(value) = line.strip_prefix("name = ") {
            current_name = Some(value.trim().trim_matches('"').to_owned());
        } else if let (Some(name), Some(value)) =
            (current_name.as_ref(), line.strip_prefix("version = "))
        {
            versions
                .entry(name.clone())
                .or_default()
                .insert(value.trim().trim_matches('"').to_owned());
            current_name = None;
        }
    }
    versions.retain(|_, found| found.len() > 1);
    versions
}

fn direct_dependencies(manifest: &str) -> Vec<String> {
    let mut in_section = false;
    let mut names = Vec::new();
    for line in manifest.lines() {
        if line.starts_with("[dependencies]") {
            in_section = true;
            continue;
        }
        if line.starts_with('[') {
            in_section = false;
        }
        if in_section {
            if let Some(name) = line.split('=').next() {
                let name = name.trim();
                if !name.is_empty() && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                {
                    names.push(name.to_owned());
                }
            }
        }
    }
    names
}

fn source_text() -> String {
    let mut combined = String::new();
    let src = workspace_root().join("src");
    let mut visit = |dir: &Path| {
        for entry in fs::read_dir(dir).expect("src directory") {
            let path = entry.expect("directory entry").path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
                combined.push_str(&fs::read_to_string(&path).expect("Rust source"));
            }
        }
    };
    visit(&src);
    combined
}

#[test]
fn only_the_reviewed_syn_major_is_duplicated() {
    let duplicates = lockfile_duplicate_names(&repo_file("Cargo.lock"));
    assert_eq!(
        duplicates.keys().collect::<Vec<_>>(),
        vec!["syn"],
        "unreviewed duplicate crate names: {duplicates:?}"
    );
    let versions = duplicates.get("syn").expect("syn duplicate");
    assert!(versions.contains("2.0.119"), "{versions:?}");
    assert!(versions.contains("3.0.6"), "{versions:?}");
}

#[test]
fn duplicate_exception_has_an_owner_and_review_date() {
    let policy = repo_file(".cargo/dependency-hygiene.toml");
    assert!(policy.contains("allow = [\"syn\"]"), "{policy}");
    assert!(policy.contains("owner: maintainers"), "{policy}");
    assert!(policy.contains("review by 20"), "{policy}");
    // The advisory policy must remain exception-free and independent.
    assert!(repo_file(".cargo/audit.toml").contains("ignore = []"));
}

#[test]
fn dependency_graph_script_self_test_detects_injected_duplicate() {
    let output = Command::new(workspace_root().join(".github/scripts/check-dependency-graph.sh"))
        .arg("--self-test")
        .output()
        .expect("script should launch");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("all self-tests passed"), "{combined}");
}

#[test]
fn direct_dependencies_are_unique_and_referenced() {
    let manifest = repo_file("Cargo.toml");
    let names = direct_dependencies(&manifest);
    let unique: BTreeSet<&String> = names.iter().collect();
    assert_eq!(
        unique.len(),
        names.len(),
        "duplicate direct dependencies: {names:?}"
    );

    let source = source_text();
    for name in &names {
        assert!(
            source.contains(&name.replace('-', "_")),
            "direct dependency '{name}' is not referenced under src/"
        );
    }
}

#[test]
fn hygiene_policy_is_documented_and_gated_in_ci() {
    let doc = repo_file("docs/dependency-auditing.md");
    assert!(doc.contains("Duplicate and unnecessary crates"), "{doc}");
    assert!(
        doc.contains("check-dependency-graph.sh --self-test"),
        "{doc}"
    );
    assert!(
        doc.contains("cargo test --test dependency_graph_cli"),
        "{doc}"
    );

    let ci = repo_file(".github/workflows/ci.yml");
    assert!(ci.contains("dependency-hygiene"), "{ci}");
    assert!(ci.contains("check-dependency-graph.sh"), "{ci}");
}
