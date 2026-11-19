use anyhow::{bail, Context, Result};
use clap::{ColorChoice, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use proc_macro2::Span;
use serde::{Deserialize, Serialize};
use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use syn::{
    spanned::Spanned,
    visit::{self, Visit},
    Expr, ExprMethodCall, FnArg, ImplItemFn, Item, ItemFn, Local, Pat, Signature, Stmt, Type,
};
use wait_timeout::ChildExt;
use walkdir::{DirEntry, WalkDir};

#[derive(Parser, Debug)]
#[command(name = "cargo-soro-mutants")]
#[command(version)]
#[command(about = "Soroban-aware semantic mutation testing for Stellar smart contracts")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// All built-in mutation operators, sorted by ID.
///
/// Each entry is `(id, one-line description)`. The slice is kept in ascending
/// lexicographic order so that `cargo soro-mutants operators` always emits a
/// deterministic list without any runtime sorting.
const BUILTIN_OPERATORS: &[(&str, &str)] = &[
    ("AUTH-001", "remove a require_auth() call"),
    (
        "AUTH-002",
        "replace require_auth() receiver with a different Address parameter",
    ),
    (
        "AUTH-003",
        "replace require_auth_for_args(...) argument set with an empty Soroban Vec",
    ),
    (
        "AUTH-004",
        "skip or redirect authorization through a simple local require_auth() helper",
    ),
    (
        "AUTH-005",
        "replace a stored admin/owner authority Address with another Address parameter",
    ),
    ("CALL-001", "replace direct Env::invoke_contract dependency address with another typed Address parameter"),
    ("CALL-002", "replace a generated contract-client call argument with another compatible typed parameter"),
    ("CALL-003", "remove a supported generated contract-client return mismatch check"),
    (
        "EVENT-001",
        "remove a Soroban event publication (env.events().publish(...))",
    ),
    (
        "EVENT-002",
        "remove a standalone #[contractevent] typed event publication",
    ),
    (
        "EVENT-003",
        "replace an Address field in a typed Soroban event with another Address parameter",
    ),
    (
        "EVENT-004",
        "replace a numeric field in a typed Soroban event with zero",
    ),
    (
        "EVENT-005",
        "replace a typed Soroban event topic Address with another Address parameter",
    ),
    (
        "STORAGE-001",
        "replace a direct Soroban storage class accessor with another storage class",
    ),
    (
        "TOKEN-001",
        "swap the sender and recipient arguments of a token transfer",
    ),
    (
        "TOKEN-002",
        "swap the owner and spender Address arguments of a token allowance call",
    ),
    (
        "TOKEN-003",
        "replace a typed token transfer amount with zero or one",
    ),
    (
        "TOKEN-004",
        "replace a recognized token client receiver with another in-scope token client",
    ),
    (
        "TTL-001",
        "remove an extend_ttl() call on a Soroban storage accessor",
    ),
    (
        "TTL-002",
        "renew storage TTL even when remaining lifetime exceeds threshold",
    ),
    (
        "TTL-003",
        "shorten storage TTL extension target to its threshold",
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Runner {
    CargoTest,
    Nextest,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Discover semantic mutants without executing them.
    List {
        /// Cargo project or workspace to inspect.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Only list mutants produced by this operator ID, such as AUTH-001.
        #[arg(long, value_parser = parse_operator)]
        operator: Option<String>,
        /// Exclude a project-relative file or directory glob (repeatable).
        #[arg(long, value_name = "GLOB", value_parser = parse_exclude_glob)]
        exclude_path: Vec<globset::Glob>,
        /// Select shard INDEX/TOTAL (one-based), after discovery filters.
        #[arg(long, value_name = "INDEX/TOTAL")]
        shard: Option<Shard>,
        /// Only list mutants in source files ending with this path.
        #[arg(long)]
        file: Option<String>,
        /// Only list mutants inside this function.
        #[arg(long)]
        function: Option<String>,
        /// Restrict selection to these project-relative paths (repeatable).
        #[arg(long, value_name = "PATH", conflicts_with = "diff_base")]
        changed_file: Vec<PathBuf>,
        /// Restrict selection to files changed against this Git revision.
        #[arg(long, value_name = "REV", conflicts_with = "changed_file")]
        diff_base: Option<String>,
        /// With --diff-base, select only functions intersecting changed lines.
        #[arg(long, requires = "diff_base")]
        changed_functions: bool,
        /// Emit machine-readable JSON on stdout.
        #[arg(long)]
        json: bool,
        /// Print only the number of matching semantic mutants.
        #[arg(long, conflicts_with = "json")]
        count_only: bool,
        /// Write the selected text, count, or JSON report to a file instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Execute semantic mutants against a Cargo test suite.
    Test {
        /// Cargo project or workspace to mutate.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Only execute mutants produced by this operator ID, such as AUTH-001.
        #[arg(long, value_parser = parse_operator)]
        operator: Option<String>,
        /// Exclude a project-relative file or directory glob (repeatable).
        #[arg(long, value_name = "GLOB", value_parser = parse_exclude_glob)]
        exclude_path: Vec<globset::Glob>,
        /// Select shard INDEX/TOTAL (one-based), after discovery filters.
        #[arg(long, value_name = "INDEX/TOTAL")]
        shard: Option<Shard>,
        /// Only execute mutants in source files ending with this path.
        #[arg(long)]
        file: Option<String>,
        /// Only execute mutants inside this function.
        #[arg(long)]
        function: Option<String>,
        /// Restrict selection to these project-relative paths (repeatable).
        #[arg(long, value_name = "PATH", conflicts_with = "diff_base")]
        changed_file: Vec<PathBuf>,
        /// Restrict selection to files changed against this Git revision.
        #[arg(long, value_name = "REV", conflicts_with = "changed_file")]
        diff_base: Option<String>,
        /// With --diff-base, select only functions intersecting changed lines.
        #[arg(long, requires = "diff_base")]
        changed_functions: bool,
        /// Run the configured test command from this directory relative to PATH.
        #[arg(long)]
        test_dir: Option<PathBuf>,
        /// Store Soro Mutants baseline and mutant Cargo caches under this directory.
        #[arg(long)]
        target_dir: Option<PathBuf>,
        /// Command used for the clean baseline and each viable mutant.
        #[arg(long, default_value = "cargo test -q")]
        test_command: String,
        /// Run tests using cargo test (default) or an installed cargo-nextest.
        #[arg(long, value_enum, default_value_t = Runner::CargoTest)]
        runner: Runner,
        /// Legacy timeout in seconds used for both phases unless overridden.
        #[arg(long, default_value_t = 120)]
        timeout: u64,
        /// Timeout in seconds for each mutant's separate compile gate.
        #[arg(long)]
        compile_timeout: Option<u64>,
        /// Timeout in seconds for baseline and mutant test commands.
        #[arg(long)]
        test_timeout: Option<u64>,
        /// Number of mutants to execute concurrently (1 keeps serial execution).
        /// Each worker uses its own scratch worktree and Cargo target directory.
        #[arg(short = 'j', long, value_name = "N", default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..))]
        jobs: u16,
        /// Emit machine-readable JSON on stdout.
        #[arg(long)]
        json: bool,
        /// Emit a Markdown report suitable for issues and pull requests.
        #[arg(long, conflicts_with_all = ["json", "html"])]
        markdown: bool,
        /// Emit a standalone, self-contained HTML report.
        #[arg(long, conflicts_with = "json")]
        html: bool,
        /// Emit SARIF 2.1.0 code-scanning results for surviving mutants.
        #[arg(long, conflicts_with_all = ["json", "markdown", "html"])]
        sarif: bool,
        /// Emit GitHub Actions warning annotations for surviving mutants on stderr.
        #[arg(long)]
        github_annotations: bool,
        /// Append a Markdown report to GITHUB_STEP_SUMMARY when set.
        #[arg(long)]
        github_summary: bool,
        /// Fail with exit code 4 when the semantic score is below PERCENT (0–100).
        #[arg(long, value_name = "PERCENT", value_parser = parse_min_score)]
        min_score: Option<f64>,
        /// Write the selected text, JSON, or Markdown report to a file.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Remove Soro Mutants scratch and build-cache directories.
    Clean {
        /// Cargo project or workspace whose generated state should be removed.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Clean the same custom cache directory previously passed to --target-dir.
        #[arg(long)]
        target_dir: Option<PathBuf>,
        /// List managed paths without removing any files.
        #[arg(long)]
        dry_run: bool,
    },
    /// Validate a versioned JSON mutation-corpus manifest.
    ValidateCorpus {
        /// Path to the JSON corpus manifest.
        path: PathBuf,
    },
    /// Print all built-in operator IDs and their descriptions.
    Operators,
    /// Generate shell completion scripts on stdout.
    Completions {
        /// Target shell (bash, zsh, or fish).
        #[arg(value_enum)]
        shell: CompletionShell,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

impl From<CompletionShell> for Shell {
    fn from(shell: CompletionShell) -> Self {
        match shell {
            CompletionShell::Bash => Shell::Bash,
            CompletionShell::Zsh => Shell::Zsh,
            CompletionShell::Fish => Shell::Fish,
        }
    }
}

fn parse_operator(value: &str) -> std::result::Result<String, String> {
    if BUILTIN_OPERATORS.iter().any(|(id, _)| *id == value) {
        Ok(value.to_owned())
    } else {
        let valid_ids = BUILTIN_OPERATORS
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
            .join(", ");
        Err(format!(
            "unknown operator ID {value:?}; valid IDs: {valid_ids}"
        ))
    }
}

fn parse_exclude_glob(value: &str) -> std::result::Result<globset::Glob, String> {
    GlobBuilder::new(value)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map_err(|error| format!("invalid exclude-path glob {value:?}: {error}"))
}

#[derive(Clone, Copy, Debug)]
struct Shard {
    index: usize,
    total: usize,
}

impl std::str::FromStr for Shard {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let invalid =
            || "expected INDEX/TOTAL with 1 <= INDEX <= TOTAL (positive integers)".to_owned();
        let (index, total) = value.split_once('/').ok_or_else(invalid)?;
        if index.is_empty()
            || total.is_empty()
            || !index.bytes().all(|b| b.is_ascii_digit())
            || !total.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid());
        }
        let index = index.parse::<usize>().map_err(|_| invalid())?;
        let total = total.parse::<usize>().map_err(|_| invalid())?;
        if index == 0 || total == 0 || index > total {
            return Err(invalid());
        }
        Ok(Self { index, total })
    }
}

fn select_shard(mutants: Vec<Mutant>, shard: Option<Shard>) -> Vec<Mutant> {
    let Some(shard) = shard else { return mutants };
    mutants
        .into_iter()
        .enumerate()
        .filter(|(index, _)| index % shard.total == shard.index - 1)
        .map(|(_, mutant)| mutant)
        .collect()
}

fn exclusion_set(patterns: Vec<globset::Glob>) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(pattern);
    }
    builder.build().context("cannot build exclude-path matcher")
}

#[derive(Clone, Debug, Serialize)]
struct SourceSpan {
    line: usize,
    column: usize,
    end_line: usize,
    end_column: usize,
}

#[derive(Clone, Debug, Serialize)]
struct Mutant {
    id: String,
    operator: String,
    file: PathBuf,
    function: Option<String>,
    span: SourceSpan,
    original: String,
    replacement: String,
    description: String,
    #[serde(skip)]
    start_byte: usize,
    #[serde(skip)]
    end_byte: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Outcome {
    Killed,
    Survived,
    Unviable,
    Timeout,
}

#[derive(Clone, Debug, Serialize)]
struct MutantResult {
    mutant: Mutant,
    outcome: Outcome,
}

const JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct ListReport<'a> {
    schema_version: u32,
    mutants: &'a [Mutant],
    #[serde(skip_serializing_if = "Option::is_none")]
    suppression: Option<SuppressionSummary<'a>>,
}

#[derive(Serialize)]
struct SuppressionSummary<'a> {
    count: usize,
    details: &'a [SuppressedMutant],
}

#[derive(Serialize)]
struct TestReport<'a> {
    schema_version: u32,
    results: &'a [MutantResult],
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<&'a RunMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    suppression: Option<SuppressionSummary<'a>>,
}

#[derive(Debug, Serialize)]
struct RunMetadata {
    rustc_version: Option<String>,
    cargo_version: Option<String>,
    test_command: String,
    compile_timeout_seconds: u64,
    test_timeout_seconds: u64,
}

impl RunMetadata {
    fn capture(test_command: &str, timeouts: Timeouts) -> Self {
        Self {
            rustc_version: tool_version("rustc"),
            cargo_version: tool_version("cargo"),
            test_command: test_command.to_owned(),
            compile_timeout_seconds: timeouts.compile,
            test_timeout_seconds: timeouts.test,
        }
    }
}

fn tool_version(binary: &str) -> Option<String> {
    let output = Command::new(binary).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8(output.stdout).ok()?;
    let version = version.trim();
    (!version.is_empty()).then(|| version.to_owned())
}

fn list_report(mutants: &[Mutant]) -> ListReport<'_> {
    ListReport {
        schema_version: JSON_SCHEMA_VERSION,
        mutants,
        suppression: None,
    }
}

fn test_report(results: &[MutantResult]) -> TestReport<'_> {
    TestReport {
        schema_version: JSON_SCHEMA_VERSION,
        results,
        metadata: None,
        suppression: None,
    }
}

fn test_report_with_metadata<'a>(
    results: &'a [MutantResult],
    metadata: &'a RunMetadata,
) -> TestReport<'a> {
    TestReport {
        metadata: Some(metadata),
        ..test_report(results)
    }
}

#[derive(Clone, Copy, Debug)]
enum ProcessResult {
    Passed,
    Failed,
    Timeout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timeouts {
    compile: u64,
    test: u64,
}

impl Timeouts {
    fn resolve(legacy: u64, compile: Option<u64>, test: Option<u64>) -> Self {
        Self {
            compile: compile.unwrap_or(legacy),
            test: test.unwrap_or(legacy),
        }
    }
}

struct FunctionState {
    current_function: Option<String>,
    address_params: Vec<String>,
    token_client_vars: Vec<String>,
    contract_client_vars: Vec<String>,
    storage_accessor_vars: Vec<String>,
    numeric_params: Vec<String>,
    env_params: Vec<String>,
    local_bindings: std::collections::HashSet<String>,
}

/// A file-level helper whose whole body is `param.require_auth();` statements
/// over explicitly typed `Address` parameters (AUTH-004).
#[derive(Clone, Debug)]
struct AuthHelper {
    arity: usize,
    /// `(argument index, helper parameter name, parameter is a reference)`.
    sinks: Vec<(usize, String, bool)>,
}

struct SemanticVisitor<'a> {
    source: &'a str,
    file: PathBuf,
    current_function: Option<String>,
    address_params: Vec<String>,
    token_client_vars: Vec<String>,
    contract_client_vars: Vec<String>,
    storage_accessor_vars: Vec<String>,
    numeric_params: Vec<String>,
    env_params: Vec<String>,
    typed_event_structs: std::collections::HashSet<String>,
    typed_event_topic_fields: std::collections::HashMap<String, std::collections::HashSet<String>>,
    auth_helpers: std::collections::HashMap<String, AuthHelper>,
    module_depth: usize,
    local_bindings: std::collections::HashSet<String>,
    mutants: Vec<Mutant>,
}

impl<'a> SemanticVisitor<'a> {
    fn new(source: &'a str, file: PathBuf) -> Self {
        Self {
            source,
            file,
            current_function: None,
            address_params: Vec::new(),
            token_client_vars: Vec::new(),
            contract_client_vars: Vec::new(),
            storage_accessor_vars: Vec::new(),
            numeric_params: Vec::new(),
            env_params: Vec::new(),
            typed_event_structs: std::collections::HashSet::new(),
            typed_event_topic_fields: std::collections::HashMap::new(),
            auth_helpers: std::collections::HashMap::new(),
            module_depth: 0,
            local_bindings: std::collections::HashSet::new(),
            mutants: Vec::new(),
        }
    }

    fn enter_signature(&mut self, sig: &Signature, body: &syn::Block) -> FunctionState {
        let mut bindings = BindingNames::default();
        bindings.visit_block(body);
        FunctionState {
            local_bindings: std::mem::replace(&mut self.local_bindings, bindings.0),
            current_function: self.current_function.replace(sig.ident.to_string()),
            address_params: std::mem::replace(&mut self.address_params, address_params(sig)),
            token_client_vars: std::mem::replace(
                &mut self.token_client_vars,
                token_client_params(sig),
            ),
            contract_client_vars: std::mem::take(&mut self.contract_client_vars),
            storage_accessor_vars: std::mem::take(&mut self.storage_accessor_vars),
            numeric_params: std::mem::replace(&mut self.numeric_params, numeric_params(sig)),
            env_params: std::mem::replace(&mut self.env_params, env_params(sig)),
        }
    }

    fn leave_signature(&mut self, previous: FunctionState) {
        self.current_function = previous.current_function;
        self.address_params = previous.address_params;
        self.token_client_vars = previous.token_client_vars;
        self.contract_client_vars = previous.contract_client_vars;
        self.storage_accessor_vars = previous.storage_accessor_vars;
        self.numeric_params = previous.numeric_params;
        self.env_params = previous.env_params;
        self.local_bindings = previous.local_bindings;
    }

    /// AUTH-004: mutate a direct statement call to a simple local auth helper.
    fn push_auth_helper_mutants(&mut self, call: &syn::ExprCall) {
        if self.module_depth != 0
            || self.current_function.is_none()
            || (self.env_params.is_empty() && self.address_params.is_empty())
        {
            return;
        }
        let Expr::Path(path) = call.func.as_ref() else {
            return;
        };
        if path.qself.is_some()
            || path.path.leading_colon.is_some()
            || path.path.segments.len() != 1
            || !path.path.segments[0].arguments.is_none()
        {
            return;
        }
        let helper_name = path.path.segments[0].ident.to_string();
        if self.local_bindings.contains(&helper_name)
            || self.address_params.contains(&helper_name)
            || self.env_params.contains(&helper_name)
        {
            return;
        }
        let Some(helper) = self.auth_helpers.get(&helper_name).cloned() else {
            return;
        };
        if call.args.len() != helper.arity {
            return;
        }

        let args = call.args.iter().collect::<Vec<_>>();
        let address_params = self.address_params.clone();
        let caller_address = |arg: &Expr| -> Option<String> {
            let name = match arg {
                Expr::Reference(reference) if reference.mutability.is_none() => {
                    receiver_ident(&reference.expr)
                }
                Expr::MethodCall(clone) if clone.method == "clone" && clone.args.is_empty() => {
                    receiver_ident(&clone.receiver)
                }
                Expr::Path(_) => receiver_ident(arg),
                _ => None,
            }?;
            address_params.contains(&name).then_some(name)
        };

        let sink_names = helper
            .sinks
            .iter()
            .map(|(_, param, _)| format!("{param}.require_auth()"))
            .collect::<Vec<_>>()
            .join(", ");
        let authorized = helper
            .sinks
            .iter()
            .map(|(index, _, _)| {
                caller_address(args[*index]).unwrap_or_else(|| "an argument".to_owned())
            })
            .collect::<Vec<_>>()
            .join(", ");
        self.push_mutant(
            "AUTH-004",
            call.span(),
            "()".to_owned(),
            format!("skip {helper_name}() authorization of {authorized} (sink: {sink_names})"),
        );

        for (index, param, is_ref) in &helper.sinks {
            let argument = args[*index];
            let Some(original) = caller_address(argument) else {
                continue;
            };
            for alternate in address_params.iter().filter(|name| *name != &original) {
                let replacement = if *is_ref {
                    format!("&{alternate}")
                } else {
                    format!("{alternate}.clone()")
                };
                self.push_mutant(
                    "AUTH-004",
                    argument.span(),
                    replacement,
                    format!(
                        "authenticate {alternate} instead of {original} via {helper_name}() (sink: {param}.require_auth())"
                    ),
                );
            }
        }
    }

    fn push_mutant(
        &mut self,
        operator: &str,
        span: Span,
        replacement: String,
        description: String,
    ) {
        let Some((start, end, source_span)) = span_to_offsets(self.source, span) else {
            return;
        };
        if start >= end || end > self.source.len() {
            return;
        }

        self.mutants.push(Mutant {
            id: String::new(),
            operator: operator.to_string(),
            file: self.file.clone(),
            function: self.current_function.clone(),
            span: source_span,
            original: self.source[start..end].to_string(),
            replacement,
            description,
            start_byte: start,
            end_byte: end,
        });
    }

    /// TOKEN-002: swap the two leading `Address` role arguments of a supported
    /// allowance-related token-client call (owner/spender or spender/owner).
    ///
    /// Both arguments must be `&ident` references to distinct, explicitly typed
    /// `Address` parameters of the enclosing function. The edit covers the span
    /// from the first role argument to the second, preserving the separator text.
    fn push_allowance_role_swap(&mut self, node: &ExprMethodCall, role_a: &str, role_b: &str) {
        let mut args = node.args.iter();
        let (Some(first), Some(second)) = (args.next(), args.next()) else {
            return;
        };
        let typed_address_ref = |arg: &Expr| match arg {
            Expr::Reference(reference) if reference.mutability.is_none() => {
                receiver_ident(&reference.expr).filter(|name| self.address_params.contains(name))
            }
            _ => None,
        };
        let (Some(first_name), Some(second_name)) =
            (typed_address_ref(first), typed_address_ref(second))
        else {
            return;
        };
        if first_name == second_name {
            return;
        }
        let (
            Some((first_start, first_end, first_span)),
            Some((second_start, second_end, second_span)),
        ) = (
            span_to_offsets(self.source, first.span()),
            span_to_offsets(self.source, second.span()),
        )
        else {
            return;
        };
        if !(first_start < first_end && first_end <= second_start && second_start < second_end)
            || second_end > self.source.len()
        {
            return;
        }

        let replacement = format!(
            "{}{}{}",
            &self.source[second_start..second_end],
            &self.source[first_end..second_start],
            &self.source[first_start..first_end],
        );
        let method = node.method.to_string();
        self.mutants.push(Mutant {
            id: String::new(),
            operator: "TOKEN-002".to_owned(),
            file: self.file.clone(),
            function: self.current_function.clone(),
            span: SourceSpan {
                line: first_span.line,
                column: first_span.column,
                end_line: second_span.end_line,
                end_column: second_span.end_column,
            },
            original: self.source[first_start..second_end].to_string(),
            replacement,
            description: format!(
                "swap token {method}() {role_a}/{role_b} arguments ({first_name} <-> {second_name})"
            ),
            start_byte: first_start,
            end_byte: second_end,
        });
    }

    /// AUTH-005: substitute a recoverable stored admin/owner authority.
    ///
    /// The supported retrieval pattern is a local bound exactly once in the
    /// enclosing function, explicitly typed `Address` (by annotation or by a
    /// `get::<_, Address>` turbofish), and initialized directly from
    /// `<env>.storage().<instance|persistent|temporary>().get(key)` followed by
    /// `.unwrap()` or `.expect(..)`, where `<env>` is an explicit `Env`
    /// parameter. The local is treated as a privileged authority only when it
    /// is the receiver of `require_auth()` / `require_auth_for_args(..)` in the
    /// same function. The loaded value is replaced with `<alternate>.clone()`
    /// for each explicit `Address` parameter, in signature order.
    fn push_stored_authority_mutants(&mut self, body: &syn::Block) {
        if self.env_params.is_empty() || self.address_params.is_empty() {
            return;
        }

        let mut scan = AuthorityScan::default();
        scan.visit_block(body);

        let mut candidates = Vec::new();
        for local in &scan.locals {
            let Some((name, init)) = local_binding_name_and_init(local) else {
                continue;
            };
            if local
                .init
                .as_ref()
                .is_some_and(|init| init.diverge.is_some())
                || !scan.auth_receivers.contains(&name)
                || scan.bound_names.get(&name).copied() != Some(1)
                || self.address_params.contains(&name)
                || self.env_params.contains(&name)
            {
                continue;
            }
            let annotated_address =
                matches!(&local.pat, Pat::Type(typed) if type_is_address(&typed.ty));
            let Some(turbofish_address) = stored_authority_lookup(init, &self.env_params) else {
                continue;
            };
            if annotated_address || turbofish_address {
                candidates.push((name, init.span()));
            }
        }

        for (name, span) in candidates {
            for alternate in self.address_params.clone() {
                self.push_mutant(
                    "AUTH-005",
                    span,
                    format!("{alternate}.clone()"),
                    format!("use {alternate} instead of stored authority {name}"),
                );
            }
        }
    }
}

/// Function-body scan used by AUTH-005. Nested items are skipped because they
/// form their own scope and are visited separately.
#[derive(Default)]
struct AuthorityScan<'ast> {
    locals: Vec<&'ast Local>,
    bound_names: std::collections::HashMap<String, usize>,
    auth_receivers: std::collections::HashSet<String>,
}

impl<'ast> Visit<'ast> for AuthorityScan<'ast> {
    fn visit_item(&mut self, _node: &'ast Item) {}

    fn visit_local(&mut self, node: &'ast Local) {
        self.locals.push(node);
        visit::visit_local(self, node);
    }

    fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
        *self.bound_names.entry(node.ident.to_string()).or_default() += 1;
        visit::visit_pat_ident(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        let is_auth_sink = (node.method == "require_auth" && node.args.is_empty())
            || (node.method == "require_auth_for_args" && node.args.len() == 1);
        if is_auth_sink {
            if let Some(name) = receiver_ident(&node.receiver) {
                self.auth_receivers.insert(name);
            }
        }
        visit::visit_expr_method_call(self, node);
    }
}

/// Recognizes `<env>.storage().<class>().get(key).unwrap()` or `.expect(..)`
/// rooted at an explicit `Env` parameter. Returns `Some(true)` when the `get`
/// call carries an explicit `Address` value type in its turbofish, `Some(false)`
/// for a supported lookup without one, and `None` for anything else.
fn stored_authority_lookup(expr: &Expr, env_params: &[String]) -> Option<bool> {
    let Expr::MethodCall(unwrap) = expr else {
        return None;
    };
    let unwraps = (unwrap.method == "unwrap" && unwrap.args.is_empty())
        || (unwrap.method == "expect" && unwrap.args.len() == 1);
    if !unwraps {
        return None;
    }
    let Expr::MethodCall(get) = unwrap.receiver.as_ref() else {
        return None;
    };
    if get.method != "get" || get.args.len() != 1 {
        return None;
    }
    let Expr::MethodCall(class) = get.receiver.as_ref() else {
        return None;
    };
    if !matches!(
        class.method.to_string().as_str(),
        "instance" | "persistent" | "temporary"
    ) || !class.args.is_empty()
        || !is_direct_env_storage_receiver(&class.receiver, env_params)
    {
        return None;
    }
    let turbofish_address = get.turbofish.as_ref().is_some_and(|turbofish| {
        matches!(
            turbofish.args.last(),
            Some(syn::GenericArgument::Type(ty)) if type_is_address(ty)
        )
    });
    Some(turbofish_address)
}

impl<'ast> Visit<'ast> for SemanticVisitor<'_> {
    fn visit_file(&mut self, node: &'ast syn::File) {
        self.typed_event_structs = node
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Struct(item)
                    if item.attrs.iter().any(|attribute| {
                        attribute
                            .path()
                            .segments
                            .last()
                            .is_some_and(|segment| segment.ident == "contractevent")
                    }) =>
                {
                    Some(item.ident.to_string())
                }
                _ => None,
            })
            .collect();
        self.typed_event_topic_fields = node
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Struct(item)
                    if self.typed_event_structs.contains(&item.ident.to_string()) =>
                {
                    let fields = match &item.fields {
                        syn::Fields::Named(fields) => fields
                            .named
                            .iter()
                            .filter(|field| {
                                field.attrs.iter().any(|attribute| {
                                    attribute
                                        .path()
                                        .segments
                                        .last()
                                        .is_some_and(|segment| segment.ident == "topic")
                                })
                            })
                            .filter_map(|field| field.ident.as_ref().map(ToString::to_string))
                            .collect::<std::collections::HashSet<_>>(),
                        _ => std::collections::HashSet::new(),
                    };
                    Some((item.ident.to_string(), fields))
                }
                _ => None,
            })
            .collect();
        self.auth_helpers = node
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Fn(function) => {
                    auth_helper(function).map(|helper| (function.sig.ident.to_string(), helper))
                }
                _ => None,
            })
            .collect();
        visit::visit_file(self, node);
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        self.module_depth += 1;
        visit::visit_item_mod(self, node);
        self.module_depth -= 1;
    }

    fn visit_stmt(&mut self, node: &'ast Stmt) {
        if let Stmt::Expr(Expr::Call(call), Some(_)) = node {
            self.push_auth_helper_mutants(call);
        }
        if let Stmt::Expr(Expr::MethodCall(call), Some(_)) = node {
            if call.method == "publish" && call.args.len() == 1 {
                let is_typed_event = match call.receiver.as_ref() {
                    Expr::Struct(event) => event
                        .path
                        .get_ident()
                        .is_some_and(|name| self.typed_event_structs.contains(&name.to_string())),
                    _ => false,
                };
                let targets_env = match call.args.first() {
                    Some(Expr::Reference(reference)) => receiver_ident(&reference.expr)
                        .is_some_and(|name| self.env_params.contains(&name)),
                    _ => false,
                };
                if is_typed_event && targets_env {
                    self.push_mutant(
                        "EVENT-002",
                        call.span(),
                        "()".to_owned(),
                        "remove direct typed Soroban contract event publication".to_owned(),
                    );

                    if let Expr::Struct(event) = call.receiver.as_ref() {
