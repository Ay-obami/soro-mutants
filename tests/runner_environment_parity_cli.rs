use std::{
    fs,
    path::PathBuf,
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
        let root =
            std::env::temp_dir().join(format!("soro-runner-home-{}-{nonce}", std::process::id()));
        fs::create_dir_all(root.join("project/src")).unwrap();
        fs::create_dir_all(root.join("isolated-home")).unwrap();
        fs::create_dir_all(root.join("isolated-xdg")).unwrap();
        fs::write(
            root.join("project/src/lib.rs"),
            "fn check(user: Address) { user.require_auth(); }\n",
        )
        .unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn baseline_and_mutants_inherit_the_same_intended_environment_with_isolated_home() {
    let fixture = Fixture::new();
    let home = fixture.0.join("isolated-home");
    let xdg = fixture.0.join("isolated-xdg");
    let source = fs::read(fixture.0.join("project/src/lib.rs")).unwrap();
    // This command is executed by both baseline and mutant runs; it must
    // receive the same inherited values and the same test-only HOME.
    let command = r#"test "$HOME" = "$SORO_TEST_HOME" && test "$XDG_CONFIG_HOME" = "$SORO_TEST_XDG" && test "$SORO_PARITY_SENTINEL" = "consistent" && test -z "${RUSTUP_TOOLCHAIN+x}" && mkdir -p "$HOME/.soro-tool-test" && printf success > "$HOME/.soro-tool-test/marker""#;
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test")
        .arg(fixture.0.join("project"))
        .args([
            "--operator",
            "AUTH-001",
            "--test-command",
            command,
            "--timeout",
            "10",
        ])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &xdg)
        .env("SORO_TEST_HOME", &home)
        .env("SORO_TEST_XDG", &xdg)
        .env("SORO_PARITY_SENTINEL", "consistent")
        .env("RUSTUP_TOOLCHAIN", "intentionally-removed")
        .output()
        .expect("soro mutants should launch");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("SURVIVED"));
    assert_eq!(
        fs::read_to_string(home.join(".soro-tool-test/marker")).unwrap(),
        "success"
    );
    assert_eq!(
        fs::read(fixture.0.join("project/src/lib.rs")).unwrap(),
        source
    );
    assert!(!fixture.0.join("project/.soro-mutants-worktree").exists());
}
