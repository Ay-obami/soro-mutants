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
            "soro-working-dir-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("runner")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub struct Address; impl Address { pub fn require_auth(&self) {} } pub fn check(user: Address) { user.require_auth(); }\n",
        )
        .unwrap();
        fs::write(root.join("runner/config.txt"), "relative-ok").unwrap();
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

fn run(project_arg: &Path, caller_cwd: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .current_dir(caller_cwd)
        .arg("test")
        .arg(project_arg)
        .args([
            "--operator",
            "AUTH-001",
            "--test-dir",
            "runner",
            "--test-command",
            "test \"$(cat config.txt)\" = relative-ok",
            "--timeout",
            "10",
            "--no-progress",
        ])
        .output()
        .unwrap()
}

#[test]
fn baseline_and_mutants_share_the_documented_relative_working_directory() {
    let fixture = Fixture::new();
    let absolute = run(fixture.path(), fixture.path().parent().unwrap());
    assert!(
        absolute.status.success(),
        "{}",
        String::from_utf8_lossy(&absolute.stderr)
    );
    assert!(String::from_utf8_lossy(&absolute.stdout).contains("Survived  : 1"));

    let parent = fixture.path().parent().unwrap();
    let relative_name = Path::new(fixture.path().file_name().unwrap());
    let relative = run(relative_name, parent);
    assert!(
        relative.status.success(),
        "{}",
        String::from_utf8_lossy(&relative.stderr)
    );
    assert!(String::from_utf8_lossy(&relative.stdout).contains("Survived  : 1"));
}
