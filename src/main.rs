use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use proc_macro2::Span;
use serde::Serialize;
use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use syn::{
    spanned::Spanned,
    visit::{self, Visit},
    Expr, ExprMethodCall, FnArg, ImplItemFn, ItemFn, Pat, Signature, Type,
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
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        operator: Option<String>,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        function: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Execute semantic mutants against a Cargo test suite.
    Test {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        operator: Option<String>,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        function: Option<String>,
        #[arg(long)]
        test_dir: Option<PathBuf>,
        #[arg(long)]
        target_dir: Option<PathBuf>,
        #[arg(long, default_value = "cargo test -q")]
        test_command: String,
        #[arg(long, default_value_t = 120)]
        timeout: u64,
        #[arg(long)]
        json: bool,
    },
    /// Remove Soro Mutants scratch and build-cache directories.
    Clean {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        target_dir: Option<PathBuf>,
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

#[derive(Clone, Copy, Debug)]
enum ProcessResult {
    Passed,
    Failed,
    Timeout,
}

struct AuthVisitor<'a> {
    source: &'a str,
    file: PathBuf,
    current_function: Option<String>,
    address_params: Vec<String>,
    mutants: Vec<Mutant>,
}

impl<'a> AuthVisitor<'a> {
    fn new(source: &'a str, file: PathBuf) -> Self {
        Self {
            source,
            file,
            current_function: None,
            address_params: Vec::new(),
            mutants: Vec::new(),
        }
    }

    fn enter_signature(&mut self, sig: &Signature) -> (Option<String>, Vec<String>) {
        let previous_name = self.current_function.replace(sig.ident.to_string());
        let previous_params = std::mem::replace(&mut self.address_params, address_params(sig));
        (previous_name, previous_params)
    }

    fn leave_signature(&mut self, previous: (Option<String>, Vec<String>)) {
        self.current_function = previous.0;
        self.address_params = previous.1;
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

impl<'ast> Visit<'ast> for AuthVisitor<'_> {
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

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        if node.method == "publish" && is_events_receiver(&node.receiver) {
            self.push_mutant(
                "EVENT-001",
                node.span(),
                "()".to_string(),
                "remove Soroban event publication".to_string(),
            );
        }

        if node.method == "transfer" && node.args.len() == 3 && !is_events_receiver(&node.receiver)
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

        if node.method == "extend_ttl" {
            self.push_mutant(
                "TTL-001",
                node.span(),
                "()".to_string(),
                "remove extend_ttl()".to_string(),
            );
        }

        if node.method == "require_auth" {
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
        } => {
            let mutants = discover_mutants(
                &path,
                operator.as_deref(),
                file.as_deref(),
                function.as_deref(),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&mutants)?);
            } else {
                print_mutants(&mutants);
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
            if mutants.is_empty() {
                if json {
                    println!("[]");
                } else {
                    println!("No matching Soroban semantic mutants found.");
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
                println!("Baseline: {}", test_command);
            }
            match run_process(&test_cwd, &test_command, timeout, &baseline_target_dir)? {
                ProcessResult::Passed => {
                    if !json {
                        println!("Baseline PASS\n");
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
                    println!(
                        "{:<10} {:<8} {}:{} {}",
                        outcome_label(&outcome),
                        mutant.operator,
                        mutant.file.display(),
                        mutant.span.line,
                        mutant.description
                    );
                }
                results.push(MutantResult { mutant, outcome });
            }

            if json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                print_summary(&results);
            }
        }
        Commands::Clean { path, target_dir } => {
            let root = path
                .canonicalize()
                .with_context(|| format!("cannot resolve {}", path.display()))?;
            let removed = clean_generated_state(&root, target_dir.as_deref())?;
            if removed.is_empty() {
                println!("No Soro Mutants generated state found.");
            } else {
                for path in removed {
                    println!("Removed {}", path.display());
                }
            }
        }
    }

    Ok(())
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

        let mut visitor = AuthVisitor::new(&source, relative);
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

fn is_events_receiver(expr: &Expr) -> bool {
    match expr {
        Expr::MethodCall(call) => call.method == "events" || is_events_receiver(&call.receiver),
        Expr::Reference(reference) => is_events_receiver(&reference.expr),
        Expr::Paren(paren) => is_events_receiver(&paren.expr),
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

fn clean_generated_state(root: &Path, target_dir: Option<&Path>) -> Result<Vec<PathBuf>> {
    let scratch = root.join(".soro-mutants-worktree");
    let (baseline, mutants) = target_dirs(root, target_dir);
    let mut removed = Vec::new();

    for path in [scratch, baseline, mutants] {
        if path.exists() {
            fs::remove_dir_all(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
            removed.push(path);
        }
    }

    if target_dir.is_none() {
        let default_root = root.join(".soro-mutants-target");
        if default_root.exists()
            && fs::read_dir(&default_root)
                .with_context(|| format!("failed to inspect {}", default_root.display()))?
                .next()
                .is_none()
        {
            fs::remove_dir(&default_root)
                .with_context(|| format!("failed to remove {}", default_root.display()))?;
        }
    }

    Ok(removed)
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

fn print_mutants(mutants: &[Mutant]) {
    println!("Discovered {} semantic mutants\n", mutants.len());
    for mutant in mutants {
        println!(
            "{} {:<8} {}:{} {}",
            mutant.id,
            mutant.operator,
            mutant.file.display(),
            mutant.span.line,
            mutant.description
        );
        println!("    - {}", mutant.original.trim());
        println!("    + {}\n", mutant.replacement.trim());
    }
}

fn print_summary(results: &[MutantResult]) {
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

    println!("\nSummary");
    println!("  Generated : {}", results.len());
    println!("  Killed    : {}", killed);
    println!("  Survived  : {}", survived);
    println!("  Unviable  : {}", unviable);
    println!("  Timeout   : {}", timeout);
    println!("  Score     : {:.1}%", score);
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

    fn mutations_for(source: &str) -> Vec<Mutant> {
        let syntax = syn::parse_file(source).expect("test source should parse");
        let mut visitor = AuthVisitor::new(source, PathBuf::from("src/lib.rs"));
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
