use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn mutant_only_command_not_found_aborts_instead_of_becoming_killed() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "soro-mutant-launch-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    let source =
        "pub struct Address; impl Address { pub fn require_auth(&self) {} } pub fn check(user: Address) { user.require_auth(); }\n";
    fs::write(root.join("src/lib.rs"), source).unwrap();

    let command = "case \"$PWD\" in *.soro-mutants-worktree* ) soro-mutants-missing-child-command ;; * ) true ;; esac";
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("test")
        .arg(&root)
        .args([
            "--operator",
            "AUTH-001",
            "--test-command",
            command,
            "--timeout",
            "10",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("test process launch failed (command_not_found)"), "{stderr}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("KILLED"), "{stdout}");
    assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), source);
    assert!(!root.join(".soro-mutants-worktree").exists());

    let _ = fs::remove_dir_all(root);
}
