# Compile Viability Fixture Suite

This document describes the compile viability gate and compile-only fixture suite in Soro Mutants.

## Background & Objective

Semantic mutants are designed to be type-preserving whenever possible. However, certain mutations (such as method substitution or storage-type mutation) may produce Rust code that does not compile against specific downstream dependencies.

Soro Mutants executes a dedicated, separate compile gate (by default using `cargo test --no-run`) before running tests on any mutant:
- If compilation fails, the mutant is classified as **`UNVIABLE`**.
- It is excluded from the test-quality score denominator (`100 * killed / viable`).
- It is never falsely counted as killed by tests.

The compile-only fixture suite verifies that representative mutants for all operator families produce syntactically valid Rust and reach this compile viability gate cleanly without parser failure or AST corruption.

## Compile-Only Fixtures

Located under `fixtures/compile-only/`:
- `fixtures/compile-only/auth`: Covers `AUTH-001` through `AUTH-005` (authorization removal, receiver substitution, arg vector replacement, helper bypass, admin substitution).
- `fixtures/compile-only/token`: Covers `TOKEN-001` through `TOKEN-004` (transfer swap, allowance swap, multi-client receivers).
- `fixtures/compile-only/event`: Covers `EVENT-001` through `EVENT-005` (events publish, contractevent publications, event fields).
- `fixtures/compile-only/storage`: Covers `STORAGE-001` and `TTL-001` through `TTL-003` (storage instance/persistent/temporary classes, TTL extensions).

## Running Locally

To run the automated compile viability verification:

```bash
cargo test --test compile_viability_fixtures
```

This verifies that:
1. Every representative operator in the fixture suite generates candidates.
2. The modified Rust code parses into a valid abstract syntax tree (`syn::File`).
3. No syntactic regressions or corruptions occur during mutant injection.
