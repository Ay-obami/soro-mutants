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
    ("CALL-001", "replace direct Env::invoke_contract dependency address with another typed Address parameter"),
    (
        "EVENT-001",
        "remove a Soroban event publication (env.events().publish(...))",
    ),
    (
        "EVENT-002",
        "remove a standalone #[contractevent] typed event publication",
    ),
    (
        "TOKEN-001",
        "swap the sender and recipient arguments of a token transfer",
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
    storage_accessor_vars: Vec<String>,
    env_params: Vec<String>,
}

struct SemanticVisitor<'a> {
    source: &'a str,
    file: PathBuf,
    current_function: Option<String>,
    address_params: Vec<String>,
    token_client_vars: Vec<String>,
    storage_accessor_vars: Vec<String>,
    env_params: Vec<String>,
    typed_event_structs: std::collections::HashSet<String>,
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
            storage_accessor_vars: Vec::new(),
            env_params: Vec::new(),
            typed_event_structs: std::collections::HashSet::new(),
            mutants: Vec::new(),
        }
    }

    fn enter_signature(&mut self, sig: &Signature) -> FunctionState {
        FunctionState {
            current_function: self.current_function.replace(sig.ident.to_string()),
            address_params: std::mem::replace(&mut self.address_params, address_params(sig)),
            token_client_vars: std::mem::replace(
                &mut self.token_client_vars,
                token_client_params(sig),
            ),
            storage_accessor_vars: std::mem::take(&mut self.storage_accessor_vars),
            env_params: std::mem::replace(&mut self.env_params, env_params(sig)),
        }
    }

    fn leave_signature(&mut self, previous: FunctionState) {
        self.current_function = previous.current_function;
        self.address_params = previous.address_params;
        self.token_client_vars = previous.token_client_vars;
        self.storage_accessor_vars = previous.storage_accessor_vars;
        self.env_params = previous.env_params;
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
        visit::visit_file(self, node);
    }

    fn visit_stmt(&mut self, node: &'ast Stmt) {
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
                }
            }
        }
        visit::visit_stmt(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        let previous = self.enter_signature(&node.sig);
        visit::visit_item_fn(self, node);
        self.leave_signature(previous);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
        let previous = self.enter_signature(&node.sig);
        visit::visit_impl_item_fn(self, node);
        self.leave_signature(previous);
    }

    fn visit_local(&mut self, node: &'ast Local) {
        if let (Pat::Ident(ident), Some(init)) = (&node.pat, &node.init) {
            let name = ident.ident.to_string();

            if is_token_client_constructor(&init.expr) && !self.token_client_vars.contains(&name) {
                self.token_client_vars.push(name.clone());
            }

            if is_soroban_storage_accessor(&init.expr)
                && !self.storage_accessor_vars.contains(&name)
            {
                self.storage_accessor_vars.push(name);
            }
        }

        visit::visit_local(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        // Only mutate a direct env.invoke_contract call with a simple typed
        // Address parameter as its first argument. Avoid arbitrary typed inference.
        if node.method == "invoke_contract"
            && node.args.len() == 3
            && receiver_ident(&node.receiver).is_some_and(|name| self.env_params.contains(&name))
        {
            if let Some(Expr::Reference(target_ref)) = node.args.first() {
                if let Some(original_target) = receiver_ident(&target_ref.expr) {
                    if self.address_params.contains(&original_target) {
                        for alternate in self
                            .address_params
                            .iter()
                            .filter(|name| *name != &original_target)
                            .cloned()
                            .collect::<Vec<_>>()
                        {
                            self.push_mutant(
                                "CALL-001",
                                target_ref.span(),
                                format!("&{alternate}"),
                                format!(
                                    "invoke contract at {alternate} instead of {original_target}"
                                ),
                            );
                        }
                    }
                }
            }
        }

        if node.method == "publish" && is_soroban_events_receiver(&node.receiver, &self.env_params)
        {
            self.push_mutant(
                "EVENT-001",
                node.span(),
                "()".to_string(),
                "remove Soroban event publication".to_string(),
            );
        }

        if node.method == "transfer"
            && node.args.len() == 3
            && is_token_transfer_receiver(&node.receiver, &self.token_client_vars)
        {
            if let Some(replacement) = swapped_transfer_replacement(self.source, node) {
                self.push_mutant(
                    "TOKEN-001",
                    node.span(),
                    replacement,
                    "swap token transfer sender and recipient".to_string(),
                );
            }
        }

        if node.method == "extend_ttl"
            && is_ttl_receiver(&node.receiver, &self.storage_accessor_vars)
        {
            self.push_mutant(
                "TTL-001",
                node.span(),
                "()".to_string(),
                "remove extend_ttl()".to_string(),
            );
            // Instance has two arguments; persistent/temporary keyed storage has three.
            // Change only the lifecycle parameters, not the key or receiver.
            if matches!(node.args.len(), 2 | 3) {
                let args: Vec<_> = node.args.iter().collect();
                let threshold = args[args.len() - 2];
                let target = args[args.len() - 1];
                if source_for_span(self.source, threshold.span())
                    .is_some_and(|value| value.trim() != "0")
                {
                    self.push_mutant(
                        "TTL-002",
                        threshold.span(),
                        "0".to_owned(),
                        "set TTL renewal threshold to zero".to_owned(),
                    );
                }
                if let (Some(threshold_text), Some(target_text)) = (
                    source_for_span(self.source, threshold.span()),
                    source_for_span(self.source, target.span()),
                ) {
                    if threshold_text.trim() != target_text.trim() {
                        self.push_mutant(
                            "TTL-003",
                            target.span(),
                            threshold_text.to_owned(),
                            "shorten TTL extension target to renewal threshold".to_owned(),
                        );
                    }
                }
            }
        }

        if node.method == "require_auth_for_args" && node.args.len() == 1 {
            if let (Some(receiver), Some(env_name)) =
                (receiver_ident(&node.receiver), self.env_params.first())
            {
                if self.address_params.contains(&receiver) {
                    let replacement = format!("soroban_sdk::Vec::new(&{})", env_name);
                    let arguments = node.args.first().expect("one argument checked");
                    if let Some((start, end, _)) = span_to_offsets(self.source, arguments.span()) {
                        if self.source[start..end] != replacement {
                            self.push_mutant(
                                "AUTH-003",
                                arguments.span(),
                                replacement,
                                format!("authorize an empty argument set for {}", receiver),
                            );
                        }
                    }
                }
            }
        }

        if node.method == "require_auth"
            && (!self.env_params.is_empty() || !self.address_params.is_empty())
        {
            let receiver = receiver_ident(&node.receiver);

            self.push_mutant(
                "AUTH-001",
                node.span(),
                "()".to_string(),
                match &receiver {
                    Some(name) => format!("remove {}.require_auth()", name),
                    None => "remove require_auth()".to_string(),
                },
            );

            if let Some(receiver_name) = receiver {
                for candidate in self
                    .address_params
                    .iter()
                    .filter(|candidate| *candidate != &receiver_name)
                    .cloned()
                    .collect::<Vec<_>>()
                {
                    self.push_mutant(
                        "AUTH-002",
                        node.span(),
                        format!("{}.require_auth()", candidate),
                        format!("authenticate {} instead of {}", candidate, receiver_name),
                    );
                }
            }
        }

        visit::visit_expr_method_call(self, node);
    }
}

#[derive(Debug)]
struct BaselineFailure(&'static str);

impl std::fmt::Display for BaselineFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for BaselineFailure {}

#[derive(Debug)]
struct ScoreThresholdFailure {
    actual: f64,
    minimum: f64,
}

impl std::fmt::Display for ScoreThresholdFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "semantic score {:.1}% below minimum {:.1}%",
            self.actual, self.minimum
        )
    }
}

impl std::error::Error for ScoreThresholdFailure {}

fn parse_min_score(value: &str) -> std::result::Result<f64, String> {
    let score = value
        .parse::<f64>()
        .map_err(|_| "score must be a number from 0 to 100".to_owned())?;
    if !score.is_finite() || !(0.0..=100.0).contains(&score) {
        return Err("score must be a finite number from 0 to 100".to_owned());
    }
    Ok(score)
}

fn ensure_min_score(results: &[MutantResult], minimum: Option<f64>) -> Result<()> {
    if let Some(minimum) = minimum {
        let killed = results
            .iter()
            .filter(|r| matches!(r.outcome, Outcome::Killed))
            .count();
        let survived = results
            .iter()
            .filter(|r| matches!(r.outcome, Outcome::Survived))
            .count();
        let viable = killed + survived;
        let actual = if viable == 0 {
            0.0
        } else {
            killed as f64 * 100.0 / viable as f64
        };
        if actual < minimum {
            return Err(ScoreThresholdFailure { actual, minimum }.into());
        }
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            if error.downcast_ref::<BaselineFailure>().is_some() {
                std::process::ExitCode::from(3)
            } else if error.downcast_ref::<ScoreThresholdFailure>().is_some() {
                std::process::ExitCode::from(4)
            } else {
                std::process::ExitCode::FAILURE
            }
        }
    }
}

fn run() -> Result<()> {
    let mut command = Cli::command();
    if env::var_os("NO_COLOR").is_some() {
        // Apply before parsing so help and argument errors also honor NO_COLOR.
        command = command.color(ColorChoice::Never);
    }
    let matches = command.get_matches_from(normalized_args());
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());

    match cli.command {
        Commands::List {
            path,
            operator,
            exclude_path,
            shard,
            file,
            function,
            changed_file,
            diff_base,
            changed_functions,
            json,
            count_only,
            output,
        } => {
            let (mutants, suppressed) = discover_mutants_with_suppressions(
                &path,
                operator.as_deref(),
                file.as_deref(),
                function.as_deref(),
                &exclusion_set(exclude_path)?,
            )?;
            let mutants = filter_changed_mutants(
                &path,
                mutants,
                &changed_file,
                diff_base.as_deref(),
                changed_functions,
            )?;
            let mutants = select_shard(mutants, shard);
            let mut report = report_writer(output.as_deref())?;
            if count_only {
                writeln!(report, "{}", mutants.len())?;
            } else if json {
                writeln!(
                    report,
                    "{}",
                    serde_json::to_string_pretty(&ListReport {
                        suppression: (!suppressed.is_empty()).then_some(SuppressionSummary {
                            count: suppressed.len(),
                            details: &suppressed,
                        }),
                        ..list_report(&mutants)
                    })?
                )?;
            } else {
                print_mutants(&mut report, &mutants)?;
                print_suppression_summary(&mut report, &suppressed)?;
            }
        }
        Commands::Test {
            path,
            operator,
            exclude_path,
            shard,
            file,
            function,
            changed_file,
            diff_base,
            changed_functions,
            test_dir,
            target_dir,
            test_command,
            runner,
            timeout,
            compile_timeout,
            test_timeout,
            json,
            markdown,
            html,
            sarif,
            github_annotations,
            github_summary,
            min_score,
            output,
        } => {
            let timeouts = Timeouts::resolve(timeout, compile_timeout, test_timeout);
            let test_command = resolve_runner_command(runner, &test_command)?;
            let metadata = RunMetadata::capture(&test_command, timeouts);
            let root = path
                .canonicalize()
                .with_context(|| format!("cannot resolve {}", path.display()))?;
            let (mutants, suppressed) = discover_mutants_with_suppressions(
                &root,
                operator.as_deref(),
                file.as_deref(),
                function.as_deref(),
                &exclusion_set(exclude_path)?,
            )?;
            let mutants = filter_changed_mutants(
                &root,
                mutants,
                &changed_file,
                diff_base.as_deref(),
                changed_functions,
            )?;
            let mutants = select_shard(mutants, shard);
            if !mutants.is_empty() && runner == Runner::Nextest {
                ensure_nextest_available()?;
            }
            let mut report = report_writer(output.as_deref())?;
            if mutants.is_empty() {
                if json {
                    writeln!(
                        report,
                        "{}",
                        serde_json::to_string_pretty(&TestReport {
                            suppression: (!suppressed.is_empty()).then_some(SuppressionSummary {
                                count: suppressed.len(),
                                details: &suppressed,
                            }),
                            ..test_report_with_metadata(&[], &metadata)
                        })?
                    )?;
                } else if markdown {
                    print_markdown_report(&mut report, &[])?;
                } else if html {
                    print_html_report(&mut report, &[])?;
                } else if sarif {
                    print_sarif_report(&mut report, &[])?;
                } else {
                    writeln!(report, "No matching Soroban semantic mutants found.")?;
                }
                if !suppressed.is_empty() && !json {
                    print_suppression_summary(&mut std::io::stderr(), &suppressed)?;
                }
                if github_annotations {
                    print_github_annotations(&mut std::io::stderr(), &[])?;
                }
                if github_summary {
                    append_github_summary(&[])?;
                }
                ensure_min_score(&[], min_score)?;
                return Ok(());
            }

            let (baseline_target_dir, mutant_target_root) =
                target_dirs(&root, target_dir.as_deref());
            // Keep the clean baseline isolated, but let mutants share a cache.
            // The scratch path is stable and each source mutation changes file contents,
            // so Cargo rebuilds the affected crate while reusing dependencies.
            let shared_target = true;
            let test_cwd = test_dir
                .as_deref()
                .map(|dir| root.join(dir))
                .unwrap_or_else(|| root.clone());
            if !json && !markdown && !html && !sarif {
                writeln!(report, "Baseline: {}", test_command)?;
            }
            match run_process(
                &test_cwd,
                &test_command,
                timeouts.test,
                &baseline_target_dir,
            )? {
                ProcessResult::Passed => {
                    if !json && !markdown && !html && !sarif {
                        writeln!(report, "Baseline PASS\n")?;
                    }
                }
                ProcessResult::Failed => {
                    return Err(BaselineFailure(
                        "baseline tests failed; mutation results would be invalid",
                    )
                    .into());
                }
                ProcessResult::Timeout => {
                    return Err(BaselineFailure("baseline tests timed out").into())
                }
            }

            let mut results = Vec::new();
            for mutant in mutants {
                let outcome = execute_mutant(
                    &root,
                    &mutant,
                    test_dir.as_deref(),
                    &test_command,
                    timeouts,
                    &mutant_target_root,
                    shared_target,
                )?;
                if !json && !markdown && !html && !sarif {
                    writeln!(
                        report,
                        "{:<10} {:<8} {}:{} {}",
                        outcome_label(&outcome),
                        mutant.operator,
                        mutant.file.display(),
                        mutant.span.line,
                        mutant.description
                    )?;
                }
                results.push(MutantResult { mutant, outcome });
            }

            if json {
                writeln!(
                    report,
                    "{}",
                    serde_json::to_string_pretty(&TestReport {
                        suppression: (!suppressed.is_empty()).then_some(SuppressionSummary {
                            count: suppressed.len(),
                            details: &suppressed,
                        }),
                        ..test_report_with_metadata(&results, &metadata)
                    })?
                )?;
            } else if markdown {
                print_markdown_report(&mut report, &results)?;
            } else if html {
                print_html_report(&mut report, &results)?;
            } else if sarif {
                print_sarif_report(&mut report, &results)?;
            } else {
                print_summary(&mut report, &results)?;
                print_suppression_summary(&mut report, &suppressed)?;
            }
            if !suppressed.is_empty() && (markdown || html || sarif) {
                print_suppression_summary(&mut std::io::stderr(), &suppressed)?;
            }
            if github_annotations {
                print_github_annotations(&mut std::io::stderr(), &results)?;
            }
            if github_summary {
                append_github_summary(&results)?;
            }
            ensure_min_score(&results, min_score)?;
        }
        Commands::Clean {
            path,
            target_dir,
            dry_run,
        } => {
            let root = path
                .canonicalize()
                .with_context(|| format!("cannot resolve {}", path.display()))?;
            let paths = clean_generated_state(&root, target_dir.as_deref(), dry_run)?;
            if paths.is_empty() {
                println!("No Soro Mutants generated state found.");
            } else {
                for path in paths {
                    println!(
                        "{} {}",
                        if dry_run { "Would remove" } else { "Removed" },
                        path.display()
                    );
                }
            }
        }
        Commands::ValidateCorpus { path } => {
            let count = validate_corpus(&path)?;
            println!("Validated {count} corpus entries (schema version 1).");
        }
        Commands::Operators => {
            print_operators(&mut io::stdout())?;
        }
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(
                Shell::from(shell),
                &mut cmd,
                "cargo-soro-mutants",
                &mut io::stdout(),
            );
        }
    }

    Ok(())
}

const CORPUS_SCHEMA_VERSION: u32 = 1;

#[derive(Deserialize)]
struct CorpusManifest {
    schema_version: u32,
    entries: Vec<CorpusEntry>,
}

#[derive(Deserialize)]
struct CorpusEntry {
    id: String,
    operator: String,
    source_pattern: CorpusSourcePattern,
    mutation: CorpusMutation,
    expectations: CorpusExpectations,
    provenance: CorpusProvenance,
}

#[derive(Deserialize)]
struct CorpusSourcePattern {
    language: String,
    code: String,
}

#[derive(Deserialize)]
struct CorpusMutation {
    original: String,
    replacement: String,
}

#[derive(Deserialize)]
struct CorpusExpectations {
    weak: String,
    strong: String,
}

#[derive(Deserialize)]
struct CorpusProvenance {
    kind: String,
    reference: String,
}

fn validate_corpus(path: &Path) -> Result<usize> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("cannot read corpus manifest {}", path.display()))?;
    let manifest: CorpusManifest = serde_json::from_str(&content)
        .with_context(|| format!("invalid corpus manifest JSON at {}", path.display()))?;
    if manifest.schema_version != CORPUS_SCHEMA_VERSION {
        bail!(
            "unsupported corpus schema_version {}; expected {}",
            manifest.schema_version,
            CORPUS_SCHEMA_VERSION
        );
    }
    if manifest.entries.is_empty() {
        bail!("corpus manifest entries must not be empty");
    }
    let mut seen = std::collections::HashSet::new();
    for (index, entry) in manifest.entries.iter().enumerate() {
        let location = format!("entries[{index}]");
        if entry.id.trim().is_empty() {
            bail!("{location}.id must not be empty");
        }
        if !seen.insert(entry.id.as_str()) {
            bail!("{location}.id is duplicate: {}", entry.id);
        }
        let valid_operator = entry
            .operator
            .split_once('-')
            .is_some_and(|(name, number)| {
                !name.is_empty()
                    && name.bytes().all(|byte| byte.is_ascii_uppercase())
                    && number.len() == 3
                    && number.bytes().all(|byte| byte.is_ascii_digit())
            });
        if !valid_operator {
            bail!("{location}.operator must match NAME-000 (uppercase family, three digits)");
        }
        if entry.source_pattern.language != "rust" {
            bail!("{location}.source_pattern.language must be rust");
        }
        for (field, value) in [
            ("source_pattern.code", entry.source_pattern.code.as_str()),
            ("mutation.original", entry.mutation.original.as_str()),
            ("mutation.replacement", entry.mutation.replacement.as_str()),
            ("provenance.kind", entry.provenance.kind.as_str()),
            ("provenance.reference", entry.provenance.reference.as_str()),
        ] {
            if value.trim().is_empty() {
                bail!("{location}.{field} must not be empty");
            }
        }
        if entry.mutation.original == entry.mutation.replacement {
            bail!("{location}.mutation replacement must differ from original");
        }
        for (field, value) in [
            ("expectations.weak", entry.expectations.weak.as_str()),
            ("expectations.strong", entry.expectations.strong.as_str()),
        ] {
            if !matches!(value, "KILLED" | "SURVIVED" | "UNVIABLE" | "TIMEOUT") {
                bail!("{location}.{field} must be a known mutation outcome");
            }
        }
    }
    Ok(manifest.entries.len())
}

fn append_github_summary(results: &[MutantResult]) -> Result<()> {
    let Some(path) = env::var_os("GITHUB_STEP_SUMMARY").filter(|value| !value.is_empty()) else {
        // An explicit opt-in remains safe for local runs and non-GitHub CI.
        return Ok(());
    };
    let path = PathBuf::from(path);
    let mut summary = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("cannot append GitHub job summary at {}", path.display()))?;
    print_markdown_report(&mut summary, results)?;
    Ok(())
}

fn report_writer(output: Option<&Path>) -> Result<Box<dyn Write>> {
    match output {
        Some(path) => {
            let file = fs::File::create(path)
                .with_context(|| format!("cannot create report at {}", path.display()))?;
            Ok(Box::new(file))
        }
        None => Ok(Box::new(io::stdout())),
    }
}

fn normalized_args() -> Vec<OsString> {
    let mut args: Vec<OsString> = env::args_os().collect();
    if args
        .get(1)
        .map(|arg| arg.to_string_lossy() == "soro-mutants")
        .unwrap_or(false)
    {
        args.remove(1);
    }
    args
}

fn filter_changed_mutants(
    root: &Path,
    mutants: Vec<Mutant>,
    changed_files: &[PathBuf],
    diff_base: Option<&str>,
    functions_only: bool,
) -> Result<Vec<Mutant>> {
    use std::collections::{BTreeMap, BTreeSet};

    if changed_files.is_empty() && diff_base.is_none() {
        return Ok(mutants);
    }
    let root = root.canonicalize()?;
    let mut selected = BTreeSet::new();
    let mut git_top = None;

    if let Some(base) = diff_base {
        if base.starts_with('-') || base.is_empty() {
            bail!("invalid Git diff base revision");
        }
        let output = Command::new("git")
            .args([
                "-C",
                root.to_str().unwrap_or("."),
                "rev-parse",
                "--show-toplevel",
            ])
            .output()
            .context("cannot locate Git repository")?;
        if !output.status.success() {
            bail!("Git repository metadata is unavailable for --diff-base");
        }
        let top = PathBuf::from(String::from_utf8(output.stdout)?.trim()).canonicalize()?;
        let diff = Command::new("git")
            .arg("-C")
            .arg(&top)
            .args([
                "diff",
                "--no-ext-diff",
                "--find-renames",
                "--diff-filter=ACMR",
                "--name-only",
                "-z",
            ])
            .arg(base)
            .arg("--")
            .output()
            .context("cannot execute git diff")?;
        if !diff.status.success() {
            bail!(
                "git diff failed for base {base}: {}",
                String::from_utf8_lossy(&diff.stderr).trim()
            );
        }
        for file in diff
            .stdout
            .split(|byte| *byte == 0)
            .filter(|value| !value.is_empty())
        {
            let full = top.join(String::from_utf8_lossy(file).as_ref());
            if let Ok(relative) = full.strip_prefix(&root) {
                selected.insert(relative.to_string_lossy().replace('\\', "/"));
            }
        }
        git_top = Some(top);
    } else {
        for file in changed_files {
            let absolute = root.join(file);
            let relative = absolute.strip_prefix(&root).unwrap_or(file);
            if relative.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            }) {
                bail!(
                    "--changed-file must be a project-relative path without ..: {}",
                    file.display()
                );
            }
            selected.insert(relative.to_string_lossy().replace('\\', "/"));
        }
    }

    let mut matching: Vec<Mutant> = mutants
        .into_iter()
        .filter(|mutant| selected.contains(&mutant.file.to_string_lossy().replace('\\', "/")))
        .collect();
    if !functions_only || matching.is_empty() {
        return Ok(matching);
    }

    let ranges = changed_line_ranges(
        &root,
        git_top.as_deref().unwrap_or(&root),
        diff_base.unwrap(),
    )?;
    type FunctionSpan = (String, usize, usize);
    let mut touched: BTreeMap<String, Option<Vec<FunctionSpan>>> = BTreeMap::new();
    for mutant in &matching {
        let path = mutant.file.to_string_lossy().replace('\\', "/");
        if touched.contains_key(&path) {
            continue;
        }
        let Some(changes) = ranges.get(&path) else {
            touched.insert(path, None);
            continue;
        };
        let source = fs::read_to_string(root.join(&mutant.file))?;
        let syntax = match syn::parse_file(&source) {
            Ok(syntax) => syntax,
            Err(_) => {
                touched.insert(path, None);
                continue;
            }
        };
        let mut collector = ChangedFunctionCollector { spans: Vec::new() };
        collector.visit_file(&syntax);
        let matching_functions: Vec<_> = collector
            .spans
            .into_iter()
            .filter(|(_, first, last)| {
                changes
                    .iter()
                    .any(|(from, to)| *first <= *to && *last >= *from)
            })
            .collect();
        touched.insert(
            path,
            if matching_functions.is_empty() {
                None
            } else {
                Some(matching_functions)
            },
        );
    }
    matching.retain(|mutant| {
        let path = mutant.file.to_string_lossy().replace('\\', "/");
        match touched.get(&path).and_then(|functions| functions.as_ref()) {
            // When the diff cannot be mapped to a function, retain file-level selection.
            None => true,
            Some(functions) => {
                let Some(name) = mutant.function.as_deref() else {
                    return true;
                };
                functions.iter().any(|(function, from, to)| {
                    function == name && *from <= mutant.span.line && mutant.span.line <= *to
                })
            }
        }
    });
    Ok(matching)
}

struct ChangedFunctionCollector {
    spans: Vec<(String, usize, usize)>,
}

impl<'ast> Visit<'ast> for ChangedFunctionCollector {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        let span = node.span();
        self.spans.push((
            node.sig.ident.to_string(),
            span.start().line,
            span.end().line,
        ));
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
        let span = node.span();
        self.spans.push((
            node.sig.ident.to_string(),
            span.start().line,
            span.end().line,
        ));
        visit::visit_impl_item_fn(self, node);
    }
}

fn changed_line_ranges(
    root: &Path,
    git_top: &Path,
    base: &str,
) -> Result<std::collections::BTreeMap<String, Vec<(usize, usize)>>> {
    use std::collections::BTreeMap;
    let diff = Command::new("git")
        .args(["-c", "core.quotePath=false", "-C"])
        .arg(git_top)
        .args([
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--no-renames",
            "--unified=0",
        ])
        .arg(base)
        .arg("--")
        .output()
        .context("cannot execute git diff for changed functions")?;
    if !diff.status.success() {
        bail!(
            "git diff failed for base {base}: {}",
            String::from_utf8_lossy(&diff.stderr).trim()
        );
    }
    let mut ranges: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    let mut current = None;
    for line in String::from_utf8_lossy(&diff.stdout).lines() {
        if let Some(name) = line.strip_prefix("+++ b/") {
            current = git_top
                .join(name)
                .strip_prefix(root)
                .ok()
                .map(|path| path.to_string_lossy().replace('\\', "/"));
        } else if let Some(hunk) = line.strip_prefix("@@ ") {
            let Some(path) = current.as_ref() else {
                continue;
            };
            let Some(new_lines) = hunk.split_whitespace().find(|part| part.starts_with('+')) else {
                continue;
            };
            let mut parts = new_lines.trim_start_matches('+').split(',');
            let Ok(start) = parts.next().unwrap_or("").parse::<usize>() else {
                continue;
            };
            let count = parts
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1);
            let end = start.saturating_add(count.saturating_sub(1));
            ranges.entry(path.clone()).or_default().push((start, end));
        }
    }
    Ok(ranges)
}

#[derive(Clone, Debug, Serialize)]
struct SuppressedMutant {
    operator: String,
    reason: &'static str,
}

fn suppress_equivalent_mutants(mutants: Vec<Mutant>) -> (Vec<Mutant>, Vec<SuppressedMutant>) {
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    let mut retained = Vec::with_capacity(mutants.len());
    let mut suppressed = Vec::new();
    for mutant in mutants {
        let reason = if mutant.original == mutant.replacement {
            Some("unchanged replacement")
        } else if !seen.insert((
            mutant.file.clone(),
            mutant.start_byte,
            mutant.end_byte,
            mutant.original.clone(),
            mutant.replacement.clone(),
        )) {
            Some("duplicate source edit")
        } else {
            None
        };
        if let Some(reason) = reason {
            suppressed.push(SuppressedMutant {
                operator: mutant.operator,
                reason,
            });
        } else {
            retained.push(mutant);
        }
    }
    (retained, suppressed)
}

#[cfg(test)]
fn discover_mutants(
    root: &Path,
    operator: Option<&str>,
    file_filter: Option<&str>,
    function_filter: Option<&str>,
    exclusions: &GlobSet,
) -> Result<Vec<Mutant>> {
    Ok(discover_mutants_with_suppressions(
        root,
        operator,
        file_filter,
        function_filter,
        exclusions,
    )?
    .0)
}

fn discover_mutants_with_suppressions(
    root: &Path,
    operator: Option<&str>,
    file_filter: Option<&str>,
    function_filter: Option<&str>,
    exclusions: &GlobSet,
) -> Result<(Vec<Mutant>, Vec<SuppressedMutant>)> {
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve {}", root.display()))?;
    let mut mutants = Vec::new();

    for entry in WalkDir::new(&root).into_iter().filter_entry(|entry| {
        !ignored_entry(entry, &root)
            && (entry.path() == root
                || !exclusions.is_match(entry.path().strip_prefix(&root).unwrap_or(entry.path())))
    }) {
        let entry = entry?;
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|s| s.to_str()) != Some("rs")
        {
            continue;
        }

        let source = fs::read_to_string(entry.path())
            .with_context(|| format!("cannot read {}", entry.path().display()))?;
        let syntax = match syn::parse_file(&source) {
            Ok(file) => file,
            Err(_) => continue,
        };
        let relative = entry
            .path()
            .strip_prefix(&root)
            .unwrap_or(entry.path())
            .to_path_buf();

        let mut visitor = SemanticVisitor::new(&source, relative);
        visitor.visit_file(&syntax);
        mutants.extend(visitor.mutants);
    }

    if let Some(operator) = operator {
        mutants.retain(|m| m.operator == operator);
    }
    if let Some(file_filter) = file_filter {
        let normalized = file_filter.replace('\\', "/");
        mutants.retain(|m| {
            m.file
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with(&normalized)
        });
    }
    if let Some(function_filter) = function_filter {
        mutants.retain(|m| m.function.as_deref() == Some(function_filter));
    }

    let (mut mutants, suppressed) = suppress_equivalent_mutants(mutants);

    // Total ordering must never inherit WalkDir's filesystem-dependent traversal order.
    // Keep the first four keys stable: IDs and shard assignments depend on them.
    mutants.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.start_byte.cmp(&b.start_byte))
            .then(a.operator.cmp(&b.operator))
            .then(a.replacement.cmp(&b.replacement))
            .then(a.end_byte.cmp(&b.end_byte))
            .then(a.original.cmp(&b.original))
            .then(a.function.cmp(&b.function))
            .then(a.description.cmp(&b.description))
    });

    for (index, mutant) in mutants.iter_mut().enumerate() {
        mutant.id = format!("M{:04}", index + 1);
    }

    Ok((mutants, suppressed))
}

fn address_params(sig: &Signature) -> Vec<String> {
    sig.inputs
        .iter()
        .filter_map(|input| match input {
            FnArg::Typed(pat_type) if type_is_address(&pat_type.ty) => {
                match pat_type.pat.as_ref() {
                    Pat::Ident(ident) => Some(ident.ident.to_string()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn type_is_address(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident == "Address")
            .unwrap_or(false),
        Type::Reference(reference) => type_is_address(&reference.elem),
        _ => false,
    }
}

fn env_params(sig: &Signature) -> Vec<String> {
    sig.inputs
        .iter()
        .filter_map(|input| match input {
            FnArg::Typed(pat_type) if type_is_env(&pat_type.ty) => match pat_type.pat.as_ref() {
                Pat::Ident(ident) => Some(ident.ident.to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn type_is_env(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident == "Env")
            .unwrap_or(false),
        Type::Reference(reference) => type_is_env(&reference.elem),
        _ => false,
    }
}

fn token_client_params(sig: &Signature) -> Vec<String> {
    sig.inputs
        .iter()
        .filter_map(|input| match input {
            FnArg::Typed(pat_type) if type_is_token_client(&pat_type.ty) => {
                match pat_type.pat.as_ref() {
                    Pat::Ident(ident) => Some(ident.ident.to_string()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn type_is_token_client(ty: &Type) -> bool {
    match ty {
        Type::Path(type_path) => path_is_token_client(&type_path.path),
        Type::Reference(reference) => type_is_token_client(&reference.elem),
        _ => false,
    }
}

fn path_is_token_client(path: &syn::Path) -> bool {
    let names = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let Some(last) = names.last() else {
        return false;
    };

    last.contains("tokenclient")
        || (last == "client"
            && names[..names.len().saturating_sub(1)]
                .iter()
                .any(|name| name.contains("token")))
}

fn is_token_client_constructor(expr: &Expr) -> bool {
    match expr {
        Expr::Call(call) => match call.func.as_ref() {
            Expr::Path(path) => {
                let names = path
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string().to_ascii_lowercase())
                    .collect::<Vec<_>>();
                names.last().map(|name| name == "new").unwrap_or(false)
                    && names[..names.len().saturating_sub(1)]
                        .iter()
                        .any(|name| name.contains("token"))
                    && names[..names.len().saturating_sub(1)]
                        .last()
                        .map(|name| name.contains("client"))
                        .unwrap_or(false)
            }
            _ => false,
        },
        Expr::Paren(paren) => is_token_client_constructor(&paren.expr),
        Expr::Reference(reference) => is_token_client_constructor(&reference.expr),
        _ => false,
    }
}

fn is_token_transfer_receiver(expr: &Expr, known_clients: &[String]) -> bool {
    if is_token_client_constructor(expr) {
        return true;
    }

    receiver_ident(expr)
        .map(|name| known_clients.iter().any(|candidate| candidate == &name))
        .unwrap_or(false)
}

fn is_soroban_storage_accessor(expr: &Expr) -> bool {
    match expr {
        Expr::MethodCall(call)
            if matches!(
                call.method.to_string().as_str(),
                "instance" | "persistent" | "temporary"
            ) =>
        {
            storage_chain_contains_storage(&call.receiver)
        }
        Expr::Paren(paren) => is_soroban_storage_accessor(&paren.expr),
        Expr::Reference(reference) => is_soroban_storage_accessor(&reference.expr),
        _ => false,
    }
}

fn storage_chain_contains_storage(expr: &Expr) -> bool {
    match expr {
        Expr::MethodCall(call) => {
            call.method == "storage" || storage_chain_contains_storage(&call.receiver)
        }
        Expr::Paren(paren) => storage_chain_contains_storage(&paren.expr),
        Expr::Reference(reference) => storage_chain_contains_storage(&reference.expr),
        _ => false,
    }
}

fn is_ttl_receiver(expr: &Expr, known_accessors: &[String]) -> bool {
    if is_soroban_storage_accessor(expr) {
        return true;
    }

    receiver_ident(expr)
        .map(|name| known_accessors.iter().any(|candidate| candidate == &name))
        .unwrap_or(false)
}

fn is_soroban_events_receiver(expr: &Expr, env_params: &[String]) -> bool {
    match expr {
        Expr::MethodCall(call) if call.method == "events" => receiver_ident(&call.receiver)
            .map(|name| env_params.iter().any(|candidate| candidate == &name))
            .unwrap_or(false),
        Expr::Reference(reference) => is_soroban_events_receiver(&reference.expr, env_params),
        Expr::Paren(paren) => is_soroban_events_receiver(&paren.expr, env_params),
        _ => false,
    }
}

fn source_for_span(source: &str, span: Span) -> Option<&str> {
    let (start, end, _) = span_to_offsets(source, span)?;
    source.get(start..end)
}

fn swapped_transfer_replacement(source: &str, node: &ExprMethodCall) -> Option<String> {
    let receiver = source_for_span(source, node.receiver.span())?;
    let mut args = node.args.iter();
    let first = source_for_span(source, args.next()?.span())?;
    let second = source_for_span(source, args.next()?.span())?;
    let third = source_for_span(source, args.next()?.span())?;

    if first.trim() == second.trim() {
        return None;
    }

    Some(format!(
        "{}.transfer({}, {}, {})",
        receiver, second, first, third
    ))
}

fn receiver_ident(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => path
            .path
            .segments
            .first()
            .map(|segment| segment.ident.to_string()),
        Expr::Reference(reference) => receiver_ident(&reference.expr),
        Expr::Paren(paren) => receiver_ident(&paren.expr),
        _ => None,
    }
}

fn span_to_offsets(source: &str, span: Span) -> Option<(usize, usize, SourceSpan)> {
    let start = span.start();
    let end = span.end();
    let start_byte = line_column_to_offset(source, start.line, start.column)?;
    let end_byte = line_column_to_offset(source, end.line, end.column)?;

    Some((
        start_byte,
        end_byte,
        SourceSpan {
            line: start.line,
            column: start.column + 1,
            end_line: end.line,
            end_column: end.column + 1,
        },
    ))
}

fn line_column_to_offset(source: &str, target_line: usize, target_column: usize) -> Option<usize> {
    if target_line == 0 {
        return None;
    }

    let mut offset = 0usize;
    for (index, line) in source.split_inclusive('\n').enumerate() {
        let line_number = index + 1;
        if line_number == target_line {
            return (target_column <= line.len()).then_some(offset + target_column);
        }
        offset += line.len();
    }

    if target_line == source.lines().count() + 1 && target_column == 0 {
        Some(source.len())
    } else {
        None
    }
}

fn ignored_entry(entry: &DirEntry, root: &Path) -> bool {
    if entry.path() == root {
        return false;
    }

    matches!(
        entry.file_name().to_str(),
        Some(
            ".git"
                | "target"
                | ".soro-mutants-target"
                | ".soro-mutants-worktree"
                | "mutants.out"
                | "node_modules"
        )
    )
}

fn target_dirs(root: &Path, target_dir: Option<&Path>) -> (PathBuf, PathBuf) {
    let base = match target_dir {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => root.join(path),
        None => root.join(".soro-mutants-target"),
    };

    (base.join("baseline"), base.join("mutants-shared"))
}

fn managed_cleanup_paths(root: &Path, target_dir: Option<&Path>) -> Result<Vec<PathBuf>> {
    let scratch = root.join(".soro-mutants-worktree");
    let (baseline, mutants) = target_dirs(root, target_dir);
    let mut paths: Vec<PathBuf> = [scratch, baseline, mutants]
        .into_iter()
        .filter(|path| path.exists())
        .collect();

    // The default cache root is also removed when no unrelated content remains.
    // Determine this without mutating anything, so a dry run is accurate.
    if target_dir.is_none() {
        let default_root = root.join(".soro-mutants-target");
        if default_root.is_dir() {
            let entries = fs::read_dir(&default_root)
                .with_context(|| format!("failed to inspect {}", default_root.display()))?
                .collect::<std::io::Result<Vec<_>>>()?;
            if entries.iter().all(|entry| {
                matches!(
                    entry.file_name().to_str(),
                    Some("baseline" | "mutants-shared")
                ) && paths.iter().any(|path| path == &entry.path())
            }) {
                paths.push(default_root);
            }
        }
    }

    Ok(paths)
}

fn clean_generated_state(
    root: &Path,
    target_dir: Option<&Path>,
    dry_run: bool,
) -> Result<Vec<PathBuf>> {
    let paths = managed_cleanup_paths(root, target_dir)?;
    if !dry_run {
        for path in &paths {
            if target_dir.is_none() && path == &root.join(".soro-mutants-target") {
                fs::remove_dir(path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
            } else {
                fs::remove_dir_all(path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
            }
        }
    }
    Ok(paths)
}

fn execute_mutant(
    root: &Path,
    mutant: &Mutant,
    test_dir: Option<&Path>,
    test_command: &str,
    timeouts: Timeouts,
    target_root: &Path,
    shared_target: bool,
) -> Result<Outcome> {
    let scratch = root.join(".soro-mutants-worktree");
    if scratch.exists() {
        fs::remove_dir_all(&scratch)?;
    }
    copy_project(root, &scratch)?;
    apply_mutant(&scratch, mutant)?;

    let result = (|| -> Result<Outcome> {
        let target_dir = if shared_target {
            target_root.to_path_buf()
        } else {
            target_root.join(&mutant.id)
        };
        let scratch_cwd = test_dir
            .map(|dir| scratch.join(dir))
            .unwrap_or_else(|| scratch.clone());

        if let Some(compile_command) = compile_command_for(test_command) {
            match run_process(
                &scratch_cwd,
                &compile_command,
                timeouts.compile,
                &target_dir,
            )? {
                ProcessResult::Failed => return Ok(Outcome::Unviable),
                ProcessResult::Timeout => return Ok(Outcome::Timeout),
                ProcessResult::Passed => {}
            }
        }

        Ok(
            match run_process(&scratch_cwd, test_command, timeouts.test, &target_dir)? {
                ProcessResult::Passed => Outcome::Survived,
                ProcessResult::Failed => Outcome::Killed,
                ProcessResult::Timeout => Outcome::Timeout,
            },
        )
    })();

    let cleanup_result = if scratch.exists() {
        fs::remove_dir_all(&scratch)
            .with_context(|| format!("failed to clean mutation scratch {}", scratch.display()))
    } else {
        Ok(())
    };

    match (result, cleanup_result) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Ok(outcome), Ok(())) => Ok(outcome),
    }
}

fn copy_project(root: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| !ignored_entry(entry, root))
    {
        let entry = entry?;
        let relative = entry.path().strip_prefix(root)?;
        if relative.as_os_str().is_empty() {
            continue;
        }

        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
        }
    }

    copy_wasm_fixtures(root, destination)?;
    Ok(())
}

fn copy_wasm_fixtures(root: &Path, destination: &Path) -> Result<()> {
    for entry in WalkDir::new(root).into_iter() {
        let entry = entry?;
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|ext| ext.to_str()) != Some("wasm")
        {
            continue;
        }

        let relative = entry.path().strip_prefix(root)?;
        let normalized = relative.to_string_lossy().replace('\\', "/");
        let is_fixture = normalized.contains("/target/wasm32-unknown-unknown/release/")
            || normalized.contains("/target/wasm32v1-none/release/")
            || normalized.contains("/target/wasm32-unknown-unknown/optimized/")
            || normalized.contains("/target/wasm32v1-none/optimized/")
            || normalized.starts_with("target/wasm32-unknown-unknown/release/")
            || normalized.starts_with("target/wasm32v1-none/release/")
            || normalized.starts_with("target/wasm32-unknown-unknown/optimized/")
            || normalized.starts_with("target/wasm32v1-none/optimized/");

        if !is_fixture {
            continue;
        }

        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(entry.path(), target)?;
    }
    Ok(())
}

fn resolve_runner_command(runner: Runner, test_command: &str) -> Result<String> {
    match runner {
        Runner::CargoTest => Ok(test_command.to_owned()),
        Runner::Nextest if test_command == "cargo test -q" => Ok("cargo nextest run".to_owned()),
        Runner::Nextest => bail!(
            "cannot combine --runner nextest with --test-command; choose one execution method"
        ),
    }
}

fn ensure_nextest_available() -> Result<()> {
    let available = Command::new("cargo")
        .args(["nextest", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !available {
        bail!(
            "cargo-nextest is not installed or cannot be executed; install it from https://nexte.st/ and retry --runner nextest"
        );
    }
    Ok(())
}

fn compile_command_for(test_command: &str) -> Option<String> {
    if test_command == "cargo nextest run" {
        return Some("cargo test --no-run".to_string());
    }
    if test_command.contains("--no-run") {
        return Some(test_command.to_string());
    }

    let cargo_index = test_command.find("cargo ")?;
    let after_cargo = cargo_index + "cargo ".len();
    let tail = &test_command[after_cargo..];

    let test_offset = if tail.starts_with("test") {
        0
    } else if tail.starts_with('+') {
        let toolchain_end = tail.find(' ')?;
        let after_toolchain = &tail[toolchain_end + 1..];
        if after_toolchain.starts_with("test") {
            toolchain_end + 1
        } else {
            return None;
        }
    } else {
        return None;
    };

    let test_start = after_cargo + test_offset;
    let test_end = test_start + "test".len();

    Some(format!(
        "{}test --no-run{}",
        &test_command[..test_start],
        &test_command[test_end..]
    ))
}

fn apply_mutant(root: &Path, mutant: &Mutant) -> Result<()> {
    let path = root.join(&mutant.file);
    let source = fs::read_to_string(&path)?;
    if mutant.end_byte > source.len() || mutant.start_byte >= mutant.end_byte {
        bail!("invalid source span for {}", mutant.id);
    }

    if source[mutant.start_byte..mutant.end_byte] != mutant.original {
        bail!(
            "source changed before applying {} at {}",
            mutant.id,
            mutant.file.display()
        );
    }

    let mut mutated = String::with_capacity(source.len() + mutant.replacement.len());
    mutated.push_str(&source[..mutant.start_byte]);
    mutated.push_str(&mutant.replacement);
    mutated.push_str(&source[mutant.end_byte..]);
    fs::write(path, mutated)?;
    Ok(())
}

fn run_process(
    cwd: &Path,
    command: &str,
    timeout_seconds: u64,
    target_dir: &Path,
) -> Result<ProcessResult> {
    fs::create_dir_all(target_dir)?;

    let mut child = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .env("CARGO_TARGET_DIR", target_dir)
        .env_remove("RUSTUP_TOOLCHAIN")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("failed to execute: {}", command))?;

    match child.wait_timeout(Duration::from_secs(timeout_seconds))? {
        Some(status) if status.success() => Ok(ProcessResult::Passed),
        Some(_) => Ok(ProcessResult::Failed),
        None => {
            let _ = child.kill();
            let _ = child.wait();
            Ok(ProcessResult::Timeout)
        }
    }
}

fn print_mutants(report: &mut dyn Write, mutants: &[Mutant]) -> Result<()> {
    writeln!(report, "Discovered {} semantic mutants\n", mutants.len())?;
    for mutant in mutants {
        writeln!(
            report,
            "{} {:<8} {}:{} {}",
            mutant.id,
            mutant.operator,
            mutant.file.display(),
            mutant.span.line,
            mutant.description
        )?;
        writeln!(report, "    - {}", mutant.original.trim())?;
        writeln!(report, "    + {}\n", mutant.replacement.trim())?;
    }
    Ok(())
}

fn print_operators(report: &mut dyn Write) -> Result<()> {
    for (id, description) in BUILTIN_OPERATORS {
        writeln!(report, "{:<10}  {}", id, description)?;
    }
    writeln!(report, "\n{} built-in operators", BUILTIN_OPERATORS.len())?;
    Ok(())
}

fn print_suppression_summary(
    report: &mut dyn Write,
    suppressed: &[SuppressedMutant],
) -> Result<()> {
    use std::collections::BTreeMap;
    let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
    for item in suppressed {
        *reasons.entry(item.reason).or_default() += 1;
    }
    writeln!(report, "Suppressed: {}", suppressed.len())?;
    for (reason, count) in reasons {
        writeln!(report, "  {reason}: {count}")?;
    }
    Ok(())
}

fn print_summary(report: &mut dyn Write, results: &[MutantResult]) -> Result<()> {
    let killed = results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Killed))
        .count();
    let survived = results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Survived))
        .count();
    let unviable = results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Unviable))
        .count();
    let timeout = results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Timeout))
        .count();
    let viable = killed + survived;
    let score = if viable == 0 {
        0.0
    } else {
        killed as f64 * 100.0 / viable as f64
    };

    writeln!(report, "\nSummary")?;
    writeln!(report, "  Generated : {}", results.len())?;
    writeln!(report, "  Killed    : {}", killed)?;
    writeln!(report, "  Survived  : {}", survived)?;
    writeln!(report, "  Unviable  : {}", unviable)?;
    writeln!(report, "  Timeout   : {}", timeout)?;
    writeln!(report, "  Score     : {:.1}%", score)?;
    Ok(())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn print_html_report(report: &mut dyn Write, results: &[MutantResult]) -> Result<()> {
    use std::collections::BTreeMap;

    let mut counts = [0usize; 4];
    let mut by_operator: BTreeMap<&str, [usize; 4]> = BTreeMap::new();
    for result in results {
        let index = match result.outcome {
            Outcome::Killed => 0,
            Outcome::Survived => 1,
            Outcome::Unviable => 2,
            Outcome::Timeout => 3,
        };
        counts[index] += 1;
        by_operator.entry(&result.mutant.operator).or_default()[index] += 1;
    }
    let viable = counts[0] + counts[1];
    let score = if viable == 0 {
        0.0
    } else {
        counts[0] as f64 * 100.0 / viable as f64
    };
    report.write_all(r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Soro Mutants Report</title>
<style>
:root { color-scheme: light; font: 16px/1.5 system-ui, sans-serif; background: #f5f7f9; color: #182b38; }
body { max-width: 1040px; margin: 2rem auto; padding: 0 1.5rem; }
h1, h2 { line-height: 1.25; }
section { padding: 1rem 1.5rem; margin: 1rem 0; background: white; border: 1px solid #dce4e9; border-radius: 10px; }
table { width: 100%; border-collapse: collapse; }
th, td { padding: .55rem; border-bottom: 1px solid #dce4e9; text-align: left; }
th { background: #edf2f5; }
article { border-top: 1px solid #dce4e9; padding: 1rem 0; }
pre { overflow-x: auto; padding: .75rem; background: #f1f4f5; border-radius: 6px; }
small { color: #516577; }
</style>
</head>
<body>
<h1>Soro Mutants Report</h1>
<section><h2>Summary</h2>
<table><thead><tr><th>Metric</th><th>Count</th></tr></thead><tbody>
"#.as_bytes())?;
    for (name, count) in [
        ("Generated", results.len()),
        ("Killed", counts[0]),
        ("Survived", counts[1]),
        ("Unviable", counts[2]),
        ("Timeout", counts[3]),
    ] {
        writeln!(report, "<tr><td>{name}</td><td>{count}</td></tr>")?;
    }
    writeln!(
        report,
        "</tbody></table><p><strong>Semantic score: {score:.1}%</strong> ({}/{viable} viable mutants killed).</p></section>",
        counts[0]
    )?;
    writeln!(report, "<section><h2>By operator</h2><table><thead><tr><th>Operator</th><th>Killed</th><th>Survived</th><th>Unviable</th><th>Timeout</th></tr></thead><tbody>")?;
    for (operator, values) in by_operator {
        writeln!(
            report,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(operator),
            values[0],
            values[1],
            values[2],
            values[3]
        )?;
    }
    writeln!(
        report,
        "</tbody></table></section><section><h2>Mutant details</h2>"
    )?;
    if results.is_empty() {
        writeln!(report, "<p>No matching mutants.</p>")?;
    }
    for result in results {
        let mutant = &result.mutant;
        writeln!(
            report,
            "<article><h3>{} — {} — {}</h3><small>{}:{} (function: {})</small><p>{}</p><p>Original</p><pre><code>{}</code></pre><p>Replacement</p><pre><code>{}</code></pre></article>",
            escape_html(&mutant.id),
            escape_html(&mutant.operator),
            outcome_label(&result.outcome),
            escape_html(&mutant.file.to_string_lossy()),
            mutant.span.line,
            escape_html(mutant.function.as_deref().unwrap_or("unknown")),
            escape_html(&mutant.description),
            escape_html(&mutant.original),
            escape_html(&mutant.replacement)
        )?;
    }
    writeln!(report, "</section></body></html>")?;
    Ok(())
}

fn print_markdown_report(report: &mut dyn Write, results: &[MutantResult]) -> Result<()> {
    let killed = results
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Killed))
        .count();
    let survived = results
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Survived))
        .count();
    let unviable = results
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Unviable))
        .count();
    let timeout = results
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Timeout))
        .count();
    let viable = killed + survived;
    let score = if viable == 0 {
        0.0
    } else {
        100.0 * killed as f64 / viable as f64
    };

    writeln!(report, "# Soro Mutants Report\n")?;
    writeln!(report, "| Metric | Count |")?;
    writeln!(report, "| --- | ---: |")?;
    for (name, count) in [
        ("Generated", results.len()),
        ("Killed", killed),
        ("Survived", survived),
        ("Unviable", unviable),
        ("Timeout", timeout),
    ] {
        writeln!(report, "| {name} | {count} |")?;
    }
    writeln!(
        report,
        "\n**Semantic score: {score:.1}%** ({killed}/{viable} viable mutants killed).\n"
    )?;

    for (heading, label, count) in [
        ("Survived", "SURVIVED", survived),
        ("Killed", "KILLED", killed),
        ("Unviable", "UNVIABLE", unviable),
        ("Timeout", "TIMEOUT", timeout),
    ] {
        writeln!(report, "## {heading} ({count})\n")?;
        if count == 0 {
            writeln!(report, "_None._\n")?;
            continue;
        }
        for result in results
            .iter()
            .filter(|r| outcome_label(&r.outcome) == label)
        {
            let mutant = &result.mutant;
            let path = mutant.file.to_string_lossy().replace('\\', "/");
            let target = path
                .replace('%', "%25")
                .replace(' ', "%20")
                .replace('(', "%28")
                .replace(')', "%29")
                .replace('#', "%23");
            let link_label = path.replace('[', "\\[").replace(']', "\\]");
            let description = mutant
                .description
                .replace(['\n', '\r'], " ")
                .replace('`', "\\`");
            writeln!(
                report,
                "- `{}` **{}** [{}:{}]({}#L{}) — {}",
                mutant.id,
                mutant.operator,
                link_label,
                mutant.span.line,
                target,
                mutant.span.line,
                description
            )?;
        }
        writeln!(report)?;
    }
    Ok(())
}

fn escape_github_annotation_property(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
        .replace(':', "%3A")
        .replace(',', "%2C")
}

fn escape_github_annotation_message(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn print_github_annotations(report: &mut dyn Write, results: &[MutantResult]) -> Result<()> {
    use std::collections::BTreeSet;

    let mut seen = BTreeSet::new();
    for result in results {
        if !matches!(result.outcome, Outcome::Survived) {
            continue;
        }
        let mutant = &result.mutant;
        let path = mutant.file.to_string_lossy().replace('\\', "/");
        let key = (
            mutant.id.clone(),
            path.clone(),
            mutant.span.line,
            mutant.span.column,
        );
        if !seen.insert(key) {
            continue;
        }
        let message = format!(
            "{} ({}) in {}: tests did not detect {} (test-quality signal, not a confirmed security issue)",
            mutant.id,
            mutant.operator,
            mutant.function.as_deref().unwrap_or("<unknown function>"),
            mutant.description
        );
        writeln!(
            report,
            "::warning file={},line={},col={},title={}::{}",
            escape_github_annotation_property(&path),
            mutant.span.line,
            mutant.span.column,
            escape_github_annotation_property(&format!("Surviving mutant {}", mutant.operator)),
            escape_github_annotation_message(&message)
        )?;
    }
    Ok(())
}

fn print_sarif_report(report: &mut dyn Write, results: &[MutantResult]) -> Result<()> {
    use std::collections::BTreeMap;

    // Surviving mutants expose test gaps, not confirmed security vulnerabilities.
    let survivors: Vec<&MutantResult> = results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Survived))
        .collect();
    let mut rules = BTreeMap::new();
    for result in &survivors {
        let operator = result.mutant.operator.as_str();
        let description = BUILTIN_OPERATORS
            .iter()
            .find(|(id, _)| *id == operator)
            .map(|(_, description)| *description)
            .unwrap_or(result.mutant.description.as_str());
        rules.insert(
            operator,
            serde_json::json!({
                "id": operator,
                "name": operator,
                "shortDescription": {"text": description},
                "properties": {"tags": ["mutation-testing", "test-quality"]}
            }),
        );
    }

    let findings: Vec<serde_json::Value> = survivors
        .iter()
        .map(|result| {
            let mutant = &result.mutant;
            let path = mutant.file.to_string_lossy().replace('\\', "/");
            serde_json::json!({
                "ruleId": mutant.operator,
                "level": "warning",
                "message": {
                    "text": format!(
                        "Surviving mutant {} ({}): tests did not detect {}. This is a test-quality signal; it does not establish a security flaw.",
                        mutant.id, mutant.operator, mutant.description
                    )
                },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": {"uri": path},
                        "region": {
                            "startLine": mutant.span.line,
                            "startColumn": mutant.span.column
                        }
                    }
                }],
                "properties": {
                    "mutantId": mutant.id,
                    "function": mutant.function,
                    "operator": mutant.operator,
                    "outcome": outcome_label(&result.outcome),
                    "description": mutant.description,
                    "original": mutant.original,
                    "replacement": mutant.replacement
                }
            })
        })
        .collect();

    let sarif = serde_json::json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "Soro Mutants",
                    "informationUri": "https://github.com/Ay-obami/soro-mutants",
                    "rules": rules.into_values().collect::<Vec<_>>()
                }
            },
            "results": findings
        }]
    });
    writeln!(report, "{}", serde_json::to_string_pretty(&sarif)?)?;
    Ok(())
}

fn outcome_label(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Killed => "KILLED",
        Outcome::Survived => "SURVIVED",
        Outcome::Unviable => "UNVIABLE",
        Outcome::Timeout => "TIMEOUT",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_suppression_recognizes_no_change_and_repeated_edit() {
        let mut candidates = discover_mutants(
            Path::new("fixtures/weak-auth"),
            None,
            None,
            None,
            &exclusion_set(Vec::new()).unwrap(),
        )
        .unwrap();
        let meaningful = candidates[0].clone();
        let mut unchanged = meaningful.clone();
        unchanged.replacement = unchanged.original.clone();
        let duplicate = meaningful.clone();
        let mut distinct = meaningful.clone();
        distinct.replacement = "some_other_edit()".to_owned();
        candidates = vec![meaningful.clone(), unchanged, duplicate, distinct];

        let (retained, reasons) = suppress_equivalent_mutants(candidates);
        assert_eq!(retained.len(), 2);
        assert!(retained
            .iter()
            .any(|m| m.replacement == "some_other_edit()"));
        assert!(retained
            .iter()
            .any(|m| m.replacement == meaningful.replacement));
        assert_eq!(
            reasons
                .iter()
                .filter(|r| r.reason == "unchanged replacement")
                .count(),
            1
        );
        assert_eq!(
            reasons
                .iter()
                .filter(|r| r.reason == "duplicate source edit")
                .count(),
            1
        );
    }

    #[test]
    fn suppression_summary_reports_total_and_reason_counts() {
        let suppressed = vec![
            SuppressedMutant {
                operator: "AUTH-001".to_owned(),
                reason: "unchanged replacement",
            },
            SuppressedMutant {
                operator: "AUTH-002".to_owned(),
                reason: "duplicate source edit",
            },
            SuppressedMutant {
                operator: "AUTH-003".to_owned(),
                reason: "duplicate source edit",
            },
        ];
        let mut output = Vec::new();
        print_suppression_summary(&mut output, &suppressed).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Suppressed: 3"));
        assert!(output.contains("duplicate source edit: 2"));
        assert!(output.contains("unchanged replacement: 1"));
    }

    #[test]
    fn equivalent_suppression_does_not_merge_edits_in_different_files_or_spans() {
        let mut candidates = discover_mutants(
            Path::new("fixtures/weak-auth"),
            None,
            None,
            None,
            &exclusion_set(Vec::new()).unwrap(),
        )
        .unwrap();
        let first = candidates.remove(0);
        let mut second = first.clone();
        second.file = PathBuf::from("other/src/lib.rs");
        let mut third = first.clone();
        third.start_byte += 1;
        let (retained, reasons) = suppress_equivalent_mutants(vec![first, second, third]);
        assert_eq!(retained.len(), 3);
        assert!(reasons.is_empty());
    }

    #[test]
    fn github_annotations_deduplicate_and_handle_files_outside_pr_diff() {
        let mutants = discover_mutants(
            Path::new("fixtures/weak-auth"),
            Some("AUTH-001"),
            None,
            None,
            &exclusion_set(Vec::new()).unwrap(),
        )
        .unwrap();
        let mut mutant = mutants.into_iter().next().unwrap();
        mutant.file = PathBuf::from("outside-pr-diff/src/lib.rs");
        let survived = MutantResult {
            mutant: mutant.clone(),
            outcome: Outcome::Survived,
        };
        let killed = MutantResult {
            mutant,
            outcome: Outcome::Killed,
        };
        let mut output = Vec::new();
        print_github_annotations(&mut output, &[survived.clone(), survived, killed]).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("file=outside-pr-diff/src/lib.rs,line=11,col=9"));
        assert!(text.contains("AUTH-001"));
    }

    #[test]
    fn github_annotation_escaping_prevents_workflow_command_injection() {
        assert_eq!(
            escape_github_annotation_property("a,b:c%\\n"),
            "a%2Cb%3Ac%25\\n"
        );
        assert_eq!(escape_github_annotation_message("a%\nb"), "a%25%0Ab");
    }

    #[test]
    fn nextest_runner_builds_its_command_and_compilation_gate() {
        let command = resolve_runner_command(Runner::Nextest, "cargo test -q").unwrap();
        assert_eq!(command, "cargo nextest run");
        assert_eq!(
            compile_command_for(&command).as_deref(),
            Some("cargo test --no-run")
        );
        assert_eq!(
            resolve_runner_command(Runner::CargoTest, "cargo test -q").unwrap(),
            "cargo test -q"
        );
    }

    #[test]
    fn nextest_runner_rejects_an_ambiguous_custom_command() {
        assert!(
            resolve_runner_command(Runner::Nextest, "cargo test --workspace")
                .unwrap_err()
                .to_string()
                .contains("cannot combine")
        );
    }

    #[test]
    fn builtin_operators_are_sorted_by_id() {
        let ids: Vec<&str> = BUILTIN_OPERATORS.iter().map(|(id, _)| *id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(
            ids, sorted,
            "BUILTIN_OPERATORS must be in ascending lexicographic order"
        );
    }

    #[test]
    fn builtin_operator_ids_are_unique() {
        let ids: Vec<&str> = BUILTIN_OPERATORS.iter().map(|(id, _)| *id).collect();
        let mut seen = std::collections::HashSet::new();
        for id in &ids {
            assert!(seen.insert(*id), "duplicate operator ID: {id}");
        }
    }

    #[test]
    fn builtin_operator_descriptions_are_non_empty() {
        for (id, desc) in BUILTIN_OPERATORS {
            assert!(!desc.is_empty(), "operator {id} has an empty description");
        }
    }

    #[test]
    fn print_operators_emits_every_id_with_count() {
        let mut buf = Vec::new();
        print_operators(&mut buf).expect("print_operators should succeed");
        let output = String::from_utf8(buf).unwrap();
        for (id, _desc) in BUILTIN_OPERATORS {
            assert!(
                output.contains(id),
                "output should contain operator id {id}"
            );
        }
        let count_line = format!("{} built-in operators", BUILTIN_OPERATORS.len());
        assert!(
            output.contains(&count_line),
            "output should contain operator count; got:\n{output}"
        );
    }

    fn report_mutant() -> Mutant {
        Mutant {
            id: "M0001".into(),
            operator: "AUTH-001".into(),
            file: PathBuf::from("src/lib.rs"),
            function: Some("guarded".into()),
            span: SourceSpan {
                line: 2,
                column: 4,
                end_line: 2,
                end_column: 23,
            },
            original: "user.require_auth()".into(),
            replacement: "()".into(),
            description: "remove user.require_auth()".into(),
            start_byte: 35,
            end_byte: 54,
        }
    }

    #[test]
    fn html_report_escapes_untrusted_mutant_source_and_description() {
        let mut mutant = report_mutant();
        mutant.original = "<script>alert('x' & \"y\")</script>".into();
        mutant.description = "<img src=x onerror=alert(1)>".into();
        let results = [MutantResult {
            mutant,
            outcome: Outcome::Survived,
        }];
        let mut output = Vec::new();
        print_html_report(&mut output, &results).unwrap();
        let html = String::from_utf8(output).unwrap();
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img"));
        assert!(
            html.contains("&lt;script&gt;alert(&#39;x&#39; &amp; &quot;y&quot;)&lt;/script&gt;")
        );
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    }

    #[test]
    fn json_list_preserves_mutant_fields() {
        let mutants = [report_mutant()];
        let report = serde_json::to_value(list_report(&mutants)).unwrap();
        assert_eq!(
            report,
            serde_json::json!({
                "schema_version": 1,
                "mutants": [{
                "id": "M0001", "operator": "AUTH-001",
                    "file": "src/lib.rs", "function": "guarded",
                "span": {"line": 2, "column": 4, "end_line": 2, "end_column": 23},
                "original": "user.require_auth()", "replacement": "()",
                "description": "remove user.require_auth()"
                }]
            })
        );
        let mut mutant = report_mutant();
        mutant.function = None;
        let report = serde_json::to_value(list_report(&[mutant])).unwrap();
        assert!(report["mutants"][0]["function"].is_null());
    }

    #[test]
    fn semantic_threshold_excludes_unviable_and_timeout_and_passes_exact_boundary() {
        let results = [
            Outcome::Killed,
            Outcome::Survived,
            Outcome::Unviable,
            Outcome::Timeout,
        ]
        .into_iter()
        .map(|outcome| MutantResult {
            mutant: report_mutant(),
            outcome,
        })
        .collect::<Vec<_>>();
        assert!(ensure_min_score(&results, Some(50.0)).is_ok());
        let failure = ensure_min_score(&results, Some(50.01)).unwrap_err();
        assert!(failure.downcast_ref::<ScoreThresholdFailure>().is_some());
        assert!(ensure_min_score(&results, None).is_ok());
    }

    #[test]
    fn json_test_preserves_results_and_outcome_labels() {
        for (outcome, label) in [
            (Outcome::Killed, "KILLED"),
            (Outcome::Survived, "SURVIVED"),
            (Outcome::Unviable, "UNVIABLE"),
            (Outcome::Timeout, "TIMEOUT"),
        ] {
            let results = [MutantResult {
                mutant: report_mutant(),
                outcome,
            }];
            let report = serde_json::to_value(test_report(&results)).unwrap();
            let example: serde_json::Value =
                serde_json::from_str(include_str!("../docs/json-report-example.json")).unwrap();
            let mut expected = example;
            expected["results"][0]["outcome"] = label.into();
            assert_eq!(report, expected);
        }
    }

    #[test]
    fn empty_json_reports_are_versioned() {
        assert_eq!(
            serde_json::to_value(list_report(&[])).unwrap(),
            serde_json::json!({"schema_version": 1, "mutants": []})
        );
        assert_eq!(
            serde_json::to_value(test_report(&[])).unwrap(),
            serde_json::json!({"schema_version": 1, "results": []})
        );
    }

    fn mutations_for(source: &str) -> Vec<Mutant> {
        let syntax = syn::parse_file(source).expect("test source should parse");
        let mut visitor = SemanticVisitor::new(source, PathBuf::from("src/lib.rs"));
        visitor.visit_file(&syntax);
        visitor.mutants
    }

    #[test]
    fn clean_removes_only_generated_state() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = env::temp_dir().join(format!("soro-mutants-clean-{nonce}"));
        fs::create_dir_all(root.join(".soro-mutants-worktree"))
            .expect("scratch directory should be created");
        fs::create_dir_all(root.join(".soro-mutants-target/baseline"))
            .expect("baseline directory should be created");
        fs::create_dir_all(root.join(".soro-mutants-target/mutants-shared"))
            .expect("mutant cache should be created");
        fs::write(root.join("keep.txt"), "source").expect("source sentinel should be written");

        let removed = clean_generated_state(&root, None, false).expect("cleanup should succeed");

        assert_eq!(removed.len(), 4);
        assert!(!root.join(".soro-mutants-worktree").exists());
        assert!(!root.join(".soro-mutants-target").exists());
        assert_eq!(
            fs::read_to_string(root.join("keep.txt")).expect("source sentinel should remain"),
            "source"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn baseline_and_mutant_targets_are_always_separate() {
        let root = Path::new("/tmp/example-project");

        let (baseline, mutants) = target_dirs(root, None);
        assert_eq!(baseline, root.join(".soro-mutants-target/baseline"));
        assert_eq!(mutants, root.join(".soro-mutants-target/mutants-shared"));
        assert_ne!(baseline, mutants);

        let (baseline, mutants) = target_dirs(root, Some(Path::new("custom-target")));
        assert_eq!(baseline, root.join("custom-target/baseline"));
        assert_eq!(mutants, root.join("custom-target/mutants-shared"));
        assert_ne!(baseline, mutants);

        let (baseline, mutants) = target_dirs(root, Some(Path::new("/tmp/shared-cache")));
        assert_eq!(baseline, PathBuf::from("/tmp/shared-cache/baseline"));
        assert_eq!(mutants, PathBuf::from("/tmp/shared-cache/mutants-shared"));
        assert_ne!(baseline, mutants);
    }

    #[test]
    fn discovers_auth_removal_and_wrong_address_mutants() {
        let mutants = mutations_for(
            r#"
            use soroban_sdk::{Address, Env};

            fn transfer(env: Env, from: Address, to: Address) {
                from.require_auth();
            }
            "#,
        );

        assert!(mutants.iter().any(|m| m.operator == "AUTH-001"));
        assert!(mutants
            .iter()
            .any(|m| { m.operator == "AUTH-002" && m.replacement.contains("to.require_auth()") }));
    }

    #[test]
    fn event_transfer_helper_is_not_treated_as_token_transfer() {
        let mutants = mutations_for(
            r#"
            fn transfer_event(e: Env, from: Address, to: Address, amount: i128) {
                TokenUtils::new(&e).events().transfer(from, to, amount);
            }
            "#,
        );

        assert!(!mutants.iter().any(|m| m.operator == "TOKEN-001"));
    }

    #[test]
    fn generic_three_argument_transfer_is_not_treated_as_token_transfer() {
        let mutants = mutations_for(
            r#"
            fn move_data(bus: MessageBus, from: Address, to: Address, amount: i128) {
                bus.transfer(&from, &to, &amount);
            }
            "#,
        );

        assert!(!mutants.iter().any(|m| m.operator == "TOKEN-001"));
    }

    #[test]
    fn local_token_client_binding_gets_token_direction_mutant() {
        let mutants = mutations_for(
            r#"
            fn pay(env: Env, token: Address, from: Address, to: Address, amount: i128) {
                let token_client = token::Client::new(&env, &token);
                token_client.transfer(&from, &to, &amount);
            }
            "#,
        );

        assert!(mutants.iter().any(|m| m.operator == "TOKEN-001"));
    }

    #[test]
    fn direct_token_client_constructor_gets_token_direction_mutant() {
        let mutants = mutations_for(
            r#"
            fn pay(env: Env, token: Address, from: Address, to: Address, amount: i128) {
                token_contract::Client::new(&env, &token).transfer(&from, &to, &amount);
            }
            "#,
        );

        assert!(mutants.iter().any(|m| m.operator == "TOKEN-001"));
    }

    #[test]
    fn direct_transfer_call_gets_token_direction_mutant() {
        let mutants = mutations_for(
            r#"
            fn pay(token: TokenClient, from: Address, to: Address, amount: i128) {
                token.transfer(&from, &to, &amount);
            }
            "#,
        );

        let mutant = mutants
            .iter()
            .find(|m| m.operator == "TOKEN-001")
            .expect("TOKEN-001 should be generated");

        assert!(mutant.replacement.contains("transfer(&to, &from, &amount)"));
    }

    #[test]
    fn call_target_mutates_only_typed_dependency_address() {
        let source = r#"
            fn route(env: Env, primary: Address, alternate: Address) -> u32 {
                env.invoke_contract::<u32>(&primary, &Symbol::new(&env, "ping"), Vec::new(&env))
            }
        "#;
        let mutants = mutations_for(source);
        let found: Vec<_> = mutants
            .iter()
            .filter(|m| m.operator == "CALL-001")
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].original, "&primary");
        assert_eq!(found[0].replacement, "&alternate");
        assert!(found[0]
            .description
            .contains("alternate instead of primary"));
    }

    #[test]
    fn call_target_ignores_untyped_or_dynamic_receivers() {
        let mutants = mutations_for(
            r#"
            fn route(env: Env, primary: Address, alternate: Address, helper: Helper) {
                helper.invoke_contract(&primary, &alternate, ());
                env.invoke_contract::<u32>(&get_target(), &Symbol::new(&env, "ping"), Vec::new(&env));
                env.invoke_contract::<u32>(&some_addr, &Symbol::new(&env, "ping"), Vec::new(&env));
            }
        "#,
        );
        assert!(!mutants.iter().any(|m| m.operator == "CALL-001"));
    }

    #[test]
    fn discovers_ttl_removal_mutant() {
        let mutants = mutations_for(
            r#"
            fn keep_alive(env: Env) {
                env.storage().instance().extend_ttl(100, 1000);
            }
            "#,
        );

        assert!(mutants.iter().any(|m| m.operator == "TTL-001"));
    }

    #[test]
    fn ttl_lifecycle_mutants_keep_call_and_key() {
        let mutants = mutations_for(
            r#"
            fn keep_alive(env: Env, key: DataKey) {
                env.storage().instance().extend_ttl(100, 1000);
                env.storage().persistent().extend_ttl(&key, 100, 1000);
            }
        "#,
        );
        assert_eq!(
            mutants.iter().filter(|m| m.operator == "TTL-002").count(),
            2
        );
        assert_eq!(
            mutants.iter().filter(|m| m.operator == "TTL-003").count(),
            2
        );
        assert!(mutants
            .iter()
            .filter(|m| m.operator == "TTL-002")
            .all(|m| m.original == "100" && m.replacement == "0"));
        assert!(mutants
            .iter()
            .filter(|m| m.operator == "TTL-003")
            .all(|m| m.original == "1000" && m.replacement == "100"));
    }

    #[test]
    fn ttl_lifecycle_skips_noop_and_non_storage_calls() {
        let mutants = mutations_for(
            r#"
            fn work(env: Env, cache: Cache) {
                env.storage().instance().extend_ttl(0, 0);
                cache.extend_ttl(100, 1000);
            }
        "#,
        );
        assert!(!mutants
            .iter()
            .any(|m| matches!(m.operator.as_str(), "TTL-002" | "TTL-003")));
    }

    #[test]
    fn generic_extend_ttl_method_is_not_treated_as_soroban_storage() {
        let mutants = mutations_for(
            r#"
            fn keep_alive(cache: Cache) {
                cache.extend_ttl(100, 1000);
            }
            "#,
        );

        assert!(!mutants.iter().any(|m| m.operator == "TTL-001"));
    }

    #[test]
    fn local_storage_accessor_gets_ttl_mutant() {
        let mutants = mutations_for(
            r#"
            fn keep_alive(env: Env, key: DataKey) {
                let persistent = env.storage().persistent();
                persistent.extend_ttl(&key, 100, 1000);
            }
            "#,
        );

        assert!(mutants.iter().any(|m| m.operator == "TTL-001"));
    }

    #[test]
    fn typed_event_publish_statement_yields_only_event_deletion() {
        let source = r#"
            #[contractevent]
            struct TransferNotice { from: Address, amount: i128 }
            fn record(env: Env, from: Address, amount: i128) {
                TransferNotice { from, amount }.publish(&env);
            }
        "#;
        let mutants = mutations_for(source);
        let typed: Vec<_> = mutants
            .iter()
            .filter(|m| m.operator == "EVENT-002")
            .collect();
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0].replacement, "()");
        assert!(typed[0]
            .original
            .contains("TransferNotice { from, amount }.publish(&env)"));
        let mutant = typed[0];
        let transformed = format!(
            "{}{}{}",
            &source[..mutant.start_byte],
            mutant.replacement,
            &source[mutant.end_byte..]
        );
        assert!(transformed.contains("();"));
        assert!(syn::parse_file(&transformed).is_ok());
    }

    #[test]
    fn typed_event_matcher_rejects_non_event_publish_and_non_statement() {
        let source = r#"
            struct FakeEvent { value: u32 }
            #[contractevent]
            struct ValidEvent { value: u32 }
            fn dispatch(env: Env, other: Logger) {
                FakeEvent { value: 1 }.publish(&env);
                ValidEvent { value: 2 }.publish(&other);
                let ignored = ValidEvent { value: 3 }.publish(&env);
                TokenHelper::new(&env).events().transfer(1, 2, 3);
            }
        "#;
        let mutants = mutations_for(source);
        assert!(!mutants.iter().any(|m| m.operator == "EVENT-002"));
        assert!(!mutants.iter().any(|m| m.operator == "TOKEN-001"));
    }

    #[test]
    fn discovers_event_publication_removal_mutant() {
        let mutants = mutations_for(
            r#"
            fn emit(env: Env) {
                env.events().publish(("admin", "changed"), ());
            }
            "#,
        );

        assert!(mutants.iter().any(|m| m.operator == "EVENT-001"));
    }

    #[test]
    fn auth_args_mutation_changes_only_argument_expression() {
        let source = r#"
            use soroban_sdk::{Address, Env, IntoVal};
            fn transfer(env: Env, admin: Address, recipient: Address) {
                admin.require_auth_for_args((recipient.clone(),).into_val(&env));
            }
        "#;
        let mutants = mutations_for(source);
        let mutant = mutants
            .iter()
            .find(|m| m.operator == "AUTH-003")
            .expect("AUTH-003 mutant");
        assert_eq!(mutant.function.as_deref(), Some("transfer"));
        assert_eq!(mutant.original, "(recipient.clone(),).into_val(&env)");
        assert_eq!(mutant.replacement, "soroban_sdk::Vec::new(&env)");
        let transformed = format!(
            "{}{}{}",
            &source[..mutant.start_byte],
            mutant.replacement,
            &source[mutant.end_byte..]
        );
        assert!(transformed.contains("admin.require_auth_for_args(soroban_sdk::Vec::new(&env));"));
        assert!(transformed.contains("use soroban_sdk::{Address, Env, IntoVal};"));
    }

    #[test]
    fn auth_args_ignores_unrelated_methods_and_unsupported_receiver_patterns() {
        let mutants = mutations_for(
            r#"
            fn unrelated(env: Env, session: Session, owner: Address) {
                session.require_auth_for_args(soroban_sdk::Vec::new(&env));
                owner.clone().require_auth_for_args(soroban_sdk::Vec::new(&env));
            }
            fn no_env(owner: Address, args: Args) {
                owner.require_auth_for_args(args);
            }
        "#,
        );
        assert!(!mutants.iter().any(|m| m.operator == "AUTH-003"));
    }

    #[test]
    fn generic_require_auth_is_not_treated_as_soroban_authorization() {
        let mutants = mutations_for(
            r#"
            fn authenticate(session: Session) {
                session.require_auth();
            }
            "#,
        );

        assert!(!mutants.iter().any(|m| m.operator == "AUTH-001"));
        assert!(!mutants.iter().any(|m| m.operator == "AUTH-002"));
    }

    #[test]
    fn auth_wrong_address_ignores_non_address_parameters() {
        let mutants = mutations_for(
            r#"
            use soroban_sdk::Address;

            fn update(admin: Address, label: String, amount: i128) {
                admin.require_auth();
            }
            "#,
        );

        assert!(mutants.iter().any(|m| m.operator == "AUTH-001"));
        assert!(!mutants.iter().any(|m| m.operator == "AUTH-002"));
    }

    #[test]
    fn auth_wrong_address_requires_a_distinct_address_parameter() {
        let mutants = mutations_for(
            r#"
            use soroban_sdk::Address;

            fn update(admin: Address) {
                admin.require_auth();
            }
            "#,
        );

        assert_eq!(
            mutants.iter().filter(|m| m.operator == "AUTH-001").count(),
            1
        );
        assert_eq!(
            mutants.iter().filter(|m| m.operator == "AUTH-002").count(),
            0
        );
    }

    #[test]
    fn generic_publish_is_not_treated_as_soroban_event_publication() {
        let mutants = mutations_for(
            r#"
            fn log(logger: Logger) {
                logger.publish("hello");
            }
            "#,
        );

        assert!(!mutants.iter().any(|m| m.operator == "EVENT-001"));
    }

    #[test]
    fn generic_events_publish_chain_is_not_treated_as_soroban_event_publication() {
        let mutants = mutations_for(
            r#"
            fn log(logger: Logger) {
                logger.events().publish("hello");
            }
            "#,
        );

        assert!(!mutants.iter().any(|m| m.operator == "EVENT-001"));
    }

    #[test]
    fn borrowed_env_event_publication_is_detected() {
        let mutants = mutations_for(
            r#"
            fn emit(env: &Env) {
                env.events().publish(("admin", "changed"), ());
            }
            "#,
        );

        assert!(mutants.iter().any(|m| m.operator == "EVENT-001"));
    }

    #[test]
    fn token_direction_mutant_preserves_complex_amount_expression() {
        let mutants = mutations_for(
            r#"
            fn pay(token: TokenClient, from: Address, to: Address, amount: i128) {
                token.transfer(&from, &to, &(amount - 1));
            }
            "#,
        );

        let mutant = mutants
            .iter()
            .find(|m| m.operator == "TOKEN-001")
            .expect("TOKEN-001 should be generated");

        assert!(mutant
            .replacement
            .contains("transfer(&to, &from, &(amount - 1))"));
    }

    #[test]
    fn token_direction_skips_identical_sender_and_recipient() {
        let mutants = mutations_for(
            r#"
            fn pay(token: TokenClient, owner: Address, amount: i128) {
                token.transfer(&owner, &owner, &amount);
            }
            "#,
        );

        assert!(!mutants.iter().any(|m| m.operator == "TOKEN-001"));
    }

    #[test]
    fn separate_timeout_overrides_do_not_change_other_phase() {
        assert_eq!(
            Timeouts::resolve(120, None, None),
            Timeouts {
                compile: 120,
                test: 120
            }
        );
        assert_eq!(
            Timeouts::resolve(120, Some(10), None),
            Timeouts {
                compile: 10,
                test: 120
            }
        );
        assert_eq!(
            Timeouts::resolve(120, None, Some(20)),
            Timeouts {
                compile: 120,
                test: 20
            }
        );
        assert_eq!(
            Timeouts::resolve(120, Some(10), Some(20)),
            Timeouts {
                compile: 10,
                test: 20
            }
        );
    }

    #[test]
    fn execute_mutant_cleans_scratch_worktree() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = env::temp_dir().join(format!("soro-mutants-runner-{nonce}"));
        let target = env::temp_dir().join(format!("soro-mutants-runner-target-{nonce}"));
        fs::create_dir_all(root.join("src")).expect("fixture directory should be created");
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "runner-fixture"
version = "0.1.0"
edition = "2021"
"#,
        )
        .expect("fixture manifest should be written");
        fs::write(
            root.join("src/lib.rs"),
            r#"
pub struct Address;

impl Address {
    pub fn require_auth(&self) {}
}

pub fn guarded(user: Address) {
    user.require_auth();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn functional_only() {
        guarded(Address);
    }
}
"#,
        )
        .expect("fixture source should be written");

        let mutants = discover_mutants(&root, Some("AUTH-001"), None, None, &GlobSet::empty())
            .expect("fixture mutation discovery should work");
        let mutant = mutants.first().expect("AUTH-001 should be discovered");

        let outcome = execute_mutant(
            &root,
            mutant,
            None,
            "cargo test -q",
            Timeouts::resolve(30, None, None),
            &target,
            true,
        )
        .expect("mutant execution should succeed");

        assert!(matches!(outcome, Outcome::Survived));
        assert!(!root.join(".soro-mutants-worktree").exists());

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn compile_gate_preserves_test_prefix_and_arguments() {
        assert_eq!(
            compile_command_for("RUSTFLAGS='-Dwarnings' cargo test -q -p pool auth").as_deref(),
            Some("RUSTFLAGS='-Dwarnings' cargo test --no-run -q -p pool auth")
        );
        assert_eq!(
            compile_command_for("cargo test --no-run -q").as_deref(),
            Some("cargo test --no-run -q")
        );
        assert_eq!(
            compile_command_for("cargo +1.91.0 test -q -p pool").as_deref(),
            Some("cargo +1.91.0 test --no-run -q -p pool")
        );
        assert_eq!(
            compile_command_for("cd contracts && cargo +stable test auth").as_deref(),
            Some("cd contracts && cargo +stable test --no-run auth")
        );
        assert_eq!(
            compile_command_for("cargo nextest run").as_deref(),
            Some("cargo test --no-run")
        );
    }
}
