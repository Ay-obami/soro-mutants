use cargo_soro_mutants::extension::v1::{
    discover, MutationCandidate, OperatorContext, SemanticOperator,
};

struct Example;

impl SemanticOperator for Example {
    fn id(&self) -> &'static str {
        "CUSTOM-001"
    }

    fn discover(
        &self,
        context: &OperatorContext<'_>,
        _: &syn::File,
    ) -> Result<Vec<MutationCandidate>, String> {
        let start = context
            .source()
            .find("true")
            .ok_or("fixture missing true")?;
        Ok(vec![MutationCandidate {
            start_byte: start,
            end_byte: start + 4,
            replacement: "false".into(),
            description: format!("flip boolean in {}", context.file().display()),
        }])
    }
}

struct Failing;

impl SemanticOperator for Failing {
    fn id(&self) -> &'static str {
        "CUSTOM-ERR"
    }

    fn discover(
        &self,
        _: &OperatorContext<'_>,
        _: &syn::File,
    ) -> Result<Vec<MutationCandidate>, String> {
        Err("operator failed intentionally".into())
    }
}

#[test]
fn validates_external_candidate() {
    let source = "fn enabled() -> bool { true }";
    let candidates = discover(&Example, source, "src/lib.rs").unwrap();
    assert_eq!(
        &source[candidates[0].start_byte..candidates[0].end_byte],
        "true"
    );
    assert_eq!(candidates[0].replacement, "false");
}

#[test]
fn extension_failure_cannot_modify_baseline_source() {
    let source = String::from("fn baseline() {}");
    let before = source.clone();
    assert!(discover(&Failing, &source, "src/lib.rs").is_err());
    assert_eq!(source, before);
}
