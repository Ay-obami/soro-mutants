use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("soro-mutants-clean-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg("clean")
        .arg(root)
        .args(args)
        .output()
        .expect("clean command should run")
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "clean failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn dry_run_lists_every_managed_path_without_removing_anything() {
    let temp = TempDir::new();
    let root = temp.path();
    let cache = root.join(".soro-mutants-target");
    let paths = [
        root.join(".soro-mutants-worktree"),
        cache.join("baseline"),
        cache.join("mutants-shared"),
        cache.clone(),
    ];
    for path in &paths[..3] {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("marker"), "keep during preview").unwrap();
    }

    let preview = stdout(&run(root, &["--dry-run"]));
    assert_eq!(preview.lines().count(), paths.len());
    for path in &paths {
        assert!(
            preview
                .lines()
                .any(|line| line == format!("Would remove {}", path.display())),
            "missing {} in {preview}",
            path.display()
        );
    }
    for path in &paths {
        assert!(path.exists(), "{} should survive preview", path.display());
    }

    let actual = stdout(&run(root, &[]));
    for path in &paths {
        assert!(actual.contains(&format!("Removed {}", path.display())));
        assert!(
            !path.exists(),
            "{} should have been removed",
            path.display()
        );
    }
}

#[test]
fn nothing_to_clean_succeeds_in_both_modes() {
    let temp = TempDir::new();
    for args in [&["--dry-run"][..], &[][..]] {
        let output = run(temp.path(), args);
        assert_eq!(
            stdout(&output).trim(),
            "No Soro Mutants generated state found."
        );
    }
}

#[test]
fn dry_run_and_clean_keep_unmanaged_siblings() {
    let temp = TempDir::new();
    let cache = temp.path().join(".soro-mutants-target");
    let baseline = cache.join("baseline");
    fs::create_dir_all(&baseline).unwrap();
    fs::write(cache.join("unmanaged.txt"), "keep").unwrap();

    let preview = stdout(&run(temp.path(), &["--dry-run"]));
    assert!(preview.contains(&format!("Would remove {}", baseline.display())));
    assert!(!preview
        .lines()
        .any(|line| line == format!("Would remove {}", cache.display())));
    assert!(baseline.exists());

    stdout(&run(temp.path(), &[]));
    assert!(!baseline.exists());
    assert_eq!(
        fs::read_to_string(cache.join("unmanaged.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn custom_cache_only_removes_managed_subdirectories() {
    let temp = TempDir::new();
    let custom = temp.path().join("custom");
    for subdir in ["baseline", "mutants-shared", "other"] {
        fs::create_dir_all(custom.join(subdir)).unwrap();
    }
    fs::write(custom.join("other/keep.txt"), "keep").unwrap();

    let preview = stdout(&run(temp.path(), &["--target-dir", "custom", "--dry-run"]));
    for subdir in ["baseline", "mutants-shared"] {
        assert!(preview.contains(&format!("Would remove {}", custom.join(subdir).display())));
        assert!(custom.join(subdir).exists());
    }
    assert!(!preview
        .lines()
        .any(|line| line == format!("Would remove {}", custom.display())));
    assert!(!preview.contains("other"));

    stdout(&run(temp.path(), &["--target-dir", "custom"]));
    assert!(!custom.join("baseline").exists());
    assert!(!custom.join("mutants-shared").exists());
    assert_eq!(
        fs::read_to_string(custom.join("other/keep.txt")).unwrap(),
        "keep"
    );
}
