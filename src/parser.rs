//! Robust parser and syntax validation entry points for Rust source inputs.

use std::path::{Path, PathBuf};
use syn::visit::Visit;

/// Maximum allowable syntax delimiter nesting depth to prevent stack overflow on pathological inputs.
pub const MAX_RUST_SYNTAX_NESTING: usize = 256;

/// Structured summary of safe source parsing and AST traversal.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ParsedSourceSummary {
    pub item_count: usize,
    pub function_count: usize,
    pub has_soroban_patterns: bool,
}

/// Visitor that counts AST nodes and checks Soroban patterns without panic.
#[derive(Default)]
struct SafeAstVisitor {
    function_count: usize,
    has_soroban_patterns: bool,
}

impl<'ast> Visit<'ast> for SafeAstVisitor {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.function_count += 1;
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.function_count += 1;
        syn::visit::visit_impl_item_fn(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let raw_name = node.method.to_string();
        let name = raw_name
            .strip_prefix("r#")
            .filter(|rest| !rest.is_empty())
            .unwrap_or(&raw_name);
        if matches!(
            name,
            "require_auth" | "extend_ttl" | "publish" | "transfer" | "transfer_from"
        ) {
            self.has_soroban_patterns = true;
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

/// Validates delimiter nesting to reject pathological deeply nested structures before full AST parsing.
pub fn validate_nesting(source: &str) -> Result<(), String> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        if line_comment {
            if bytes[i] == b'\n' {
                line_comment = false;
            }
            i += 1;
            continue;
        }

        if block_comment > 0 {
            if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                block_comment -= 1;
                i += 2;
                continue;
            }
            if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                block_comment += 1;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if bytes[i] == b'\\' {
                escaped = true;
            } else if bytes[i] == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        match bytes[i] {
            b'"' => in_string = true,
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                line_comment = true;
                i += 1;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                block_comment = 1;
                i += 1;
            }
            b'{' | b'(' | b'[' => {
                depth += 1;
                if depth > MAX_RUST_SYNTAX_NESTING {
                    return Err(format!(
                        "source exceeds maximum syntax nesting depth of {MAX_RUST_SYNTAX_NESTING}"
                    ));
                }
            }
            b'}' | b')' | b']' => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        i += 1;
    }

    Ok(())
}

/// Safely validates, parses, and traverses arbitrary Rust source input.
/// Returns structured diagnostics instead of panicking on malformed or pathological syntax.
pub fn parse_and_discover_source(
    source: &str,
    _file: &Path,
) -> Result<ParsedSourceSummary, String> {
    validate_nesting(source)?;

    let syntax = match syn::parse_file(source) {
        Ok(syntax) => syntax,
        Err(err) => return Err(format!("syn parse failure: {err}")),
    };

    let mut visitor = SafeAstVisitor::default();
    visitor.visit_file(&syntax);

    Ok(ParsedSourceSummary {
        item_count: syntax.items.len(),
        function_count: visitor.function_count,
        has_soroban_patterns: visitor.has_soroban_patterns,
    })
}

/// Fuzzing harness helper exercising parser validation and discovery entry points panic-free.
pub fn fuzz_parse_source(data: &str) {
    let dummy_path = PathBuf::from("src/fuzz_input.rs");
    let _ = parse_and_discover_source(data, &dummy_path);
}
