use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("soro-selection-{}-{nonce}", std::process::id()));
        let project = Self(root);
        // Deliberately create files out of lexical order, with multiple functions/operators.
        for file in [
            "src/z.rs",
            "vendor/nested/lib.rs",
            "src/a.rs",
            "generated/lib.rs",
        ] {
            project.write(file, "fn second(a: Address, b: Address) { a.require_auth(); }\nfn first(a: Address) { a.require_auth(); }\n");
        }
        project
    }

    fn write(&self, file: &str, content: &str) {
        let path = self.0.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn run(&self, command: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .arg(command)
            .arg(&self.0)
            .args(args)
            .output()
            .unwrap()
    }

    fn json(&self, command: &str, args: &[&str]) -> Value {
        let mut args = args.to_vec();
        args.push("--json");
        let output = self.run(command, &args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn operator_ids_are_validated_before_discovery_or_execution() {
    let project = Project::new();
    for command in ["list", "test"] {
        for invalid in ["AUTH-01", "auth-001", "", "AUTH-999"] {
            let output = project.run(command, &["--operator", invalid]);
            assert_eq!(output.status.code(), Some(2));
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(error.contains("unknown operator ID"), "{error}");
            for id in ["AUTH-001", "AUTH-002", "EVENT-001", "TOKEN-001", "TTL-001"] {
                assert!(error.contains(id), "{error}");
            }
        }
        for id in ["AUTH-001", "AUTH-002", "EVENT-001", "TOKEN-001", "TTL-001"] {
            let report = project.json(command, &["--operator", id, "--function", "absent"]);
            assert_eq!(
                report[if command == "list" {
                    "mutants"
                } else {
                    "results"
                }],
                serde_json::json!([])
            );
        }
    }
    let report = project.json("list", &["--operator", "AUTH-001"]);
    assert_eq!(report["mutants"].as_array().unwrap().len(), 8);
    assert!(report["mutants"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["operator"] == "AUTH-001"));
    assert!(!project.0.join(".soro-mutants-worktree").exists());
}

#[test]
fn exclusions_prune_files_and_directories_and_override_includes() {
    let project = Project::new();
    // Reading this invalid UTF-8 file would fail if pruning happened after discovery.
    fs::write(project.0.join("vendor/nested/bad.rs"), [0xff]).unwrap();
    for dir in [
        "target",
        ".git",
        ".soro-mutants-target",
        ".soro-mutants-worktree",
        "mutants.out",
        "node_modules",
    ] {
        project.write(&format!("{dir}/bad.rs"), "placeholder");
        fs::write(project.0.join(dir).join("bad.rs"), [0xff]).unwrap();
    }
    let args = [
        "--exclude-path",
        "vendor",
        "--exclude-path",
        "generated/**",
        "--exclude-path",
        "src/z.rs",
    ];
    let report = project.json("list", &args);
    let mutants = report["mutants"].as_array().unwrap();
    assert_eq!(mutants.len(), 3);
    assert!(mutants.iter().all(|m| m["file"] == "src/a.rs"));
    let mut overlap = args.to_vec();
    overlap.extend(["--file", "src/z.rs"]);
    assert_eq!(
        project.json("list", &overlap)["mutants"],
        serde_json::json!([])
    );
    assert_eq!(
        project.json("test", &overlap)["results"],
        serde_json::json!([])
    );
    assert_eq!(
        project.json("list", &["--exclude-path", "**"])["mutants"],
        serde_json::json!([])
    );
    // Exclusion is discovery-only: source remains available to baseline/build commands.
    assert!(project.0.join("vendor/nested/lib.rs").exists());
}

#[test]
fn glob_semantics_are_root_relative_and_separator_aware() {
    let project = Project::new();
    let report = project.json("list", &["--exclude-path", "*.rs"]);
    assert_eq!(report["mutants"].as_array().unwrap().len(), 12);
    let report = project.json("list", &["--exclude-path", "**/lib.rs"]);
    assert_eq!(report["mutants"].as_array().unwrap().len(), 6);
    let report = project.json(
        "list",
        &[
            "--exclude-path",
            "src/[az].rs",
            "--exclude-path",
            "{generated,vendor}/**",
        ],
    );
    assert_eq!(report["mutants"], serde_json::json!([]));
    for command in ["list", "test"] {
        let output = project.run(command, &["--exclude-path", "["]);
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("invalid exclude-path glob"));
    }
}

#[test]
fn shards_partition_filtered_mutants_without_renumbering() {
    let project = Project::new();
    for filters in [
        vec![],
        vec![
            "--exclude-path",
            "vendor",
            "--operator",
            "AUTH-001",
            "--file",
            "src/a.rs",
            "--function",
            "second",
        ],
    ] {
        let all = project.json("list", &filters)["mutants"]
            .as_array()
            .unwrap()
            .clone();
        for total in [1, 2, 5, 20] {
            let mut seen = BTreeSet::new();
            let mut union = Vec::new();
            for index in 1..=total {
                let shard = format!("{index}/{total}");
                let mut args = filters.clone();
                args.extend(["--shard", &shard]);
                let report = project.json("list", &args);
                assert_eq!(report, project.json("list", &args));
                let actual = report["mutants"].as_array().unwrap();
                let expected: Vec<_> = all.iter().skip(index - 1).step_by(total).cloned().collect();
                assert_eq!(actual, &expected);
                let mut count_args = args.clone();
                count_args.push("--count-only");
                let count = project.run("list", &count_args);
                assert!(count.status.success());
                assert_eq!(
                    String::from_utf8(count.stdout).unwrap().trim(),
                    actual.len().to_string()
                );
                for mutant in actual {
                    assert!(seen.insert(mutant["id"].as_str().unwrap().to_owned()));
                    union.push(mutant.clone());
                }
            }
            union.sort_by_key(|m| m["id"].as_str().unwrap().to_owned());
            assert_eq!(union, all);
        }
    }
    assert_eq!(
        project.json("list", &["--function", "missing", "--shard", "1/3"])["mutants"],
        serde_json::json!([])
    );
}

#[test]
fn invalid_shards_fail_before_execution() {
    let project = Project::new();
    for command in ["list", "test"] {
        for shard in [
            "",
            "0/1",
            "1/0",
            "2/1",
            "1",
            "1/2/3",
            "a/2",
            "-1/2",
            "+1/2",
            "1/999999999999999999999999999999",
        ] {
            let output = project.run(command, &[&format!("--shard={shard}")]);
            assert_eq!(output.status.code(), Some(2), "{shard}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("INDEX/TOTAL"));
        }
    }
}

#[test]
fn list_and_test_share_shards_order_and_baseline_gating() {
    let project = Project::new();
    let filters = ["--shard", "2/3", "--exclude-path", "vendor"];
    let expected = project.json("list", &filters)["mutants"]
        .as_array()
        .unwrap()
        .clone();
    let list_text = project.run("list", &filters);
    assert!(list_text.status.success());
    let list_text = String::from_utf8(list_text.stdout).unwrap();
    let ids: Vec<_> = list_text
        .lines()
        .filter(|l| l.starts_with('M'))
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(
        ids,
        expected
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect::<Vec<_>>()
    );
    let output_path = project.0.join("shard.json");
    let mut saved_args = filters.to_vec();
    saved_args.extend(["--json", "--output", output_path.to_str().unwrap()]);
    let saved = project.run("list", &saved_args);
    assert!(saved.status.success());
    assert!(saved.stdout.is_empty());
    let saved: Value = serde_json::from_slice(&fs::read(output_path).unwrap()).unwrap();
    assert_eq!(saved["mutants"], serde_json::json!(expected));
    let mut args = filters.to_vec();
    // A lightweight custom command exercises execution/reporting without SDK dependencies.
    args.extend(["--test-command", "test -f vendor/nested/lib.rs"]);
    let results = project.json("test", &args)["results"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        results
            .iter()
            .map(|r| r["mutant"].clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(results.iter().all(|r| r["outcome"] == "SURVIVED"));
    let text = project.run("test", &args);
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    let lines: Vec<_> = text.lines().filter(|l| l.starts_with("SURVIVED")).collect();
    assert_eq!(lines.len(), expected.len());
    for (line, mutant) in lines.iter().zip(&expected) {
        assert!(line.contains(mutant["operator"].as_str().unwrap()));
        assert!(line.contains(&format!(
            "{}:{}",
            mutant["file"].as_str().unwrap(),
            mutant["span"]["line"]
        )));
    }
    let failed = project.run("test", &["--shard", "1/2", "--test-command", "false"]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("baseline tests failed"));
    let empty = project.json("test", &["--shard", "20/20", "--test-command", "false"]);
    assert_eq!(empty["results"], serde_json::json!([]));
}
