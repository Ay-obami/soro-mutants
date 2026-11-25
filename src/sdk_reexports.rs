//! Conservative, syntax-only recognition of explicitly re-exported Soroban SDK types.
//! This is not a general Rust name resolver: only pub use paths in this parsed
//! file and its inline modules are eligible, never globs or external modules.

use std::collections::BTreeMap;
use syn::{
    visit_mut::{self, VisitMut},
    Expr, Item, UseTree, Visibility,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SdkType {
    Address,
    Env,
    TokenClient,
}

impl SdkType {
    fn canonical_name(self) -> &'static str {
        match self {
            Self::Address => "Address",
            Self::Env => "Env",
            Self::TokenClient => "TokenClient",
        }
    }
}

type Reexports = BTreeMap<Vec<String>, BTreeMap<String, SdkType>>;

/// Only explicit public re-exports from the Soroban SDK are accepted.
/// Renames are aliases for the source item, not new semantic types.
fn sdk_type(path: &[String]) -> Option<SdkType> {
    let segments = path.iter().map(String::as_str).collect::<Vec<_>>();
    match segments.as_slice() {
        ["soroban_sdk", "Address"] => Some(SdkType::Address),
        ["soroban_sdk", "Env"] => Some(SdkType::Env),
        ["soroban_sdk", "token", "Client"] => Some(SdkType::TokenClient),
        _ => None,
    }
}

fn collect_use(tree: &UseTree, path: &mut Vec<String>, output: &mut BTreeMap<String, SdkType>) {
    match tree {
        UseTree::Path(node) => {
            path.push(node.ident.to_string());
            collect_use(&node.tree, path, output);
            path.pop();
        }
        UseTree::Name(node) => {
            path.push(node.ident.to_string());
            if let Some(kind) = sdk_type(path) {
                output.insert(node.ident.to_string(), kind);
            }
            path.pop();
        }
        UseTree::Rename(node) => {
            path.push(node.ident.to_string());
            if let Some(kind) = sdk_type(path) {
                output.insert(node.rename.to_string(), kind);
            }
            path.pop();
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use(item, path, output);
            }
        }
        // Globs require name resolution and cannot be proven locally.
        UseTree::Glob(_) => {}
    }
}

fn collect_reexports(items: &[Item], module: &mut Vec<String>, output: &mut Reexports) {
    let mut local = BTreeMap::new();
    for item in items {
        if let Item::Use(import) = item {
            if matches!(import.vis, Visibility::Public(_)) && import.leading_colon.is_none() {
                collect_use(&import.tree, &mut Vec::new(), &mut local);
            }
        }
    }
    output.insert(module.clone(), local);
    for item in items {
        if let Item::Mod(module_item) = item {
            if let Some((_, children)) = &module_item.content {
                module.push(module_item.ident.to_string());
                collect_reexports(children, module, output);
                module.pop();
            }
        }
    }
}

struct Normalizer {
    reexports: Reexports,
    scope: Vec<String>,
}

impl Normalizer {
    fn resolve(&self, path: &syn::Path) -> Option<SdkType> {
        if path.leading_colon.is_some()
            || path.segments.iter().any(|segment| !segment.arguments.is_none())
        {
            return None;
        }
        let mut parts = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let alias = parts.pop()?;
        let mut module = if parts.first().is_some_and(|part| part == "crate") {
            parts.remove(0);
            Vec::new()
        } else if parts.first().is_some_and(|part| part == "self") {
            parts.remove(0);
            self.scope.clone()
        } else if parts.first().is_some_and(|part| part == "super") {
            parts.remove(0);
            let mut parent = self.scope.clone();
            parent.pop()?;
            parent
        } else {
            self.scope.clone()
        };
        module.extend(parts);
        self.reexports.get(&module)?.get(&alias).copied()
    }
}

impl VisitMut for Normalizer {
    fn visit_item_mod_mut(&mut self, item: &mut syn::ItemMod) {
        if item.content.is_none() {
            // A mod declaration alone proves nothing about an external file.
            return;
        }
        self.scope.push(item.ident.to_string());
        visit_mut::visit_item_mod_mut(self, item);
        self.scope.pop();
    }

    fn visit_type_path_mut(&mut self, node: &mut syn::TypePath) {
        if node.qself.is_none() {
            if let Some(kind) = self.resolve(&node.path) {
                if let Some(last) = node.path.segments.last_mut() {
                    last.ident = syn::Ident::new(kind.canonical_name(), last.ident.span());
                }
            }
        }
        visit_mut::visit_type_path_mut(self, node);
    }

    fn visit_expr_call_mut(&mut self, call: &mut syn::ExprCall) {
        if let Expr::Path(path) = call.func.as_mut() {
            let len = path.path.segments.len();
            if len >= 2
                && path.path.segments[len - 1].ident == "new"
                && path.qself.is_none()
            {
                let mut prefix = path.path.clone();
                prefix.segments.pop();
                if self.resolve(&prefix) == Some(SdkType::TokenClient) {
                    let segment = &mut path.path.segments[len - 2];
                    segment.ident = syn::Ident::new("TokenClient", segment.ident.span());
                }
            }
        }
        visit_mut::visit_expr_call_mut(self, call);
    }
}

/// Normalize known *syntactic* re-exports for the existing, conservative
/// recognizers. Original source and proc-macro spans remain untouched.
pub fn normalize(file: &mut syn::File) {
    let mut reexports = BTreeMap::new();
    collect_reexports(&file.items, &mut Vec::new(), &mut reexports);
    let mut normalizer = Normalizer {
        reexports,
        scope: Vec::new(),
    };
    normalizer.visit_file_mut(file);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature_types(source: &str) -> Vec<String> {
        let mut file = syn::parse_file(source).unwrap();
        normalize(&mut file);
        fn gather(items: &[Item], found: &mut Vec<String>) {
            for item in items {
                match item {
                    Item::Fn(f) => {
                        for arg in &f.sig.inputs {
                            if let syn::FnArg::Typed(typed) = arg {
                                if let syn::Type::Path(p) = typed.ty.as_ref() {
                                    found.push(
                                        p.path.segments.last().unwrap().ident.to_string(),
                                    );
                                }
                            }
                        }
                    }
                    Item::Mod(m) => {
                        if let Some((_, items)) = &m.content {
                            gather(items, found);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut found = Vec::new();
        gather(&file.items, &mut found);
        found
    }

    #[test]
    fn accepts_direct_and_renamed_public_reexports() {
        let types = signature_types(
            "pub use soroban_sdk::{Address, Env as HostEnv};\n\
             pub fn guard(env: HostEnv, account: Address) { account.require_auth(); }",
        );
        assert_eq!(types, ["Env", "Address"]);
    }

    #[test]
    fn traces_qualified_inline_module_reexports_only() {
        let types = signature_types(
            "mod exports { pub use soroban_sdk::Address as Identity; }\n\
             fn known(user: crate::exports::Identity) {}\n\
             fn unknown(user: Identity) {}",
        );
        assert_eq!(types, ["Address", "Identity"]);
    }

    #[test]
    fn excludes_unrelated_imports_and_glob_resolution() {
        let types = signature_types(
            "pub use unrelated::Address as Unrelated;\n\
             pub use soroban_sdk::*;\n\
             fn guarded(user: Unrelated, env: AlienEnv) {}",
        );
        assert_eq!(types, ["Unrelated", "AlienEnv"]);
    }

    #[test]
    fn does_not_leak_aliases_across_sibling_modules() {
        let types = signature_types(
            "mod a { pub use soroban_sdk::Address as Identity;\n\
                      fn yes(user: Identity) {} }\n\
             mod b { fn no(user: Identity) {} }",
        );
        assert_eq!(types, ["Address", "Identity"]);
    }

    #[test]
    fn normalizes_token_client_constructor_without_touching_source_text() {
        let source = "pub use soroban_sdk::token::Client as TransferClient;\n\
                      fn x(env: &Env, address: &Address) {\n\
                          let client = TransferClient::new(env, address);\n\
                      }";
        let mut file = syn::parse_file(source).unwrap();
        normalize(&mut file);
        let Item::Fn(function) = &file.items[1] else { panic!("expected function") };
        let syn::Stmt::Local(local) = &function.block.stmts[0] else { panic!("expected let") };
        let Expr::Call(call) = local.init.as_ref().unwrap().expr.as_ref() else {
            panic!("expected constructor")
        };
        let Expr::Path(path) = call.func.as_ref() else { panic!("expected path") };
        assert_eq!(path.path.segments[0].ident, "TokenClient");
        assert!(source.contains("TransferClient::new"));
    }
}
