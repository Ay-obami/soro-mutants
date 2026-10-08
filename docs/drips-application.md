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
- automatic scratch cleanup plus an explicit cache-clean command;
- five Soroban semantic operators;
- weak/strong controlled fixtures;
- CI;
- contributor and security documentation;
- public benchmark notes against real Soroban repositories.

## Proof that the signal is useful

Controlled fixtures show the intended discrimination:

- a functional-only authorization test allows both `AUTH-001` and `AUTH-002` to survive;
- an explicit authorization-tree assertion kills both mutants.

The public `semantic-fixtures` GitHub Actions job enforces those opposite outcomes on every push and pull request, so the project's core claim is regression-tested rather than documented only.

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

- [#1 — `require_auth_for_args` mutation support](https://github.com/Ay-obami/soro-mutants/issues/1)
- [#2 — typed Soroban event mutation](https://github.com/Ay-obami/soro-mutants/issues/2)
- [#3 — TTL threshold/target mutation](https://github.com/Ay-obami/soro-mutants/issues/3)
- [#4 — allowance owner/spender substitution](https://github.com/Ay-obami/soro-mutants/issues/4)
- [#5 — cross-contract address substitution](https://github.com/Ay-obami/soro-mutants/issues/5)

### Runner and performance

- [#6 — changed-file / changed-function selection](https://github.com/Ay-obami/soro-mutants/issues/6)
- [#7 — isolated parallel mutant execution](https://github.com/Ay-obami/soro-mutants/issues/7)
- [#10 — optional `cargo nextest` runner](https://github.com/Ay-obami/soro-mutants/issues/10)

### Reporting

- [#8 — SARIF output](https://github.com/Ay-obami/soro-mutants/issues/8)
- [#9 — versioned JSON result schema](https://github.com/Ay-obami/soro-mutants/issues/9)

The roadmap leaves additional room for storage-class mutations, upgrade/admin semantics, event-value substitutions, richer token mutations, equivalent-mutant reduction, caching, sharding, corpus expansion, and additional integrations.

## First-Wave issue priority

Do not add the entire GitHub backlog to the Program at once. Drips counts every active issue against the repository's configured points budget, and unresolved issues carry over to later Waves.

Recommended priority order:

1. **#1 — AUTH-003 `require_auth_for_args` support** — Medium (150 points).
2. **#9 — versioned JSON result schema** — Medium (150 points); also the best newcomer entry point.
3. **#2 — typed Soroban event mutation** — High (200 points).
4. **#6 — changed-file / changed-function selection** — Medium (150 points).
5. **#8 — SARIF output** — Medium (150 points).

If the repository receives a 500-point per-Wave budget, #1 + #9 + #2 form a balanced 500-point initial set: one authorization operator, one bounded reporting task, and one deeper Soroban event operator. If the actual budget differs, keep the same priority order and add only work that fits the dashboard's current budget.

Set Medium/High complexity in the Drips maintainer dashboard when adding these issues. Adding an issue only through the GitHub Wave label defaults it to Trivial (100 points), so the label-only workflow would not preserve the 150/200-point plan above.

## Application checklist

Repository-side preparation is complete:

- public GitHub repository with Apache-2.0 license;
- tagged `v0.1.0` pre-release;
- CI for formatting, unit tests, Clippy, packaging, and weak/strong semantic fixture outcomes;
- reproducible benchmark notes against public Soroban repositories;
- contribution guide, security policy, architecture, operator specification, and roadmap;
- ten open contribution issues with scoped acceptance criteria and suggested difficulty documented in each issue.

Drips-side steps still require the maintainer account:

1. Sign in to Drips Wave with GitHub.
2. Install/authorize the Drips Wave GitHub App for the account or organization hosting this repository.
3. Sync `Ay-obami/soro-mutants`.
4. Apply the repository to the Stellar Wave Program.
5. Wait for organizer approval.
6. After approval, add only the prioritized issues that fit the repository's displayed points budget and set their complexity in the Drips dashboard.

## Maintainer principles

Contributor work should remain real and reviewable:

1. Each issue must produce an independently useful result.
2. New operators need a Soroban-specific rationale, weak/strong tests, and false-positive boundaries.
3. Compile-invalid mutants are `UNVIABLE`, not falsely counted as killed.
4. Broken baselines are excluded rather than turned into findings.
5. Survivors are test-quality observations, not vulnerability claims.
6. Performance optimizations must preserve build isolation and deterministic classification.

## Repository

- Repository: https://github.com/Ay-obami/soro-mutants
- v0.1.0 pre-release: https://github.com/Ay-obami/soro-mutants/releases/tag/v0.1.0

## Suggested application summary

Soro Mutants is an open-source semantic mutation-testing engine for Stellar Soroban contracts. It complements generic Rust mutation tools by injecting Stellar-specific faults such as removed/wrong-address authorization, missing TTL extension, removed contract events, and reversed token transfer direction, then runs the project's existing tests to determine whether those behaviors are actually enforced. The v0.1.0 core is publicly released, CI-backed, and validated against controlled fixtures plus multiple public Soroban repositories. The repository has a scoped contributor roadmap spanning new semantic operators, runner performance, reporting, and integrations, with each task defined around testable acceptance criteria.
