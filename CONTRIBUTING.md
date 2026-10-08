# Contributing to Soro Mutants

Soro Mutants is a semantic mutation-testing tool for Soroban smart contracts. Contributions should improve the quality of mutations, the accuracy of classification, or the developer workflow without turning the project into a generic Rust mutation framework.

## Before opening a pull request

1. Open or reference an issue that describes the behavior being changed.
2. Keep the change scoped to one independently reviewable outcome.
3. Add or update a fixture or unit test for mutation-engine changes.
4. Run:

```bash
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## Adding a semantic operator

A new operator should satisfy all of these:

- It represents a Soroban/Stellar semantic fault, not a generic Rust syntax mutation already handled well by tools such as `cargo-mutants`.
- The mutation preserves syntactic validity whenever reasonably possible.
- The operator has a narrow recognizer to avoid unrelated method calls with the same name.
- At least one test demonstrates discovery of the intended mutation.
- At least one test covers an important false-positive boundary.
- The operator documentation explains what a surviving mutant means and what it does **not** prove.

Use IDs grouped by semantic family:

- `AUTH-xxx` — authorization
- `TTL-xxx` — state lifetime / TTL
- `EVENT-xxx` — contract events
- `TOKEN-xxx` — asset/value movement
- future families should receive their own stable prefix

Existing operator IDs should not be repurposed after release.

## Issue quality

Issues intended for contributors should include:

- problem and why it matters for Soroban;
- affected module(s);
- expected mutation or behavior;
- examples of in-scope syntax;
- false-positive boundaries;
- acceptance criteria;
- tests required;
- explicit out-of-scope behavior.

A large issue should be split only when the resulting tasks remain useful independently.

## Benchmark evidence

A benchmark result is valid only when the unmodified configured baseline passes.

A surviving mutant must be described as a **test-suite signal**, not automatically as a vulnerability. Public benchmark notes should separate:

1. the original secure/intended behavior;
2. the injected mutation;
3. the test command;
4. the observed result;
5. human interpretation.

Do not publish a claim that a third-party project is exploitable based only on a surviving mutant.

## Design constraints

- Prefer AST-aware recognition over regex replacement.
- Preserve original source formatting and source spans.
- Treat compile-invalid mutants as `UNVIABLE`, not `KILLED`.
- Treat timeouts separately from killed/survived mutants.
- Correct classification is more important than build-cache speed.

## Pull request scope

Avoid unrelated refactors in operator PRs. Performance work, output formats, runners, operator additions, and benchmark-corpus changes should remain separable whenever possible.
