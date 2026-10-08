use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use proc_macro2::Span;
use serde::Serialize;
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
    Expr, ExprMethodCall, FnArg, ImplItemFn, ItemFn, Local, Pat, Signature, Type,
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

#[derive(Subcommand, Debug)]
enum Commands {
    /// Discover semantic mutants without executing them.
    List {
        /// Cargo project or workspace to inspect.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Only list mutants produced by this operator ID, such as AUTH-001.
        #[arg(long)]
        operator: Option<String>,
        /// Only list mutants in source files ending with this path.
        #[arg(long)]
        file: Option<String>,
        /// Only list mutants inside this function.
        #[arg(long)]
        function: Option<String>,
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
        #[arg(long)]
        operator: Option<String>,
        /// Only execute mutants in source files ending with this path.
        #[arg(long)]
        file: Option<String>,
        /// Only execute mutants inside this function.
        #[arg(long)]
        function: Option<String>,
        /// Run the configured test command from this directory relative to PATH.
        #[arg(long)]
        test_dir: Option<PathBuf>,
        /// Store Soro Mutants baseline and mutant Cargo caches under this directory.
        #[arg(long)]
        target_dir: Option<PathBuf>,
        /// Command used for the clean baseline and each viable mutant.
        #[arg(long, default_value = "cargo test -q")]
        test_command: String,
        /// Per-command timeout in seconds.
        #[arg(long, default_value_t = 120)]
        timeout: u64,
        /// Emit machine-readable JSON on stdout.
        #[arg(long)]
        json: bool,
        /// Write the selected text or JSON report to a file instead of stdout.
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
}

#[derive(Serialize)]
struct TestReport<'a> {
    schema_version: u32,
    results: &'a [MutantResult],
}

fn list_report(mutants: &[Mutant]) -> ListReport<'_> {
    ListReport {
        schema_version: JSON_SCHEMA_VERSION,
        mutants,
    }
}

fn test_report(results: &[MutantResult]) -> TestReport<'_> {
    TestReport {
        schema_version: JSON_SCHEMA_VERSION,
        results,
    }
}

#[derive(Clone, Copy, Debug)]
enum ProcessResult {
    Passed,
    Failed,
    Timeout,
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

fn main() -> Result<()> {
    let cli = Cli::parse_from(normalized_args());

    match cli.command {
        Commands::List {
            path,
            operator,
            file,
            function,
            json,
            count_only,
            output,
        } => {
            let mutants = discover_mutants(
                &path,
                operator.as_deref(),
                file.as_deref(),
                function.as_deref(),
            )?;
            let mut report = report_writer(output.as_deref())?;
            if count_only {
                writeln!(report, "{}", mutants.len())?;
            } else if json {
                writeln!(report, "{}", serde_json::to_string_pretty(&list_report(&mutants))?)?;
            } else {
                print_mutants(&mut report, &mutants)?;
            }
        }
        Commands::Test {
            path,
            operator,
            file,
            function,
            test_dir,
            target_dir,
            test_command,
            timeout,
            json,
            output,
        } => {
            let root = path
                .canonicalize()
                .with_context(|| format!("cannot resolve {}", path.display()))?;
            let mutants = discover_mutants(
                &root,
                operator.as_deref(),
                file.as_deref(),
                function.as_deref(),
            )?;
            let mut report = report_writer(output.as_deref())?;
            if mutants.is_empty() {
                if json {
                    writeln!(report, "{}", serde_json::to_string_pretty(&test_report(&[]))?)?;
                } else {
                    writeln!(report, "No matching Soroban semantic mutants found.")?;
                }
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
            if !json {
                writeln!(report, "Baseline: {}", test_command)?;
            }
            match run_process(&test_cwd, &test_command, timeout, &baseline_target_dir)? {
                ProcessResult::Passed => {
                    if !json {
                        writeln!(report, "Baseline PASS\n")?;
                    }
                }
                ProcessResult::Failed => {
                    bail!("baseline tests failed; mutation results would be invalid")
                }
                ProcessResult::Timeout => bail!("baseline tests timed out"),
            }

            let mut results = Vec::new();
            for mutant in mutants {
                let outcome = execute_mutant(
                    &root,
                    &mutant,
                    test_dir.as_deref(),
                    &test_command,
                    timeout,
                    &mutant_target_root,
                    shared_target,
                )?;
                if !json {
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
                writeln!(report, "{}", serde_json::to_string_pretty(&test_report(&results))?)?;
            } else {
                print_summary(&mut report, &results)?;
            }
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
    }

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

fn discover_mutants(
    root: &Path,
    operator: Option<&str>,
    file_filter: Option<&str>,
    function_filter: Option<&str>,
) -> Result<Vec<Mutant>> {
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve {}", root.display()))?;
    let mut mutants = Vec::new();

    for entry in WalkDir::new(&root)
        .into_iter()
        .filter_entry(|entry| !ignored_entry(entry, &root))
    {
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

    mutants.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.start_byte.cmp(&b.start_byte))
            .then(a.operator.cmp(&b.operator))
            .then(a.replacement.cmp(&b.replacement))
    });

    for (index, mutant) in mutants.iter_mut().enumerate() {
        mutant.id = format!("M{:04}", index + 1);
    }

    Ok(mutants)
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
    timeout: u64,
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
            match run_process(&scratch_cwd, &compile_command, timeout, &target_dir)? {
                ProcessResult::Failed => return Ok(Outcome::Unviable),
                ProcessResult::Timeout => return Ok(Outcome::Timeout),
                ProcessResult::Passed => {}
            }
        }

        Ok(
            match run_process(&scratch_cwd, test_command, timeout, &target_dir)? {
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

fn compile_command_for(test_command: &str) -> Option<String> {
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

        let removed = clean_generated_state(&root, None).expect("cleanup should succeed");

        assert_eq!(removed.len(), 3);
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

        let mutants = discover_mutants(&root, Some("AUTH-001"), None, None)
            .expect("fixture mutation discovery should work");
        let mutant = mutants.first().expect("AUTH-001 should be discovered");

        let outcome = execute_mutant(&root, mutant, None, "cargo test -q", 30, &target, true)
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
        assert_eq!(compile_command_for("cargo nextest run"), None);
    }
}
