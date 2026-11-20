# v1.0 maintainer architecture overview

This is an implementation-oriented map of the current **v0.1.5** codebase as
it moves toward v1.0. It describes existing behavior rather than promising a
modular rewrite. See [the detailed runner architecture](architecture.md),
[extension prototype](extensions.md), and [operator specification](operators.md).

## End-to-end data flow

```text
CLI inputs (list/test) -- source selection and config
  -> Rust source collection / Cargo workspace discovery
  -> syn parsing and conservative Soroban semantic recognition
  -> structured mutants with original source byte spans and replacements
  -> stable filtering, ordering, operator and shard selection
  -> [list] text / count / JSON report
  -> [test] baseline gate in separate Cargo cache
       -> per-mutant scratch copy + compile-only gate
       -> configured test command, timeout and outcome classification
       -> stable results and reports, progress/checkpoint state
```

## Code ownership and boundaries

| Boundary | Where it lives | Contract and failure behavior |
| --- | --- | --- |
| CLI and validation | `src/main.rs` Clap `Cli`/`Commands`, `run` | Validate selectors, timeouts, runner selection; invalid input returns an error |
| Source parsing | `src/main.rs` discovery helpers, `syn` | Only parse source within selected crates; never execute input during discovery |
| Operator registry | `BUILTIN_OPERATORS` in `src/main.rs` | Stable IDs and descriptions, surfaced by `operators` |
| Semantic recognition | `src/main.rs` AST visitors and matcher helpers | Require Soroban-specific evidence; ambiguous lookalikes are suppressed |
| Candidate selection | `src/main.rs` filtering and shard logic | Deterministic IDs/order for the same sources and configuration |
| Runner | `run_process`, `execute_mutant(s)` in `src/main.rs` | Baseline must pass; compile-invalid mutants are `UNVIABLE`; timeouts remain separate |
| Isolation | Scratch and cache helpers in `src/main.rs` | Original project unchanged; baseline and mutants have separate `CARGO_TARGET_DIR` roots |
| Reporting | `ListReport`, `TestReport`, report writers in `src/main.rs` | JSON v1 envelopes, text, Markdown, HTML, SARIF and GitHub annotations |
| Security boundary | `src/redaction.rs` and stderr capture | Environment credential values removed from diagnostics and metadata, not from explicitly supplied source excerpts |
| Extension prototype | `src/lib.rs` `extension::v1` | Embedder-provided Rust code; no dynamic loading and no automatic CLI registration |

The implementation still has a large `src/main.rs`; these are **logical**
boundaries, not separate crates. Avoid assuming a public plugin registration
interface: `extension::v1` is a compile-time prototype.

## Runnable commands

From the repository root (Rust toolchain and Cargo required):

```bash
cargo run -- operators
cargo run -- list fixtures/weak-auth --operator AUTH-001 --json
cargo run -- test fixtures/weak-auth --operator TOKEN-001 --json
cargo test --all
cargo fmt --all -- --check
```

`TOKEN-001` against the weak-auth fixture demonstrates an empty result
without executing a mutant. For a real workload, replace the fixture with a
Cargo project that has a green test baseline, then run:

```bash
cargo run -- test /path/to/soroban-project --operator AUTH-001
```

## Invariants maintainers must preserve

- List and test use the same discovery and selection rules.
- Recognition uses AST context, not broad textual method-name matching.
- Changes are applied to original source byte spans and must respect UTF-8.
- The clean baseline runs before mutants; a failed baseline gives no mutation score.
- `KILLED` and `SURVIVED` are meaningful only for viable mutants.
  `UNVIABLE` and `TIMEOUT` are excluded from the score.
- Scratch worktrees do not modify the checkout; cache identities prevent
  unrelated executions from sharing build artifacts.
- Reports and shard assignment are stable in mutant ID order, even with
  concurrent workers. Never print environment-derived secrets in diagnostic
  channels.

## Known limitations and troubleshooting

- The engine recognizes a limited set of *supported syntactic patterns*, not
  all valid Soroban code; unsupported aliases and indirect calls may be skipped.
- A surviving mutant signals a gap in the configured tests, **not** proof of
  an exploitable vulnerability.
- Each parallel worker uses substantial build cache disk space; try `--jobs 1`
  or `cargo run -- clean` when storage is constrained.
- Subprocesses use a Bash test command, so tests should be run in a compatible
  environment; command stderr is sanitized before being forwarded.
- The reporter intentionally includes original and replacement source in
  mutation reports. Do not put secrets in source fixtures.
- The compile-time extension API is not yet wired into the production CLI.

For error messages and recovery paths, use
[troubleshooting](troubleshooting.md). For security-related boundaries, see
[SECURITY.md](../SECURITY.md).
