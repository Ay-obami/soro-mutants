use std::fs;
use std::path::Path;

const KNOWN_OPERATOR_IDS: &[&str] = &[
    "AUTH-001",
    "AUTH-002",
    "AUTH-003",
    "AUTH-004",
    "AUTH-005",
    "AUTH-006",
    "AUTH-007",
    "CALL-001",
    "CALL-002",
    "CALL-003",
    "EVENT-001",
    "EVENT-002",
    "EVENT-003",
    "EVENT-004",
    "EVENT-005",
    "STORAGE-001",
    "TOKEN-001",
    "TOKEN-002",
    "TOKEN-003",
    "TOKEN-004",
    "TTL-001",
    "TTL-002",
    "TTL-003",
];

#[test]
fn fixture_coverage_matrix_covers_all_known_operators() {
    let matrix_path = Path::new("docs/fixture-coverage-matrix.md");
    assert!(
        matrix_path.exists(),
        "docs/fixture-coverage-matrix.md must exist"
    );

    let content = fs::read_to_string(matrix_path).expect("read fixture coverage matrix");

    for id in KNOWN_OPERATOR_IDS {
        assert!(
            content.contains(id),
            "docs/fixture-coverage-matrix.md must document operator {id}"
        );
    }
}

#[test]
fn referenced_fixtures_in_matrix_exist_on_filesystem() {
    let matrix_path = Path::new("docs/fixture-coverage-matrix.md");
    let content = fs::read_to_string(matrix_path).expect("read fixture coverage matrix");

    for word in content.split(&[' ', '`', '|', '\n', '\t'][..]) {
        let trimmed = word.trim();
        if trimmed.starts_with("fixtures/") {
            let path = Path::new(trimmed);
            assert!(
                path.exists(),
                "fixture path '{trimmed}' referenced in matrix must exist on filesystem"
            );
        }
    }
}
