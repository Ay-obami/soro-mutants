use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn project(source: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "soro-semantic-accuracy-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        concat!(
            "[package]\n",
            "name = \"semantic_accuracy_fixture\"\n",
            "version = \"0.1.0\"\n",
            "edition = \"2021\"\n\n",
            "[workspace]\n",
        ),
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), source).unwrap();
    root
}

fn run(root: &Path, command: &str, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .arg(command)
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{command} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn list(root: &Path) -> Vec<serde_json::Value> {
    let json: serde_json::Value = serde_json::from_str(&run(root, "list", &["--json"])).unwrap();
    json["mutants"].as_array().unwrap().clone()
}

fn count(mutants: &[serde_json::Value], operator: &str) -> usize {
    mutants.iter().filter(|m| m["operator"] == operator).count()
}

#[test]
fn generic_wrappers_and_nested_modules_do_not_create_soroban_false_positives() {
    let root = project(
        r#"
#[derive(Clone)]
pub struct Address;
impl Address { pub fn require_auth(&self) {} }
pub struct Env;
impl Env {
    pub fn events(&self) -> Events { Events }
    pub fn storage(&self) -> Storage { Storage }
}
pub struct Events;
impl Events { pub fn publish(&self, _: (&str,), _: &Address) {} }
pub struct Storage;
impl Storage { pub fn instance(&self) -> Instance { Instance } }
pub struct Instance;
impl Instance { pub fn extend_ttl(&self, _: u32, _: u32) {} }
pub mod token {
    use super::{Address, Env};
    pub struct Client;
    impl Client {
        pub fn new(_: &Env, _: &Address) -> Self { Self }
        pub fn transfer(&self, _: &Address, _: &Address, _: &i128) {}
    }
}
pub struct Wrapper<T>(pub T);
impl<T> Wrapper<T> {
    pub fn require_auth(&self) {}
    pub fn transfer(&self, _: &Address, _: &Address, _: &i128) {}
    pub fn publish(&self, _: (&str,), _: &Address) {}
    pub fn extend_ttl(&self, _: u32, _: u32) {}
    pub fn storage(&self) -> Storage { Storage }
}
pub fn mixed(env: Env, from: Address, to: Address, amount: i128, wrapper: Wrapper<u32>) {
    wrapper.require_auth();
    wrapper.transfer(&from, &to, &amount);
    wrapper.publish(("fake",), &from);
    wrapper.extend_ttl(2, 8);
    wrapper.storage().instance().extend_ttl(2, 8);
    from.require_auth();
    env.events().publish(("real",), &from);
    token::Client::new(&env, &from).transfer(&from, &to, &amount);
    env.storage().instance().extend_ttl(2, 8);
}
mod nested {
    pub struct Wrapper<T>(pub T);
    impl<T> Wrapper<T> { pub fn require_auth(&self) {} }
    pub fn exercise(env: super::Env, from: super::Address, wrapper: Wrapper<u32>) {
        wrapper.require_auth();
        from.require_auth();
        let _ = env;
    }
}
"#,
    );
    let mutants = list(&root);
    assert_eq!(count(&mutants, "AUTH-001"), 2, "{mutants:#?}");
    assert_eq!(count(&mutants, "EVENT-001"), 1, "{mutants:#?}");
    assert_eq!(count(&mutants, "TOKEN-001"), 1, "{mutants:#?}");
    assert_eq!(count(&mutants, "TTL-001"), 1, "{mutants:#?}");
    assert_eq!(count(&mutants, "STORAGE-001"), 2, "{mutants:#?}");
    assert!(!mutants
        .iter()
        .any(|m| { m["original"].as_str().unwrap_or("").contains("wrapper.") }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn address_substitutions_obey_nested_block_match_loop_and_closure_shadowing() {
    let source = r#"
#[derive(Clone)]
pub struct Address;
impl Address { pub fn require_auth(&self) {} }
pub fn scopes(owner: Address, alternate: Address, amounts: [u32; 2], option: Option<u32>) {
    owner.require_auth();
    {
        let owner: u32 = 3;
        alternate.require_auth();
        let _ = owner;
    }
    for owner in amounts {
        alternate.require_auth();
        let _ = owner;
    }
    match option {
        Some(owner) => { let _ = owner; alternate.require_auth(); }
        None => { owner.require_auth(); }
    }
    let _closure = |owner: u32| {
        let _ = owner;
        alternate.require_auth();
    };
    alternate.require_auth();
}
"#;
    let root = project(source);
    let mutants = list(&root);
    let substitutions: Vec<_> = mutants
        .iter()
        .filter(|m| m["operator"] == "AUTH-002")
        .collect();
    assert_eq!(substitutions.len(), 3, "{mutants:#?}");
    assert_eq!(
        substitutions
            .iter()
            .filter(|m| m["replacement"] == "alternate.require_auth()")
            .count(),
        2
    );
    assert_eq!(
        substitutions
            .iter()
            .filter(|m| m["replacement"] == "owner.require_auth()")
            .count(),
        1
    );

    // Apply each selected mutation at its original byte span, then compile.
    for mutant in substitutions {
        let span = &mutant["span"];
        let (start, end) = (
            byte_offset(
                source,
                span["line"].as_u64().unwrap() as usize,
                span["column"].as_u64().unwrap() as usize,
            ),
            byte_offset(
                source,
                span["end_line"].as_u64().unwrap() as usize,
                span["end_column"].as_u64().unwrap() as usize,
            ),
        );
        assert_eq!(&source[start..end], mutant["original"].as_str().unwrap());
        let mutated = format!(
            "{}{}{}",
            &source[..start],
            mutant["replacement"].as_str().unwrap(),
            &source[end..]
        );
        fs::write(root.join("src/lib.rs"), mutated).unwrap();
        let check = Command::new("cargo")
            .args(["check", "--offline", "--quiet", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "substitution did not compile: {}",
            String::from_utf8_lossy(&check.stderr)
        );
        // Restore the original source for the next mutation and avoid
        // carrying an earlier edit into a later compile check.
        fs::write(root.join("src/lib.rs"), source).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

fn byte_offset(source: &str, line: usize, column: usize) -> usize {
    let line_start = source
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum::<usize>();
    line_start + column - 1
}

#[test]
fn direct_sdk_aliases_work_but_unrelated_aliases_are_rejected() {
    let root = project(
        r#"
extern crate self as soroban_sdk;
#[derive(Clone)]
pub struct Address;
impl Address { pub fn require_auth(&self) {} }
pub struct Env;
impl Env {
    pub fn events(&self) -> Events { Events }
    pub fn storage(&self) -> Storage { Storage }
}
pub struct Events;
impl Events { pub fn publish(&self, _: (&str,), _: &Address) {} }
pub struct Storage;
impl Storage { pub fn instance(&self) -> storage::Instance { storage::Instance } }
pub mod storage {
    pub struct Instance;
    impl Instance { pub fn extend_ttl(&self, _: u32, _: u32) {} }
}
pub mod token {
    use super::{Address, Env};
    pub struct Client;
    impl Client {
        pub fn new(_: &Env, _: &Address) -> Self { Self }
        pub fn transfer(&self, _: &Address, _: &Address, _: &i128) {}
    }
}
pub mod positive {
    use soroban_sdk::{Address as Account, Env as Runtime};
    use soroban_sdk::token::Client as AssetClient;
    use soroban_sdk::storage::Instance as Store;
    pub fn execute(env: Runtime, from: Account, to: Account, asset: AssetClient, store: Store) {
        from.require_auth();
        env.events().publish(("ok",), &from);
        asset.transfer(&from, &to, &1);
        store.extend_ttl(2, 8);
        let other = AssetClient::new(&env, &to);
        other.transfer(&from, &to, &1);
    }
}
pub mod unrelated {
    pub mod types {
        pub struct Address;
        impl Address { pub fn require_auth(&self) {} }
        pub struct Env;
        impl Env { pub fn events(&self) -> super::super::Events { super::super::Events } }
    }
    use self::types::{Address as Account, Env as Runtime};
    pub fn execute(from: Account, env: Runtime) {
        from.require_auth();
        let _ = env.events();
    }
}
"#,
    );
    let mutants = list(&root);
    assert_eq!(count(&mutants, "AUTH-001"), 1, "{mutants:#?}");
    assert_eq!(count(&mutants, "AUTH-002"), 1, "{mutants:#?}");
    assert_eq!(count(&mutants, "EVENT-001"), 1, "{mutants:#?}");
    assert_eq!(count(&mutants, "TOKEN-001"), 2, "{mutants:#?}");
    assert_eq!(count(&mutants, "TTL-001"), 1, "{mutants:#?}");
    assert!(mutants
        .iter()
        .all(|m| { !m["function"].as_str().unwrap_or("").contains("unrelated") }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn multiline_locations_agree_across_all_report_formats() {
    let source = concat!(
        "pub fn multiline(admin: Address, env: Env) {\n",
        "    admin.require_auth(\n",
        "    ); // auth tail\n",
        "    env.events().publish(\n",
        "        (\"topic\",), &admin\n",
        "    ); // event tail\n",
        "}\n"
    );
    let root = project(source);
    let mutants = list(&root);
    let auth = mutants
        .iter()
        .find(|m| m["operator"] == "AUTH-001")
        .unwrap();
    assert_eq!(
        auth["span"],
        serde_json::json!({
            "line": 2, "column": 5, "end_line": 3, "end_column": 6
        })
    );
    assert_eq!(auth["original"], "admin.require_auth(\n    )");
    let text = run(&root, "list", &["--operator", "AUTH-001"]);
    assert!(text.contains("src/lib.rs:2:5-3:6"), "{text}");
    let markdown = run(
        &root,
        "test",
        &[
            "--operator",
            "AUTH-001",
            "--test-command",
            "true",
            "--markdown",
        ],
    );
    assert!(
        markdown.contains("[src/lib.rs:2:5-3:6](src/lib.rs#L2-L3)"),
        "{markdown}"
    );
    let html = run(
        &root,
        "test",
        &["--operator", "AUTH-001", "--test-command", "true", "--html"],
    );
    assert!(html.contains("src/lib.rs:2:5-3:6"), "{html}");
    let sarif: serde_json::Value = serde_json::from_str(&run(
        &root,
        "test",
        &[
            "--operator",
            "AUTH-001",
            "--test-command",
            "true",
            "--sarif",
        ],
    ))
    .unwrap();
    let region = &sarif["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"];
    assert_eq!(
        region,
        &serde_json::json!({
            "startLine": 2, "startColumn": 5, "endLine": 3, "endColumn": 6
        })
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn multiline_event_token_and_ttl_mutations_retain_original_byte_ranges() {
    let source = concat!(
        "pub fn mutate(env: Env, from: Address, to: Address, amount: i128) {\n",
        "    let client = token::Client::new(&env, &from);\n",
        "    client.transfer(\n",
        "        &from,\n",
        "        &to,\n",
        "        &amount,\n",
        "    ); // transfer tail\n",
        "    env.events().publish(\n",
        "        (\"transfer\",),\n",
        "        (&from, &to),\n",
        "    ); // event tail\n",
        "    env.storage().instance().extend_ttl(\n",
        "        3,\n",
        "        9,\n",
        "    ); // ttl tail\n",
        "}\n"
    );
    let root = project(source);
    let mutants = list(&root);
    for operator in ["TOKEN-001", "EVENT-001", "TTL-001"] {
        let mutant = mutants
            .iter()
            .find(|m| m["operator"] == operator)
            .unwrap_or_else(|| panic!("missing {operator}: {mutants:#?}"));
        let span = &mutant["span"];
        let start_line = span["line"].as_u64().unwrap() as usize;
        let end_line = span["end_line"].as_u64().unwrap() as usize;
        assert!(end_line > start_line, "{operator} needs a multiline span");
        let start = byte_offset(
            source,
            start_line,
            span["column"].as_u64().unwrap() as usize,
        );
        let end = byte_offset(
            source,
            end_line,
            span["end_column"].as_u64().unwrap() as usize,
        );
        assert_eq!(&source[start..end], mutant["original"].as_str().unwrap());
        let mutated = format!(
            "{}{}{}",
            &source[..start],
            mutant["replacement"].as_str().unwrap(),
            &source[end..]
        );
        assert!(mutated.starts_with(&source[..start]));
        assert!(mutated.ends_with(&source[end..]));
        assert!(mutated.contains("// transfer tail"));
        assert!(mutated.contains("// event tail"));
        assert!(mutated.contains("// ttl tail"));
    }
    fs::remove_dir_all(root).unwrap();
}
