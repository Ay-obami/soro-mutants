use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct TempProject {
    root: PathBuf,
}

impl TempProject {
    fn new(suffix: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("soro-order-prop-{}-{}", std::process::id(), suffix));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();

        // Minimal Cargo.toml to satisfy project discovery
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "soro-order-prop"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();

        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn run_list_json(&self) -> serde_json::Value {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
            .arg("list")
            .arg(&self.root)
            .arg("--json")
            .output()
            .expect("cargo-soro-mutants list should run successfully");

        assert!(
            output.status.success(),
            "list command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        serde_json::from_slice(&output.stdout).expect("output must be valid JSON")
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Simple deterministic pseudo-random number generator for reproducible permutations.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.0 >> 32) as u32
    }

    fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = (self.next_u32() as usize) % (i + 1);
            slice.swap(i, j);
        }
    }
}

#[test]
fn property_mutant_ordering_is_invariant_under_randomized_filesystem_discovery() {
    // Defines a multi-module project layout with multiple mutation sites across functions and files
    let file_specs = [
        (
            "src/alpha.rs",
            r#"
fn auth_alpha(user: Address, admin: Address) {
    user.require_auth();
    admin.require_auth();
}
"#,
        ),
        (
            "src/beta.rs",
            r#"
fn auth_beta(caller: Address) {
    caller.require_auth();
}
"#,
        ),
        (
            "src/sub/gamma.rs",
            r#"
fn auth_gamma(owner: Address, spender: Address) {
    owner.require_auth();
}
"#,
        ),
        (
            "src/zeta.rs",
            r#"
fn auth_zeta(root: Address) {
    root.require_auth();
}
"#,
        ),
    ];

    let mut baseline_json: Option<serde_json::Value> = None;

    // Run 10 iterations with randomized file creation and directory traversal sequences
    let mut rng = Lcg::new(0x4d27192a3b5c8e11);

    for iteration in 0..10 {
        let project = TempProject::new(&format!("iter-{}", iteration));
        fs::create_dir_all(project.path().join("src/sub")).unwrap();

        let mut shuffled = file_specs.to_vec();
        rng.shuffle(&mut shuffled);

        for (rel_path, content) in shuffled {
            fs::write(project.path().join(rel_path), content).unwrap();
        }

        let json = project.run_list_json();
        let mutants = json["mutants"]
            .as_array()
            .expect("report should contain mutants array");
        assert!(
            mutants.len() >= 5,
            "expected at least 5 discovered mutants across files"
        );

        if let Some(ref baseline) = baseline_json {
            assert_eq!(
                json, *baseline,
                "mutant ordering diverged on iteration {iteration} with different file creation order"
            );
        } else {
            baseline_json = Some(json);
        }
    }
}

#[test]
fn property_tie_breaking_order_is_deterministic_on_coincident_spans() {
    // Regression seed demonstrating coincident source location tie-breaking:
    // Two operators targeting the same function/statement must have stable, non-flapping IDs.
    let project = TempProject::new("regression-seed");
    fs::write(
        project.path().join("src/lib.rs"),
        r#"
fn transfer_check(sender: Address, receiver: Address) {
    sender.require_auth();
}
"#,
    )
    .unwrap();

    let json1 = project.run_list_json();
    let json2 = project.run_list_json();

    assert_eq!(
        json1, json2,
        "coincident span mutants must produce stable JSON reports across invocations"
    );

    let mutants = json1["mutants"].as_array().unwrap();
    for (idx, m) in mutants.iter().enumerate() {
        assert_eq!(
            m["id"],
            format!("M{:04}", idx + 1),
            "mutant IDs must remain sequential and deterministic"
        );
    }
}

#[test]
fn property_discovered_mutant_canonical_sorting_is_permutation_invariant() {
    // Simulated mutant key tuple matching Soro Mutants' total ordering contract:
    // (file, start_byte, operator, replacement, end_byte, original, function, description)
    #[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
    struct OrderingKey {
        file: String,
        start_byte: usize,
        operator: String,
        replacement: String,
        end_byte: usize,
        original: String,
        function: Option<String>,
        description: String,
    }

    let canonical_keys = vec![
        OrderingKey {
            file: "src/a.rs".into(),
            start_byte: 10,
            operator: "AUTH-001".into(),
            replacement: "()".into(),
            end_byte: 32,
            original: "user.require_auth()".into(),
            function: Some("do_work".into()),
            description: "Remove require_auth()".into(),
        },
        OrderingKey {
            file: "src/a.rs".into(),
            start_byte: 10,
            operator: "AUTH-002".into(),
            replacement: "admin.require_auth()".into(),
            end_byte: 32,
            original: "user.require_auth()".into(),
            function: Some("do_work".into()),
            description: "Authenticate admin instead of user".into(),
        },
        OrderingKey {
            file: "src/a.rs".into(),
            start_byte: 50,
            operator: "TTL-001".into(),
            replacement: "()".into(),
            end_byte: 75,
            original: "env.storage().extend_ttl(...)".into(),
            function: Some("do_work".into()),
            description: "Remove extend_ttl()".into(),
        },
        OrderingKey {
            file: "src/b.rs".into(),
            start_byte: 5,
            operator: "EVENT-001".into(),
            replacement: "()".into(),
            end_byte: 25,
            original: "env.events().publish(...)".into(),
            function: Some("log_event".into()),
            description: "Remove event publication".into(),
        },
    ];

    let mut rng = Lcg::new(0xabcdef0123456789);

    for permutation in 0..100 {
        let mut shuffled = canonical_keys.clone();
        rng.shuffle(&mut shuffled);

        // Sort using the canonical key comparator
        shuffled.sort();

        assert_eq!(
            shuffled, canonical_keys,
            "canonical sorting failed to restore original order at permutation {permutation}"
        );
    }
}
