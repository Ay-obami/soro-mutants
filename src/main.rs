mod command;
mod diagnostics;
mod pathguard;
mod redaction;
mod sdk_reexports;

use anyhow::{bail, Context, Result};
use clap::{ColorChoice, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};
use globset::{GlobSet, GlobSetBuilder};
use proc_macro2::Span;
use redaction::redact_environment_secrets;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use syn::{
    spanned::Spanned,
    visit::{self, Visit},
    Expr, ExprMethodCall, FnArg, ImplItemFn, Item, ItemFn, Local, Pat, Signature, Stmt, Type,
};
use wait_timeout::ChildExt;
use walkdir::{DirEntry, WalkDir};

mod filters;
use filters::{parse_exclude_glob, parse_file_filter, parse_function_filter, Shard};

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
    (
        "AUTH-006",
        "remove or redirect authorization in a recognized Soroban initialization entrypoint",
    ),
    (
        "AUTH-007",
        "remove or redirect authorization in a recognized Soroban contract upgrade entrypoint",
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
        /// Only list mutants in the specified Cargo workspace package (repeatable).
        #[arg(short = 'p', long = "package", value_name = "SPEC")]
        package: Vec<String>,
        /// Only list mutants produced by this operator ID, such as AUTH-001.
        #[arg(long, value_parser = clap::builder::PossibleValuesParser::new([
            "AUTH-001", "AUTH-002", "AUTH-003", "AUTH-004", "AUTH-005", "AUTH-006", "AUTH-007",
            "CALL-001", "CALL-002", "CALL-003", "EVENT-001", "EVENT-002", "EVENT-003", "EVENT-004", "EVENT-005",
            "STORAGE-001", "TOKEN-001", "TOKEN-002", "TOKEN-003", "TOKEN-004", "TTL-001", "TTL-002", "TTL-003",
        ]))]
        operator: Option<String>,
        /// Exclude a project-relative file or directory glob (repeatable).
        #[arg(long, value_name = "GLOB", value_parser = parse_exclude_glob)]
        exclude_path: Vec<globset::Glob>,
        /// Select shard INDEX/TOTAL (one-based), after discovery filters.
        #[arg(long, value_name = "INDEX/TOTAL")]
        shard: Option<Shard>,
        /// Only list mutants in source files ending with this path.
        #[arg(long, value_parser = parse_file_filter)]
        file: Option<String>,
        /// Only list mutants inside this function.
        #[arg(long, value_parser = parse_function_filter)]
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
        /// Only execute mutants in the specified Cargo workspace package (repeatable).
        #[arg(short = 'p', long = "package", value_name = "SPEC")]
        package: Vec<String>,
        /// Only execute mutants produced by this operator ID, such as AUTH-001.
        #[arg(long, value_parser = clap::builder::PossibleValuesParser::new([
            "AUTH-001", "AUTH-002", "AUTH-003", "AUTH-004", "AUTH-005", "AUTH-006", "AUTH-007",
            "CALL-001", "CALL-002", "CALL-003", "EVENT-001", "EVENT-002", "EVENT-003", "EVENT-004", "EVENT-005",
            "STORAGE-001", "TOKEN-001", "TOKEN-002", "TOKEN-003", "TOKEN-004", "TTL-001", "TTL-002", "TTL-003",
        ]))]
        operator: Option<String>,
        /// Exclude a project-relative file or directory glob (repeatable).
        #[arg(long, value_name = "GLOB", value_parser = parse_exclude_glob)]
        exclude_path: Vec<globset::Glob>,
        /// Select shard INDEX/TOTAL (one-based), after discovery filters.
        #[arg(long, value_name = "INDEX/TOTAL")]
        shard: Option<Shard>,
        /// Only execute mutants in source files ending with this path.
        #[arg(long, value_parser = parse_file_filter)]
        file: Option<String>,
        /// Only execute mutants inside this function.
        #[arg(long, value_parser = parse_function_filter)]
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
        /// Place mutation worktrees under DIR/.soro-mutants-worktree instead of the project root.
        #[arg(long, value_name = "DIR")]
        scratch_dir: Option<PathBuf>,
        /// Command used for the clean baseline and each viable mutant.
        #[arg(long, default_value = "cargo test -q")]
        test_command: String,
        /// Run tests using cargo test (default) or an installed cargo-nextest.
        #[arg(long, value_enum, default_value_t = Runner::CargoTest)]
        runner: Runner,
        /// Legacy timeout in seconds used for both phases unless overridden.
        #[arg(long, default_value = "120s", value_parser = parse_duration, allow_hyphen_values = true)]
        timeout: u64,
        /// Timeout in seconds for each mutant's separate compile gate.
        #[arg(long, value_parser = parse_duration, allow_hyphen_values = true)]
        compile_timeout: Option<u64>,
        /// Timeout in seconds for baseline and mutant test commands.
        #[arg(long, value_parser = parse_duration, allow_hyphen_values = true)]
        test_timeout: Option<u64>,
        /// Number of mutants to execute concurrently (1 keeps serial execution).
        /// Each worker uses its own scratch worktree and Cargo target directory.
        #[arg(short = 'j', long, value_name = "N", default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..))]
        jobs: u16,
        /// Record wall-clock timing for discovery, baseline, and executed mutants.
        #[arg(long)]
        timing: bool,
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
        /// Suppress per-mutant progress and baseline lines in text output; the final summary is still printed.
        #[arg(long)]
        no_progress: bool,
        /// Resume an interrupted mutation run from the local progress file.
        #[arg(long)]
        resume: bool,
        /// Path to local progress file for resuming interrupted runs [default: <target-dir>/progress.json].
        #[arg(long, value_name = "PATH")]
        progress_file: Option<PathBuf>,
        /// Reuse persistent mutation outcomes when the complete cache identity matches (off by default).
        #[arg(long)]
        cache: bool,
        /// Do not read or write the persistent mutation-result cache.
        #[arg(long, conflicts_with = "cache")]
        no_cache: bool,
        /// Clear persistent mutation results under --target-dir before starting this run.
        #[arg(long)]
        clear_cache: bool,
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
    /// Run the opt-in benchmark matrix (never part of unit CI; external targets need --allow-external).
    BenchmarkMatrix {
        /// Machine-readable benchmark target manifest.
        #[arg(default_value = "docs/benchmark-matrix.json")]
        manifest: PathBuf,
        /// Fetch and execute pinned external repositories. Required for kind=external targets.
        #[arg(long)]
        allow_external: bool,
        /// Execute only local fixture targets and skip every external repository.
        #[arg(long)]
        local_only: bool,
        /// Directory used to store pinned external checkouts.
        #[arg(long, default_value = ".soro-mutants-benchmarks")]
        checkout_dir: PathBuf,
        /// Timeout in seconds for each target's baseline and mutant commands.
        #[arg(long, default_value = "120s", value_parser = parse_duration, allow_hyphen_values = true)]
        timeout: u64,
        /// Only run the target with this manifest id.
        #[arg(long, value_name = "ID")]
        target: Option<String>,
        /// Emit machine-readable JSON on stdout.
        #[arg(long)]
        json: bool,
        /// Write the Markdown or JSON summary to a file instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Verify this environment against the v1.0 supported-platform policy (docs/supported-platforms.md).
    CheckPlatform {
        /// Emit machine-readable JSON on stdout.
        #[arg(long)]
        json: bool,
    },
    /// Print all built-in operator IDs and their descriptions.
    Operators {
        /// Emit a versioned machine-readable document.
        #[arg(long)]
        json: bool,
    },
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

fn completion_script(shell: CompletionShell) -> Vec<u8> {
    let mut cmd = Cli::command();
    let mut output = Vec::new();
    generate(
        Shell::from(shell),
        &mut cmd,
        "cargo-soro-mutants",
        &mut output,
    );

    if matches!(shell, CompletionShell::Bash) {
        let script = String::from_utf8_lossy(&output);
        return script
            .replace(
                r#"COMPREPLY=($(compgen -f "${cur}"))"#,
                r#"COMPREPLY=()
                    while IFS= read -r completion; do
                        COMPREPLY+=("$completion")
                    done < <(compgen -f -- "${cur}")"#,
            )
            .into_bytes();
    }

    output
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

fn normalized_report_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn serialize_portable_path<S>(path: &Path, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&normalized_report_path(path))
}

#[derive(Clone, Debug, Serialize)]
struct Mutant {
    id: String,
    operator: String,
    #[serde(serialize_with = "serialize_portable_path")]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Outcome {
    Killed,
    Survived,
    Unviable,
    Timeout,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
struct MutantTiming {
    total_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    compile_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
struct RunTiming {
    discovery_ms: u64,
    baseline_ms: u64,
    mutant_wall_ms: u64,
    compile_ms: u64,
    test_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
struct MutantResult {
    mutant: Mutant,
    outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    timing: Option<MutantTiming>,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
struct OutcomeCounts {
    generated: usize,
    killed: usize,
    survived: usize,
    unviable: usize,
    timeout: usize,
}

impl OutcomeCounts {
    fn record(&mut self, outcome: Outcome) {
        self.generated += 1;
        match outcome {
            Outcome::Killed => self.killed += 1,
            Outcome::Survived => self.survived += 1,
            Outcome::Unviable => self.unviable += 1,
            Outcome::Timeout => self.timeout += 1,
        }
    }
}

fn summary_by_operator(results: &[MutantResult]) -> BTreeMap<String, OutcomeCounts> {
    let mut summary: BTreeMap<String, OutcomeCounts> = BTreeMap::new();
    for result in results {
        summary
            .entry(result.mutant.operator.clone())
            .or_default()
            .record(result.outcome.clone());
    }
    summary
}

fn summary_by_file(results: &[MutantResult]) -> BTreeMap<String, OutcomeCounts> {
    let mut summary: BTreeMap<String, OutcomeCounts> = BTreeMap::new();
    for result in results {
        summary
            .entry(result.mutant.file.to_string_lossy().replace('\\', "/"))
            .or_default()
            .record(result.outcome.clone());
    }
    summary
}

fn summary_by_function(results: &[MutantResult]) -> BTreeMap<String, OutcomeCounts> {
    let mut summary: BTreeMap<String, OutcomeCounts> = BTreeMap::new();
    for result in results {
        if let Some(function) = result.mutant.function.as_ref() {
            summary
                .entry(function.clone())
                .or_default()
                .record(result.outcome.clone());
        }
    }
    summary
}

const JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct ListReport<'a> {
    schema_version: u32,
    mutants: &'a [Mutant],
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<SelectionMetadata>,
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
    by_operator: BTreeMap<String, OutcomeCounts>,
    by_file: BTreeMap<String, OutcomeCounts>,
    by_function: BTreeMap<String, OutcomeCounts>,
}

#[derive(Clone, Debug, Serialize)]
struct SelectionMetadata {
    operator_filter: Option<String>,
    file_filter: Option<String>,
    function_filter: Option<String>,
    shard_index: Option<usize>,
    shard_total: Option<usize>,
}

impl SelectionMetadata {
    fn capture(
        operator_filter: Option<&str>,
        file_filter: Option<&str>,
        function_filter: Option<&str>,
        shard: Option<Shard>,
    ) -> Option<Self> {
        if operator_filter.is_none()
            && file_filter.is_none()
            && function_filter.is_none()
            && shard.is_none()
        {
            return None;
        }
        Some(Self {
            operator_filter: operator_filter.map(str::to_owned),
            file_filter: file_filter.map(str::to_owned),
            function_filter: function_filter.map(str::to_owned),
            shard_index: shard.map(|value| value.index),
            shard_total: shard.map(|value| value.total),
        })
    }
}

#[derive(Debug, Serialize)]
struct RunMetadata {
    rustc_version: Option<String>,
    cargo_version: Option<String>,
    test_command: String,
    compile_timeout_seconds: u64,
    test_timeout_seconds: u64,
    operator_filter: Option<String>,
    file_filter: Option<String>,
    function_filter: Option<String>,
    shard_index: Option<usize>,
    shard_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scratch_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timing: Option<RunTiming>,
}

impl RunMetadata {
    fn capture(
        test_command: &str,
        timeouts: Timeouts,
        operator_filter: Option<&str>,
        file_filter: Option<&str>,
        function_filter: Option<&str>,
        shard: Option<Shard>,
    ) -> Self {
        Self {
            rustc_version: tool_version("rustc"),
            cargo_version: tool_version("cargo"),
            test_command: redact_environment_secrets(test_command),
            compile_timeout_seconds: timeouts.compile,
            test_timeout_seconds: timeouts.test,
            operator_filter: operator_filter.map(str::to_owned),
            file_filter: file_filter.map(str::to_owned),
            function_filter: function_filter.map(str::to_owned),
            shard_index: shard.map(|value| value.index),
            shard_total: shard.map(|value| value.total),
            scratch_dir: None,
            timing: None,
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
struct CacheIdentity {
    workspace_root: PathBuf,
    source_hash: String,
    test_command: String,
    rustc_version: Option<String>,
    cargo_version: Option<String>,
    packages: Vec<String>,
    operator: Option<String>,
}

const RESULT_CACHE_VERSION: u32 = 1;

/// Every input that may change a mutation outcome is part of this identity.
/// `docs/result-cache.md` documents each field and the reason it is included.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
struct ResultCacheIdentity {
    workspace_root: String,
    source_hash: String,
    workspace_source_hash: String,
    lockfile_hash: String,
    toolchain_file_hash: String,
    test_command: String,
    rustc_version: Option<String>,
    cargo_version: Option<String>,
    packages: Vec<String>,
    operator: Option<String>,
    file: Option<String>,
    function: Option<String>,
    test_dir: Option<String>,
    exclude_globs: Vec<String>,
    shard: Option<String>,
    changed_file: Vec<String>,
    diff_base: Option<String>,
    changed_functions: bool,
    jobs: usize,
    #[serde(default)]
    scratch_dir: Option<String>,
    execution_env_hash: String,
    compile_timeout_seconds: u64,
    test_timeout_seconds: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PersistentResultCache {
    version: u32,
    identity: ResultCacheIdentity,
    /// Digest of `entries`; any out-of-band edit invalidates the whole cache.
    entries_hash: String,
    entries: Vec<CompletedMutant>,
}

impl PersistentResultCache {
    fn empty(identity: ResultCacheIdentity) -> Self {
        Self {
            version: RESULT_CACHE_VERSION,
            identity,
            entries_hash: Self::hash_entries(&[]),
            entries: Vec::new(),
        }
    }

    fn hash_entries(entries: &[CompletedMutant]) -> String {
        let Ok(json) = serde_json::to_string(entries) else {
            return "unserializable".to_owned();
        };
        let mut hasher = DeterministicHasher::new();
        hasher.write_str(&json);
        hasher.hex()
    }

    fn refresh_hash(&mut self) {
        self.entries_hash = Self::hash_entries(&self.entries);
    }

    fn find(&self, mutant: &Mutant) -> Option<&CompletedMutant> {
        self.entries.iter().find(|entry| {
            entry.operator == mutant.operator
                && entry.file == mutant.file
                && entry.line == mutant.span.line
                && entry.original == mutant.original
                && entry.replacement == mutant.replacement
        })
    }

    fn record(&mut self, mutant: &Mutant, outcome: Outcome) {
        // Timeouts depend on host load and are never reused as cached evidence.
        if matches!(outcome, Outcome::Timeout) {
            return;
        }
        if let Some(existing) = self.entries.iter_mut().find(|entry| {
            entry.operator == mutant.operator
                && entry.file == mutant.file
                && entry.line == mutant.span.line
                && entry.original == mutant.original
                && entry.replacement == mutant.replacement
        }) {
            existing.outcome = outcome;
            existing.id = mutant.id.clone();
        } else {
            self.entries.push(CompletedMutant {
                id: mutant.id.clone(),
                operator: mutant.operator.clone(),
                file: mutant.file.clone(),
                line: mutant.span.line,
                original: mutant.original.clone(),
                replacement: mutant.replacement.clone(),
                outcome,
            });
        }
        self.refresh_hash();
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RunProgress {
    version: u32,
    run_identity: CacheIdentity,
    completed: Vec<CompletedMutant>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct CompletedMutant {
    id: String,
    operator: String,
    file: PathBuf,
    line: usize,
    original: String,
    replacement: String,
    outcome: Outcome,
}

impl RunProgress {
    fn new(run_identity: CacheIdentity) -> Self {
        Self {
            version: 1,
            run_identity,
            completed: Vec::new(),
        }
    }

    fn find_completed(&self, mutant: &Mutant) -> Option<&CompletedMutant> {
        self.completed.iter().find(|c| {
            c.operator == mutant.operator
                && c.file == mutant.file
                && c.line == mutant.span.line
                && c.original == mutant.original
                && c.replacement == mutant.replacement
        })
    }

    fn record_completed(&mut self, mutant: &Mutant, outcome: Outcome) {
        if let Some(pos) = self.completed.iter().position(|c| {
            c.operator == mutant.operator
                && c.file == mutant.file
                && c.line == mutant.span.line
                && c.original == mutant.original
                && c.replacement == mutant.replacement
        }) {
            self.completed[pos].outcome = outcome;
        } else {
            self.completed.push(CompletedMutant {
                id: mutant.id.clone(),
                operator: mutant.operator.clone(),
                file: mutant.file.clone(),
                line: mutant.span.line,
                original: mutant.original.clone(),
                replacement: mutant.replacement.clone(),
                outcome,
            });
        }
    }
}

fn load_and_validate_progress(path: &Path, expected: &CacheIdentity) -> Result<RunProgress> {
    if !path.exists() {
        bail!(
            "cannot resume: progress file not found at {}",
            path.display()
        );
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("cannot read progress file at {}", path.display()))?;

    let progress: RunProgress = serde_json::from_str(&content).with_context(|| {
        format!(
            "cannot resume: malformed progress file at {}",
            path.display()
        )
    })?;

    if progress.version != 1 {
        bail!(
            "cannot resume: unsupported progress file version {}",
            progress.version
        );
    }

    let prev = &progress.run_identity;
    if prev.workspace_root != expected.workspace_root {
        bail!(
            "cannot resume: workspace changed from '{}' to '{}'",
            prev.workspace_root.display(),
            expected.workspace_root.display()
        );
    }
    if prev.source_hash != expected.source_hash {
        bail!("cannot resume: source files have changed since the previous run");
    }
    if prev.test_command != expected.test_command {
        bail!(
            "cannot resume: test command changed from '{}' to '{}'",
            prev.test_command,
            expected.test_command
        );
    }
    if prev.rustc_version != expected.rustc_version || prev.cargo_version != expected.cargo_version
    {
        bail!("cannot resume: toolchain changed since the previous run");
    }
    if prev.packages != expected.packages {
        bail!("cannot resume: package selection changed since the previous run");
    }
    if prev.operator != expected.operator {
        bail!("cannot resume: operator configuration changed since the previous run");
    }

    Ok(progress)
}

fn save_progress_atomic(path: &Path, progress: &RunProgress) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));
    let json = serde_json::to_string_pretty(progress)?;
    fs::write(&tmp_path, json)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[derive(Default)]
struct DeterministicHasher(u64);

impl DeterministicHasher {
    fn new() -> Self {
        Self(0xcbf29ce484222325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= byte as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    fn write_str(&mut self, s: &str) {
        self.write(s.as_bytes());
        self.write(&[0xff]);
    }

    fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

fn compute_source_hash(root: &Path, package_dirs: Option<&[PathBuf]>) -> Result<String> {
    let mut hasher = DeterministicHasher::new();
    let mut files = Vec::new();

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !ignored_entry(e, root))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().is_file() {
            let path = entry.path();
            let is_rs = path.extension().and_then(|ext| ext.to_str()) == Some("rs");
            let is_manifest = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == "Cargo.toml" || n == "Cargo.lock");
            if is_rs || is_manifest {
                if let Some(dirs) = package_dirs {
                    if !dirs.iter().any(|d| path.starts_with(d)) {
                        continue;
                    }
                }
                if let Ok(rel) = path.strip_prefix(root) {
                    files.push((rel.to_path_buf(), path.to_path_buf()));
                }
            }
        }
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));

    for (rel, abs) in files {
        hasher.write_str(&rel.to_string_lossy());
        if let Ok(meta) = fs::metadata(&abs) {
            hasher.write(&meta.len().to_le_bytes());
        }
        if let Ok(bytes) = fs::read(&abs) {
            hasher.write(&bytes);
        }
    }

    Ok(hasher.hex())
}

fn hash_optional_file(root: &Path, names: &[&str]) -> String {
    let mut hasher = DeterministicHasher::new();
    let mut found = false;
    for name in names {
        let path = root.join(name);
        if path.is_file() {
            hasher.write_str(name);
            if let Ok(bytes) = fs::read(&path) {
                hasher.write(&bytes);
            }
            found = true;
            break;
        }
    }
    if found {
        hasher.hex()
    } else {
        "absent".to_owned()
    }
}

fn lockfile_hash(root: &Path) -> String {
    hash_optional_file(root, &["Cargo.lock"])
}

fn toolchain_file_hash(root: &Path) -> String {
    hash_optional_file(root, &["rust-toolchain.toml", "rust-toolchain"])
}

fn result_cache_path(cache_base: &Path) -> PathBuf {
    cache_base.join("result-cache.json")
}

fn load_result_cache(path: &Path, expected: &ResultCacheIdentity) -> Result<PersistentResultCache> {
    if !path.exists() {
        return Ok(PersistentResultCache::empty(expected.clone()));
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("cannot read result cache at {}", path.display()))?;
    let cache: PersistentResultCache = match serde_json::from_str(&content) {
        Ok(cache) => cache,
        Err(_) => {
            diagnostics::warning(&format!(
                "result cache miss: ignoring malformed cache at {}",
                path.display()
            ));
            return Ok(PersistentResultCache::empty(expected.clone()));
        }
    };
    if cache.version != RESULT_CACHE_VERSION {
        diagnostics::warning(&format!(
            "result cache miss: unsupported cache version {} at {}",
            cache.version,
            path.display()
        ));
        return Ok(PersistentResultCache::empty(expected.clone()));
    }
    if cache.identity != *expected {
        diagnostics::warning("result cache miss: cache identity does not match the current run");
        return Ok(PersistentResultCache::empty(expected.clone()));
    }
    if cache.entries_hash != PersistentResultCache::hash_entries(&cache.entries) {
        diagnostics::warning("result cache miss: stored outcomes failed their integrity check");
        return Ok(PersistentResultCache::empty(expected.clone()));
    }
    Ok(cache)
}

fn save_result_cache(path: &Path, cache: &mut PersistentResultCache) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    cache.refresh_hash();
    let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));
    let json = serde_json::to_string_pretty(&*cache)?;
    fs::write(&tmp_path, json)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Environment variables that can change how a test command behaves even when
/// the command string itself is identical. Values are hashed, never logged.
fn execution_env_hash() -> String {
    const KEYS: &[&str] = &[
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_BUILD_RUSTFLAGS",
        "CARGO_BUILD_TARGET",
        "RUSTC_WRAPPER",
        "RUST_TEST_THREADS",
        "RUSTUP_TOOLCHAIN",
    ];
    let mut hasher = DeterministicHasher::new();
    for key in KEYS {
        if let Some(value) = env::var_os(key) {
            hasher.write_str(key);
            hasher.write_str(&value.to_string_lossy());
        }
    }
    hasher.hex()
}

fn validate_and_isolate_cache(
    cache_base: &Path,
    baseline: &Path,
    mutants: &Path,
    parallel_mutants: &Path,
    identity: &CacheIdentity,
) -> Result<()> {
    // #184: every path this function may delete must be a managed child of the
    // resolved cache root, never a symlink pointing outside it.
    pathguard::require_owned_cache_path(&cache_base.join("cache-identity.json"), cache_base)?;
    pathguard::require_owned_cache_path(baseline, cache_base)?;
    pathguard::require_owned_cache_path(mutants, cache_base)?;
    pathguard::require_owned_cache_path(parallel_mutants, cache_base)?;
    fs::create_dir_all(cache_base)?;
    let identity_path = cache_base.join("cache-identity.json");

    let is_stale = if identity_path.exists() {
        if let Ok(content) = fs::read_to_string(&identity_path) {
            if let Ok(existing) = serde_json::from_str::<CacheIdentity>(&content) {
                &existing != identity
            } else {
                true
            }
        } else {
            true
        }
    } else {
        false
    };

    if is_stale {
        // Errors are surfaced instead of silently discarded: a failed removal
        // must never leave a stale cache that is mistaken for a valid one.
        if baseline.exists() {
            fs::remove_dir_all(baseline)
                .with_context(|| format!("failed to remove stale cache {}", baseline.display()))?;
        }
        if mutants.exists() {
            fs::remove_dir_all(mutants)
                .with_context(|| format!("failed to remove stale cache {}", mutants.display()))?;
        }
        if parallel_mutants.exists() {
            fs::remove_dir_all(parallel_mutants).with_context(|| {
                format!(
                    "failed to remove stale cache {}",
                    parallel_mutants.display()
                )
            })?;
        }
    }

    let json = serde_json::to_string_pretty(identity)?;
    fs::write(&identity_path, json)?;

    fs::create_dir_all(baseline)?;
    fs::create_dir_all(mutants)?;

    let baseline_marker = baseline.join(".soro-baseline-artifact");
    let mutant_in_baseline = baseline.join(".soro-mutant-artifact");
    let mutant_marker = mutants.join(".soro-mutant-artifact");
    let baseline_in_mutant = mutants.join(".soro-baseline-artifact");

    if mutant_in_baseline.exists() {
        fs::remove_dir_all(baseline)
            .with_context(|| format!("failed to reset cache {}", baseline.display()))?;
        fs::create_dir_all(baseline)?;
    }
    if baseline_in_mutant.exists() {
        fs::remove_dir_all(mutants)
            .with_context(|| format!("failed to reset cache {}", mutants.display()))?;
        fs::create_dir_all(mutants)?;
    }

    fs::write(&baseline_marker, "baseline")?;
    fs::write(&mutant_marker, "mutant")?;

    Ok(())
}

#[derive(Clone, Debug)]
struct WorkspacePackage {
    name: String,
    package_dir: PathBuf,
}

fn discover_workspace_packages(root: &Path) -> Result<Vec<WorkspacePackage>> {
    let manifest_path = root.join("Cargo.toml");
    if !manifest_path.exists() {
        return Ok(Vec::new());
    }

    let output = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(&manifest_path)
        .current_dir(root)
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                if let Some(packages) = val.get("packages").and_then(|p| p.as_array()) {
                    let mut result = Vec::new();
                    for pkg in packages {
                        if let (Some(name), Some(manifest)) = (
                            pkg.get("name").and_then(|n| n.as_str()),
                            pkg.get("manifest_path").and_then(|m| m.as_str()),
                        ) {
                            let manifest_buf = PathBuf::from(manifest);
                            let pkg_dir = manifest_buf.parent().unwrap_or(root).to_path_buf();
                            result.push(WorkspacePackage {
                                name: name.to_string(),
                                package_dir: pkg_dir,
                            });
                        }
                    }
                    if !result.is_empty() {
                        return Ok(result);
                    }
                }
            }
        }
    }

    // Fallback: parse manifest directly
    let mut packages = Vec::new();
    let content = fs::read_to_string(&manifest_path).unwrap_or_default();
    if let Some(name) = parse_package_name_from_toml(&content) {
        packages.push(WorkspacePackage {
            name,
            package_dir: root.to_path_buf(),
        });
    }

    // Also look for workspace members if [workspace]
    if let Some(members) = parse_workspace_members_from_toml(&content) {
        for member in members {
            let member_dir = root.join(&member);
            let member_manifest = member_dir.join("Cargo.toml");
            if member_manifest.exists() {
                if let Ok(member_content) = fs::read_to_string(&member_manifest) {
                    if let Some(name) = parse_package_name_from_toml(&member_content) {
                        packages.push(WorkspacePackage {
                            name,
                            package_dir: member_dir,
                        });
                    }
                }
            }
        }
    }

    Ok(packages)
}

fn parse_package_name_from_toml(content: &str) -> Option<String> {
    let mut in_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package && trimmed.starts_with("name") {
            if let Some((_, val)) = trimmed.split_once('=') {
                return Some(val.trim().trim_matches('"').trim_matches('\'').to_string());
            }
        }
    }
    None
}

fn parse_workspace_members_from_toml(content: &str) -> Option<Vec<String>> {
    let mut in_workspace = false;
    let mut in_members = false;
    let mut members = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_workspace = trimmed == "[workspace]";
            in_members = false;
            continue;
        }
        if in_workspace && trimmed.starts_with("members") {
            if let Some((_, val)) = trimmed.split_once('=') {
                let val = val.trim();
                if val.starts_with('[') {
                    in_members = true;
                }
            }
        }
        if in_members {
            for part in trimmed.split(',') {
                let clean = part
                    .trim()
                    .trim_matches('[')
                    .trim_matches(']')
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'');
                if !clean.is_empty() {
                    members.push(clean.to_string());
                }
            }
            if trimmed.ends_with(']') {
                in_members = false;
            }
        }
    }

    if members.is_empty() {
        None
    } else {
        Some(members)
    }
}

fn resolve_package_filter(root: &Path, selected: &[String]) -> Result<Option<Vec<PathBuf>>> {
    if selected.is_empty() {
        return Ok(None);
    }

    let all_packages = discover_workspace_packages(root)?;
    let mut selected_dirs = Vec::new();
    for name in selected {
        if let Some(pkg) = all_packages.iter().find(|p| &p.name == name) {
            let canonical = pkg
                .package_dir
                .canonicalize()
                .unwrap_or_else(|_| pkg.package_dir.clone());
            selected_dirs.push(canonical);
        } else {
            bail!("package `{}` not found in workspace", name);
        }
    }

    Ok(Some(selected_dirs))
}

fn list_report(mutants: &[Mutant], metadata: Option<SelectionMetadata>) -> ListReport<'_> {
    ListReport {
        schema_version: JSON_SCHEMA_VERSION,
        mutants,
        metadata,
        suppression: None,
    }
}

fn test_report(results: &[MutantResult]) -> TestReport<'_> {
    TestReport {
        schema_version: JSON_SCHEMA_VERSION,
        results,
        metadata: None,
        suppression: None,
        by_operator: summary_by_operator(results),
        by_file: summary_by_file(results),
        by_function: summary_by_function(results),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessResult {
    Passed,
    Failed,
    Timeout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessFailureKind {
    Compile,
    Test,
    CommandNotFound,
    Spawn,
}

impl ProcessFailureKind {
    fn label(self) -> &'static str {
        match self {
            Self::Compile => "compile_failure",
            Self::Test => "test_failure",
            Self::CommandNotFound => "command_not_found",
            Self::Spawn => "spawn_failure",
        }
    }
}

#[derive(Debug)]
struct ProcessExecution {
    result: ProcessResult,
    failure_kind: Option<ProcessFailureKind>,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
}

#[derive(Clone, Debug)]
struct MutantExecution {
    outcome: Outcome,
    timing: Option<MutantTiming>,
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LifecycleKind {
    Initialization,
    Upgrade,
}

struct FunctionState {
    current_function: Option<String>,
    lifecycle: Option<LifecycleKind>,
    address_params: Vec<String>,
    address_vec_params: Vec<String>,
    address_option_params: Vec<String>,
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
    lifecycle: Option<LifecycleKind>,
    address_params: Vec<String>,
    address_vec_params: Vec<String>,
    address_option_params: Vec<String>,
    token_client_vars: Vec<String>,
    contract_client_vars: Vec<String>,
    storage_accessor_vars: Vec<String>,
    numeric_params: Vec<String>,
    env_params: Vec<String>,
    typed_event_structs: std::collections::HashSet<String>,
    typed_event_topic_fields: std::collections::HashMap<String, std::collections::HashSet<String>>,
    auth_helpers: std::collections::HashMap<String, AuthHelper>,
    module_depth: usize,
    module_path: Vec<String>,
    impl_type: Option<String>,
    impl_method_collisions: std::collections::HashSet<String>,
    local_bindings: std::collections::HashSet<String>,
    mutants: Vec<Mutant>,
}

impl<'a> SemanticVisitor<'a> {
    fn new(source: &'a str, file: PathBuf) -> Self {
        Self {
            source,
            file,
            current_function: None,
            lifecycle: None,
            address_params: Vec::new(),
            address_vec_params: Vec::new(),
            address_option_params: Vec::new(),
            token_client_vars: Vec::new(),
            contract_client_vars: Vec::new(),
            storage_accessor_vars: Vec::new(),
            numeric_params: Vec::new(),
            env_params: Vec::new(),
            typed_event_structs: std::collections::HashSet::new(),
            typed_event_topic_fields: std::collections::HashMap::new(),
            auth_helpers: std::collections::HashMap::new(),
            module_depth: 0,
            module_path: Vec::new(),
            impl_type: None,
            impl_method_collisions: std::collections::HashSet::new(),
            local_bindings: std::collections::HashSet::new(),
            mutants: Vec::new(),
        }
    }

    fn enter_signature(&mut self, sig: &Signature, body: &syn::Block) -> FunctionState {
        let mut bindings = BindingNames::default();
        bindings.visit_block(body);
        FunctionState {
            local_bindings: std::mem::replace(&mut self.local_bindings, bindings.0),
            current_function: self.current_function.replace({
                let name = sig.ident.to_string();
                let mut parts = self.module_path.clone();
                let collision_key = if parts.is_empty() {
                    name.clone()
                } else {
                    format!("{}::{name}", parts.join("::"))
                };
                if self.impl_method_collisions.contains(&collision_key) {
                    if let Some(impl_type) = &self.impl_type {
                        parts.push(impl_type.clone());
                    }
                }
                parts.push(name);
                parts.join("::")
            }),
            lifecycle: std::mem::replace(&mut self.lifecycle, recognized_lifecycle(sig, body)),
            address_params: std::mem::replace(&mut self.address_params, address_params(sig)),
            address_vec_params: std::mem::replace(
                &mut self.address_vec_params,
                address_vec_params(sig),
            ),
            address_option_params: std::mem::replace(
                &mut self.address_option_params,
                address_option_params(sig),
            ),
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
        self.lifecycle = previous.lifecycle;
        self.address_params = previous.address_params;
        self.address_vec_params = previous.address_vec_params;
        self.address_option_params = previous.address_option_params;
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
        self.impl_method_collisions = impl_method_collisions(node);
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
        let is_inline = node.content.is_some();
        if is_inline {
            self.module_path.push(node.ident.to_string());
        }
        visit::visit_item_mod(self, node);
        if is_inline {
            self.module_path.pop();
        }
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
                        let event_name = event.path.get_ident().map(ToString::to_string);
                        let topic_fields = event_name
                            .as_ref()
                            .and_then(|name| self.typed_event_topic_fields.get(name))
                            .cloned()
                            .unwrap_or_default();
                        for field in &event.fields {
                            let field_name = match &field.member {
                                syn::Member::Named(name) => name.to_string(),
                                syn::Member::Unnamed(_) => continue,
                            };

                            if let Some(original_actor) = receiver_ident(&field.expr) {
                                if self.address_params.contains(&original_actor) {
                                    for alternate in self
                                        .address_params
                                        .iter()
                                        .filter(|name| *name != &original_actor)
                                        .cloned()
                                        .collect::<Vec<_>>()
                                    {
                                        let replacement = if field.colon_token.is_none() {
                                            format!("{field_name}: {alternate}")
                                        } else {
                                            alternate.clone()
                                        };
                                        self.push_mutant(
                                            "EVENT-003",
                                            field.expr.span(),
                                            replacement,
                                            format!(
                                                "emit {alternate} instead of {original_actor} in event field {field_name}"
                                            ),
                                        );
                                    }
                                }
                            }

                            if let Some(original_value) = receiver_ident(&field.expr) {
                                if self.numeric_params.contains(&original_value) {
                                    let replacement = if field.colon_token.is_none() {
                                        format!("{field_name}: 0")
                                    } else {
                                        "0".to_owned()
                                    };
                                    self.push_mutant(
                                        "EVENT-004",
                                        field.expr.span(),
                                        replacement,
                                        format!(
                                            "emit zero instead of {original_value} in event field {field_name}"
                                        ),
                                    );
                                }
                            }

                            if topic_fields.contains(&field_name) {
                                if let Some(original_topic) = receiver_ident(&field.expr) {
                                    if self.address_params.contains(&original_topic) {
                                        for alternate in self
                                            .address_params
                                            .iter()
                                            .filter(|name| *name != &original_topic)
                                            .cloned()
                                            .collect::<Vec<_>>()
                                        {
                                            let replacement = if field.colon_token.is_none() {
                                                format!("{field_name}: {alternate}")
                                            } else {
                                                alternate.clone()
                                            };
                                            self.push_mutant(
                                                "EVENT-005",
                                                field.expr.span(),
                                                replacement,
                                                format!(
                                                    "emit topic {alternate} instead of {original_topic} in event field {field_name}"
                                                ),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        visit::visit_stmt(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        let previous = self.enter_signature(&node.sig, &node.block);
        self.push_stored_authority_mutants(&node.block);
        visit::visit_item_fn(self, node);
        self.leave_signature(previous);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let previous = self.impl_type.replace(type_display_name(&node.self_ty));
        visit::visit_item_impl(self, node);
        self.impl_type = previous;
    }

    fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
        let previous = self.enter_signature(&node.sig, &node.block);
        self.push_stored_authority_mutants(&node.block);
        visit::visit_impl_item_fn(self, node);
        self.leave_signature(previous);
    }

    fn visit_local(&mut self, node: &'ast Local) {
        if let Some((name, init)) = local_binding_name_and_init(node) {
            if is_token_client_constructor(init) && !self.token_client_vars.contains(&name) {
                self.token_client_vars.push(name.clone());
            }

            if is_generated_contract_client_constructor(init)
                && !self.contract_client_vars.contains(&name)
            {
                self.contract_client_vars.push(name.clone());
            }

            if is_soroban_storage_accessor(init) && !self.storage_accessor_vars.contains(&name) {
                self.storage_accessor_vars.push(name);
            }
        }

        visit::visit_local(self, node);
    }

    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        let previous = self.address_params.clone();
        let mut shadowed = std::collections::HashSet::new();
        let mut typed_address_bindings = Vec::new();

        for input in &node.inputs {
            let mut bindings = BindingNames::default();
            bindings.visit_pat(input);
            shadowed.extend(bindings.0);

            if let Pat::Type(pat_type) = input {
                if type_is_address(&pat_type.ty) {
                    if let Pat::Ident(ident) = pat_type.pat.as_ref() {
                        typed_address_bindings.push(ident.ident.to_string());
                    }
                }
            }
        }

        self.address_params
            .retain(|candidate| !shadowed.contains(candidate));
        for binding in typed_address_bindings {
            if !self.address_params.contains(&binding) {
                self.address_params.push(binding);
            }
        }

        for input in &node.inputs {
            visit::visit_pat(self, input);
        }
        visit::visit_expr(self, &node.body);
        self.address_params = previous;
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        let old_len = self.address_params.len();
        let loop_binding = match (node.pat.as_ref(), node.expr.as_ref()) {
            (Pat::Ident(binding), Expr::MethodCall(call))
                if call.method == "iter" && call.args.is_empty() =>
            {
                receiver_ident(&call.receiver).and_then(|receiver| {
                    self.address_vec_params
                        .iter()
                        .any(|candidate| candidate == &receiver)
                        .then(|| binding.ident.to_string())
                })
            }
            _ => None,
        };
        if let Some(binding) = loop_binding {
            if !self.address_params.contains(&binding) {
                self.address_params.push(binding);
            }
        }

        visit::visit_expr_for_loop(self, node);
        self.address_params.truncate(old_len);
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        // Match-arm bindings are lexical: visit the scrutinee once, then visit
        // each arm with a temporary candidate scope that is discarded before
        // the next arm. We only infer bindings from Option<Address> parameters;
        // arbitrary pattern type inference remains intentionally out of scope.
        visit::visit_expr(self, &node.expr);
        let option_address = receiver_ident(&node.expr)
            .is_some_and(|name| self.address_option_params.contains(&name));

        for arm in &node.arms {
            let old_len = self.address_params.len();
            if option_address {
                if let Some(binding) = option_address_binding(&arm.pat) {
                    if !self.address_params.contains(&binding) {
                        self.address_params.push(binding);
                    }
                }
            }

            visit::visit_pat(self, &arm.pat);
            if let Some((_, guard)) = &arm.guard {
                visit::visit_expr(self, guard);
            }
            visit::visit_expr(self, &arm.body);
            self.address_params.truncate(old_len);
        }
    }

    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        if let Expr::Binary(binary) = node.cond.as_ref() {
            if matches!(binary.op, syn::BinOp::Ne(_))
                && (is_generated_contract_client_method_call(
                    &binary.left,
                    &self.contract_client_vars,
                ) || is_generated_contract_client_method_call(
                    &binary.right,
                    &self.contract_client_vars,
                ))
            {
                self.push_mutant(
                    "CALL-003",
                    binary.span(),
                    "false".to_owned(),
                    "bypass generated contract-client return mismatch check".to_owned(),
                );
            }
        }

        if let Expr::Let(let_expr) = node.cond.as_ref() {
            // A simple `if let Some(binding) = option_address` introduces an
            // Address candidate only in the then branch. Visit the scrutinee
            // outside that temporary scope and restore the candidate set
            // before visiting the else branch.
            visit::visit_expr(self, &let_expr.expr);
            let old_len = self.address_params.len();
            let option_address = receiver_ident(&let_expr.expr)
                .is_some_and(|name| self.address_option_params.contains(&name));
            if option_address {
                if let Some(binding) = option_address_binding(&let_expr.pat) {
                    if !self.address_params.contains(&binding) {
                        self.address_params.push(binding);
                    }
                }
            }
            visit::visit_pat(self, &let_expr.pat);
            visit::visit_block(self, &node.then_branch);
            self.address_params.truncate(old_len);
            if let Some((_, else_expr)) = &node.else_branch {
                visit::visit_expr(self, else_expr);
            }
            return;
        }

        visit::visit_expr_if(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        if matches!(
            node.method.to_string().as_str(),
            "instance" | "persistent" | "temporary"
        ) && is_direct_env_storage_receiver(&node.receiver, &self.env_params)
        {
            let original = node.method.to_string();
            if let Some(receiver) = source_for_span(self.source, node.receiver.span()) {
                for alternate in ["instance", "persistent", "temporary"]
                    .into_iter()
                    .filter(|candidate| *candidate != original)
                {
                    self.push_mutant(
                        "STORAGE-001",
                        node.span(),
                        format!("{receiver}.{alternate}()"),
                        format!("use {alternate} storage instead of {original}"),
                    );
                }
            }
        }

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

        if receiver_ident(&node.receiver)
            .is_some_and(|name| self.contract_client_vars.contains(&name))
        {
            for argument in &node.args {
                if let Expr::Reference(reference) = argument {
                    if let Some(original) = receiver_ident(&reference.expr) {
                        let candidates = if self.address_params.contains(&original) {
                            self.address_params
                                .iter()
                                .filter(|name| *name != &original)
                                .cloned()
                                .collect::<Vec<_>>()
                        } else if self.numeric_params.contains(&original) {
                            self.numeric_params
                                .iter()
                                .filter(|name| *name != &original)
                                .cloned()
                                .collect::<Vec<_>>()
                        } else {
                            Vec::new()
                        };
                        for alternate in candidates {
                            self.push_mutant(
                                "CALL-002",
                                argument.span(),
                                format!("&{alternate}"),
                                format!(
                                    "pass {alternate} instead of {original} to generated contract client call"
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

            if let Some(original_client) = receiver_ident(&node.receiver) {
                if self.token_client_vars.contains(&original_client) {
                    for alternate in self
                        .token_client_vars
                        .iter()
                        .filter(|name| *name != &original_client)
                        .cloned()
                        .collect::<Vec<_>>()
                    {
                        self.push_mutant(
                            "TOKEN-004",
                            node.receiver.span(),
                            alternate.clone(),
                            format!("use token client {alternate} instead of {original_client}"),
                        );
                    }
                }
            }

            if let Some(amount_arg) = node.args.iter().nth(2) {
                if let Expr::Reference(reference) = amount_arg {
                    if let Some(amount_name) = receiver_ident(&reference.expr) {
                        if self.numeric_params.contains(&amount_name) {
                            for (replacement, label) in [("&0", "zero"), ("&1", "one")] {
                                self.push_mutant(
                                    "TOKEN-003",
                                    amount_arg.span(),
                                    replacement.to_owned(),
                                    format!("transfer {label} instead of {amount_name}"),
                                );
                            }
                        }
                    }
                }
            }
        }

        if let Some((role_a, role_b)) =
            allowance_role_labels(&node.method.to_string(), node.args.len())
        {
            if is_token_transfer_receiver(&node.receiver, &self.token_client_vars) {
                self.push_allowance_role_swap(node, role_a, role_b);
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
                    let Some(arguments) = node.args.first() else {
                        return;
                    };
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

        if node.method == "require_auth" && node.args.is_empty() {
            if let Some(kind) = self.lifecycle {
                if let Some(receiver) = receiver_ident(&node.receiver) {
                    if self.address_params.contains(&receiver) {
                        let operator = match kind {
                            LifecycleKind::Initialization => "AUTH-006",
                            LifecycleKind::Upgrade => "AUTH-007",
                        };
                        let action = match kind {
                            LifecycleKind::Initialization => "initialization",
                            LifecycleKind::Upgrade => "contract upgrade",
                        };
                        self.push_mutant(
                            operator,
                            node.span(),
                            "()".to_owned(),
                            format!(
                                "remove {receiver}.require_auth() authorization in recognized Soroban {action} entrypoint"
                            ),
                        );
                        for alternate in self
                            .address_params
                            .iter()
                            .filter(|name| *name != &receiver)
                            .cloned()
                            .collect::<Vec<_>>()
                        {
                            self.push_mutant(
                                operator,
                                node.span(),
                                format!("{alternate}.require_auth()"),
                                format!(
                                    "authenticate {alternate} instead of {receiver} in recognized Soroban {action} entrypoint"
                                ),
                            );
                        }
                    }
                }
            }
        }

        if node.method == "require_auth" && node.args.is_empty() {
            if let Some(receiver_name) = receiver_ident(&node.receiver) {
                if self.address_params.contains(&receiver_name) {
                    self.push_mutant(
                        "AUTH-001",
                        node.span(),
                        "()".to_string(),
                        format!("remove {}.require_auth()", receiver_name),
                    );

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
        }

        visit::visit_expr_method_call(self, node);
    }
}

#[derive(Debug)]
struct BaselineFailure(String);

impl BaselineFailure {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for BaselineFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
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

fn parse_duration(value: &str) -> std::result::Result<u64, String> {
    if value.starts_with('-') {
        return Err(format!("duration must be positive, got {value}"));
    }

    let (number, multiplier) = match value.chars().last() {
        Some('s') => (&value[..value.len() - 1], 1_u64),
        Some('m') => (&value[..value.len() - 1], 60),
        Some('h') => (&value[..value.len() - 1], 3_600),
        Some('d') => (&value[..value.len() - 1], 86_400),
        _ => (value, 1),
    };

    let amount = number.parse::<u64>().map_err(|_| {
        "duration must be a positive integer with an optional s, m, h, or d suffix".to_owned()
    })?;
    if amount == 0 {
        return Err("duration must be greater than zero".to_owned());
    }
    amount
        .checked_mul(multiplier)
        .ok_or_else(|| "duration is too large".to_owned())
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

/// The active selection options, rendered for empty-selection diagnostics.
struct SelectionFilters<'a> {
    package: &'a [String],
    operator: Option<&'a str>,
    exclude_globs: Vec<String>,
    shard: Option<Shard>,
    file: Option<&'a str>,
    function: Option<&'a str>,
    changed_file: &'a [PathBuf],
    diff_base: Option<&'a str>,
    changed_functions: bool,
}

impl SelectionFilters<'_> {
    fn discovery_filters(&self) -> Vec<String> {
        let mut filters = Vec::new();
        filters.extend(self.package.iter().map(|name| format!("--package {name}")));
        if let Some(operator) = self.operator {
            filters.push(format!("--operator {operator}"));
        }
        if let Some(file) = self.file {
            filters.push(format!("--file {file}"));
        }
        if let Some(function) = self.function {
            filters.push(format!("--function {function}"));
        }
        filters.extend(
            self.exclude_globs
                .iter()
                .map(|glob| format!("--exclude-path {glob}")),
        );
        filters
    }

    fn changed_filters(&self) -> Vec<String> {
        let mut filters = Vec::new();
        filters.extend(
            self.changed_file
                .iter()
                .map(|path| format!("--changed-file {}", path.display())),
        );
        if let Some(base) = self.diff_base {
            filters.push(format!("--diff-base {base}"));
        }
        if self.changed_functions {
            filters.push("--changed-functions".to_owned());
        }
        filters
    }

    fn empty_selection(
        &self,
        root: &Path,
        stats: DiscoveryStats,
        after_discovery: usize,
        after_changed: usize,
    ) -> diagnostics::EmptySelection {
        let discovery_filters = self.discovery_filters();
        let changed_filters = self.changed_filters();
        let shard = self
            .shard
            .map(|value| format!("--shard {}/{}", value.index, value.total));
        let mut filters = discovery_filters.clone();
        filters.extend(changed_filters.iter().cloned());
        filters.extend(shard.iter().cloned());
        diagnostics::EmptySelection {
            root: root.to_path_buf(),
            filters,
            discovery_filters,
            changed_filters,
            shard,
            files_scanned: stats.files_scanned,
            unparsable_files: stats.unparsable_files,
            candidates: stats.candidates,
            after_discovery,
            after_changed,
        }
    }
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            diagnostics::fatal(&error);
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
            package,
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
            let root = path
                .canonicalize()
                .with_context(|| format!("cannot resolve {}", path.display()))?;
            let package_dirs = resolve_package_filter(&root, &package)?;
            let selection_filters = SelectionFilters {
                package: &package,
                operator: operator.as_deref(),
                exclude_globs: exclude_path
                    .iter()
                    .map(|glob| glob.glob().to_string())
                    .collect(),
                shard,
                file: file.as_deref(),
                function: function.as_deref(),
                changed_file: &changed_file,
                diff_base: diff_base.as_deref(),
                changed_functions,
            };
            let Discovery {
                mutants,
                suppressed,
                stats,
            } = discover_mutants_detailed(
                &root,
                package_dirs.as_deref(),
                operator.as_deref(),
                file.as_deref(),
                function.as_deref(),
                &exclusion_set(exclude_path)?,
            )?;
            let after_discovery = mutants.len();
            let mutants = filter_changed_mutants(
                &root,
                mutants,
                &changed_file,
                diff_base.as_deref(),
                changed_functions,
            )?;
            let after_changed = mutants.len();
            let mutants = select_shard(mutants, shard);
            if mutants.is_empty() {
                diagnostics::report_empty_selection(&selection_filters.empty_selection(
                    &root,
                    stats,
                    after_discovery,
                    after_changed,
                ));
            }
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
                        ..list_report(
                            &mutants,
                            SelectionMetadata::capture(
                                operator.as_deref(),
                                file.as_deref(),
                                function.as_deref(),
                                shard,
                            ),
                        )
                    })?
                )?;
            } else {
                print_mutants(&mut report, &mutants)?;
                print_suppression_summary(&mut report, &suppressed)?;
            }
        }
        Commands::Test {
            path,
            package,
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
            scratch_dir,
            test_command,
            runner,
            timeout,
            compile_timeout,
            test_timeout,
            jobs,
            timing,
            json,
            markdown,
            html,
            sarif,
            github_annotations,
            github_summary,
            min_score,
            output,
            no_progress,
            resume,
            progress_file,
            cache,
            no_cache,
            clear_cache,
        } => {
            let baseline_timeout_source = if test_timeout.is_some() {
                "--test-timeout"
            } else {
                "--timeout"
            };
            let timeouts = Timeouts::resolve(timeout, compile_timeout, test_timeout);
            let mut test_command = resolve_runner_command(runner, &test_command)?;
            if !package.is_empty() {
                // #183: validate package names before they become shell words.
                let package_words = command::package_words(&package)?;
                if runner == Runner::Nextest && test_command == "cargo nextest run" {
                    test_command = format!("cargo nextest run {package_words}");
                } else if runner == Runner::CargoTest && test_command == "cargo test -q" {
                    test_command = format!("cargo test -q {package_words}");
                }
            }
            let mut metadata = RunMetadata::capture(
                &test_command,
                timeouts,
                operator.as_deref(),
                file.as_deref(),
                function.as_deref(),
                shard,
            );
            let root = path
                .canonicalize()
                .with_context(|| format!("cannot resolve {}", path.display()))?;
            let scratch_root = resolve_scratch_root(&root, scratch_dir.as_deref())?;
            validate_scratch_root_writable(&scratch_root)?;
            if scratch_dir.is_some() {
                metadata.scratch_dir = Some(scratch_root.display().to_string());
            }

            let discovery_started = Instant::now();
            let package_dirs = resolve_package_filter(&root, &package)?;
            let exclude_globs = exclude_path
                .iter()
                .map(|glob| glob.glob().to_string())
                .collect::<Vec<_>>();
            let selection_filters = SelectionFilters {
                package: &package,
                operator: operator.as_deref(),
                exclude_globs: exclude_globs.clone(),
                shard,
                file: file.as_deref(),
                function: function.as_deref(),
                changed_file: &changed_file,
                diff_base: diff_base.as_deref(),
                changed_functions,
            };
            let Discovery {
                mutants,
                suppressed,
                stats,
            } = discover_mutants_detailed(
                &root,
                package_dirs.as_deref(),
                operator.as_deref(),
                file.as_deref(),
                function.as_deref(),
                &exclusion_set(exclude_path)?,
            )?;
            let after_discovery = mutants.len();
            let mutants = filter_changed_mutants(
                &root,
                mutants,
                &changed_file,
                diff_base.as_deref(),
                changed_functions,
            )?;
            let after_changed = mutants.len();
            let mutants = select_shard(mutants, shard);
            let discovery_ms = duration_ms(discovery_started.elapsed());
            if timing {
                metadata.timing = Some(RunTiming {
                    discovery_ms,
                    ..RunTiming::default()
                });
            }
            if mutants.is_empty() {
                diagnostics::report_empty_selection(&selection_filters.empty_selection(
                    &root,
                    stats,
                    after_discovery,
                    after_changed,
                ));
            }
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
                    if let Some(run_timing) = metadata.timing {
                        writeln!(
                            report,
                            "Timing: discovery={}ms baseline=0ms mutants=0ms compile=0ms test=0ms",
                            run_timing.discovery_ms
                        )?;
                    }
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

            let cache_base = target_dir_base(&root, target_dir.as_deref());
            let (baseline_target_dir, mutant_target_root) =
                target_dirs(&root, target_dir.as_deref());
            let parallel_mutant_root = parallel_target_root(&root, target_dir.as_deref());

            let source_hash = compute_source_hash(&root, package_dirs.as_deref())?;
            // The result cache is intentionally more conservative than the
            // build cache: path dependencies outside a selected package can
            // still affect outcomes, so hash the full workspace as well.
            let workspace_source_hash = compute_source_hash(&root, None)?;
            let current_identity = CacheIdentity {
                workspace_root: root.clone(),
                source_hash: source_hash.clone(),
                test_command: test_command.clone(),
                rustc_version: tool_version("rustc"),
                cargo_version: tool_version("cargo"),
                packages: package.clone(),
                operator: operator.clone(),
            };
            let result_identity = ResultCacheIdentity {
                workspace_root: root.display().to_string(),
                source_hash,
                workspace_source_hash,
                lockfile_hash: lockfile_hash(&root),
                toolchain_file_hash: toolchain_file_hash(&root),
                test_command: test_command.clone(),
                rustc_version: current_identity.rustc_version.clone(),
                cargo_version: current_identity.cargo_version.clone(),
                packages: package.clone(),
                operator: operator.clone(),
                file: file.clone(),
                function: function.clone(),
                test_dir: test_dir.as_ref().map(|dir| dir.display().to_string()),
                exclude_globs,
                shard: shard.map(|value| format!("{}/{}", value.index, value.total)),
                changed_file: changed_file
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                diff_base: diff_base.clone(),
                changed_functions,
                jobs: usize::from(jobs),
                scratch_dir: scratch_dir
                    .as_ref()
                    .map(|_| scratch_root.display().to_string()),
                execution_env_hash: execution_env_hash(),
                compile_timeout_seconds: timeouts.compile,
                test_timeout_seconds: timeouts.test,
            };

            validate_and_isolate_cache(
                &cache_base,
                &baseline_target_dir,
                &mutant_target_root,
                &parallel_mutant_root,
                &current_identity,
            )?;

            let progress_path = progress_file.unwrap_or_else(|| cache_base.join("progress.json"));
            let mut progress = if resume {
                load_and_validate_progress(&progress_path, &current_identity)?
            } else {
                let prog = RunProgress::new(current_identity);
                save_progress_atomic(&progress_path, &prog)?;
                prog
            };

            let cache_file = result_cache_path(&cache_base);
            if clear_cache && cache_file.exists() {
                pathguard::require_owned_cache_path(&cache_file, &cache_base)?;
                fs::remove_file(&cache_file).with_context(|| {
                    format!("cannot clear result cache at {}", cache_file.display())
                })?;
                diagnostics::note(&format!(
                    "cleared persistent result cache at {}",
                    cache_file.display()
                ));
            }
            let cache_enabled = cache && !no_cache;
            let mut result_cache = if cache_enabled {
                diagnostics::note(&format!(
                    "persistent result cache enabled at {}",
                    cache_file.display()
                ));
                load_result_cache(&cache_file, &result_identity)?
            } else {
                if no_cache {
                    diagnostics::note("persistent result cache disabled");
                }
                PersistentResultCache::empty(result_identity.clone())
            };
            let mut cache_hits = 0usize;
            let mut cache_misses = 0usize;

            let test_cwd = test_dir
                .as_deref()
                .map(|dir| root.join(dir))
                .unwrap_or_else(|| root.clone());
            let text_progress = !json && !markdown && !html && !sarif && !no_progress;
            if text_progress {
                writeln!(
                    report,
                    "Baseline: {}",
                    redact_environment_secrets(&test_command)
                )?;
            }
            let baseline_started = Instant::now();
            let baseline_execution = run_process_detailed(
                &test_cwd,
                &test_command,
                timeouts.test,
                &baseline_target_dir,
            )?;
            let baseline_ms = duration_ms(baseline_started.elapsed());
            match baseline_execution.result {
                ProcessResult::Passed => {
                    if text_progress {
                        writeln!(report, "Baseline PASS\n")?;
                    }
                }
                ProcessResult::Failed => {
                    let kind = baseline_execution
                        .failure_kind
                        .unwrap_or(ProcessFailureKind::Test);
                    let mut message = format!(
                        "baseline tests failed ({}) in {} using `{}`; mutation results were not created",
                        kind.label(),
                        test_cwd.display(),
                        redact_environment_secrets(&test_command)
                    );
                    let stdout = one_line_excerpt(&baseline_execution.stdout);
                    let stderr = one_line_excerpt(&baseline_execution.stderr);
                    if !stdout.is_empty() {
                        message.push_str(&format!("; stdout: {stdout}"));
                    }
                    if !stderr.is_empty() {
                        message.push_str(&format!("; stderr: {stderr}"));
                    }
                    if let Some(code) = baseline_execution.exit_code {
                        message.push_str(&format!("; exit code: {code}"));
                    }
                    return Err(BaselineFailure::new(message).into());
                }
                ProcessResult::Timeout => {
                    return Err(BaselineFailure::new(format!(
                        "baseline tests timed out after {}s (effective timeout from {}) in {} using `{}`; mutation results were not created",
                        timeouts.test,
                        baseline_timeout_source,
                        test_cwd.display(),
                        redact_environment_secrets(&test_command)
                    ))
                    .into());
                }
            }
            let mutant_phase_started = Instant::now();
            let mut outcome_slots: Vec<Option<Outcome>> = vec![None; mutants.len()];
            let mut timing_slots: Vec<Option<MutantTiming>> = vec![None; mutants.len()];
            let mut pending_mutants = Vec::new();
            let mut pending_indices = Vec::new();

            for (index, mutant) in mutants.iter().enumerate() {
                if let Some(completed) = progress.find_completed(mutant) {
                    if text_progress {
                        writeln!(
                            report,
                            "{:<10} {:<8} {}:{} {} (resumed)",
                            outcome_label(&completed.outcome),
                            mutant.operator,
                            mutant.file.display(),
                            mutant.span.line,
                            mutant.description
                        )?;
                    }
                    outcome_slots[index] = Some(completed.outcome.clone());
                } else if cache_enabled {
                    if let Some(completed) = result_cache.find(mutant) {
                        cache_hits += 1;
                        if text_progress {
                            writeln!(
                                report,
                                "{:<10} {:<8} {}:{} {} (cache hit)",
                                outcome_label(&completed.outcome),
                                mutant.operator,
                                mutant.file.display(),
                                mutant.span.line,
                                mutant.description
                            )?;
                        }
                        outcome_slots[index] = Some(completed.outcome.clone());
                    } else {
                        cache_misses += 1;
                        if text_progress {
                            writeln!(
                                report,
                                "cache miss  {:<8} {}:{} {}",
                                mutant.operator,
                                mutant.file.display(),
                                mutant.span.line,
                                mutant.description
                            )?;
                        }
                        pending_indices.push(index);
                        pending_mutants.push(mutant.clone());
                    }
                } else {
                    pending_indices.push(index);
                    pending_mutants.push(mutant.clone());
                }
            }

            let pending_executions = execute_mutants(
                &root,
                &pending_mutants,
                &ExecutionSettings {
                    test_dir: test_dir.as_deref(),
                    test_command: &test_command,
                    timeouts,
                    target_dir: target_dir.as_deref(),
                    scratch_root: &scratch_root,
                    jobs: usize::from(jobs),
                    timing,
                },
                &mut |mutant, outcome| {
                    if text_progress {
                        writeln!(
                            report,
                            "{:<10} {:<8} {}:{} {}",
                            outcome_label(outcome),
                            mutant.operator,
                            mutant.file.display(),
                            mutant.span.line,
                            mutant.description
                        )?;
                    }
                    progress.record_completed(mutant, outcome.clone());
                    save_progress_atomic(&progress_path, &progress)?;
                    if cache_enabled {
                        result_cache.record(mutant, outcome.clone());
                        save_result_cache(&cache_file, &mut result_cache)?;
                    }
                    Ok(())
                },
            )?;

            for (pending_pos, execution) in pending_executions.into_iter().enumerate() {
                let index = pending_indices[pending_pos];
                outcome_slots[index] = Some(execution.outcome);
                timing_slots[index] = execution.timing;
            }

            let results = mutants
                .into_iter()
                .zip(outcome_slots)
                .zip(timing_slots)
                .map(|((mutant, outcome), mutant_timing)| {
                    Ok(MutantResult {
                        mutant,
                        outcome: outcome.context("mutation execution ended without an outcome")?,
                        timing: mutant_timing,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let mutant_wall_ms = duration_ms(mutant_phase_started.elapsed());

            if timing {
                let compile_ms = results.iter().fold(0u64, |total, result| {
                    total.saturating_add(
                        result
                            .timing
                            .and_then(|value| value.compile_ms)
                            .unwrap_or(0),
                    )
                });
                let test_ms = results.iter().fold(0u64, |total, result| {
                    total.saturating_add(result.timing.and_then(|value| value.test_ms).unwrap_or(0))
                });
                metadata.timing = Some(RunTiming {
                    discovery_ms,
                    baseline_ms,
                    mutant_wall_ms,
                    compile_ms,
                    test_ms,
                });
            }

            if cache_enabled {
                result_cache.identity = result_identity;
                save_result_cache(&cache_file, &mut result_cache)?;
                diagnostics::note(&format!(
                    "result cache: {cache_hits} hit(s), {cache_misses} miss(es), {} stored",
                    result_cache.entries.len()
                ));
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
                if let Some(run_timing) = metadata.timing {
                    writeln!(
                        report,
                        "Timing: discovery={}ms baseline={}ms mutants={}ms compile={}ms test={}ms",
                        run_timing.discovery_ms,
                        run_timing.baseline_ms,
                        run_timing.mutant_wall_ms,
                        run_timing.compile_ms,
                        run_timing.test_ms
                    )?;
                }
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
        Commands::BenchmarkMatrix {
            manifest,
            allow_external,
            local_only,
            checkout_dir,
            timeout,
            target,
            json,
            output,
        } => {
            let summary = run_benchmark_matrix(
                &manifest,
                allow_external,
                local_only,
                &checkout_dir,
                timeout,
                target.as_deref(),
            )?;
            let mut report = report_writer(output.as_deref())?;
            if json {
                writeln!(report, "{}", serde_json::to_string_pretty(&summary)?)?;
            } else {
                print_benchmark_markdown(&mut report, &summary)?;
            }
            if summary
                .targets
                .iter()
                .any(|target| target.status == "failed")
            {
                bail!("one or more benchmark targets failed baseline/execution; see the summary above");
            }
        }
        Commands::CheckPlatform { json } => {
            let report = check_platform_policy()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_platform_report(&mut io::stdout(), &report)?;
            }
            if report.status == "unsupported" {
                bail!(
                    "unsupported platform; run `check-platform --json` for per-check remediation"
                );
            }
        }
        Commands::Operators { json } => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&OperatorsReport::current())?
                );
            } else {
                print_operators(&mut io::stdout())?;
            }
        }
        Commands::Completions { shell } => {
            io::stdout().write_all(&completion_script(shell))?;
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

// ---------------------------------------------------------------------------
// Opt-in benchmark matrix. This code path is never used by ordinary `test`
// runs and is deliberately absent from unit CI.
// ---------------------------------------------------------------------------

const BENCHMARK_MANIFEST_VERSION: u32 = 1;

#[derive(Deserialize)]
struct BenchmarkManifest {
    schema_version: u32,
    #[serde(default)]
    description: Option<String>,
    targets: Vec<BenchmarkTarget>,
}

#[derive(Deserialize, Clone)]
struct BenchmarkTarget {
    id: String,
    kind: BenchmarkTargetKind,
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default)]
    repository: Option<String>,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    operator: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    function: Option<String>,
    test_command: String,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum BenchmarkTargetKind {
    Local,
    External,
}

#[derive(Serialize)]
struct BenchmarkSummary {
    schema_version: u32,
    generated_by: String,
    notice: String,
    description: Option<String>,
    allow_external: bool,
    local_only: bool,
    checkout_dir: String,
    timeout_seconds: u64,
    rustc_version: Option<String>,
    cargo_version: Option<String>,
    targets: Vec<BenchmarkTargetSummary>,
}

#[derive(Serialize)]
struct BenchmarkTargetSummary {
    id: String,
    kind: BenchmarkTargetKind,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_revision: Option<String>,
    test_command: String,
    baseline: Option<BenchmarkCommandRecord>,
    counts: Option<BenchmarkCounts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    score_percent: Option<f64>,
    mutants: Vec<BenchmarkMutantRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Serialize)]
struct BenchmarkCounts {
    generated: usize,
    killed: usize,
    survived: usize,
    unviable: usize,
    timeout: usize,
}

#[derive(Serialize)]
struct BenchmarkCommandRecord {
    phase: String,
    command: String,
    exit_code: Option<i32>,
    outcome: String,
    duration_ms: u128,
    cwd: String,
    target_dir: String,
}

#[derive(Serialize)]
struct BenchmarkMutantRecord {
    id: String,
    operator: String,
    file: String,
    line: usize,
    description: String,
    original: String,
    replacement: String,
    outcome: String,
}

#[derive(Deserialize)]
struct BenchmarkTestReport {
    schema_version: u32,
    results: Vec<BenchmarkReportedResult>,
}

#[derive(Deserialize)]
struct BenchmarkReportedResult {
    mutant: BenchmarkReportedMutant,
    outcome: Outcome,
}

#[derive(Deserialize)]
struct BenchmarkReportedMutant {
    id: String,
    operator: String,
    file: PathBuf,
    span: BenchmarkReportedSpan,
    original: String,
    replacement: String,
    description: String,
}

#[derive(Deserialize)]
struct BenchmarkReportedSpan {
    line: usize,
}

fn benchmark_id_is_safe(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn pinned_revision_is_valid(revision: &str) -> bool {
    (7..=40).contains(&revision.len()) && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn load_benchmark_manifest(path: &Path) -> Result<BenchmarkManifest> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("cannot read benchmark manifest {}", path.display()))?;
    let manifest: BenchmarkManifest = serde_json::from_str(&content)
        .with_context(|| format!("invalid benchmark manifest JSON at {}", path.display()))?;
    if manifest.schema_version != BENCHMARK_MANIFEST_VERSION {
        bail!(
            "unsupported benchmark schema_version {}; expected {}",
            manifest.schema_version,
            BENCHMARK_MANIFEST_VERSION
        );
    }
    if manifest.targets.is_empty() {
        bail!("benchmark manifest targets must not be empty");
    }

    let repo_root = env::current_dir()?.canonicalize()?;
    let mut seen = std::collections::HashSet::new();
    for (index, target) in manifest.targets.iter().enumerate() {
        let location = format!("targets[{index}]");
        if !benchmark_id_is_safe(&target.id) {
            bail!("{location}.id must be 1-64 characters of A-Z, a-z, 0-9, '-' or '_'");
        }
        if !seen.insert(target.id.as_str()) {
            bail!("{location}.id is duplicate: {}", target.id);
        }
        if target.test_command.trim().is_empty() {
            bail!("{location}.test_command must not be empty");
        }
        if let Some(operator) = target.operator.as_deref() {
            if !BUILTIN_OPERATORS.iter().any(|(id, _)| *id == operator) {
                bail!("{location}.operator is not a built-in operator: {operator}");
            }
        }
        match target.kind {
            BenchmarkTargetKind::Local => {
                let Some(declared) = target.path.as_deref() else {
                    bail!("{location}.path is required for kind=local");
                };
                if target.repository.is_some() || target.revision.is_some() {
                    bail!("{location} must not declare repository/revision for kind=local");
                }
                let resolved = declared.canonicalize().with_context(|| {
                    format!("{location}.path does not exist: {}", declared.display())
                })?;
                if !resolved.starts_with(&repo_root) {
                    bail!(
                        "{location}.path must be inside the repository root {}",
                        repo_root.display()
                    );
                }
                if !resolved.is_dir() {
                    bail!("{location}.path is not a directory: {}", declared.display());
                }
            }
            BenchmarkTargetKind::External => {
                let Some(repository) = target.repository.as_deref() else {
                    bail!("{location}.repository is required for kind=external");
                };
                if !repository.starts_with("https://") {
                    bail!("{location}.repository must be a public https:// URL");
                }
                let Some(revision) = target.revision.as_deref() else {
                    bail!("{location}.revision is required for kind=external");
                };
                if !pinned_revision_is_valid(revision) {
                    bail!("{location}.revision must be 7-40 hexadecimal characters");
                }
                if target.path.is_some() {
                    bail!("{location} must not declare path for kind=external");
                }
            }
        }
    }
    Ok(manifest)
}

fn git_output(args: Vec<std::ffi::OsString>, label: &str) -> Result<String> {
    let output = Command::new("git")
        .args(&args)
        .output()
        .with_context(|| format!("cannot run git {label}; is git installed?"))?;
    if !output.status.success() {
        bail!(
            "git {label} failed with exit code {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn revision_matches(revision: &str, resolved: &str) -> bool {
    let (requested, resolved) = (revision.to_ascii_lowercase(), resolved.to_ascii_lowercase());
    if requested.len() == 40 {
        requested == resolved
    } else {
        resolved.starts_with(&requested)
    }
}

fn prepare_external_checkout(
    target: &BenchmarkTarget,
    checkout_dir: &Path,
) -> Result<(PathBuf, String)> {
    let repository = target.repository.as_deref().unwrap_or_default();
    let revision = target.revision.as_deref().unwrap_or_default();
    let checkout = checkout_dir.join(&target.id);

    if checkout.exists() {
        reject_cleanup_symlink(&checkout)?;
        if !checkout.join(".git").exists() {
            bail!(
                "benchmark checkout {} exists but is not a git checkout; move it aside or use another --checkout-dir",
                checkout.display()
            );
        }
        let label = format!("rev-parse HEAD in {}", target.id);
        let resolved = git_output(
            vec![
                std::ffi::OsString::from("-C"),
                checkout.as_os_str().to_os_string(),
                std::ffi::OsString::from("rev-parse"),
                std::ffi::OsString::from("HEAD"),
            ],
            &label,
        )?;
        if !revision_matches(revision, &resolved) {
            bail!(
                "benchmark checkout {} is at {} but manifest pins {}; move it aside or use another --checkout-dir",
                checkout.display(),
                resolved,
                revision
            );
        }
        let remote = git_output(
            vec![
                std::ffi::OsString::from("-C"),
                checkout.as_os_str().to_os_string(),
                std::ffi::OsString::from("remote"),
                std::ffi::OsString::from("get-url"),
                std::ffi::OsString::from("origin"),
            ],
            &format!("remote get-url in {}", target.id),
        )?;
        if remote.trim_end_matches(".git").trim_end_matches('/')
            != repository.trim_end_matches(".git").trim_end_matches('/')
        {
            bail!(
                "benchmark checkout {} points at {} but manifest declares {}; move it aside or use another --checkout-dir",
                checkout.display(),
                remote,
                repository
            );
        }
        return Ok((checkout, resolved));
    }

    fs::create_dir_all(checkout_dir)?;
    git_output(
        vec![
            std::ffi::OsString::from("clone"),
            std::ffi::OsString::from("--quiet"),
            std::ffi::OsString::from(repository),
            checkout.as_os_str().to_os_string(),
        ],
        &format!("clone {repository}"),
    )?;
    git_output(
        vec![
            std::ffi::OsString::from("-C"),
            checkout.as_os_str().to_os_string(),
            std::ffi::OsString::from("checkout"),
            std::ffi::OsString::from("--quiet"),
            std::ffi::OsString::from("--detach"),
            std::ffi::OsString::from(revision),
        ],
        &format!("checkout {revision} in {}", target.id),
    )?;
    let resolved = git_output(
        vec![
            std::ffi::OsString::from("-C"),
            checkout.as_os_str().to_os_string(),
            std::ffi::OsString::from("rev-parse"),
            std::ffi::OsString::from("HEAD"),
        ],
        &format!("rev-parse HEAD in {}", target.id),
    )?;
    if !revision_matches(revision, &resolved) {
        bail!(
            "benchmark checkout {} resolved to {} which does not match pinned revision {}",
            checkout.display(),
            resolved,
            revision
        );
    }
    Ok((checkout, resolved))
}

fn format_command_line(program: &std::ffi::OsStr, args: &[std::ffi::OsString]) -> String {
    fn quote(value: &std::ffi::OsStr) -> String {
        let text = value.to_string_lossy();
        if !text.is_empty()
            && text
                .chars()
                .all(|ch| !ch.is_whitespace() && ch != '\'' && ch != '"')
        {
            text.into_owned()
        } else {
            format!("'{}'", text.replace('\'', "'\\''"))
        }
    }
    std::iter::once(program.to_os_string())
        .chain(args.iter().cloned())
        .map(|arg| quote(arg.as_os_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Runs one benchmark command with a wall-clock timeout, writing both streams
/// to files so a verbose test run cannot fill a pipe and deadlock the runner.
#[allow(clippy::too_many_arguments)]
fn run_timed_command(
    mut command: Command,
    label: String,
    phase: &str,
    timeout_seconds: u64,
    cwd: &Path,
    target_dir: &Path,
    log_dir: &Path,
) -> Result<(BenchmarkCommandRecord, String)> {
    fs::create_dir_all(target_dir)?;
    fs::create_dir_all(log_dir)?;
    let stdout_path = log_dir.join(format!("{phase}.stdout.log"));
    let stderr_path = log_dir.join(format!("{phase}.stderr.log"));
    let mut child = command
        .current_dir(cwd)
        .env("CARGO_TARGET_DIR", target_dir)
        // Baseline, mutant compile and mutant test commands all inherit the
        // same parent environment. Only the per-phase target directory and
        // toolchain override are intentional differences.
        .env_remove("RUSTUP_TOOLCHAIN")
        .stdout(Stdio::from(fs::File::create(&stdout_path)?))
        .stderr(Stdio::from(fs::File::create(&stderr_path)?))
        .spawn()
        .with_context(|| format!("failed to execute benchmark command: {label}"))?;

    let started = std::time::Instant::now();
    let (outcome, exit_code) = match child.wait_timeout(Duration::from_secs(timeout_seconds))? {
        Some(status) if status.success() => ("passed".to_owned(), status.code()),
        Some(status) => ("failed".to_owned(), status.code()),
        None => {
            let _ = child.kill();
            let _ = child.wait();
            ("timeout".to_owned(), None)
        }
    };
    let duration_ms = started.elapsed().as_millis();

    let stdout = fs::read_to_string(&stdout_path).unwrap_or_default();
    let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
    if !stderr.trim().is_empty() {
        eprintln!("{}", stderr.trim_end());
    }

    Ok((
        BenchmarkCommandRecord {
            phase: phase.to_owned(),
            command: label,
            exit_code,
            outcome,
            duration_ms,
            cwd: cwd.display().to_string(),
            target_dir: target_dir.display().to_string(),
        },
        stdout,
    ))
}

fn benchmark_empty_summary(id: &str, reason: &str) -> String {
    format!("benchmark target {id}: {reason}")
}

fn run_benchmark_matrix(
    manifest_path: &Path,
    allow_external: bool,
    local_only: bool,
    checkout_dir: &Path,
    timeout: u64,
    only_target: Option<&str>,
) -> Result<BenchmarkSummary> {
    if local_only && allow_external {
        bail!("--local-only and --allow-external are mutually exclusive");
    }
    let manifest = load_benchmark_manifest(manifest_path)?;
    let selected = manifest
        .targets
        .iter()
        .filter(|target| only_target.is_none_or(|id| id == target.id))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        match only_target {
            Some(id) => bail!(
                "no benchmark target with id {id} in {}",
                manifest_path.display()
            ),
            None => bail!("benchmark manifest contains no targets"),
        }
    }

    let external = selected
        .iter()
        .filter(|target| target.kind == BenchmarkTargetKind::External && !local_only)
        .map(|target| target.id.as_str())
        .collect::<Vec<_>>();
    if !external.is_empty() && !allow_external {
        bail!(
            "external benchmark targets require explicit --allow-external: {}",
            external.join(", ")
        );
    }

    let checkout_dir = if checkout_dir.is_absolute() {
        checkout_dir.to_path_buf()
    } else {
        env::current_dir()?.join(checkout_dir)
    };
    fs::create_dir_all(&checkout_dir)?;

    let rustc_version = tool_version("rustc");
    let cargo_version = tool_version("cargo");
    let mut summaries = Vec::with_capacity(selected.len());
    let mut failures = Vec::new();

    for target in selected {
        let mut summary = BenchmarkTargetSummary {
            id: target.id.clone(),
            kind: target.kind,
            status: "completed".to_owned(),
            reason: None,
            path: None,
            repository: target.repository.clone(),
            requested_revision: target.revision.clone(),
            resolved_revision: None,
            test_command: target.test_command.clone(),
            baseline: None,
            counts: None,
            score_percent: None,
            mutants: Vec::new(),
            note: target.note.clone(),
        };

        if target.kind == BenchmarkTargetKind::External && local_only {
            summary.status = "skipped".to_owned();
            summary.reason = Some(benchmark_empty_summary(
                &target.id,
                "skipped by --local-only",
            ));
            summaries.push(summary);
            continue;
        }

        let (root, resolved_revision) = match target.kind {
            BenchmarkTargetKind::Local => {
                let declared = target.path.as_deref().unwrap_or(Path::new(""));
                let resolved = declared.canonicalize().with_context(|| {
                    format!("cannot resolve local benchmark path {}", declared.display())
                })?;
                summary.path = Some(resolved.display().to_string());
                (resolved, None)
            }
            BenchmarkTargetKind::External => {
                let (checkout, resolved) = prepare_external_checkout(target, &checkout_dir)?;
                summary.path = Some(checkout.display().to_string());
                (checkout, Some(resolved))
            }
        };
        summary.resolved_revision = resolved_revision;

        let workspace_dir = checkout_dir.join(&target.id);
        let target_dir = workspace_dir.join("target");
        // #183: benchmark manifests are JSON input; reject control bytes
        // before their test_command reaches `bash -lc`.
        command::validate_shell_command(&target.test_command, "benchmark test_command")?;
        let baseline_command = format!("bash -lc '{}'", target.test_command.replace('\'', "'\\''"));
        let (baseline, _) = run_timed_command(
            {
                let mut command = Command::new("bash");
                command.arg("-lc").arg(&target.test_command);
                command
            },
            baseline_command,
            "baseline",
            timeout,
            &root,
            &target_dir,
            &workspace_dir,
        )?;
        summary.baseline = Some(baseline);

        // A red baseline invalidates every mutant outcome, so no scoring is
        // attempted and no mutant command is launched for this target.
        if summary
            .baseline
            .as_ref()
            .is_some_and(|record| record.outcome != "passed")
        {
            let reason = benchmark_empty_summary(
                &target.id,
                "baseline did not pass; mutation results were not scored",
            );
            summary.status = "failed".to_owned();
            summary.reason = Some(reason.clone());
            failures.push(reason);
            summaries.push(summary);
            continue;
        }

        let (mutants, _) = discover_mutants_with_suppressions(
            &root,
            None,
            target.operator.as_deref(),
            target.file.as_deref(),
            target.function.as_deref(),
            &exclusion_set(Vec::new())?,
        )?;

        let executable = env::current_exe()?;
        let mut args = vec![
            std::ffi::OsString::from("test"),
            root.as_os_str().to_os_string(),
            std::ffi::OsString::from("--target-dir"),
            target_dir.as_os_str().to_os_string(),
            std::ffi::OsString::from("--timeout"),
            std::ffi::OsString::from(timeout.to_string()),
            std::ffi::OsString::from("--test-command"),
            std::ffi::OsString::from(&target.test_command),
            std::ffi::OsString::from("--json"),
            // Benchmark evidence is never served from the interactive result cache.
            std::ffi::OsString::from("--no-cache"),
        ];
        if let Some(operator) = target.operator.as_deref() {
            args.push(std::ffi::OsString::from("--operator"));
            args.push(std::ffi::OsString::from(operator));
        }
        if let Some(file) = target.file.as_deref() {
            args.push(std::ffi::OsString::from("--file"));
            args.push(std::ffi::OsString::from(file));
        }
        if let Some(function) = target.function.as_deref() {
            args.push(std::ffi::OsString::from("--function"));
            args.push(std::ffi::OsString::from(function));
        }
        let label = format_command_line(executable.as_os_str(), &args);
        let mutant_timeout = timeout
            .saturating_mul(mutants.len().max(1) as u64)
            .saturating_add(60);
        let mut command = Command::new(&executable);
        command.args(&args);
        let (record, stdout) = run_timed_command(
            command,
            label.clone(),
            "mutants",
            mutant_timeout,
            &root,
            &target_dir,
            &workspace_dir,
        )?;

        if record.outcome != "passed" {
            let reason = benchmark_empty_summary(
                &target.id,
                &format!(
                    "mutant command exited with code {} ({})",
                    record.exit_code.unwrap_or(-1),
                    record.outcome
                ),
            );
            summary.status = "failed".to_owned();
            summary.reason = Some(reason.clone());
            failures.push(reason);
            summaries.push(summary);
            continue;
        }

        let report: BenchmarkTestReport = serde_json::from_str(&stdout).with_context(|| {
            format!(
                "benchmark target {} did not emit a valid mutation JSON report",
                target.id
            )
        })?;
        if report.schema_version != JSON_SCHEMA_VERSION {
            bail!(
                "benchmark target {} emitted unsupported JSON schema_version {}",
                target.id,
                report.schema_version
            );
        }

        let mut counts = BenchmarkCounts {
            generated: report.results.len(),
            killed: 0,
            survived: 0,
            unviable: 0,
            timeout: 0,
        };
        for result in &report.results {
            match result.outcome {
                Outcome::Killed => counts.killed += 1,
                Outcome::Survived => counts.survived += 1,
                Outcome::Unviable => counts.unviable += 1,
                Outcome::Timeout => counts.timeout += 1,
            }
            summary.mutants.push(BenchmarkMutantRecord {
                id: result.mutant.id.clone(),
                operator: result.mutant.operator.clone(),
                file: result.mutant.file.display().to_string(),
                line: result.mutant.span.line,
                description: result.mutant.description.clone(),
                original: result.mutant.original.clone(),
                replacement: result.mutant.replacement.clone(),
                outcome: outcome_label(&result.outcome).to_owned(),
            });
        }
        let viable = counts.killed + counts.survived;
        summary.score_percent = Some(if viable == 0 {
            0.0
        } else {
            counts.killed as f64 * 100.0 / viable as f64
        });
        summary.counts = Some(counts);
        summaries.push(summary);
    }

    let summary = BenchmarkSummary {
        schema_version: BENCHMARK_MANIFEST_VERSION,
        generated_by: "cargo soro-mutants benchmark-matrix".to_owned(),
        notice: "Mutation-testing observation only; not a security finding about any third-party project.".to_owned(),
        description: manifest.description.clone(),
        allow_external,
        local_only,
        checkout_dir: checkout_dir.display().to_string(),
        timeout_seconds: timeout,
        rustc_version,
        cargo_version,
        targets: summaries,
    };
    if !failures.is_empty() {
        diagnostics::warning(&format!(
            "benchmark baseline/execution failures: {}",
            failures.join("; ")
        ));
    }
    Ok(summary)
}

fn print_benchmark_markdown(report: &mut dyn Write, summary: &BenchmarkSummary) -> Result<()> {
    writeln!(report, "# Benchmark matrix summary")?;
    writeln!(report)?;
    writeln!(report, "> {}", summary.notice)?;
    writeln!(report)?;
    if let Some(description) = summary.description.as_deref() {
        writeln!(report, "{description}")?;
        writeln!(report)?;
    }
    writeln!(report, "- Schema version: {}", summary.schema_version)?;
    writeln!(
        report,
        "- External targets allowed: {}",
        summary.allow_external
    )?;
    writeln!(report, "- Local-only mode: {}", summary.local_only)?;
    writeln!(report, "- Checkout directory: `{}`", summary.checkout_dir)?;
    writeln!(
        report,
        "- Per-command timeout: {}s",
        summary.timeout_seconds
    )?;
    writeln!(
        report,
        "- Toolchain: {} / {}",
        summary.rustc_version.as_deref().unwrap_or("rustc unknown"),
        summary.cargo_version.as_deref().unwrap_or("cargo unknown")
    )?;
    writeln!(report)?;

    for target in &summary.targets {
        writeln!(report, "## `{}` ({:?})", target.id, target.kind)?;
        writeln!(report)?;
        writeln!(report, "- Status: {}", target.status)?;
        if let Some(reason) = target.reason.as_deref() {
            writeln!(report, "- Reason: {reason}")?;
        }
        if let Some(path) = target.path.as_deref() {
            writeln!(report, "- Checked-out or local path: `{path}`")?;
        }
        if let Some(repository) = target.repository.as_deref() {
            writeln!(report, "- Repository: `{repository}`")?;
        }
        if let Some(revision) = target.requested_revision.as_deref() {
            writeln!(report, "- Pinned revision: `{revision}`")?;
        }
        if let Some(revision) = target.resolved_revision.as_deref() {
            writeln!(report, "- Resolved revision: `{revision}`")?;
        }
        writeln!(report, "- Test command: `{}`", target.test_command)?;
        if let Some(note) = target.note.as_deref() {
            writeln!(report, "- Note: {note}")?;
        }
        writeln!(report)?;

        if let Some(baseline) = target.baseline.as_ref() {
            writeln!(report, "### Baseline")?;
            writeln!(report)?;
            writeln!(
                report,
                "| Phase | Command | Exit code | Outcome | Duration |"
            )?;
            writeln!(report, "| --- | --- | --- | --- | --- |")?;
            writeln!(
                report,
                "| {} | `{}` | {} | {} | {} ms |",
                baseline.phase,
                baseline.command.replace('|', "\\|"),
                baseline
                    .exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "n/a".to_owned()),
                baseline.outcome,
                baseline.duration_ms
            )?;
            writeln!(report)?;
        }

        if let Some(counts) = target.counts.as_ref() {
            writeln!(
                report,
                "| Generated | Killed | Survived | Unviable | Timeout | Score |"
            )?;
            writeln!(report, "| --- | --- | --- | --- | --- | --- |")?;
            writeln!(
                report,
                "| {} | {} | {} | {} | {} | {:.1}% |",
                counts.generated,
                counts.killed,
                counts.survived,
                counts.unviable,
                counts.timeout,
                target.score_percent.unwrap_or(0.0)
            )?;
            writeln!(report)?;
        }

        if !target.mutants.is_empty() {
            writeln!(report, "| Mutant | Operator | Location | Outcome |")?;
            writeln!(report, "| --- | --- | --- | --- |")?;
            for mutant in &target.mutants {
                writeln!(
                    report,
                    "| {} | {} | `{}:{}` | {} |",
                    mutant.id, mutant.operator, mutant.file, mutant.line, mutant.outcome
                )?;
            }
            writeln!(report)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// v1.0 supported-platform policy. The check is pure observation: it never
// changes the environment and returns structured findings instead of panics.
// ---------------------------------------------------------------------------

const PLATFORM_POLICY_VERSION: &str = "1.0";
const MSRV: (u32, u32, u32) = (1, 85, 0);

#[derive(Serialize)]
struct PlatformCheck {
    id: String,
    requirement: String,
    observed: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    remediation: Option<String>,
}

#[derive(Serialize)]
struct PlatformReport {
    schema_version: u32,
    policy_version: String,
    status: String,
    summary: String,
    msrv: String,
    host: String,
    rustc_version: Option<String>,
    cargo_version: Option<String>,
    checks: Vec<PlatformCheck>,
    limitations: Vec<String>,
}

fn parse_semver(raw: Option<&str>) -> Option<(u32, u32, u32)> {
    let raw = raw?;
    let token = raw
        .split_whitespace()
        .find(|token| token.as_bytes().first().is_some_and(u8::is_ascii_digit))?;
    let mut parts = token.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    let patch = parts
        .next()
        .and_then(|value| {
            value
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>()
                .parse::<u32>()
                .ok()
        })
        .unwrap_or(0);
    Some((major, minor, patch))
}

fn probe_tool(binary: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(binary).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn check_platform_policy() -> Result<PlatformReport> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;
    let rustc_raw = probe_tool("rustc", &["--version"]);
    let cargo_raw = probe_tool("cargo", &["--version"]);
    let rustc_verbose = probe_tool("rustc", &["-Vv"]);
    let rustc_version = parse_semver(rustc_raw.as_deref());
    let cargo_version = parse_semver(cargo_raw.as_deref());
    let host = rustc_verbose
        .as_deref()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("host: ").map(str::trim))
        })
        .unwrap_or("unknown")
        .to_owned();

    let mut checks = Vec::new();
    let mut push = |id: &str,
                    requirement: &str,
                    observed: String,
                    status: &str,
                    remediation: Option<String>| {
        checks.push(PlatformCheck {
            id: id.to_owned(),
            requirement: requirement.to_owned(),
            observed,
            status: status.to_owned(),
            remediation,
        });
    };

    let (os_status, os_remediation) = match os {
        "linux" | "macos" => ("pass", None),
        "windows" => (
            "warning",
            Some(
                "Use WSL2 with a Linux distribution for reproducible test and benchmark runs."
                    .to_owned(),
            ),
        ),
        _ => (
            "fail",
            Some("Use a supported Linux or macOS host, or WSL2 on Windows.".to_owned()),
        ),
    };
    push(
        "operating-system",
        "Linux or macOS; Windows is best-effort via WSL2",
        os.to_owned(),
        os_status,
        os_remediation,
    );

    let arch_ok = matches!(arch, "x86_64" | "aarch64");
    push(
        "architecture",
        "x86_64 or aarch64",
        arch.to_owned(),
        if arch_ok { "pass" } else { "fail" },
        (!arch_ok).then(|| "Use a 64-bit x86_64 or aarch64 host.".to_owned()),
    );

    let rustc_ok = rustc_version.is_some_and(|version| version >= MSRV);
    push(
        "rustc-msrv",
        "rustc >= 1.85.0 (MSRV)",
        rustc_raw.clone().unwrap_or_else(|| "not found".to_owned()),
        if rustc_ok { "pass" } else { "fail" },
        (!rustc_ok).then(|| "Install Rust 1.85.0 or newer with rustup and retry.".to_owned()),
    );

    let cargo_ok = cargo_version.is_some_and(|version| version >= MSRV);
    push(
        "cargo-version",
        "cargo >= 1.85.0, matching the selected Rust toolchain",
        cargo_raw.clone().unwrap_or_else(|| "not found".to_owned()),
        if cargo_ok { "pass" } else { "fail" },
        (!cargo_ok).then(|| "Install Cargo 1.85.0 or newer with rustup and retry.".to_owned()),
    );

    let toolchain_match = match (rustc_version, cargo_version) {
        (Some(rustc), Some(cargo)) => (rustc.0, rustc.1) == (cargo.0, cargo.1),
        _ => false,
    };
    push(
        "toolchain-pair",
        "cargo and rustc come from the same installed toolchain series",
        format!("rustc={:?}, cargo={:?}", rustc_version, cargo_version),
        if toolchain_match { "pass" } else { "warning" },
        (!toolchain_match).then(|| {
            "Use `rustup default stable` (or a pinned 1.85 toolchain) so Cargo and rustc match."
                .to_owned()
        }),
    );

    let bash_ok = probe_tool("bash", &["--version"]).is_some();
    push(
        "shell",
        "bash is on PATH (the runner executes test commands with bash -lc)",
        if bash_ok { "available" } else { "not found" }.to_owned(),
        if bash_ok { "pass" } else { "fail" },
        (!bash_ok)
            .then(|| "Install bash or use a supported Linux/macOS/WSL2 environment.".to_owned()),
    );

    let git_ok = probe_tool("git", &["--version"]).is_some();
    push(
        "git",
        "git is on PATH (required only for pinned external benchmark targets)",
        if git_ok { "available" } else { "not found" }.to_owned(),
        if git_ok { "pass" } else { "warning" },
        (!git_ok).then(|| "Install git, or run the benchmark matrix with --local-only.".to_owned()),
    );

    let status = if checks.iter().any(|check| check.status == "fail") {
        "unsupported"
    } else if checks.iter().any(|check| check.status == "warning") {
        "best-effort"
    } else {
        "supported"
    };
    let summary = match status {
        "supported" => "This environment meets the v1.0 supported-platform policy.".to_owned(),
        "best-effort" => {
            "This environment is supported on a best-effort basis; see warnings.".to_owned()
        }
        _ => "This environment does not meet the v1.0 supported-platform policy.".to_owned(),
    };

    Ok(PlatformReport {
        schema_version: 1,
        policy_version: PLATFORM_POLICY_VERSION.to_owned(),
        status: status.to_owned(),
        summary,
        msrv: format!("{}.{}.{}", MSRV.0, MSRV.1, MSRV.2),
        host,
        rustc_version: rustc_raw,
        cargo_version: cargo_raw,
        checks,
        limitations: vec![
            "Windows without WSL2 is best-effort because command execution uses bash -lc.".to_owned(),
            "External benchmark targets additionally require network access and git.".to_owned(),
            "Soroban host tests depend on the SDK's testutils feature and a compatible installed toolchain.".to_owned(),
        ],
    })
}

fn print_platform_report(report: &mut dyn Write, policy: &PlatformReport) -> Result<()> {
    writeln!(
        report,
        "Soro Mutants v{} platform check",
        policy.policy_version
    )?;
    writeln!(report, "Status: {}", policy.status)?;
    writeln!(report, "{}", policy.summary)?;
    writeln!(report, "MSRV: {}", policy.msrv)?;
    writeln!(report, "Host: {}", policy.host)?;
    writeln!(report)?;
    for check in &policy.checks {
        writeln!(
            report,
            "{:<18} {:<10} {} ({})",
            check.id, check.status, check.requirement, check.observed
        )?;
        if let Some(remediation) = check.remediation.as_deref() {
            writeln!(report, "  fix: {remediation}")?;
        }
    }
    writeln!(report)?;
    writeln!(report, "Known limitations:")?;
    for limitation in &policy.limitations {
        writeln!(report, "  - {limitation}")?;
    }
    writeln!(report)?;
    writeln!(report, "Policy: docs/supported-platforms.md")?;
    Ok(())
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
            if fs::symlink_metadata(path)
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false)
            {
                bail!(
                    "refusing to write report through symbolic link {}",
                    path.display()
                );
            }
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

fn function_name_matches(qualified: &str, requested: &str) -> bool {
    qualified == requested || qualified.rsplit("::").next() == Some(requested)
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
            .arg("-C")
            .arg(&root)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("cannot locate Git repository")?;
        if !output.status.success() {
            bail!("cannot read Git repository metadata for --diff-base");
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

    let Some(diff_base) = diff_base else {
        return Ok(matching);
    };
    let ranges = changed_line_ranges(&root, git_top.as_deref().unwrap_or(&root), diff_base)?;
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
        let source_path = root.join(&mutant.file);
        let source = read_rust_source(&source_path)?;
        validate_source_nesting(&source, &source_path)?;
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
                    function_name_matches(name, function)
                        && *from <= mutant.span.line
                        && mutant.span.line <= *to
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

/// Stable candidate precedence: file and starting byte first, then operator
/// and replacement. Exact-range alternative replacements are independent
/// single-mutant experiments and remain valid. Distinct operator IDs are
/// also independent semantic experiments (e.g. TTL removal vs its arguments).
/// Conflicting non-identical ranges *within one operator* are suppressed.
fn mutant_order(a: &Mutant, b: &Mutant) -> std::cmp::Ordering {
    a.file
        .cmp(&b.file)
        .then(a.start_byte.cmp(&b.start_byte))
        .then(a.operator.cmp(&b.operator))
        .then(a.replacement.cmp(&b.replacement))
        .then(a.end_byte.cmp(&b.end_byte))
        .then(a.original.cmp(&b.original))
        .then(a.function.cmp(&b.function))
        .then(a.description.cmp(&b.description))
}

fn suppress_equivalent_mutants(mut mutants: Vec<Mutant>) -> (Vec<Mutant>, Vec<SuppressedMutant>) {
    use std::collections::HashSet;

    // Selection must not inherit recognizer registration or filesystem order.
    mutants.sort_by(mutant_order);

    let mut seen = HashSet::new();
    let mut retained = Vec::with_capacity(mutants.len());
    let mut suppressed = Vec::new();
    let mut file = None;
    let mut active_spans: Vec<(usize, usize, String)> = Vec::new();

    for mutant in mutants {
        if file.as_ref() != Some(&mutant.file) {
            file = Some(mutant.file.clone());
            active_spans.clear();
        }
        // Sorted starts make intervals ending before the next candidate safe
        // to forget; only concurrently active spans can overlap it.
        active_spans.retain(|(_, end, _)| *end > mutant.start_byte);

        let duplicate = (
            mutant.file.clone(),
            mutant.start_byte,
            mutant.end_byte,
            mutant.original.clone(),
            mutant.replacement.clone(),
        );
        let reason = if mutant.original == mutant.replacement {
            Some("unchanged replacement")
        } else if !seen.insert(duplicate) {
            Some("duplicate source edit")
        } else if active_spans.iter().any(|(start, end, operator)| {
            operator == &mutant.operator
                && *start < mutant.end_byte
                && mutant.start_byte < *end
                && (*start != mutant.start_byte || *end != mutant.end_byte)
        }) {
            Some("overlapping source span")
        } else {
            None
        };
        if let Some(reason) = reason {
            suppressed.push(SuppressedMutant {
                operator: mutant.operator,
                reason,
            });
        } else {
            active_spans.push((mutant.start_byte, mutant.end_byte, mutant.operator.clone()));
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
        None,
        operator,
        file_filter,
        function_filter,
        exclusions,
    )?
    .0)
}

const MAX_RUST_SYNTAX_NESTING: usize = 256;

fn validate_source_nesting(source: &str, path: &Path) -> Result<()> {
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
            if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                block_comment += 1;
                i += 2;
                continue;
            }
            if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                block_comment -= 1;
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

        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            line_comment = true;
            i += 2;
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            block_comment = 1;
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            in_string = true;
            i += 1;
            continue;
        }

        match bytes[i] {
            b'(' | b'[' | b'{' => {
                depth += 1;
                if depth > MAX_RUST_SYNTAX_NESTING {
                    bail!(
                        "Rust source nesting in {} exceeds supported depth {}",
                        path.display(),
                        MAX_RUST_SYNTAX_NESTING
                    );
                }
            }
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    Ok(())
}

const MAX_RUST_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

fn read_rust_source(path: &Path) -> Result<String> {
    let metadata =
        fs::metadata(path).with_context(|| format!("cannot inspect {}", path.display()))?;
    if metadata.len() > MAX_RUST_SOURCE_BYTES {
        bail!(
            "Rust source file {} is {} bytes; maximum supported size is {} bytes",
            path.display(),
            metadata.len(),
            MAX_RUST_SOURCE_BYTES
        );
    }
    fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))
}

fn discover_mutants_with_suppressions(
    root: &Path,
    package_dirs: Option<&[PathBuf]>,
    operator: Option<&str>,
    file_filter: Option<&str>,
    function_filter: Option<&str>,
    exclusions: &GlobSet,
) -> Result<(Vec<Mutant>, Vec<SuppressedMutant>)> {
    let discovery = discover_mutants_detailed(
        root,
        package_dirs,
        operator,
        file_filter,
        function_filter,
        exclusions,
    )?;
    Ok((discovery.mutants, discovery.suppressed))
}

/// Discovery output plus the facts needed to explain an empty selection.
struct Discovery {
    mutants: Vec<Mutant>,
    suppressed: Vec<SuppressedMutant>,
    stats: DiscoveryStats,
}

#[derive(Default)]
struct DiscoveryStats {
    /// Rust source files that were read and parsed successfully.
    files_scanned: usize,
    /// Project-relative Rust source files skipped because they failed to parse.
    unparsable_files: Vec<PathBuf>,
    /// Mutants found before operator/file/function filters and suppression.
    candidates: usize,
}

fn discover_mutants_detailed(
    root: &Path,
    package_dirs: Option<&[PathBuf]>,
    operator: Option<&str>,
    file_filter: Option<&str>,
    function_filter: Option<&str>,
    exclusions: &GlobSet,
) -> Result<Discovery> {
    let mut stats = DiscoveryStats::default();
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve {}", root.display()))?;
    let mut mutants = Vec::new();

    for entry in WalkDir::new(&root).into_iter().filter_entry(|entry| {
        if ignored_entry(entry, &root) {
            return false;
        }
        if entry.path() != root
            && exclusions.is_match(entry.path().strip_prefix(&root).unwrap_or(entry.path()))
        {
            return false;
        }
        if let Some(allowed) = package_dirs {
            let p = entry.path();
            if entry.file_type().is_dir() {
                if !allowed
                    .iter()
                    .any(|dir| dir.starts_with(p) || p.starts_with(dir))
                {
                    return false;
                }
            } else if !allowed.iter().any(|dir| p.starts_with(dir)) {
                return false;
            }
        }
        true
    }) {
        let entry = entry?;
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|s| s.to_str()) != Some("rs")
        {
            continue;
        }

        let source = read_rust_source(entry.path())?;
        validate_source_nesting(&source, entry.path())?;
        let relative = entry
            .path()
            .strip_prefix(&root)
            .unwrap_or(entry.path())
            .to_path_buf();
        let mut syntax = match syn::parse_file(&source) {
            Ok(file) => file,
            Err(_) => {
                stats.unparsable_files.push(relative);
                continue;
            }
        };
        stats.files_scanned += 1;

        // Resolve only explicit local pub use soroban_sdk re-exports inside
        // this parsed file. Never consult other modules, crates, or glob imports.
        sdk_reexports::normalize(&mut syntax);
        let mut visitor = SemanticVisitor::new(&source, relative);
        visitor.visit_file(&syntax);
        mutants.extend(visitor.mutants);
    }

    stats.candidates = mutants.len();
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
        mutants.retain(|m| {
            m.function
                .as_deref()
                .is_some_and(|name| function_name_matches(name, function_filter))
        });
    }

    let (mut mutants, suppressed) = suppress_equivalent_mutants(mutants);

    // Total ordering must never inherit WalkDir's filesystem-dependent traversal order.
    // Keep the first four keys stable: IDs and shard assignments depend on them.
    mutants.sort_by(mutant_order);

    for (index, mutant) in mutants.iter_mut().enumerate() {
        mutant.id = format!("M{:04}", index + 1);
    }

    stats.unparsable_files.sort();
    Ok(Discovery {
        mutants,
        suppressed,
        stats,
    })
}

/// Names bound anywhere in a function body (let, closure, match and loop
/// patterns), excluding nested items. Used so that a local closure or value
/// shadowing a helper name is never mistaken for the helper (AUTH-004).
#[derive(Default)]
struct BindingNames(std::collections::HashSet<String>);

impl<'ast> Visit<'ast> for BindingNames {
    fn visit_item(&mut self, _node: &'ast Item) {}

    fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
        self.0.insert(node.ident.to_string());
        visit::visit_pat_ident(self, node);
    }
}

/// Recognizes the single supported AUTH-004 helper shape: a non-generic,
/// non-async, non-variadic free function returning `()` whose body consists
/// only of `param.require_auth();` statements, where every `param` is a
/// simple parameter explicitly typed `Address` or `&Address`.
fn auth_helper(function: &ItemFn) -> Option<AuthHelper> {
    let sig = &function.sig;
    if !sig.generics.params.is_empty()
        || sig.generics.where_clause.is_some()
        || sig.asyncness.is_some()
        || sig.constness.is_some()
        || sig.unsafety.is_some()
        || sig.abi.is_some()
        || sig.variadic.is_some()
        || !matches!(sig.output, syn::ReturnType::Default)
        || function.block.stmts.is_empty()
    {
        return None;
    }

    let mut address_params = std::collections::HashMap::new();
    for (index, input) in sig.inputs.iter().enumerate() {
        let FnArg::Typed(pat_type) = input else {
            return None;
        };
        if let (Pat::Ident(ident), true) = (pat_type.pat.as_ref(), type_is_address(&pat_type.ty)) {
            if ident.by_ref.is_none() && ident.subpat.is_none() {
                let is_ref = matches!(pat_type.ty.as_ref(), Type::Reference(_));
                address_params.insert(ident.ident.to_string(), (index, is_ref));
            }
        }
    }

    let mut sinks: Vec<(usize, String, bool)> = Vec::new();
    for stmt in &function.block.stmts {
        let Stmt::Expr(Expr::MethodCall(call), Some(_)) = stmt else {
            return None;
        };
        if call.method != "require_auth" || !call.args.is_empty() || call.turbofish.is_some() {
            return None;
        }
        let Expr::Path(receiver) = call.receiver.as_ref() else {
            return None;
        };
        let name = receiver_ident(&Expr::Path(receiver.clone()))?;
        let (index, is_ref) = *address_params.get(&name)?;
        if !sinks.iter().any(|(existing, _, _)| *existing == index) {
            sinks.push((index, name, is_ref));
        }
    }

    Some(AuthHelper {
        arity: sig.inputs.len(),
        sinks,
    })
}

fn recognized_lifecycle(sig: &Signature, body: &syn::Block) -> Option<LifecycleKind> {
    let envs = env_params(sig);
    if envs.is_empty() || address_params(sig).is_empty() {
        return None;
    }
    match sig.ident.to_string().as_str() {
        "initialize" | "init" | "__constructor" | "constructor" => {
            Some(LifecycleKind::Initialization)
        }
        "upgrade" if block_contains_upgrade_wasm(body, &envs) => Some(LifecycleKind::Upgrade),
        _ => None,
    }
}

fn block_contains_upgrade_wasm(body: &syn::Block, env_names: &[String]) -> bool {
    struct UpgradeScan<'a> {
        env_names: &'a [String],
        found: bool,
    }
    impl<'ast> Visit<'ast> for UpgradeScan<'_> {
        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "update_current_contract_wasm" && call.args.len() == 1 {
                if let Expr::MethodCall(deployer) = call.receiver.as_ref() {
                    if deployer.method == "deployer"
                        && deployer.args.is_empty()
                        && receiver_ident(&deployer.receiver)
                            .is_some_and(|name| self.env_names.contains(&name))
                    {
                        self.found = true;
                    }
                }
            }
            visit::visit_expr_method_call(self, call);
        }
    }
    let mut scan = UpgradeScan {
        env_names,
        found: false,
    };
    scan.visit_block(body);
    scan.found
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

fn impl_method_collisions(file: &syn::File) -> std::collections::HashSet<String> {
    fn visit_items(
        items: &[syn::Item],
        module_path: &mut Vec<String>,
        counts: &mut std::collections::HashMap<String, usize>,
    ) {
        for item in items {
            match item {
                syn::Item::Impl(item_impl) => {
                    for impl_item in &item_impl.items {
                        if let syn::ImplItem::Fn(function) = impl_item {
                            let name = function.sig.ident.to_string();
                            let key = if module_path.is_empty() {
                                name
                            } else {
                                format!("{}::{name}", module_path.join("::"))
                            };
                            *counts.entry(key).or_default() += 1;
                        }
                    }
                }
                syn::Item::Mod(item_mod) => {
                    if let Some((_, items)) = &item_mod.content {
                        module_path.push(item_mod.ident.to_string());
                        visit_items(items, module_path, counts);
                        module_path.pop();
                    }
                }
                _ => {}
            }
        }
    }

    let mut counts = std::collections::HashMap::new();
    visit_items(&file.items, &mut Vec::new(), &mut counts);
    counts
        .into_iter()
        .filter_map(|(name, count)| (count > 1).then_some(name))
        .collect()
}

fn type_display_name(ty: &Type) -> String {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        Type::Reference(reference) => type_display_name(&reference.elem),
        _ => "impl".to_owned(),
    }
}

fn address_option_params(sig: &Signature) -> Vec<String> {
    sig.inputs
        .iter()
        .filter_map(|input| match input {
            FnArg::Typed(pat_type) if type_is_option_of_address(&pat_type.ty) => {
                match pat_type.pat.as_ref() {
                    Pat::Ident(ident) => Some(ident.ident.to_string()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn type_is_option_of_address(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => {
            let Some(segment) = path.path.segments.last() else {
                return false;
            };
            if segment.ident != "Option" {
                return false;
            }
            let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
                return false;
            };
            arguments.args.iter().any(|argument| {
                matches!(argument, syn::GenericArgument::Type(inner) if type_is_address(inner))
            })
        }
        Type::Reference(reference) => type_is_option_of_address(&reference.elem),
        _ => false,
    }
}

fn option_address_binding(pat: &Pat) -> Option<String> {
    match pat {
        Pat::TupleStruct(tuple)
            if tuple
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "Some")
                && tuple.elems.len() == 1 =>
        {
            match tuple.elems.first() {
                Some(Pat::Ident(ident)) => Some(ident.ident.to_string()),
                Some(Pat::Reference(reference)) => match reference.pat.as_ref() {
                    Pat::Ident(ident) => Some(ident.ident.to_string()),
                    _ => None,
                },
                _ => None,
            }
        }
        Pat::Reference(reference) => option_address_binding(&reference.pat),
        _ => None,
    }
}

fn address_vec_params(sig: &Signature) -> Vec<String> {
    sig.inputs
        .iter()
        .filter_map(|input| match input {
            FnArg::Typed(pat_type) if type_is_vec_of_address(&pat_type.ty) => {
                match pat_type.pat.as_ref() {
                    Pat::Ident(ident) => Some(ident.ident.to_string()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn type_is_vec_of_address(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => {
            let Some(segment) = path.path.segments.last() else {
                return false;
            };
            if segment.ident != "Vec" {
                return false;
            }
            let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
                return false;
            };
            arguments.args.iter().any(|argument| {
                matches!(argument, syn::GenericArgument::Type(inner) if type_is_address(inner))
            })
        }
        Type::Reference(reference) => type_is_vec_of_address(&reference.elem),
        _ => false,
    }
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

fn numeric_params(sig: &Signature) -> Vec<String> {
    sig.inputs
        .iter()
        .filter_map(|input| match input {
            FnArg::Typed(pat_type) if type_is_numeric_primitive(&pat_type.ty) => {
                match pat_type.pat.as_ref() {
                    Pat::Ident(ident) => Some(ident.ident.to_string()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn type_is_numeric_primitive(ty: &Type) -> bool {
    match ty {
        Type::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => {
            matches!(
                path.path.segments[0].ident.to_string().as_str(),
                "i8" | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "isize"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "usize"
            )
        }
        Type::Reference(reference) => type_is_numeric_primitive(&reference.elem),
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

fn is_generated_contract_client_constructor(expr: &Expr) -> bool {
    match expr {
        Expr::Call(call) => match call.func.as_ref() {
            Expr::Path(path) => {
                let names = path
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>();
                names.last().is_some_and(|name| name == "new")
                    && names.len() >= 2
                    && names[names.len() - 2].ends_with("Client")
                    && !names[..names.len() - 1]
                        .iter()
                        .any(|name| name.to_ascii_lowercase().contains("token"))
            }
            _ => false,
        },
        Expr::Paren(paren) => is_generated_contract_client_constructor(&paren.expr),
        Expr::Reference(reference) => is_generated_contract_client_constructor(&reference.expr),
        _ => false,
    }
}

fn is_generated_contract_client_method_call(expr: &Expr, known_clients: &[String]) -> bool {
    match expr {
        Expr::MethodCall(call) => receiver_ident(&call.receiver)
            .is_some_and(|name| known_clients.iter().any(|candidate| candidate == &name)),
        Expr::Paren(paren) => is_generated_contract_client_method_call(&paren.expr, known_clients),
        _ => false,
    }
}

/// Supported allowance-related Soroban token-interface calls for TOKEN-002,
/// keyed by method name and exact argument count. Returns the labels of the
/// two leading `Address` role arguments that the operator swaps.
///
/// - `approve(from, spender, amount, expiration_ledger)`
/// - `allowance(from, spender)`
/// - `transfer_from(spender, from, to, amount)`
/// - `burn_from(spender, from, amount)`
fn allowance_role_labels(method: &str, arg_count: usize) -> Option<(&'static str, &'static str)> {
    match (method, arg_count) {
        ("approve", 4) | ("allowance", 2) => Some(("owner", "spender")),
        ("transfer_from", 4) | ("burn_from", 3) => Some(("spender", "owner")),
        _ => None,
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

fn local_binding_name_and_init(local: &Local) -> Option<(String, &Expr)> {
    let init = local.init.as_ref()?;
    let ident = match &local.pat {
        Pat::Ident(ident) => ident,
        Pat::Type(typed) => match typed.pat.as_ref() {
            Pat::Ident(ident) => ident,
            _ => return None,
        },
        _ => return None,
    };
    Some((ident.ident.to_string(), init.expr.as_ref()))
}

fn is_direct_env_storage_receiver(expr: &Expr, env_params: &[String]) -> bool {
    match expr {
        Expr::MethodCall(call) if call.method == "storage" && call.args.is_empty() => {
            receiver_ident(&call.receiver)
                .is_some_and(|name| env_params.iter().any(|candidate| candidate == &name))
        }
        Expr::Paren(paren) => is_direct_env_storage_receiver(&paren.expr, env_params),
        Expr::Reference(reference) => is_direct_env_storage_receiver(&reference.expr, env_params),
        _ => false,
    }
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

    // proc-macro2 line/column positions count Unicode scalar values. The
    // mutation engine uses UTF-8 byte offsets instead; report byte columns
    // explicitly so all consumers see one stable, documented convention.
    source.get(start_byte..end_byte)?;
    let first_on_start_line = line_column_to_offset(source, start.line, 0)?;
    let first_on_end_line = line_column_to_offset(source, end.line, 0)?;
    Some((
        start_byte,
        end_byte,
        SourceSpan {
            line: start.line,
            column: start_byte - first_on_start_line + 1,
            end_line: end.line,
            end_column: end_byte - first_on_end_line + 1,
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
            // Span columns count Unicode characters. Convert the character
            // boundary to its byte index before slicing source.
            return line
                .char_indices()
                .map(|(byte, _)| byte)
                .chain(std::iter::once(line.len()))
                .nth(target_column)
                .map(|byte| offset + byte);
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

fn target_dir_base(root: &Path, target_dir: Option<&Path>) -> PathBuf {
    match target_dir {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => root.join(path),
        None => root.join(".soro-mutants-target"),
    }
}

fn target_dirs(root: &Path, target_dir: Option<&Path>) -> (PathBuf, PathBuf) {
    let base = target_dir_base(root, target_dir);
    (base.join("baseline"), base.join("mutants-shared"))
}

fn resolve_scratch_root(root: &Path, scratch_dir: Option<&Path>) -> Result<PathBuf> {
    let Some(requested) = scratch_dir else {
        return Ok(root.join(pathguard::SCRATCH_NAME));
    };

    let base = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        env::current_dir()
            .context("cannot resolve current directory for --scratch-dir")?
            .join(requested)
    };
    fs::create_dir_all(&base)
        .with_context(|| format!("cannot create --scratch-dir {}", base.display()))?;
    let base = base
        .canonicalize()
        .with_context(|| format!("cannot resolve --scratch-dir {}", base.display()))?;

    if base == root || root.starts_with(&base) {
        bail!(
            "--scratch-dir {} must not be the project root or one of its parents",
            base.display()
        );
    }

    let scratch_root = base.join(pathguard::SCRATCH_NAME);
    pathguard::require_owned_scratch_path_under(&scratch_root, &scratch_root)?;
    Ok(scratch_root)
}

fn validate_scratch_root_writable(scratch_root: &Path) -> Result<()> {
    pathguard::require_owned_scratch_path_under(scratch_root, scratch_root)?;
    let existed = scratch_root.exists();
    fs::create_dir_all(scratch_root).with_context(|| {
        format!(
            "scratch root {} cannot be created; choose a writable --scratch-dir",
            scratch_root.display()
        )
    })?;

    let probe = scratch_root.join(format!(".soro-mutants-write-probe-{}", std::process::id()));
    let probe_result = (|| -> Result<()> {
        fs::write(&probe, b"soro-mutants scratch probe").with_context(|| {
            format!(
                "scratch root {} is not writable; choose a writable --scratch-dir",
                scratch_root.display()
            )
        })?;
        fs::remove_file(&probe).with_context(|| {
            format!(
                "scratch root {} does not permit deleting temporary files",
                scratch_root.display()
            )
        })?;
        Ok(())
    })();

    if !existed {
        let _ = fs::remove_dir(scratch_root);
    }
    probe_result
}

fn reject_cleanup_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "refusing to clean symlinked Soro Mutants path {}",
                path.display()
            )
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot inspect {}", path.display())),
    }
}

/// Root of the per-worker Cargo caches used by parallel execution (`--jobs` > 1).
///
/// It is a sibling of the baseline and serial mutant caches, never a parent or
/// child of either, so mutant builds cannot write into baseline artifacts.
fn parallel_target_root(root: &Path, target_dir: Option<&Path>) -> PathBuf {
    let (baseline, _) = target_dirs(root, target_dir);
    baseline
        .parent()
        .map(|base| base.join("mutants-parallel"))
        .unwrap_or_else(|| root.join(".soro-mutants-target/mutants-parallel"))
}

/// Scratch worktree and Cargo target directory for one parallel worker.
///
/// Each worker owns both paths for the whole run: no two workers ever share a
/// source tree or a `CARGO_TARGET_DIR`, while a worker's stable paths still let
/// Cargo reuse dependency builds between the mutants it executes.
fn worker_paths_with_scratch(
    root: &Path,
    target_dir: Option<&Path>,
    scratch_root: &Path,
    worker: usize,
) -> (PathBuf, PathBuf) {
    let name = format!("worker-{}", worker + 1);
    (
        scratch_root.join(&name),
        parallel_target_root(root, target_dir).join(name),
    )
}

#[cfg(test)]
fn worker_paths(root: &Path, target_dir: Option<&Path>, worker: usize) -> (PathBuf, PathBuf) {
    worker_paths_with_scratch(
        root,
        target_dir,
        &root.join(pathguard::SCRATCH_NAME),
        worker,
    )
}

fn managed_cleanup_paths(root: &Path, target_dir: Option<&Path>) -> Result<Vec<PathBuf>> {
    let scratch = root.join(".soro-mutants-worktree");
    let (baseline, mutants) = target_dirs(root, target_dir);
    let parallel = parallel_target_root(root, target_dir);
    let base = target_dir_base(root, target_dir);
    let progress = base.join("progress.json");
    let identity = base.join("cache-identity.json");
    let result_cache = result_cache_path(&base);
    let target_root = baseline.parent().unwrap_or(root);
    reject_cleanup_symlink(&scratch)?;
    reject_cleanup_symlink(target_root)?;
    reject_cleanup_symlink(&baseline)?;
    reject_cleanup_symlink(&mutants)?;
    reject_cleanup_symlink(&parallel)?;
    let mut paths: Vec<PathBuf> = [
        scratch,
        baseline,
        mutants,
        parallel,
        progress,
        identity,
        result_cache,
    ]
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
                    Some(
                        "baseline"
                            | "mutants-shared"
                            | "mutants-parallel"
                            | "progress.json"
                            | "cache-identity.json"
                            | "result-cache.json"
                    )
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
                // Content-based ownership for the cache root was already
                // verified in `managed_cleanup_paths` without mutating it.
                if path.exists() {
                    fs::remove_dir(path)
                        .with_context(|| format!("failed to remove {}", path.display()))?;
                }
            } else {
                // #184: cover the metadata files too, not just directories.
                let cache_root = target_dir_base(root, target_dir);
                if path.starts_with(root.join(pathguard::SCRATCH_NAME)) {
                    pathguard::require_owned_scratch_path(path, root)?;
                } else {
                    pathguard::require_owned_cache_path(path, &cache_root)?;
                }
                if path.is_file() {
                    fs::remove_file(path)
                        .with_context(|| format!("failed to remove {}", path.display()))?;
                } else if path.is_dir() {
                    fs::remove_dir_all(path)
                        .with_context(|| format!("failed to remove {}", path.display()))?;
                }
            }
        }
    }
    Ok(paths)
}

struct ExecutionSettings<'a> {
    test_dir: Option<&'a Path>,
    test_command: &'a str,
    timeouts: Timeouts,
    /// The user's `--target-dir`, if any; Soro Mutants derives its caches from it.
    target_dir: Option<&'a Path>,
    /// Resolved root directory for mutation worktrees.
    scratch_root: &'a Path,
    jobs: usize,
    timing: bool,
}

/// Execute every mutant and return their outcomes in input order.
///
/// `on_result` is called on the calling thread, in input order, as soon as the
/// next outcome in sequence is available. Output is therefore identical for
/// serial and parallel runs, independent of worker completion order.
///
/// With `jobs == 1` the historical serial layout is used: one scratch worktree
/// at `.soro-mutants-worktree` and the shared `mutants-shared` Cargo cache.
/// With `jobs > 1`, each worker gets its own scratch worktree under
/// `.soro-mutants-worktree/worker-N` and its own Cargo target directory under
/// `mutants-parallel/worker-N`, so concurrent mutants never share source files
/// or build output, and no worker can write to the baseline cache.
fn execute_mutants(
    root: &Path,
    mutants: &[Mutant],
    settings: &ExecutionSettings<'_>,
    on_result: &mut dyn FnMut(&Mutant, &Outcome) -> Result<()>,
) -> Result<Vec<MutantExecution>> {
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    };

    let (baseline_target, serial_target) = target_dirs(root, settings.target_dir);
    let mut ignored = soro_target_base(root, settings.target_dir)
        .into_iter()
        .collect::<Vec<_>>();
    if settings.scratch_root.starts_with(root) {
        ignored.push(settings.scratch_root.to_path_buf());
    }
    let workers = settings.jobs.max(1).min(mutants.len());

    if workers <= 1 {
        let scratch = settings.scratch_root.to_path_buf();
        let mut executions = Vec::with_capacity(mutants.len());
        for mutant in mutants {
            let execution = execute_mutant(
                root,
                mutant,
                settings.test_dir,
                settings.test_command,
                settings.timeouts,
                &scratch,
                settings.scratch_root,
                &serial_target,
                &ignored,
                settings.timing,
            )?;
            on_result(mutant, &execution.outcome)?;
            executions.push(execution);
        }
        return Ok(executions);
    }

    let worker_dirs = (0..workers)
        .map(|worker| {
            worker_paths_with_scratch(root, settings.target_dir, settings.scratch_root, worker)
        })
        .collect::<Vec<_>>();
    for (_, target) in &worker_dirs {
        if target == &baseline_target || target.starts_with(&baseline_target) {
            bail!(
                "refusing to share the baseline cache {} with a mutant worker",
                baseline_target.display()
            );
        }
    }

    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let mut slots: Vec<Option<MutantExecution>> = vec![None; mutants.len()];
    let mut errors: Vec<(usize, anyhow::Error)> = Vec::new();

    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel::<(usize, Result<MutantExecution>)>();
        for (scratch, target) in &worker_dirs {
            let sender = sender.clone();
            let (next, stop, ignored) = (&next, &stop, &ignored);
            scope.spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(mutant) = mutants.get(index) else {
                        break;
                    };
                    let result = execute_mutant(
                        root,
                        mutant,
                        settings.test_dir,
                        settings.test_command,
                        settings.timeouts,
                        scratch,
                        settings.scratch_root,
                        target,
                        ignored,
                        settings.timing,
                    );
                    let failed = result.is_err();
                    if failed {
                        stop.store(true, Ordering::SeqCst);
                    }
                    if sender.send((index, result)).is_err() || failed {
                        break;
                    }
                }
            });
        }
        drop(sender);

        let mut reported = 0;
        for (index, result) in receiver {
            match result {
                Ok(execution) => slots[index] = Some(execution),
                Err(error) => {
                    stop.store(true, Ordering::SeqCst);
                    errors.push((index, error));
                }
            }
            while errors.is_empty() {
                let Some(Some(execution)) = slots.get(reported) else {
                    break;
                };
                if let Err(error) = on_result(&mutants[reported], &execution.outcome) {
                    stop.store(true, Ordering::SeqCst);
                    errors.push((reported, error));
                    break;
                }
                reported += 1;
            }
        }
    });

    pathguard::require_owned_scratch_path_under(settings.scratch_root, settings.scratch_root)?;
    let cleanup = if settings.scratch_root.exists() {
        fs::remove_dir_all(settings.scratch_root).with_context(|| {
            format!(
                "failed to clean mutation scratch {}",
                settings.scratch_root.display()
            )
        })
    } else {
        Ok(())
    };

    if let Some((_, error)) = errors.into_iter().min_by_key(|(index, _)| *index) {
        return Err(error);
    }
    cleanup?;
    slots
        .into_iter()
        .map(|slot| slot.context("parallel mutant execution ended without an outcome"))
        .collect()
}

/// The Soro Mutants cache base when it lives inside the project root. It is
/// excluded from scratch copies so that one worker never copies another
/// worker's (concurrently changing) build output into its own source tree.
fn soro_target_base(root: &Path, target_dir: Option<&Path>) -> Option<PathBuf> {
    let (baseline, _) = target_dirs(root, target_dir);
    let base = baseline.parent()?.to_path_buf();
    (base != root && base.starts_with(root)).then_some(base)
}

#[allow(clippy::too_many_arguments)]
fn execute_mutant(
    root: &Path,
    mutant: &Mutant,
    test_dir: Option<&Path>,
    test_command: &str,
    timeouts: Timeouts,
    scratch: &Path,
    scratch_root: &Path,
    target_dir: &Path,
    ignored: &[PathBuf],
    timing: bool,
) -> Result<MutantExecution> {
    let total_started = Instant::now();

    // #184: verify ownership before either the pre-run clear or post-run cleanup.
    pathguard::require_owned_scratch_path_under(scratch, scratch_root)?;
    if scratch.exists() {
        fs::remove_dir_all(scratch)
            .with_context(|| format!("failed to clear scratch {}", scratch.display()))?;
    }
    copy_project(root, scratch, ignored)?;
    apply_mutant(scratch, mutant)?;

    let result = (|| -> Result<(Outcome, Option<u64>, Option<u64>)> {
        let scratch_cwd = test_dir
            .map(|dir| scratch.join(dir))
            .unwrap_or_else(|| scratch.to_path_buf());

        let mut compile_ms = None;
        if let Some(compile_command) = compile_command_for(test_command) {
            let compile_started = Instant::now();
            let compile_result =
                run_process(&scratch_cwd, &compile_command, timeouts.compile, target_dir)?;
            if timing {
                compile_ms = Some(duration_ms(compile_started.elapsed()));
            }
            match compile_result {
                ProcessResult::Failed => return Ok((Outcome::Unviable, compile_ms, None)),
                ProcessResult::Timeout => return Ok((Outcome::Timeout, compile_ms, None)),
                ProcessResult::Passed => {}
            }
        }

        let test_started = Instant::now();
        let test_result = run_process(&scratch_cwd, test_command, timeouts.test, target_dir)?;
        let test_ms = timing.then(|| duration_ms(test_started.elapsed()));
        let outcome = match test_result {
            ProcessResult::Passed => Outcome::Survived,
            ProcessResult::Failed => Outcome::Killed,
            ProcessResult::Timeout => Outcome::Timeout,
        };
        Ok((outcome, compile_ms, test_ms))
    })();

    let cleanup_result = if scratch.exists() {
        fs::remove_dir_all(scratch)
            .with_context(|| format!("failed to clean mutation scratch {}", scratch.display()))
    } else {
        Ok(())
    };

    match (result, cleanup_result) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Ok((outcome, compile_ms, test_ms)), Ok(())) => Ok(MutantExecution {
            outcome,
            timing: timing.then(|| MutantTiming {
                total_ms: duration_ms(total_started.elapsed()),
                compile_ms,
                test_ms,
            }),
        }),
    }
}

/// Preserve the original I/O error while explaining a filesystem path-length failure.
fn path_io_error(action: &str, path: &Path, error: io::Error) -> anyhow::Error {
    // ENAMETOOLONG (Unix) and ERROR_FILENAME_EXCED_RANGE (Windows).
    if matches!(error.raw_os_error(), Some(36 | 206)) {
        anyhow::anyhow!(
            "filesystem path is too long while {action} {}: use a shorter project or --target-dir path ({error})",
            path.display()
        )
    } else {
        anyhow::anyhow!("failed while {action} {}: {error}", path.display())
    }
}

fn create_copy_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|error| path_io_error("creating directory", path, error))
}

fn copy_with_permissions(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        create_copy_directory(parent)?;
    }
    fs::copy(source, target).map_err(|error| path_io_error("copying file to", target, error))?;
    let permissions = fs::metadata(source)
        .map_err(|error| path_io_error("reading permissions of", source, error))?
        .permissions();
    fs::set_permissions(target, permissions)
        .map_err(|error| path_io_error("setting permissions on", target, error))?;
    Ok(())
}

fn copy_project(root: &Path, destination: &Path, ignored: &[PathBuf]) -> Result<()> {
    create_copy_directory(destination)?;
    // Restore directory permissions after copying children, otherwise read-only
    // source directories could prevent us from creating their contents.
    let mut directories = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_entry(|entry| {
        !ignored_entry(entry, root) && !ignored.iter().any(|path| entry.path() == path)
    }) {
        let entry = entry.with_context(|| format!("cannot traverse project {}", root.display()))?;
        let relative = entry.path().strip_prefix(root)?;
        if relative.as_os_str().is_empty() {
            continue;
        }

        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            create_copy_directory(&target)?;
            directories.push((target, entry.metadata()?.permissions()));
        } else if entry.file_type().is_file() {
            copy_with_permissions(entry.path(), &target)?;
        }
    }

    copy_wasm_fixtures(root, destination, ignored)?;
    for (path, permissions) in directories.into_iter().rev() {
        fs::set_permissions(&path, permissions)
            .map_err(|error| path_io_error("restoring directory permissions on", &path, error))?;
    }
    Ok(())
}

fn copy_wasm_fixtures(root: &Path, destination: &Path, ignored: &[PathBuf]) -> Result<()> {
    // Prebuilt WASM fixtures may live under `target/`, so that directory is
    // walked here, but Soro Mutants' own scratch worktrees and caches are not:
    // another worker may be writing to them concurrently.
    for entry in WalkDir::new(root).into_iter().filter_entry(|entry| {
        entry.path() == root
            || (!matches!(
                entry.file_name().to_str(),
                Some(".git" | ".soro-mutants-target" | ".soro-mutants-worktree" | "node_modules")
            ) && !ignored.iter().any(|path| entry.path() == path))
    }) {
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
        copy_with_permissions(entry.path(), &target)?;
    }
    Ok(())
}

fn resolve_runner_command(runner: Runner, test_command: &str) -> Result<String> {
    let resolved = match runner {
        Runner::CargoTest => test_command.to_owned(),
        Runner::Nextest if test_command == "cargo test -q" => "cargo nextest run".to_owned(),
        Runner::Nextest => bail!(
            "cannot combine --runner nextest with --test-command; choose one execution method"
        ),
    };
    // #183: every CLI test command reaches a shell; reject corrupt bytes here
    // so the error is actionable before any worktree or cache is created.
    command::validate_shell_command(&resolved, "--test-command")?;
    Ok(resolved)
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
    if let Some(stripped) = test_command.strip_prefix("cargo nextest run") {
        let tail = stripped.trim();
        if tail.is_empty() {
            return Some("cargo test --no-run".to_string());
        } else {
            return Some(format!("cargo test --no-run {}", tail));
        }
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
    let source = read_rust_source(&path)?;
    let mutated = replace_source_range(
        &source,
        mutant.start_byte,
        mutant.end_byte,
        &mutant.original,
        &mutant.replacement,
    )
    .with_context(|| format!("cannot apply {} at {}", mutant.id, mutant.file.display()))?;
    fs::write(path, mutated)?;
    Ok(())
}

fn replace_source_range(
    source: &str,
    start_byte: usize,
    end_byte: usize,
    expected: &str,
    replacement: &str,
) -> Result<String> {
    if start_byte >= end_byte
        || end_byte > source.len()
        || !source.is_char_boundary(start_byte)
        || !source.is_char_boundary(end_byte)
    {
        bail!("invalid UTF-8 source span {start_byte}..{end_byte}");
    }

    if &source[start_byte..end_byte] != expected {
        bail!("source text does not match the selected mutation range");
    }

    let mut mutated = String::with_capacity(source.len() + replacement.len());
    mutated.push_str(&source[..start_byte]);
    mutated.push_str(replacement);
    mutated.push_str(&source[end_byte..]);
    Ok(mutated)
}

const PROCESS_EXCERPT_LIMIT: usize = 8 * 1024;
const TERMINATION_GRACE: Duration = Duration::from_millis(300);

fn read_bounded<R: Read>(mut reader: R) -> io::Result<Vec<u8>> {
    let mut kept = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let remaining = PROCESS_EXCERPT_LIMIT.saturating_sub(kept.len());
        if remaining > 0 {
            kept.extend_from_slice(&buffer[..count.min(remaining)]);
        }
    }
    Ok(kept)
}

fn one_line_excerpt(value: &str) -> String {
    redact_environment_secrets(value)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
}

fn classify_failed_process(exit_code: Option<i32>, stderr: &str) -> ProcessFailureKind {
    let lower = stderr.to_ascii_lowercase();
    if exit_code == Some(127) || lower.contains("command not found") {
        ProcessFailureKind::CommandNotFound
    } else if lower.contains("could not compile") {
        ProcessFailureKind::Compile
    } else {
        ProcessFailureKind::Test
    }
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_process_tree(child: &mut Child) -> io::Result<()> {
    let group = format!("-{}", child.id());
    let _ = Command::new("kill").args(["-TERM", "--", &group]).status();
    let started = Instant::now();
    while started.elapsed() < TERMINATION_GRACE {
        let _ = child.try_wait()?;
        std::thread::sleep(Duration::from_millis(20));
    }
    // Always target the whole group after the grace period. The shell leader
    // may have exited while a descendant still owns stdout/stderr pipes.
    let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
    let _ = child.wait();
    Ok(())
}

#[cfg(not(unix))]
fn terminate_process_tree(child: &mut Child) -> io::Result<()> {
    child.kill()?;
    let _ = child.wait();
    Ok(())
}

fn run_process_detailed(
    cwd: &Path,
    command: &str,
    timeout_seconds: u64,
    target_dir: &Path,
) -> Result<ProcessExecution> {
    command::validate_shell_command(command, "test command")?;
    fs::create_dir_all(target_dir)?;

    let mut command_builder = Command::new("bash");
    command_builder
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .env("CARGO_TARGET_DIR", target_dir)
        .env_remove("RUSTUP_TOOLCHAIN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut command_builder);

    let mut child = match command_builder.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Ok(ProcessExecution {
                result: ProcessResult::Failed,
                failure_kind: Some(ProcessFailureKind::Spawn),
                stdout: String::new(),
                stderr: error.to_string(),
                exit_code: None,
            });
        }
    };

    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("cannot capture test command stdout"))?;
    let child_stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("cannot capture test command stderr"))?;
    let stdout_reader = std::thread::spawn(move || read_bounded(child_stdout));
    let stderr_reader = std::thread::spawn(move || read_bounded(child_stderr));

    let (result, exit_code) = match child.wait_timeout(Duration::from_secs(timeout_seconds))? {
        Some(status) if status.success() => (ProcessResult::Passed, status.code()),
        Some(status) => (ProcessResult::Failed, status.code()),
        None => {
            terminate_process_tree(&mut child)?;
            (ProcessResult::Timeout, None)
        }
    };

    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow::anyhow!("cannot collect test command stdout"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow::anyhow!("cannot collect test command stderr"))??;
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
    let failure_kind = if result == ProcessResult::Failed {
        Some(classify_failed_process(exit_code, &stderr))
    } else {
        None
    };

    Ok(ProcessExecution {
        result,
        failure_kind,
        stdout,
        stderr,
        exit_code,
    })
}

fn run_process(
    cwd: &Path,
    command: &str,
    timeout_seconds: u64,
    target_dir: &Path,
) -> Result<ProcessResult> {
    let execution = run_process_detailed(cwd, command, timeout_seconds, target_dir)?;
    if !execution.stderr.is_empty() {
        io::stderr().write_all(redact_environment_secrets(&execution.stderr).as_bytes())?;
    }
    if let Some(kind @ (ProcessFailureKind::CommandNotFound | ProcessFailureKind::Spawn)) =
        execution.failure_kind
    {
        bail!(
            "test process launch failed ({}) in {} for `{}`",
            kind.label(),
            cwd.display(),
            redact_environment_secrets(command)
        );
    }
    Ok(execution.result)
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

#[derive(Serialize)]
struct OperatorsReport {
    schema_version: u32,
    operators: Vec<OperatorDescription>,
}

#[derive(Serialize)]
struct OperatorDescription {
    id: &'static str,
    description: &'static str,
}

impl OperatorsReport {
    fn current() -> Self {
        Self {
            schema_version: 1,
            operators: BUILTIN_OPERATORS
                .iter()
                .map(|(id, description)| OperatorDescription { id, description })
                .collect(),
        }
    }
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
            let path = normalized_report_path(&mutant.file);
            let target = encode_markdown_link_path(&path);
            let link_label = escape_markdown_link_label(&path);
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

fn encode_markdown_link_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

fn escape_markdown_link_label(path: &str) -> String {
    let mut escaped = String::with_capacity(path.len());
    for character in path.chars() {
        if matches!(character, '\\' | '`' | '*' | '_' | '[' | ']' | '!') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
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
    fn path_length_failure_recommends_shorter_project_or_target_directory() {
        let error = path_io_error(
            "copying file to",
            Path::new("/deep/nested/file.rs"),
            io::Error::from_raw_os_error(36),
        );
        let message = error.to_string();
        assert!(message.contains("/deep/nested/file.rs"));
        assert!(message.contains("--target-dir"));
        assert!(message.contains("too long"));
    }

    #[test]
    fn ordinary_copy_errors_keep_the_original_cause() {
        let error = path_io_error(
            "copying file to",
            Path::new("example.rs"),
            io::Error::from_raw_os_error(2),
        );
        assert!(error.to_string().contains("example.rs"));
        assert!(
            error.to_string().contains("No such file") || error.to_string().contains("cannot find")
        );
    }

    #[cfg(unix)]
    #[test]
    fn copying_project_preserves_executable_and_directory_permission_bits() {
        use std::os::unix::fs::PermissionsExt;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let fixture =
            env::temp_dir().join(format!("soro-copy-mode-{}-{nonce}", std::process::id()));
        let root = fixture.join("source");
        let destination = fixture.join("mutant");
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(root.join("bin/run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(root.join("bin/run.sh"), fs::Permissions::from_mode(0o750)).unwrap();
        fs::set_permissions(root.join("bin"), fs::Permissions::from_mode(0o750)).unwrap();
        copy_project(&root, &destination, &[]).unwrap();
        assert_eq!(
            fs::metadata(destination.join("bin/run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o750
        );
        assert_eq!(
            fs::metadata(destination.join("bin"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o750
        );
        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn machine_report_paths_use_forward_slashes() {
        #[derive(Serialize)]
        struct PathReport {
            #[serde(serialize_with = "serialize_portable_path")]
            file: PathBuf,
        }

        let report = PathReport {
            file: PathBuf::from(r"src\space dir\lib.rs"),
        };
        assert_eq!(
            serde_json::to_string(&report).unwrap(),
            r#"{"file":"src/space dir/lib.rs"}"#
        );
    }

    #[test]
    fn markdown_link_paths_encode_special_characters() {
        assert_eq!(
            encode_markdown_link_path("src/space dir/a#b%[c](d)?e&é.rs"),
            "src/space%20dir/a%23b%25%5Bc%5D%28d%29%3Fe%26%C3%A9.rs"
        );
        assert_eq!(
            escape_markdown_link_label("src/[x]_y.rs"),
            "src/\\[x\\]\\_y.rs"
        );
    }

    #[test]
    fn property_source_replacements_preserve_bytes_outside_selected_range() {
        const CHARS: &[char] = &['a', 'Z', '0', ' ', '\n', 'é', 'λ', '🦀'];
        let mut state = 0x7f4a_7c15_d1ce_4e5b_u64;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (state >> 32) as usize
        };

        for case in 0..256 {
            let source_len = 2 + next() % 39;
            let source: String = (0..source_len)
                .map(|_| CHARS[next() % CHARS.len()])
                .collect();
            let boundaries: Vec<usize> = source
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(source.len()))
                .collect();
            let start_index = next() % (boundaries.len() - 1);
            let end_index = start_index + 1 + next() % (boundaries.len() - start_index - 1);
            let start_byte = boundaries[start_index];
            let end_byte = boundaries[end_index];
            let replacement: String = (0..next() % 9)
                .map(|_| CHARS[next() % CHARS.len()])
                .collect();
            let mutated = replace_source_range(
                &source,
                start_byte,
                end_byte,
                &source[start_byte..end_byte],
                &replacement,
            )
            .expect("generated valid source ranges should be replaceable");

            assert_eq!(
                &mutated.as_bytes()[..start_byte],
                &source.as_bytes()[..start_byte],
                "prefix changed outside case {case}"
            );
            assert_eq!(
                &mutated.as_bytes()[start_byte + replacement.len()..],
                &source.as_bytes()[end_byte..],
                "suffix changed outside case {case}"
            );
        }
    }

    #[test]
    fn source_replacement_preserves_comments_indentation_and_crlf_exactly() {
        let source =
            "fn f() {\r\n    call(/* before */ admin.require_auth() /* after */); // tail\r\n}\r\n";
        let target = "admin.require_auth()";
        let start = source.find(target).unwrap();
        let end = start + target.len();
        let mutated = replace_source_range(source, start, end, target, "other.require_auth()")
            .expect("minimal source replacement should succeed");
        assert_eq!(
            mutated,
            "fn f() {\r\n    call(/* before */ other.require_auth() /* after */); // tail\r\n}\r\n"
        );
    }

    #[test]
    fn source_replacement_rejects_invalid_utf8_boundaries_and_stale_text() {
        assert!(replace_source_range("aλb", 2, 3, "λ", "x").is_err());
        assert!(replace_source_range("abc", 1, 4, "bc", "x").is_err());
        assert!(replace_source_range("abc", 1, 2, "x", "y").is_err());
    }

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
        // A distinct, non-overlapping synthetic edit in the same file.
        third.start_byte = first.end_byte;
        third.end_byte = first.end_byte + 2;
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
            timing: None,
        };
        let killed = MutantResult {
            mutant,
            outcome: Outcome::Killed,
            timing: None,
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

    fn runner_cleanup_unit_fixture(replacement: &str) -> (PathBuf, Mutant) {
        static FIXTURE_SEQUENCE: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "soro-runner-cleanup-unit-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"cleanup-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let source = "pub fn value() -> i32 { 1 }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn value_is_one() { assert_eq!(super::value(), 1); }\n}\n";
        fs::write(root.join("src/lib.rs"), source).unwrap();
        let marker = "{ 1 }";
        let start_byte = source.find(marker).unwrap() + 2;
        let mutant = Mutant {
            id: "M-CLEANUP".into(),
            operator: "TEST".into(),
            file: PathBuf::from("src/lib.rs"),
            function: Some("value".into()),
            span: SourceSpan {
                line: 1,
                column: 24,
                end_line: 1,
                end_column: 25,
            },
            original: "1".into(),
            replacement: replacement.into(),
            description: "cleanup regression".into(),
            start_byte,
            end_byte: start_byte + 1,
        };
        (root, mutant)
    }

    #[test]
    fn execute_mutant_cleans_scratch_after_compile_failure() {
        let (root, mutant) = runner_cleanup_unit_fixture("unknown_symbol");
        let scratch = root.join(".soro-mutants-worktree");
        let target = root
            .join(".soro-mutants-target")
            .join("unit-compile-failure");
        let outcome = execute_mutant(
            &root,
            &mutant,
            None,
            "cargo test -q",
            Timeouts {
                compile: 60,
                test: 60,
            },
            &scratch,
            &scratch,
            &target,
            &[],
            false,
        )
        .unwrap();
        assert_eq!(outcome.outcome, Outcome::Unviable);
        assert!(!scratch.exists(), "scratch worktree must be removed");
        assert!(fs::read_to_string(root.join("src/lib.rs"))
            .unwrap()
            .contains("pub fn value() -> i32 { 1 }"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn execute_mutant_cleans_scratch_after_test_failure() {
        let (root, mutant) = runner_cleanup_unit_fixture("2");
        let scratch = root.join(".soro-mutants-worktree");
        let target = root.join(".soro-mutants-target").join("unit-test-failure");
        let outcome = execute_mutant(
            &root,
            &mutant,
            None,
            "cargo test -q",
            Timeouts {
                compile: 60,
                test: 60,
            },
            &scratch,
            &scratch,
            &target,
            &[],
            false,
        )
        .unwrap();
        assert_eq!(outcome.outcome, Outcome::Killed);
        assert!(!scratch.exists(), "scratch worktree must be removed");
        assert!(fs::read_to_string(root.join("src/lib.rs"))
            .unwrap()
            .contains("pub fn value() -> i32 { 1 }"));
        fs::remove_dir_all(root).unwrap();
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
            timing: None,
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
        let report = serde_json::to_value(list_report(&mutants, None)).unwrap();
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
        let report = serde_json::to_value(list_report(&[mutant], None)).unwrap();
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
            timing: None,
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
                timing: None,
            }];
            let report = serde_json::to_value(test_report(&results)).unwrap();
            let example: serde_json::Value =
                serde_json::from_str(include_str!("../docs/json-report-example.json")).unwrap();
            let mut expected_results = example["results"].clone();
            expected_results[0]["outcome"] = label.into();
            assert_eq!(report["schema_version"], example["schema_version"]);
            assert_eq!(report["results"], expected_results);
            assert_eq!(report["by_operator"]["AUTH-001"]["generated"], 1);
            assert_eq!(report["by_file"]["src/lib.rs"]["generated"], 1);
            assert_eq!(report["by_function"]["guarded"]["generated"], 1);
        }
    }

    #[test]
    fn empty_json_reports_are_versioned() {
        assert_eq!(
            serde_json::to_value(list_report(&[], None)).unwrap(),
            serde_json::json!({"schema_version": 1, "mutants": []})
        );
        assert_eq!(
            serde_json::to_value(test_report(&[])).unwrap(),
            serde_json::json!({
                "schema_version": 1,
                "results": [],
                "by_operator": {},
                "by_file": {},
                "by_function": {}
            })
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
    fn generated_contract_client_arguments_substitute_only_compatible_params() {
        let mutants = mutations_for(
            r#"
            fn route(env: Env, dep: Address, actor: Address, alternate: Address, amount: i128, other: i128) {
                let client = RecorderClient::new(&env, &dep);
                client.record(&actor, &amount);
            }
            "#,
        );
        let calls: Vec<_> = mutants
            .iter()
            .filter(|m| m.operator == "CALL-002")
            .collect();
        assert_eq!(calls.len(), 3);
        assert!(calls
            .iter()
            .any(|m| m.original == "&actor" && m.replacement == "&dep"));
        assert!(calls
            .iter()
            .any(|m| m.original == "&actor" && m.replacement == "&alternate"));
        assert!(calls
            .iter()
            .any(|m| m.original == "&amount" && m.replacement == "&other"));
    }

    #[test]
    fn call_argument_substitution_ignores_unrecognized_clients_and_expressions() {
        let mutants = mutations_for(
            r#"
            fn route(env: Env, dep: Address, actor: Address, alternate: Address, amount: i128) {
                helper.record(&actor, &amount);
                let client = RecorderClient::new(&env, &dep);
                client.record(&actor.clone(), &(amount + 1));
            }
            "#,
        );
        assert!(!mutants.iter().any(|m| m.operator == "CALL-002"));
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
    fn generated_client_return_mismatch_check_can_be_bypassed() {
        let mutants = mutations_for(
            r#"
            fn verify(env: Env, dep: Address, expected: u32) -> bool {
                let client = OracleClient::new(&env, &dep);
                if client.value() != expected { panic!("mismatch"); }
                true
            }
            "#,
        );
        let calls: Vec<_> = mutants
            .iter()
            .filter(|m| m.operator == "CALL-003")
            .collect();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].original.contains("client.value() != expected"));
        assert_eq!(calls[0].replacement, "false");
    }

    #[test]
    fn call_return_check_ignores_ordinary_local_results() {
        let mutants = mutations_for(
            r#"
            fn verify(expected: u32) -> bool {
                if local_value() != expected { panic!("mismatch"); }
                true
            }
            "#,
        );
        assert!(!mutants.iter().any(|m| m.operator == "CALL-003"));
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
    fn storage_class_substitution_targets_only_direct_env_storage_accessors() {
        let mutants = mutations_for(
            r#"
            fn write(env: Env, key: u32, value: u32) {
                env.storage().persistent().set(&key, &value);
            }
            "#,
        );
        let storage: Vec<_> = mutants
            .iter()
            .filter(|m| m.operator == "STORAGE-001")
            .collect();
        assert_eq!(storage.len(), 2);
        assert!(storage
            .iter()
            .any(|m| m.replacement.ends_with(".instance()")));
        assert!(storage
            .iter()
            .any(|m| m.replacement.ends_with(".temporary()")));
    }

    #[test]
    fn storage_class_substitution_ignores_unrelated_storage_chains() {
        let mutants = mutations_for(
            r#"
            fn write(env: Env, cache: Cache, key: u32, value: u32) {
                cache.storage().persistent().set(&key, &value);
            }
            "#,
        );
        assert!(!mutants.iter().any(|m| m.operator == "STORAGE-001"));
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
    fn typed_local_storage_accessor_gets_ttl_mutants_without_following_aliases() {
        let mutants = mutations_for(
            r#"
            fn keep_alive(env: Env, key: DataKey) {
                let persistent: soroban_sdk::storage::Persistent = env.storage().persistent();
                persistent.extend_ttl(&key, 100, 1000);
                let alias = persistent;
                alias.extend_ttl(&key, 100, 1000);
            }
            "#,
        );

        assert_eq!(
            mutants.iter().filter(|m| m.operator == "TTL-001").count(),
            1
        );
        assert_eq!(
            mutants.iter().filter(|m| m.operator == "TTL-002").count(),
            1
        );
        assert_eq!(
            mutants.iter().filter(|m| m.operator == "TTL-003").count(),
            1
        );
        assert!(mutants
            .iter()
            .all(|m| !m.original.contains("alias.extend_ttl")));
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
    fn typed_event_actor_substitution_uses_only_explicit_address_parameters() {
        let mutants = mutations_for(
            r#"
            #[contractevent]
            struct TransferNotice { actor: Address, amount: i128 }
            fn record(env: Env, actor: Address, alternate: Address, amount: i128) {
                TransferNotice { actor, amount }.publish(&env);
            }
            "#,
        );
        let actor = mutants
            .iter()
            .find(|m| m.operator == "EVENT-003")
            .expect("EVENT-003 mutant");
        assert_eq!(actor.original, "actor");
        assert_eq!(actor.replacement, "actor: alternate");
        assert!(actor.description.contains("alternate instead of actor"));
    }

    #[test]
    fn event_actor_substitution_ignores_untyped_and_non_event_structs() {
        let mutants = mutations_for(
            r#"
            struct FakeEvent { actor: Address }
            #[contractevent]
            struct RealEvent { actor: Address }
            fn record(env: Env, actor: Address, alternate: String) {
                FakeEvent { actor }.publish(&env);
                RealEvent { actor }.publish(&env);
            }
            "#,
        );
        assert!(!mutants.iter().any(|m| m.operator == "EVENT-003"));
    }

    #[test]
    fn typed_event_numeric_value_substitution_targets_primitive_numeric_params() {
        let mutants = mutations_for(
            r#"
            #[contractevent]
            struct TransferNotice { recipient: Address, amount: i128 }
            fn record(env: Env, recipient: Address, amount: i128) {
                TransferNotice { recipient, amount }.publish(&env);
            }
            "#,
        );
        let value = mutants
            .iter()
            .find(|m| m.operator == "EVENT-004")
            .expect("EVENT-004 mutant");
        assert_eq!(value.original, "amount");
        assert_eq!(value.replacement, "amount: 0");
    }

    #[test]
    fn event_value_substitution_ignores_non_numeric_and_non_event_fields() {
        let mutants = mutations_for(
            r#"
            struct FakeEvent { amount: i128 }
            #[contractevent]
            struct RealEvent { label: String }
            fn record(env: Env, amount: i128, label: String) {
                FakeEvent { amount }.publish(&env);
                RealEvent { label }.publish(&env);
            }
            "#,
        );
        assert!(!mutants.iter().any(|m| m.operator == "EVENT-004"));
    }

    #[test]
    fn typed_event_topic_substitution_requires_topic_attribute_and_address_params() {
        let mutants = mutations_for(
            r#"
            #[contractevent]
            struct TransferNotice { #[topic] actor: Address, recipient: Address }
            fn record(env: Env, actor: Address, alternate: Address, recipient: Address) {
                TransferNotice { actor, recipient }.publish(&env);
            }
            "#,
        );
        let topics: Vec<_> = mutants
            .iter()
            .filter(|m| m.operator == "EVENT-005")
            .collect();
        assert_eq!(topics.len(), 2);
        assert!(topics.iter().all(|m| m.original == "actor"));
        assert!(topics.iter().any(|m| m.replacement == "actor: alternate"));
        assert!(topics.iter().any(|m| m.replacement == "actor: recipient"));
    }

    #[test]
    fn event_topic_substitution_ignores_non_topic_fields_and_arbitrary_structs() {
        let mutants = mutations_for(
            r#"
            struct FakeEvent { actor: Address }
            #[contractevent]
            struct RealEvent { actor: Address }
            fn record(env: Env, actor: Address, alternate: Address) {
                FakeEvent { actor }.publish(&env);
                RealEvent { actor }.publish(&env);
            }
            "#,
        );
        assert!(!mutants.iter().any(|m| m.operator == "EVENT-005"));
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

    fn auth_005(source: &str) -> Vec<(String, String, String)> {
        mutations_for(source)
            .into_iter()
            .filter(|m| m.operator == "AUTH-005")
            .map(|m| (m.original, m.replacement, m.description))
            .collect()
    }

    #[test]
    fn stored_admin_authority_is_replaced_with_each_address_parameter() {
        let mutants = auth_005(
            r#"
            fn set_admin(env: Env, new_admin: Address, observer: &Address) {
                let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
                admin.require_auth();
                env.storage().instance().set(&DataKey::Admin, &new_admin);
            }
            "#,
        );
        let lookup = "env.storage().instance().get(&DataKey::Admin).unwrap()".to_owned();
        assert_eq!(
            mutants,
            vec![
                (
                    lookup.clone(),
                    "new_admin.clone()".to_owned(),
                    "use new_admin instead of stored authority admin".to_owned(),
                ),
                (
                    lookup,
                    "observer.clone()".to_owned(),
                    "use observer instead of stored authority admin".to_owned(),
                ),
            ]
        );
    }

    #[test]
    fn stored_authority_supports_turbofish_expect_and_auth_for_args() {
        let mutants = auth_005(
            r#"
            fn rotate(env: &Env, candidate: Address) {
                let owner = env
                    .storage()
                    .persistent()
                    .get::<_, Address>(&DataKey::Owner)
                    .expect("owner must be initialized");
                owner.require_auth_for_args((candidate.clone(),).into_val(env));
            }
            "#,
        );
        assert_eq!(mutants.len(), 1, "{mutants:#?}");
        assert_eq!(mutants[0].1, "candidate.clone()");
        assert!(mutants[0].0.starts_with("env\n"));
        assert!(mutants[0]
            .0
            .ends_with(".expect(\"owner must be initialized\")"));
    }

    #[test]
    fn generic_state_lookups_are_not_treated_as_privileged_authorities() {
        let mutants = auth_005(
            r#"
            fn not_authorities(env: Env, user: Address, amount: i128) {
                // A typed Address loaded from storage but never used as an auth sink.
                let recipient: Address = env.storage().instance().get(&DataKey::Recipient).unwrap();
                token.transfer(&user, &recipient, &amount);

                // Non-Address state, even when it is unwrapped.
                let balance: i128 = env.storage().persistent().get(&user).unwrap();
                let fee: u32 = env.storage().instance().get(&DataKey::Fee).unwrap_or(0);

                // A name that merely sounds privileged is not enough on its own.
                let admin_count: u32 = env.storage().instance().get(&DataKey::Admins).unwrap();
            }

            fn untyped(env: Env, user: Address) {
                let admin = env.storage().instance().get(&DataKey::Admin).unwrap();
                admin.require_auth();
            }

            fn fallback_or_option(env: Env, user: Address) {
                let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap_or(user.clone());
                admin.require_auth();
                let owner: Option<Address> = env.storage().instance().get(&DataKey::Owner);
                owner.unwrap().require_auth();
            }

            fn not_env_storage(env: Env, store: Store, user: Address) {
                let admin: Address = store.storage().instance().get(&DataKey::Admin).unwrap();
                admin.require_auth();
                let owner: Address = load_owner(&env);
                owner.require_auth();
                let storage = env.storage().instance();
                let manager: Address = storage.get(&DataKey::Manager).unwrap();
                manager.require_auth();
            }

            fn shadowed(env: Env, user: Address) {
                let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
                let admin = user.clone();
                admin.require_auth();
            }

            fn no_alternate(env: Env) {
                let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
                admin.require_auth();
            }

            fn no_env(user: Address, storage: Storage) {
                let admin: Address = storage.instance().get(&DataKey::Admin).unwrap();
                admin.require_auth();
            }
            "#,
        );
        assert!(mutants.is_empty(), "{mutants:#?}");
    }

    #[test]
    fn stored_authority_scan_does_not_cross_nested_functions() {
        let mutants = auth_005(
            r#"
            fn outer(env: Env, user: Address) {
                let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
                fn inner(admin: Address) {
                    admin.require_auth();
                }
                inner(admin);
            }
            "#,
        );
        assert!(mutants.is_empty(), "{mutants:#?}");
    }

    fn auth_004(source: &str) -> Vec<(Option<String>, String, String, String)> {
        mutations_for(source)
            .into_iter()
            .filter(|m| m.operator == "AUTH-004")
            .map(|m| (m.function, m.original, m.replacement, m.description))
            .collect()
    }

    #[test]
    fn helper_authorization_is_mutated_at_each_call_site() {
        let mutants = auth_004(
            r#"
            fn require_owner(owner: &Address) {
                owner.require_auth();
            }

            fn transfer(env: Env, from: Address, to: Address, amount: i128) {
                require_owner(&from);
                move_balance(&env, &from, &to, amount);
            }
            "#,
        );
        assert_eq!(
            mutants,
            vec![
                (
                    Some("transfer".to_owned()),
                    "require_owner(&from)".to_owned(),
                    "()".to_owned(),
                    "skip require_owner() authorization of from (sink: owner.require_auth())"
                        .to_owned(),
                ),
                (
                    Some("transfer".to_owned()),
                    "&from".to_owned(),
                    "&to".to_owned(),
                    "authenticate to instead of from via require_owner() (sink: owner.require_auth())"
                        .to_owned(),
                ),
            ]
        );
    }

    #[test]
    fn helper_authorization_supports_by_value_and_multi_sink_helpers() {
        let mutants = auth_004(
            r#"
            fn require_both(env: &Env, first: Address, second: &Address) {
                first.require_auth();
                second.require_auth();
                first.require_auth();
            }

            impl Contract {
                pub fn swap(env: Env, buyer: Address, seller: Address, arbiter: Address) {
                    require_both(&env, buyer.clone(), &seller);
                }
            }
            "#,
        );
        let edits = mutants
            .iter()
            .map(|(_, original, replacement, _)| (original.as_str(), replacement.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            edits,
            vec![
                ("require_both(&env, buyer.clone(), &seller)", "()"),
                ("buyer.clone()", "seller.clone()"),
                ("buyer.clone()", "arbiter.clone()"),
                ("&seller", "&buyer"),
                ("&seller", "&arbiter"),
            ]
        );
        assert!(mutants[0]
            .3
            .contains("of buyer, seller (sink: first.require_auth(), second.require_auth())"));
    }

    #[test]
    fn helper_authorization_rejects_unsupported_helper_shapes() {
        let mutants = auth_004(
            r#"
            fn has_side_effects(owner: &Address, env: &Env) {
                owner.require_auth();
                env.storage().instance().set(&DataKey::Seen, &true);
            }
            fn authorizes_stored_admin(env: &Env) {
                let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
                admin.require_auth();
            }
            fn untyped_param(owner: &Account) {
                owner.require_auth();
            }
            fn conditional(owner: &Address, enabled: bool) {
                if enabled {
                    owner.require_auth();
                }
            }
            fn returns_value(owner: &Address) -> bool {
                owner.require_auth();
                true
            }
            fn generic<A: Auth>(owner: &A) {
                owner.require_auth();
            }
            fn with_args(owner: &Address, env: &Env) {
                owner.require_auth_for_args(soroban_sdk::Vec::new(env));
            }
            fn empty(owner: &Address) {}

            fn caller(env: Env, from: Address, to: Address) {
                has_side_effects(&from, &env);
                authorizes_stored_admin(&env);
                untyped_param(&from);
                conditional(&from, true);
                let _ = returns_value(&from);
                generic(&from);
                with_args(&from, &env);
                empty(&from);
            }
            "#,
        );
        assert!(mutants.is_empty(), "{mutants:#?}");
    }

    #[test]
    fn helper_authorization_rejects_unsupported_call_sites() {
        let mutants = auth_004(
            r#"
            fn require_owner(owner: &Address) {
                owner.require_auth();
            }

            fn no_soroban_context(count: u32) {
                require_owner(&OWNER);
            }

            fn shadowed(env: Env, from: Address) {
                let require_owner = |owner: &Address| {};
                require_owner(&from);
            }

            fn qualified_or_method(env: Env, from: Address, auth: Auth) {
                Self::require_owner(&from);
                crate::require_owner(&from);
                auth.require_owner(&from);
                other::require_owner(&from);
                (require_owner)(&from);
                require_owner(&from, &from);
            }

            fn embedded(env: Env, from: Address) -> () {
                require_owner(&from)
            }

            mod nested {
                fn caller(env: Env, from: Address, to: Address) {
                    require_owner(&from);
                }
            }
            "#,
        );
        assert!(mutants.is_empty(), "{mutants:#?}");
    }

    #[test]
    fn helper_authorization_removes_but_does_not_redirect_untyped_arguments() {
        let mutants = auth_004(
            r#"
            fn require_owner(owner: &Address) {
                owner.require_auth();
            }

            fn caller(env: Env, from: Address, to: Address) {
                let stored = load_owner(&env);
                require_owner(&stored);
                require_owner(&from);
            }
            "#,
        );
        let edits = mutants
            .iter()
            .map(|(_, original, replacement, _)| (original.as_str(), replacement.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            edits,
            vec![
                ("require_owner(&stored)", "()"),
                ("require_owner(&from)", "()"),
                ("&from", "&to"),
            ]
        );
        assert!(mutants[0].3.contains("authorization of an argument"));
    }

    #[test]
    fn helper_sink_still_gets_direct_auth_removal() {
        let mutants = mutations_for(
            r#"
            fn require_owner(owner: &Address) {
                owner.require_auth();
            }
            "#,
        );
        assert!(mutants
            .iter()
            .any(|m| m.operator == "AUTH-001" && m.function.as_deref() == Some("require_owner")));
        assert!(mutants.iter().all(|m| m.operator != "AUTH-004"));
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
    fn token_client_substitution_uses_other_recognized_clients_in_order() {
        let mutants = mutations_for(
            r#"
            fn pay(env: Env, a: Address, b: Address, from: Address, to: Address, amount: i128) {
                let primary = token::Client::new(&env, &a);
                let secondary = token::Client::new(&env, &b);
                primary.transfer(&from, &to, &amount);
            }
            "#,
        );
        let clients: Vec<_> = mutants
            .iter()
            .filter(|m| m.operator == "TOKEN-004")
            .collect();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].original, "primary");
        assert_eq!(clients[0].replacement, "secondary");
    }

    #[test]
    fn token_client_substitution_ignores_constructor_and_unrecognized_receivers() {
        let mutants = mutations_for(
            r#"
            fn pay(env: Env, a: Address, from: Address, to: Address, amount: i128) {
                token::Client::new(&env, &a).transfer(&from, &to, &amount);
                helper.transfer(&from, &to, &amount);
            }
            "#,
        );
        assert!(!mutants.iter().any(|m| m.operator == "TOKEN-004"));
    }

    #[test]
    fn allowance_role_swaps_cover_supported_token_calls() {
        let mutants = mutations_for(
            r#"
            fn allowances(env: Env, token: Address, owner: Address, spender: Address, to: Address, amount: i128, expiration_ledger: u32) -> i128 {
                let client = token::Client::new(&env, &token);
                client.approve(&owner, &spender, &amount, &expiration_ledger);
                client.transfer_from(&spender, &owner, &to, &amount);
                client.burn_from(&spender, &owner, &amount);
                client.allowance(&owner, &spender)
            }
            "#,
        );
        let swaps = mutants
            .iter()
            .filter(|m| m.operator == "TOKEN-002")
            .map(|m| {
                (
                    m.original.as_str(),
                    m.replacement.as_str(),
                    m.description.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            swaps,
            vec![
                (
                    "&owner, &spender",
                    "&spender, &owner",
                    "swap token approve() owner/spender arguments (owner <-> spender)",
                ),
                (
                    "&spender, &owner",
                    "&owner, &spender",
                    "swap token transfer_from() spender/owner arguments (spender <-> owner)",
                ),
                (
                    "&spender, &owner",
                    "&owner, &spender",
                    "swap token burn_from() spender/owner arguments (spender <-> owner)",
                ),
                (
                    "&owner, &spender",
                    "&spender, &owner",
                    "swap token allowance() owner/spender arguments (owner <-> spender)",
                ),
            ]
        );
    }

    #[test]
    fn allowance_role_swap_preserves_separator_and_other_arguments() {
        let source = r#"
            fn approve(env: Env, token: TokenClient, from: Address, spender: Address, amount: i128) {
                token.approve(&from,   /* role */ &spender, &amount, &200);
            }
            "#;
        let mutants = mutations_for(source);
        let swap = mutants
            .iter()
            .find(|m| m.operator == "TOKEN-002")
            .expect("typed token-client parameter should be recognized");
        assert_eq!(swap.original, "&from,   /* role */ &spender");
        assert_eq!(swap.replacement, "&spender,   /* role */ &from");
        assert_eq!(&source[swap.start_byte..swap.end_byte], swap.original);
    }

    #[test]
    fn allowance_role_swap_ignores_unrelated_receivers_shapes_and_values() {
        let mutants = mutations_for(
            r#"
            fn not_token(env: Env, registry: Registry, owner: Address, spender: Address, amount: i128) {
                registry.approve(&owner, &spender, &amount, &100);
                TokenUtils::new(&env).events().approve(&owner, &spender, &amount, &100);
                env.events().publish((symbol_short!("approve"),), (&owner, &spender));
            }

            fn unsupported(env: Env, token: Address, owner: Address, spender: Address, amount: i128) {
                let client = token::Client::new(&env, &token);
                let alias = owner.clone();
                client.approve(&alias, &spender, &amount, &100);
                client.approve(&owner, &owner, &amount, &100);
                client.approve(&owner.clone(), &spender, &amount, &100);
                client.approve(&owner, &spender, &amount);
                client.transfer_from(&spender, &owner, &amount);
                client.allowance(&owner);
                client.transfer(&owner, &spender, &amount);
            }
            "#,
        );
        assert!(
            mutants.iter().all(|m| m.operator != "TOKEN-002"),
            "{mutants:#?}"
        );
    }

    #[test]
    fn token_amount_mutants_use_zero_and_one_for_typed_numeric_reference() {
        let mutants = mutations_for(
            r#"
            fn pay(token: TokenClient, from: Address, to: Address, amount: i128) {
                token.transfer(&from, &to, &amount);
            }
            "#,
        );
        let amounts: Vec<_> = mutants
            .iter()
            .filter(|m| m.operator == "TOKEN-003")
            .collect();
        assert_eq!(amounts.len(), 2);
        assert_eq!(amounts[0].original, "&amount");
        assert!(amounts.iter().any(|m| m.replacement == "&0"));
        assert!(amounts.iter().any(|m| m.replacement == "&1"));
    }

    #[test]
    fn token_amount_mutation_rejects_untyped_or_computed_amounts() {
        let mutants = mutations_for(
            r#"
            fn pay(token: TokenClient, from: Address, to: Address, amount: Amount, count: i128) {
                token.transfer(&from, &to, &amount);
                token.transfer(&from, &to, &(count + 1));
            }
            "#,
        );
        assert!(!mutants.iter().any(|m| m.operator == "TOKEN-003"));
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
            &root.join(".soro-mutants-worktree"),
            &root.join(".soro-mutants-worktree"),
            &target,
            &[],
            false,
        )
        .expect("mutant execution should succeed");

        assert!(matches!(outcome.outcome, Outcome::Survived));
        assert!(!root.join(".soro-mutants-worktree").exists());

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn parallel_workers_never_share_scratch_or_cargo_state_with_each_other_or_baseline() {
        for (root, target_dir) in [
            (Path::new("/tmp/example-project"), None),
            (Path::new("/tmp/example-project"), Some(Path::new("custom"))),
            (
                Path::new("/tmp/example-project"),
                Some(Path::new("/tmp/shared-cache")),
            ),
        ] {
            let (baseline, serial) = target_dirs(root, target_dir);
            let parallel = parallel_target_root(root, target_dir);
            assert_eq!(parallel.parent(), baseline.parent());
            assert!(!parallel.starts_with(&baseline) && !baseline.starts_with(&parallel));
            assert!(!parallel.starts_with(&serial) && !serial.starts_with(&parallel));

            let workers = (0..4)
                .map(|worker| worker_paths(root, target_dir, worker))
                .collect::<Vec<_>>();
            for (index, (scratch, target)) in workers.iter().enumerate() {
                assert!(scratch.starts_with(root.join(".soro-mutants-worktree")));
                assert!(target.starts_with(&parallel));
                assert!(!target.starts_with(&baseline));
                for (other_scratch, other_target) in &workers[index + 1..] {
                    assert!(!scratch.starts_with(other_scratch));
                    assert!(!other_scratch.starts_with(scratch));
                    assert!(!target.starts_with(other_target));
                    assert!(!other_target.starts_with(target));
                }
            }
        }
    }

    #[test]
    fn clean_also_removes_parallel_worker_caches() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = env::temp_dir().join(format!("soro-mutants-clean-parallel-{nonce}"));
        let (_, worker_target) = worker_paths(&root, None, 0);
        fs::create_dir_all(&worker_target).expect("worker cache should be created");
        fs::create_dir_all(root.join(".soro-mutants-target/baseline"))
            .expect("baseline directory should be created");

        let removed = clean_generated_state(&root, None, false).expect("cleanup should succeed");

        assert!(removed.contains(&parallel_target_root(&root, None)));
        assert!(!root.join(".soro-mutants-target").exists());
        let _ = fs::remove_dir_all(&root);
    }

    /// Three AUTH-001 mutants whose expected outcomes differ. The test suite
    /// tolerates exactly one missing guard, so if any mutant leaked into another
    /// mutant's source tree (two guards missing at once) `b` or `c` would be
    /// KILLED instead of SURVIVED.
    fn write_leak_detection_fixture(root: &Path) {
        fs::create_dir_all(root.join("src")).expect("fixture directory should be created");
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "parallel-leak-fixture"
version = "0.1.0"
edition = "2021"

[workspace]
"#,
        )
        .expect("fixture manifest should be written");
        fs::write(
            root.join("src/lib.rs"),
            r#"
use std::cell::Cell;

thread_local! {
    static AUTHS: Cell<u32> = const { Cell::new(0) };
}

pub struct Address;

impl Address {
    pub fn require_auth(&self) {
        AUTHS.with(|count| count.set(count.get() + 1));
    }
}

pub fn auths() -> u32 {
    AUTHS.with(Cell::get)
}

pub fn a(user: Address) {
    user.require_auth();
}

pub fn b(user: Address) {
    user.require_auth();
}

pub fn c(user: Address) {
    user.require_auth();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_is_guarded() {
        a(Address);
        assert_eq!(auths(), 1);
    }

    #[test]
    fn at_most_one_guard_is_missing() {
        a(Address);
        b(Address);
        c(Address);
        assert!(auths() >= 2);
    }
}
"#,
        )
        .expect("fixture source should be written");
    }

    #[test]
    fn parallel_execution_isolates_mutants_and_reports_in_input_order() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = env::temp_dir().join(format!("soro-mutants-parallel-{nonce}"));
        let cache = env::temp_dir().join(format!("soro-mutants-parallel-cache-{nonce}"));
        write_leak_detection_fixture(&root);
        let original_source = fs::read_to_string(root.join("src/lib.rs")).unwrap();
        let (baseline, _) = target_dirs(&root, Some(&cache));
        fs::create_dir_all(&baseline).unwrap();
        fs::write(baseline.join("sentinel"), "baseline").unwrap();

        let mutants = discover_mutants(&root, Some("AUTH-001"), None, None, &GlobSet::empty())
            .expect("fixture mutation discovery should work");
        let functions = mutants
            .iter()
            .map(|m| m.function.clone().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(functions, ["a", "b", "c"]);

        for jobs in [1, 3, 8] {
            let mut reported = Vec::new();
            let outcomes = execute_mutants(
                &root,
                &mutants,
                &ExecutionSettings {
                    test_dir: None,
                    test_command: "cargo test -q",
                    timeouts: Timeouts::resolve(120, None, None),
                    target_dir: Some(&cache),
                    scratch_root: &root.join(pathguard::SCRATCH_NAME),
                    jobs,
                    timing: false,
                },
                &mut |mutant, outcome| {
                    reported.push((mutant.id.clone(), outcome_label(outcome)));
                    Ok(())
                },
            )
            .expect("mutant execution should succeed");

            let labels = outcomes
                .iter()
                .map(|execution| outcome_label(&execution.outcome))
                .collect::<Vec<_>>();
            assert_eq!(labels, ["KILLED", "SURVIVED", "SURVIVED"], "jobs={jobs}");
            assert_eq!(
                reported,
                [
                    ("M0001".to_owned(), "KILLED"),
                    ("M0002".to_owned(), "SURVIVED"),
                    ("M0003".to_owned(), "SURVIVED"),
                ],
                "jobs={jobs}: results must be reported in mutant order"
            );
            assert!(!root.join(".soro-mutants-worktree").exists(), "jobs={jobs}");
            assert_eq!(
                fs::read_to_string(root.join("src/lib.rs")).unwrap(),
                original_source,
                "jobs={jobs}: the target repository must never be mutated"
            );
        }

        // Mutant builds never touched the baseline cache.
        let baseline_entries = fs::read_dir(&baseline)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(baseline_entries, ["sentinel"]);
        for worker in 0..3 {
            assert!(worker_paths(&root, Some(&cache), worker).1.exists());
        }

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&cache);
    }

    #[test]
    fn jobs_defaults_to_serial_and_rejects_non_positive_values() {
        let jobs = |args: &[&str]| {
            Cli::try_parse_from(["cargo-soro-mutants", "test", "."].iter().chain(args)).map(|cli| {
                match cli.command {
                    Commands::Test { jobs, .. } => jobs,
                    _ => unreachable!("test subcommand"),
                }
            })
        };
        assert_eq!(jobs(&[]).unwrap(), 1);
        assert_eq!(jobs(&["--jobs", "4"]).unwrap(), 4);
        assert_eq!(jobs(&["-j", "2"]).unwrap(), 2);
        for invalid in ["0", "-1", "two"] {
            assert!(jobs(&["--jobs", invalid]).is_err(), "--jobs {invalid}");
        }
    }

    #[test]
    fn timing_and_scratch_flags_are_opt_in_and_parse_together() {
        let parsed = Cli::try_parse_from([
            "cargo-soro-mutants",
            "test",
            ".",
            "--timing",
            "--scratch-dir",
            "/tmp/soro-mutants-scratch",
        ])
        .unwrap();
        match parsed.command {
            Commands::Test {
                timing,
                scratch_dir,
                ..
            } => {
                assert!(timing);
                assert_eq!(
                    scratch_dir.as_deref(),
                    Some(Path::new("/tmp/soro-mutants-scratch"))
                );
            }
            _ => unreachable!("test subcommand"),
        }

        let defaults = Cli::try_parse_from(["cargo-soro-mutants", "test", "."]).unwrap();
        match defaults.command {
            Commands::Test {
                timing,
                scratch_dir,
                ..
            } => {
                assert!(!timing);
                assert!(scratch_dir.is_none());
            }
            _ => unreachable!("test subcommand"),
        }
    }

    #[test]
    fn optional_timing_fields_serialize_only_when_enabled() {
        let mutant = report_mutant();
        let without = serde_json::to_value(MutantResult {
            mutant: mutant.clone(),
            outcome: Outcome::Survived,
            timing: None,
        })
        .unwrap();
        assert!(without.get("timing").is_none());

        let with = serde_json::to_value(MutantResult {
            mutant,
            outcome: Outcome::Killed,
            timing: Some(MutantTiming {
                total_ms: 9,
                compile_ms: Some(4),
                test_ms: Some(3),
            }),
        })
        .unwrap();
        assert_eq!(with["timing"]["total_ms"], 9);
        assert_eq!(with["timing"]["compile_ms"], 4);
        assert_eq!(with["timing"]["test_ms"], 3);
    }

    #[test]
    fn duration_parser_accepts_human_suffixes_and_rejects_zero() {
        assert_eq!(parse_duration("2s"), Ok(2));
        assert_eq!(parse_duration("3m"), Ok(180));
        assert_eq!(parse_duration("1h"), Ok(3_600));
        assert_eq!(parse_duration("1d"), Ok(86_400));
        assert_eq!(parse_duration("7"), Ok(7));
        assert!(parse_duration("0s").is_err());
        assert!(parse_duration("1w").is_err());
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

#[cfg(test)]
mod discovery_quality_regressions {
    use super::*;

    fn in_memory(source: &str) -> Vec<Mutant> {
        let mut file = syn::parse_file(source).expect("fixture must parse");
        sdk_reexports::normalize(&mut file);
        let mut visitor = SemanticVisitor::new(source, PathBuf::from("src/lib.rs"));
        visitor.visit_file(&file);
        let (mut mutants, _) = suppress_equivalent_mutants(visitor.mutants);
        mutants.sort_by(mutant_order);
        for (index, mutant) in mutants.iter_mut().enumerate() {
            mutant.id = format!("M{:04}", index + 1);
        }
        mutants
    }

    #[test]
    fn overlap_suppresses_containment_and_partial_edits_deterministically() {
        let source = "fn guard(a: Address, b: Address) { a.require_auth(); }";
        let base = in_memory(source).into_iter().next().unwrap();
        let mut nested = base.clone();
        nested.start_byte += 2;
        nested.end_byte -= 2;
        nested.operator = base.operator.clone();
        let mut partial = base.clone();
        partial.start_byte = base.end_byte - 3;
        partial.end_byte = base.end_byte + 2;
        partial.operator = base.operator.clone();
        let other_file = Mutant {
            file: PathBuf::from("src/other.rs"),
            ..nested.clone()
        };

        let expected = suppress_equivalent_mutants(vec![
            partial.clone(),
            other_file.clone(),
            nested.clone(),
            base.clone(),
        ]);
        for _ in 0..8 {
            let (mut retained, suppressed) = suppress_equivalent_mutants(vec![
                nested.clone(),
                base.clone(),
                partial.clone(),
                other_file.clone(),
            ]);
            retained.sort_by(mutant_order);
            let (mut canonical, reasons) = expected.clone();
            canonical.sort_by(mutant_order);
            assert_eq!(
                retained
                    .iter()
                    .map(|m| (&m.file, m.start_byte, m.end_byte, &m.operator))
                    .collect::<Vec<_>>(),
                canonical
                    .iter()
                    .map(|m| (&m.file, m.start_byte, m.end_byte, &m.operator))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                suppressed.iter().map(|s| s.reason).collect::<Vec<_>>(),
                reasons.iter().map(|s| s.reason).collect::<Vec<_>>()
            );
            assert!(suppressed
                .iter()
                .any(|s| s.reason == "overlapping source span"));
        }
    }

    #[test]
    fn exact_span_alternatives_are_preserved_and_duplicates_are_suppressed() {
        // Each alternative is executed independently, so identical spans with
        // distinct replacements are not the ambiguous *partial* overlap case.
        let source = "fn guard(a: Address, b: Address) { a.require_auth(); }";
        let original = in_memory(source).into_iter().next().unwrap();
        let mut alternate = original.clone();
        alternate.operator = "AUTH-002".to_string();
        alternate.replacement = "b.require_auth()".to_string();
        let (retained, suppressed) =
            suppress_equivalent_mutants(vec![alternate, original.clone(), original]);
        assert_eq!(retained.len(), 2);
        assert_eq!(suppressed.len(), 1);
        assert_eq!(suppressed[0].reason, "duplicate source edit");
    }

    #[test]
    fn distinct_operators_keep_independent_nested_edits() {
        let source = "fn guard(a: Address) { a.require_auth(); }";
        let outer = in_memory(source).into_iter().next().unwrap();
        let mut inner = outer.clone();
        inner.operator = "TTL-002".to_owned();
        inner.start_byte += 2;
        inner.end_byte -= 2;
        inner.replacement = "0".to_owned();
        let (retained, suppressed) = suppress_equivalent_mutants(vec![inner, outer]);
        assert_eq!(retained.len(), 2);
        assert!(suppressed.is_empty());
    }

    #[test]
    fn real_nested_recognizers_have_stable_ids_after_overlap_resolution() {
        let source = "fn initialize(env: Env, admin: Address, replacement: Address) { admin.require_auth(); }";
        let first = in_memory(source);
        let second = in_memory(source);
        assert_eq!(
            first
                .iter()
                .map(|m| (&m.id, &m.operator, &m.replacement))
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|m| (&m.id, &m.operator, &m.replacement))
                .collect::<Vec<_>>()
        );
        assert!(!first.is_empty());
        assert_eq!(first[0].id, "M0001");
    }

    #[test]
    fn unicode_offsets_and_spans_preserve_rust_source_and_parse() {
        let cases = [
            "use soroban_sdk::Address;\n// 🦀 before\nfn guarded(user: Address) { let café = \"日本語\"; user.require_auth(); // ✅ after\n}\n",
            "use soroban_sdk::Address;\nfn guarded(user: Address) { let δεδομένα = \"🔥\"; /* 🍀 */ user.require_auth(); }\n",
            "use soroban_sdk::Address;\nfn guarded(user: Address) { user.require_auth(); let résumé = \"🧪\"; }\n",
        ];
        for source in cases {
            let mutants = in_memory(source);
            assert!(!mutants.is_empty(), "must discover auth in Unicode source");
            for mutant in &mutants {
                let edited = replace_source_range(
                    source,
                    mutant.start_byte,
                    mutant.end_byte,
                    &mutant.original,
                    &mutant.replacement,
                )
                .expect("byte-safe mutation");
                assert!(
                    syn::parse_file(&edited).is_ok(),
                    "mutated fixture must parse"
                );
                assert_eq!(
                    &edited.as_bytes()[..mutant.start_byte],
                    &source.as_bytes()[..mutant.start_byte]
                );
                assert_eq!(
                    &edited.as_bytes()[mutant.start_byte + mutant.replacement.len()..],
                    &source.as_bytes()[mutant.end_byte..]
                );
                let current = source.lines().nth(mutant.span.line - 1).unwrap();
                let byte_column = current.find(&mutant.original).unwrap() + 1;
                assert_eq!(mutant.span.column, byte_column);
            }
        }
    }

    #[test]
    fn utf8_column_conversion_rejects_mid_character_and_bad_ranges() {
        assert_eq!(line_column_to_offset("λx\n", 1, 1), Some(2));
        assert_eq!(line_column_to_offset("λx\n", 1, 2), Some(3));
        assert_eq!(line_column_to_offset("🦀\n", 1, 1), Some(4));
        assert_eq!(line_column_to_offset("🦀\n", 1, 3), None);
        assert!(replace_source_range("🐛require_auth()", 1, 10, "", "()").is_err());
    }

    #[test]
    fn sdk_local_reexports_are_conservative_and_operator_counts_repeatable() {
        let renamed = "pub use soroban_sdk::{Address as Person, Env as Host};\n\
             fn guard(_env: Host, user: Person) { user.require_auth(); }\n";
        let direct = "pub use soroban_sdk::Address;\n\
             fn guard(user: Address) { user.require_auth(); }\n";
        let qualified = "mod sdk_types { pub use soroban_sdk::Address as Person; }\n\
             fn guard(user: crate::sdk_types::Person) { user.require_auth(); }\n";
        let unrelated = "pub use other_sdk::Address as Person;\n\
             fn guard(user: Person) { user.require_auth(); }\n";
        for source in [renamed, direct, qualified] {
            let first = in_memory(source);
            let second = in_memory(source);
            assert!(first.iter().any(|m| m.operator == "AUTH-001"));
            assert_eq!(first.len(), second.len());
            assert_eq!(
                first
                    .iter()
                    .map(|m| (&m.id, &m.operator))
                    .collect::<Vec<_>>(),
                second
                    .iter()
                    .map(|m| (&m.id, &m.operator))
                    .collect::<Vec<_>>()
            );
        }
        assert!(in_memory(unrelated).is_empty());
    }
}
