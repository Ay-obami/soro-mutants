use std::{path::PathBuf, process::Command};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/weak-auth")
}

#[test]
fn invalid_diff_base_is_an_error_not_a_panic() {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))
        .args(["list"])
        .arg(fixture())
        .arg("--diff-base=-bad")
        .output()
        .expect("CLI should launch");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid Git diff base revision"),
        "{stderr}"
    );
    assert!(!stderr.contains("panicked at"), "{stderr}");
}

#[test]
fn production_code_has_no_direct_unwrap_or_expect_calls() {
    use syn::visit::Visit;

    struct PanicCallVisitor {
        calls: Vec<String>,
    }

    impl<'ast> Visit<'ast> for PanicCallVisitor {
        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            let method = node.method.to_string();
            if method == "unwrap" || method == "expect" {
                self.calls.push(method);
            }
            syn::visit::visit_expr_method_call(self, node);
        }
    }

    let source = include_str!("../src/main.rs");
    let production = source.split("#[cfg(test)]").next().unwrap_or(source);
    let syntax = syn::parse_file(production).expect("production source parses");
    let mut visitor = PanicCallVisitor { calls: Vec::new() };
    visitor.visit_file(&syntax);
    assert!(
        visitor.calls.is_empty(),
        "production unwrap/expect calls found: {:?}",
        visitor.calls
    );
}