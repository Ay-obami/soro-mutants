use cargo_soro_mutants::extension::v1::{
    discover, MutationCandidate, OperatorContext, SemanticOperator,
};

struct FlipTrue;

impl SemanticOperator for FlipTrue {
    fn id(&self) -> &'static str {
        "CUSTOM-001"
    }

    fn discover(
        &self,
        context: &OperatorContext<'_>,
        _: &syn::File,
    ) -> Result<Vec<MutationCandidate>, String> {
        let Some(start) = context.source().find("true") else {
            return Ok(vec![]);
        };
        Ok(vec![MutationCandidate {
            start_byte: start,
            end_byte: start + 4,
            replacement: "false".into(),
            description: "example boolean mutation".into(),
        }])
    }
}

fn main() {
    let candidates = discover(&FlipTrue, "fn enabled() -> bool { true }", "src/lib.rs").unwrap();
    println!("{candidates:?}");
}
