# Roadmap

Soro Mutants v0.1 intentionally implements a small semantic core and leaves substantial, independently useful work for future contributors.

## v0.1 core

Implemented:

- `AUTH-001` — remove `require_auth()`
- `AUTH-002` — authenticate a different in-scope `Address`
- `TTL-001` — remove `extend_ttl()`
- `EVENT-001` — remove direct `env.events().publish(...)`
- `TOKEN-001` — swap sender and recipient on a direct three-argument token transfer
- file/function/operator filtering
- baseline gating
- compile viability gating
- `KILLED / SURVIVED / UNVIABLE / TIMEOUT` outcomes
- text and JSON output
- real-repository benchmark notes
- explicit cleanup command for Soro Mutants scratch/build caches

## Next operator families

These are roadmap areas, not promises that every possible mutation is useful.

### Authorization

- `require_auth_for_args` removal or argument mutation
- helper/interprocedural authorization recognition
- admin/owner substitution where the intended authority is recoverable from Soroban types or contract structure
- initialization/upgrade authorization mutations

### Storage lifecycle

- TTL threshold mutation
- TTL target mutation
- persistent/temporary/instance storage-class substitution where type-safe
- removal of lifecycle refresh on selected state paths

### Events

- typed `#[contractevent]` publication support
- event actor substitution
- event amount/value substitution
- topic/schema mutations with strict false-positive controls

### Token and value movement

- allowance spender/owner substitution
- transfer amount substitution using semantically meaningful values
- token-contract address substitution when multiple typed token clients are in scope

### Cross-contract behavior

- argument substitution in contract-to-contract calls
- dependency-address substitution
- expected-return handling mutations

## Runner and performance

The clean baseline and mutated runs use separate Cargo target roots so mutant artifacts cannot contaminate the baseline. Mutants share a dedicated cache with one another, allowing Cargo to reuse unchanged dependencies while rebuilding the copied source tree for each mutation.

Useful future work includes:

- stronger cache keys and invalidation regression coverage across toolchains/workspaces
- incremental execution
- changed-function / changed-file mutation selection
- parallel workers
- deterministic sharding
- result caching keyed by source, operator, test command, toolchain, and dependency state
- optional `cargo nextest` execution

Any cache optimization must have regression tests proving that artifacts from one mutant cannot contaminate another mutant or the clean baseline.

## Reporting and integrations

Potential extensions:

- SARIF output
- GitHub Actions integration
- Markdown/HTML reports
- machine-readable schema versioning
- CI mutation-score thresholds
- PR annotations for survived mutants

## Corpus

Build a curated Soroban semantic mutation corpus containing:

- original source pattern
- injected mutant
- expected classification under a weak test
- expected classification under a strong semantic assertion
- source/provenance notes when generalized from a public ecosystem issue

The corpus should generalize bug classes rather than copy third-party code unnecessarily.

## Non-goals

Soro Mutants should not become:

- a static vulnerability scanner;
- a replacement for `cargo-mutants`;
- a fuzzing framework;
- a formal verifier;
- a statement that every surviving mutant is exploitable.

Those tools answer different questions and should remain complementary.
