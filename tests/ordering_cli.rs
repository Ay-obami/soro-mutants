use std::{fs, path::PathBuf, process::Command};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("soro-order-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/z.rs"),
            "fn late(user: Address) { user.require_auth(); }\n",
        )
        .unwrap();
        fs::write(root.join("src/a.rs"), "fn first(user: Address, admin: Address) { user.require_auth(); }\nfn second(admin: Address) { admin.require_auth(); }\n").unwrap();
        Self(root)
    }

    fn run(&self, extra: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .arg("list")
            .arg(&self.0)
            .args(extra)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn multiple_files_functions_and_operators_have_stable_order() {
    let fixture = Fixture::new();
    let first = fixture.run(&["--json"]);
    for _ in 0..5 {
        assert_eq!(first, fixture.run(&["--json"]));
    }
    let report: serde_json::Value = serde_json::from_str(&first).unwrap();
    let mutants = report["mutants"].as_array().unwrap();
    assert!(mutants.len() >= 4, "expected multi-operator discovery");
    let keys: Vec<_> = mutants
        .iter()
        .map(|m| {
            (
                m["file"].as_str().unwrap().to_owned(),
                m["span"]["line"].as_u64().unwrap(),
                m["span"]["column"].as_u64().unwrap(),
                m["operator"].as_str().unwrap().to_owned(),
                m["replacement"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    for (i, m) in mutants.iter().enumerate() {
        assert_eq!(m["id"], format!("M{:04}", i + 1));
    }
    let text = fixture.run(&[]);
    let text_ids: Vec<_> = text
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|s| s.starts_with('M') && s[1..].bytes().all(|b| b.is_ascii_digit()))
        .collect();
    let json_ids: Vec<_> = mutants.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(text_ids, json_ids);
}
