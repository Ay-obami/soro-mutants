# Architecture

Soro Mutants separates semantic mutation discovery from test execution.

```text
Rust source
   |
   v
syn parser
   |
   v
Soroban semantic recognizers
   |
   v
structured Mutant
   |
   v
fresh scratch worktree
   |
   +--> compile-only viability gate
   |
   v
configured Cargo tests
   |
   v
KILLED / SURVIVED / UNVIABLE / TIMEOUT
```

## Discovery

The engine parses Rust source with `syn` and visits functions and method calls. Each operator recognizes a narrow Soroban-specific pattern and records a source span, original text, replacement text, enclosing function, and operator ID.

The AST is used for recognition only. Mutations are applied to the original source text so formatting, comments, and useful line locations remain intact.

## Execution isolation

The target repository is never mutated in place. Each mutant is applied to a copied scratch worktree under `.soro-mutants-worktree`. The scratch worktree is removed after each mutant, including compile-invalid and timed-out outcomes.

The clean baseline uses:

```text
.soro-mutants-target/baseline
```

Mutated runs use a separate cache:

```text
.soro-mutants-target/mutants-shared
```

This prevents mutant build artifacts from contaminating the baseline while still allowing dependencies to be reused between mutant runs. Build caches persist deliberately for performance and can be removed with `cargo soro-mutants clean`; cleanup never removes the target repository's normal Cargo cache or source files.

Each cache root includes an artifact marker (`.soro-baseline-artifact` or `.soro-mutant-artifact`) to ensure baseline and mutant artifacts cannot be mixed or confused. In addition, `cache-identity.json` maintains a cryptographic fingerprint of source files, active operators, test commands, toolchain versions, and workspace packages. If any of these attributes change, stale caches are invalidated and purged automatically.

## Resumability and progress tracking

Mutation test runs maintain a local progress file (`<target-dir>/progress.json` or `--progress-file <path>`).
- State is updated atomically after each mutant completes execution.
- If interrupted, runs can be resumed with `--resume`, safely skipping already completed mutants and executing pending or interrupted ones.
- Incompatible resumes (detecting changes in source files, toolchains, commands, or configurations) are refused safely.
- The progress file is removed when running `cargo soro-mutants clean`.

## Workspace and crate scoping

In multi-package Cargo workspaces, `--package` (`-p`) scopes mutation discovery to the source directories of selected packages, pruning unrelated packages. Test commands are automatically scoped with `-p <pkg>` flags when using default runners (`cargo test` or `cargo nextest`), while custom `--test-command` invocations are executed as provided.

## Classification

A mutant must first pass a compile-only Cargo test command. Compile failures are `UNVIABLE`.

For viable mutants:

- configured tests fail -> `KILLED`;
- configured tests pass -> `SURVIVED`;
- execution exceeds the timeout -> `TIMEOUT`.

Only killed and survived mutants are included in the semantic mutation score.

## Boundaries

The engine does not infer that every survivor is exploitable. It measures whether a configured test suite distinguishes a deliberately injected Soroban semantic change.

Generic arithmetic and boolean mutations remain the job of tools such as `cargo-mutants`. Soro Mutants focuses on semantics that require Stellar/Soroban context.
