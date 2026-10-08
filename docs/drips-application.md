# Drips / Stellar Wave Maintainer Brief

## Project

**Soro Mutants** is a Soroban-aware semantic mutation-testing tool for Stellar smart contracts.

It injects realistic Soroban faults into Rust contract source and runs the project's existing test suite against each mutant. The tool does not ask whether the current source already contains a vulnerability. It asks whether the tests would detect a security-critical behavior if that behavior were removed or changed.

## Problem

Generic Rust mutation testing is useful for syntax- and language-level changes such as arithmetic, comparisons, and return values. Soroban contracts also depend on behaviors that are meaningful only in Stellar's smart-contract model, including:

- which `Address` must authorize a call;
- whether contract and ledger state has its intended TTL behavior;
- whether contract events form part of the externally observable API;
- whether token value moves between the intended parties.

Those behaviors can be exercised by tests without actually being asserted. A suite can therefore have strong line or functional coverage while still failing to prove an important Soroban security invariant.

## Why this is Stellar-specific

The initial mutation operators target Soroban SDK semantics directly:

| Operator | Semantic mutation |
| --- | --- |
| `AUTH-001` | remove `require_auth()` |
| `AUTH-002` | authenticate a different in-scope `Address` |
| `TTL-001` | remove `extend_ttl()` |
| `EVENT-001` | remove direct contract event publication |
| `TOKEN-001` | swap sender and recipient in a direct token transfer |

The project deliberately avoids duplicating generic arithmetic/boolean mutation already handled well by existing Rust mutation tools.

## What exists today

The public repository already contains:

- AST-aware Rust discovery using `syn`;
- source-span-preserving mutation patches;
- clean baseline gating;
- compile viability gating;
- `KILLED / SURVIVED / UNVIABLE / TIMEOUT` classifications;
- text and JSON output;
- operator/file/function filtering;
- five Soroban semantic operators;
- weak/strong controlled fixtures;
- CI;
- contributor and security documentation;
- public benchmark notes against real Soroban repositories.

## Proof that the signal is useful

Controlled fixtures show the intended discrimination:

- a functional-only authorization test allows both `AUTH-001` and `AUTH-002` to survive;
- an explicit authorization-tree assertion kills both mutants.

Public repository benchmarks provide both positive and negative controls:

- Phoenix Protocol contains selected admin-change tests where authorization/event semantic mutants survive;
- Phoenix token and TTL tests kill corresponding auth/TTL mutants where those semantics are explicitly asserted;
- Soroswap's focused token authorization test kills both auth removal and wrong-address authentication;
- Stellar's official `soroban-examples` contains an authorization assertion that kills the corresponding auth mutant;
- the RWA Toolkit benchmark kills sender-auth mutants while selected event-removal mutants survive.

See [benchmark.md](benchmark.md) for exact commits, commands, exclusions, and interpretation.

A surviving mutant is treated as a **test-suite signal**, never automatically as an exploitable vulnerability.

## Relationship to existing tooling

Soro Mutants is complementary to, not a replacement for:

- `cargo-mutants`: generic Rust mutation testing;
- static Soroban security analyzers: inspect the current contract for suspicious patterns;
- fuzzing: changes inputs rather than the program;
- formal verification: proves specified properties;
- coverage tooling: shows which code executed.

Soro Mutants changes the program using Soroban-specific fault models and asks whether the existing tests notice.

## Contribution surface

The repository intentionally stops at a useful v0.1 core. Open work is split into independently reviewable issues with acceptance criteria and explicit non-goals.

Current areas include:

### Semantic operators

- #1 — `require_auth_for_args` mutation support
- #2 — typed Soroban event mutation
- #3 — TTL threshold/target mutation
- #4 — allowance owner/spender substitution
- #5 — cross-contract address substitution

### Runner and performance

- #6 — changed-file / changed-function selection
- #7 — isolated parallel mutant execution
- #10 — optional `cargo nextest` runner

### Reporting

- #8 — SARIF output
- #9 — versioned JSON result schema

The roadmap leaves additional room for storage-class mutations, upgrade/admin semantics, event-value substitutions, richer token mutations, equivalent-mutant reduction, caching, sharding, corpus expansion, and additional integrations.

## Maintainer principles

Contributor work should remain real and reviewable:

1. Each issue must produce an independently useful result.
2. New operators need a Soroban-specific rationale, weak/strong tests, and false-positive boundaries.
3. Compile-invalid mutants are `UNVIABLE`, not falsely counted as killed.
4. Broken baselines are excluded rather than turned into findings.
5. Survivors are test-quality observations, not vulnerability claims.
6. Performance optimizations must preserve build isolation and deterministic classification.

## Repository

https://github.com/Ay-obami/soro-mutants

## Suggested application summary

Soro Mutants is an open-source semantic mutation-testing engine for Stellar Soroban contracts. It complements generic Rust mutation tools by injecting Stellar-specific faults such as removed/wrong-address authorization, missing TTL extension, removed contract events, and reversed token transfer direction, then runs the project's existing tests to determine whether those behaviors are actually enforced. The v0.1 core is working, CI-backed, and validated against controlled fixtures plus multiple public Soroban repositories. The repository has a scoped contributor roadmap spanning new semantic operators, runner performance, reporting, and integrations, with each task defined around testable acceptance criteria.
