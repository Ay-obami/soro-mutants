//! Versioned extension boundary for custom semantic operator prototypes.

pub mod command;
pub mod filters;
pub mod parser;
pub mod report;

pub mod extension {
    pub mod v1 {
        use std::path::{Path, PathBuf};

        pub struct OperatorContext<'a> {
            source: &'a str,
            file: &'a Path,
        }

        impl<'a> OperatorContext<'a> {
            pub fn source(&self) -> &'a str {
                self.source
            }

            pub fn file(&self) -> &'a Path {
                self.file
            }
        }

        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct MutationCandidate {
            pub start_byte: usize,
            pub end_byte: usize,
            pub replacement: String,
            pub description: String,
        }

        pub trait SemanticOperator {
            fn id(&self) -> &'static str;

            fn discover(
                &self,
                context: &OperatorContext<'_>,
                syntax: &syn::File,
            ) -> Result<Vec<MutationCandidate>, String>;
        }

        pub fn discover(
            operator: &dyn SemanticOperator,
            source: &str,
            file: impl Into<PathBuf>,
        ) -> Result<Vec<MutationCandidate>, String> {
            let id = operator.id();
            if id.is_empty()
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
            {
                return Err("extension operator ID must contain only A-Z, 0-9, and '-'".into());
            }

            let syntax = syn::parse_file(source)
                .map_err(|error| format!("cannot parse source for extension {id}: {error}"))?;
            let file = file.into();
            let context = OperatorContext {
                source,
                file: &file,
            };
            let candidates = operator.discover(&context, &syntax)?;

            for candidate in &candidates {
                if candidate.start_byte >= candidate.end_byte || candidate.end_byte > source.len() {
                    return Err(format!("extension {id} returned an invalid source range"));
                }
                if !source.is_char_boundary(candidate.start_byte)
                    || !source.is_char_boundary(candidate.end_byte)
                {
                    return Err(format!(
                        "extension {id} returned a non-UTF-8-boundary source range"
                    ));
                }
            }

            Ok(candidates)
        }
    }
}
