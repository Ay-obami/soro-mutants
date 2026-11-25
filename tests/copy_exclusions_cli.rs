use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "soro-copy-exclusions-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"copy-exclusions-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        )
        .unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub struct Address; impl Address { pub fn require_auth(&self) {} } pub fn check(user: Address) { user.require_auth(); }\n",
        )
        .unwrap();
        for path in [
            "target/sentinel",
            ".soro-mutants-target/sentinel",
            ".soro-mutants-worktree/sentinel",
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "must-not-be-copied").unwrap();
        }
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn mutant_source_copy_excludes_target_and_soro_managed_cache_trees() {
    let fixture = Fixture::new();
    let command = r#"case "$PWD" in
        *.soro-mutants-worktree*)
            test ! -e target/sentinel &&
            test ! -e .soro-mutants-target/sentinel &&
            test ! -e .soro-mutants-worktree/sentinel
            ;;
        *) true ;;
    esac"#;

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test")
        .arg(fixture.path())
        .args([
            "--operator",
            "AUTH-001",
            "--test-command",
            command,
            "--timeout",
            "10",
            "--no-progress",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.path().join("target/sentinel")).unwrap(),
        "must-not-be-copied"
    );
    assert_eq!(
        fs::read_to_string(fixture.path().join(".soro-mutants-target/sentinel")).unwrap(),
        "must-not-be-copied"
    );
}
