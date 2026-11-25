use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn project(name: &str, source: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "soro-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n"
        ),
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), source).unwrap();
    root
}

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn trait_and_ufcs_soroban_like_names_do_not_create_false_positives() {
    let root = project(
        "trait_false_positive",
        r#"
pub struct Address;
impl Address { pub fn require_auth(&self) {} }
pub struct Env;
pub struct Events;
impl Env { pub fn events(&self) -> Events { Events } }
impl Events { pub fn publish(&self, _: (&str,), _: &Address) {} }

trait Sensitive {
    fn require_auth(&self);
    fn transfer(&self, from: &Address, to: &Address, amount: &i128);
    fn publish(&self, topic: (&str,), who: &Address);
    fn extend_ttl(&self, threshold: u32, extend_to: u32);
}
pub struct Wrapper;
impl Sensitive for Wrapper {
    fn require_auth(&self) {}
    fn transfer(&self, _: &Address, _: &Address, _: &i128) {}
    fn publish(&self, _: (&str,), _: &Address) {}
    fn extend_ttl(&self, _: u32, _: u32) {}
}

pub fn mixed(env: Env, admin: Address, recipient: Address, wrapper: Wrapper) {
    wrapper.require_auth();
    wrapper.transfer(&admin, &recipient, &1);
    wrapper.publish(("fake",), &admin);
    wrapper.extend_ttl(2, 8);
    Sensitive::transfer(&wrapper, &admin, &recipient, &1);
    admin.require_auth();
    env.events().publish(("real",), &admin);
}
"#,
    );

    let output = cli(&["list", root.to_str().unwrap(), "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mutants = report["mutants"].as_array().unwrap();
    assert!(mutants.iter().any(|m| m["operator"] == "AUTH-001"));
    assert!(mutants.iter().any(|m| m["operator"] == "EVENT-001"));
    assert!(!mutants.iter().any(|m| {
        m["original"]
            .as_str()
            .is_some_and(|original| original.contains("wrapper.") || original.contains("Sensitive::"))
    }));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn closure_parameters_shadow_outer_addresses_without_leaking_candidates() {
    let source = r#"
#[derive(Clone)]
pub struct Address;
impl Address { pub fn require_auth(&self) {} }

pub fn scopes(owner: Address, alternate: Address) {
    let non_move = |owner: u32| {
        let _ = owner;
        alternate.require_auth();
    };
    let move_closure = move |inner: Address| {
        alternate.require_auth();
        inner.require_auth();
    };
    let _ = (non_move, move_closure);
    owner.require_auth();
}
"#;
    let root = project("closure_scope", source);

    let list = cli(&[
        "list",
        root.to_str().unwrap(),
        "--operator",
        "AUTH-002",
        "--json",
    ]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let mutants = report["mutants"].as_array().unwrap();

    assert!(!mutants.iter().any(|m| {
        m["replacement"] == "owner.require_auth()"
            && m["span"]["line"].as_u64() == Some(9)
    }));
    assert!(mutants.iter().any(|m| {
        m["replacement"] == "inner.require_auth()"
            && m["span"]["line"].as_u64() == Some(12)
    }));
    assert!(!mutants.iter().any(|m| {
        m["replacement"] == "inner.require_auth()"
            && m["span"]["line"].as_u64().is_some_and(|line| line >= 16)
    }));

    let test = cli(&[
        "test",
        root.to_str().unwrap(),
        "--operator",
        "AUTH-002",
        "--test-command",
        "cargo test -q",
        "--timeout",
        "30",
        "--no-progress",
    ]);
    assert!(
        test.status.success(),
        "{}",
        String::from_utf8_lossy(&test.stderr)
    );
    assert!(!String::from_utf8_lossy(&test.stdout).contains("Unviable  : 1"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn invalid_scratch_root_fails_before_baseline_execution() {
    let root = project(
        "scratch_probe",
        "pub struct Address; impl Address { pub fn require_auth(&self) {} } pub fn check(user: Address) { user.require_auth(); }\n",
    );
    let scratch_base = root.join("external-scratch");
    fs::create_dir_all(&scratch_base).unwrap();
    fs::write(
        scratch_base.join(".soro-mutants-worktree"),
        "a file cannot be used as a scratch directory",
    )
    .unwrap();

    let output = cli(&[
        "test",
        root.to_str().unwrap(),
        "--operator",
        "AUTH-001",
        "--scratch-dir",
        scratch_base.to_str().unwrap(),
        "--test-command",
        "echo ran > baseline-ran",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scratch root") || stderr.contains("cannot create"),
        "{stderr}"
    );
    assert!(
        !root.join("baseline-ran").exists(),
        "baseline must not run when scratch validation fails"
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn symlinked_project_root_is_canonicalized_but_reports_project_relative_paths() {
    use std::os::unix::fs::symlink;

    let real = project(
        "symlink_root",
        "pub struct Address; impl Address { pub fn require_auth(&self) {} } pub fn check(user: Address) { user.require_auth(); }\n",
    );
    let link = real.with_extension("link");
    symlink(&real, &link).unwrap();
    let original = fs::read(real.join("src/lib.rs")).unwrap();

    let listed = cli(&["list", link.to_str().unwrap(), "--json"]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(report["mutants"][0]["file"], "src/lib.rs");

    let tested = cli(&[
        "test",
        link.to_str().unwrap(),
        "--operator",
        "AUTH-001",
        "--test-command",
        "true",
        "--no-progress",
    ]);
    assert!(
        tested.status.success(),
        "{}",
        String::from_utf8_lossy(&tested.stderr)
    );
    assert_eq!(fs::read(real.join("src/lib.rs")).unwrap(), original);
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert!(!real.join(".soro-mutants-worktree").exists());

    let _ = fs::remove_file(link);
    let _ = fs::remove_dir_all(real);
}

#[cfg(not(unix))]
#[test]
fn symlink_root_platform_behavior_is_documented() {
    let docs = fs::read_to_string("docs/runner-path-safety.md").unwrap();
    assert!(docs.contains("project root"));
}
