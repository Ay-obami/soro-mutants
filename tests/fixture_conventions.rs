use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[test]
fn fixture_directories_follow_naming_and_layout_conventions() {
    let fixtures_dir = Path::new("fixtures");
    assert!(fixtures_dir.exists(), "fixtures directory must exist");

    let entries = fs::read_dir(fixtures_dir).expect("read fixtures dir");
    let mut weak_fixtures = BTreeSet::new();
    let mut strong_fixtures = BTreeSet::new();

    for entry in entries {
        let entry = entry.expect("valid entry");
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();

        let valid_prefix = name.starts_with("weak-")
            || name.starts_with("strong-")
            || name.starts_with("equivalence-")
            || name.starts_with("compile-only")
            || name.starts_with("workspace-");

        assert!(
            valid_prefix,
            "fixture directory '{name}' does not follow naming convention: must start with weak-, strong-, equivalence-, compile-only, or workspace-"
        );

        if let Some(suffix) = name.strip_prefix("weak-") {
            weak_fixtures.insert(suffix.to_string());
        } else if let Some(suffix) = name.strip_prefix("strong-") {
            strong_fixtures.insert(suffix.to_string());
        }

        if name != "compile-only" {
            let cargo_toml = path.join("Cargo.toml");
            let src_lib = path.join("src").join("lib.rs");
            if cargo_toml.exists() && !name.starts_with("workspace-") {
                assert!(
                    src_lib.exists(),
                    "fixture '{name}' has Cargo.toml but missing src/lib.rs"
                );
            }
            if name.starts_with("workspace-") {
                let manifest = fs::read_to_string(&cargo_toml)
                    .expect("workspace fixture must have a readable Cargo.toml");
                assert!(
                    manifest.contains("[workspace]") && manifest.contains("members"),
                    "workspace fixture '{name}' must declare workspace members"
                );
            }
        }
    }

    for weak in &weak_fixtures {
        assert!(
            strong_fixtures.contains(weak),
            "weak fixture 'weak-{weak}' is missing paired 'strong-{weak}'"
        );
    }
    for strong in &strong_fixtures {
        assert!(
            weak_fixtures.contains(strong),
            "strong fixture 'strong-{strong}' is missing paired 'weak-{strong}'"
        );
    }
}
