use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_project(prefix: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"raw-ident-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    root
}

fn list(root: &std::path::Path, extra_args: &[&str]) -> serde_json::Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"));
    command.arg("list").arg(root).args(extra_args).arg("--json");
    let output = command.output().expect("CLI should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid JSON output")
}

#[test]
fn recognizes_soroban_operators_with_raw_identifiers_and_compiles_mutants() {
    let root = unique_project("soro-raw-ident-ops");
    fs::write(
        root.join("src/lib.rs"),
        r#"
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Address(u32);
impl Address {
    pub fn require_auth(&self) {}
    pub fn require_auth_for_args(&self, _args: soroban_sdk::Vec<u32>) {}
}

#[derive(Clone, Copy)]
pub struct Env;
#[derive(Clone, Copy)]
pub struct StorageAccessor;
#[derive(Clone, Copy)]
pub struct Storage;
#[derive(Clone, Copy)]
pub struct Events;

impl Env {
    pub fn storage(&self) -> Storage { Storage }
    pub fn events(&self) -> Events { Events }
    pub fn invoke_contract(&self, _target: &Address, _fn_name: &str, _args: ()) {}
}
impl Storage {
    pub fn instance(&self) -> StorageAccessor { StorageAccessor }
    pub fn persistent(&self) -> StorageAccessor { StorageAccessor }
    pub fn temporary(&self) -> StorageAccessor { StorageAccessor }
}
impl StorageAccessor {
    pub fn get(&self, _key: &u32) -> Option<Address> { Some(Address(1)) }
    pub fn extend_ttl(&self, _key: &u32, _threshold: u32, _extend_to: u32) {}
}
impl Events {
    pub fn publish(&self, _topics: (), _data: ()) {}
}

pub mod soroban_sdk {
    pub struct Vec<T>(core::marker::PhantomData<T>);
    impl<T> Vec<T> {
        pub fn new(_env: &super::Env) -> Self { Self(core::marker::PhantomData) }
        pub fn from_array(_env: &super::Env, _items: [T; 1]) -> Self {
            Self(core::marker::PhantomData)
        }
    }
}

pub mod token {
    use super::{Address, Env};
    pub struct Client;
    impl Client {
        pub fn new(_env: &Env, _id: &Address) -> Self { Self }
        pub fn transfer(&self, _from: &Address, _to: &Address, _amount: &i128) {}
        pub fn approve(&self, _from: &Address, _spender: &Address, _amount: &i128, _exp: &u32) {}
    }
}

pub struct r#PairClient;
impl r#PairClient {
    pub fn new(_env: &Env, _id: &Address) -> Self { Self }
    pub fn swap(&self, _actor: &Address, _amount: &i128) -> i128 { 1 }
}

pub fn r#check(r#who: Address) {
    who.require_auth();
}

pub fn r#match(
    r#for: Env,
    r#type: Address,
    r#else: Address,
    r#in: i128,
    r#loop: Option<Address>,
    r#vec: Vec<Address>,
) {
    r#type.require_auth();
    r#type.require_auth_for_args(soroban_sdk::Vec::from_array(&r#for, [1]));
    check(r#type);

    let r#stored: Address = r#for.storage().instance().get(&1).unwrap();
    stored.require_auth();

    let r#accessor = r#for.storage().persistent();
    r#accessor.extend_ttl(&1, 10, 100);

    let r#tok = token::Client::new(&r#for, &r#type);
    r#tok.transfer(&r#type, &r#else, &r#in);
    r#tok.approve(&r#type, &r#else, &r#in, &50);

    let r#client = r#PairClient::new(&r#for, &r#type);
    if r#client.swap(&r#type, &r#in) != 0 {
        r#for.invoke_contract(&r#type, "go", ());
    }

    r#for.events().publish((), ());

    for r#item in r#vec.iter() {
        r#item.require_auth();
    }
    match r#loop {
        Some(r#inner) => r#inner.require_auth(),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke() {
        r#match(
            Env,
            Address(1),
            Address(2),
            10,
            Some(Address(3)),
            vec![Address(4)],
        );
    }
}
"#,
    )
    .unwrap();

    let report = list(&root, &[]);
    let mutants = report["mutants"].as_array().unwrap();

    let has_op = |op: &str| mutants.iter().any(|m| m["operator"] == op);
    for op in [
        "AUTH-001",
        "AUTH-002",
        "AUTH-003",
        "AUTH-004",
        "AUTH-005",
        "CALL-001",
        "CALL-002",
        "CALL-003",
        "EVENT-001",
        "STORAGE-001",
        "TOKEN-001",
        "TOKEN-002",
        "TOKEN-003",
        "TTL-001",
        "TTL-002",
        "TTL-003",
    ] {
        assert!(has_op(op), "expected operator {op} to be discovered");
    }

    assert!(mutants.iter().any(|m| m["operator"] == "AUTH-002"
        && m["original"] == "r#type.require_auth()"
        && m["replacement"] == "r#else.require_auth()"));
    assert!(mutants.iter().any(
        |m| m["operator"] == "AUTH-003" && m["replacement"] == "soroban_sdk::Vec::new(&r#for)"
    ));
    assert!(mutants
        .iter()
        .any(|m| m["operator"] == "AUTH-005" && m["replacement"] == "r#else.clone()"));
    // Mixed raw/non-raw (`r#stored` vs `stored`) must never emit an equivalent self-replacement.
    assert!(!mutants.iter().any(|m| m["operator"] == "AUTH-002"
        && m["original"] == "stored.require_auth()"
        && m["replacement"] == "r#stored.require_auth()"));

    let execution = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args([
            "test",
            root.to_str().unwrap(),
            "--function",
            "r#match",
            "--test-command",
            "cargo test -q",
            "--timeout",
            "30",
            "--no-progress",
            "--json",
        ])
        .output()
        .expect("test CLI should run");
    assert!(
        execution.status.success(),
        "execution failed: {}",
        String::from_utf8_lossy(&execution.stderr)
    );
    let exec_report: serde_json::Value = serde_json::from_slice(&execution.stdout).unwrap();
    let results = exec_report["results"].as_array().unwrap();
    assert!(!results.is_empty(), "expected executed mutants for r#match");
    let unviable: Vec<_> = results
        .iter()
        .filter(|r| r["outcome"] == "UNVIABLE")
        .collect();
    assert!(
        unviable.is_empty(),
        "raw-identifier mutants should all compile cleanly, got UNVIABLE: {unviable:#?}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn typed_events_and_lifecycle_recognize_raw_identifiers() {
    let root = unique_project("soro-raw-ident-events");
    fs::write(
        root.join("src/lib.rs"),
        r#"
#[contractevent]
pub struct r#Type {
    #[topic]
    pub r#match: Address,
    pub r#in: i128,
}

pub fn r#initialize(r#for: Env, r#type: Address, r#else: Address, r#in: i128) {
    r#type.require_auth();
    r#Type { r#match: r#type, r#in }.publish(&r#for);
}

pub fn shorthand(r#for: Env, r#match: Address, r#else: Address, r#in: i128) {
    r#Type { r#match, r#in }.publish(&r#for);
}
"#,
    )
    .unwrap();

    let report = list(&root, &[]);
    let mutants = report["mutants"].as_array().unwrap();

    assert!(mutants.iter().any(|m| m["operator"] == "AUTH-006"));
    assert!(mutants.iter().any(|m| m["operator"] == "EVENT-002"));
    assert!(mutants
        .iter()
        .any(|m| m["operator"] == "EVENT-003" && m["replacement"] == "r#else"));
    assert!(mutants
        .iter()
        .any(|m| m["operator"] == "EVENT-003" && m["replacement"] == "r#match: r#else"));
    assert!(mutants
        .iter()
        .any(|m| m["operator"] == "EVENT-004" && m["replacement"] == "r#in: 0"));

    let event_005 = list(&root, &["--operator", "EVENT-005"]);
    let event_005_mutants = event_005["mutants"].as_array().unwrap();
    assert!(event_005_mutants
        .iter()
        .any(|m| m["operator"] == "EVENT-005" && m["replacement"] == "r#else"));
    assert!(event_005_mutants
        .iter()
        .any(|m| m["operator"] == "EVENT-005" && m["replacement"] == "r#match: r#else"));

    let summary = cargo_soro_mutants::parser::parse_and_discover_source(
        "fn r#match(r#type: Address) { r#type.r#require_auth(); }",
        std::path::Path::new("src/lib.rs"),
    )
    .expect("parser should handle raw identifiers");
    assert!(summary.has_soroban_patterns);
    assert_eq!(summary.function_count, 1);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn function_filtering_and_disambiguation_support_raw_and_unraw_names() {
    let root = unique_project("soro-raw-ident-filter");
    fs::write(
        root.join("src/lib.rs"),
        r#"
mod r#mod {
    pub fn r#type(r#match: Address) {
        r#match.require_auth();
    }
}

pub struct r#Alpha;
pub struct Beta;

impl r#Alpha {
    pub fn r#match(user: Address) {
        user.require_auth();
    }
}

impl Beta {
    pub fn match_alias(user: Address) {
        let _ = user;
    }

    pub fn r#match(user: Address) {
        user.require_auth();
    }

    pub fn process(user: Address) {
        user.require_auth();
    }
}

impl r#Alpha {
    pub fn r#process(user: Address) {
        user.require_auth();
    }
}
"#,
    )
    .unwrap();

    let all = list(&root, &[]);
    let functions: Vec<_> = all["mutants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["function"].as_str().unwrap())
        .collect();
    assert!(functions.contains(&"r#mod::r#type"));
    assert!(functions.contains(&"r#Alpha::r#match"));
    assert!(functions.contains(&"Beta::r#match"));
    assert!(functions.contains(&"Beta::process"));
    assert!(functions.contains(&"r#Alpha::r#process"));

    for query in [
        "type",
        "r#type",
        "mod::type",
        "r#mod::r#type",
        "mod::r#type",
    ] {
        let filtered = list(&root, &["--function", query]);
        let items = filtered["mutants"].as_array().unwrap();
        assert_eq!(items.len(), 1, "expected 1 mutant for --function {query}");
        assert_eq!(items[0]["function"], "r#mod::r#type");
    }

    for query in ["match", "r#match"] {
        let filtered = list(&root, &["--function", query]);
        let items = filtered["mutants"].as_array().unwrap();
        assert_eq!(items.len(), 2, "expected 2 mutants for --function {query}");
    }

    for query in ["Alpha::match", "r#Alpha::r#match"] {
        let filtered = list(&root, &["--function", query]);
        let items = filtered["mutants"].as_array().unwrap();
        assert_eq!(items.len(), 1, "expected 1 mutant for --function {query}");
        assert_eq!(items[0]["function"], "r#Alpha::r#match");
    }

    for query in ["process", "r#process"] {
        let filtered = list(&root, &["--function", query]);
        let items = filtered["mutants"].as_array().unwrap();
        assert_eq!(items.len(), 2, "expected 2 mutants for --function {query}");
    }

    let _ = fs::remove_dir_all(&root);
}
