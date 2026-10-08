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

The target repository is never mutated in place. Each mutant is applied to a copied scratch worktree under `.soro-mutants-worktree`.

The clean baseline uses:

```text
.soro-mutants-target/baseline
```

Mutated runs use a separate cache:

```text
.soro-mutants-target/mutants-shared
```

This prevents mutant build artifacts from contaminating the baseline while still allowing dependencies to be reused between mutant runs.

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
