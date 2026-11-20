# Soro Mutants v1.0 Release Acceptance Checklist

This is the objective release gate for v1.0. A release candidate is ready only when every Required item is verified against the exact release commit. Future roadmap ideas are not blockers unless listed here.

## Build and test — Required

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --all-targets --all-features`
- [ ] Semantic weak/strong fixture CI passes.
- [ ] Exit-code and deterministic-ordering tests pass.

## Operator correctness — Required

- [ ] Every operator from `cargo soro-mutants operators` is documented in `docs/operators.md`.
- [ ] Every shipped operator has positive and false-positive regression coverage.
- [ ] CLI filter parsing is fuzz-tested panic-free (`docs/fuzzing.md`).
- [ ] Fixture organization follows `docs/fixture-conventions.md`.
- [ ] Fixture coverage matrix in `docs/fixture-coverage-matrix.md` tracks all operators.
- [ ] Compile-only fixture suite in `fixtures/compile-only/` validates compile viability.
- [ ] Equivalent-mutant suppression tests pass.
- [ ] Documentation never equates a surviving mutant with a vulnerability.

## Reporting — Required

- [ ] JSON empty/populated reports are valid and schema examples match emitted fields.
- [ ] Markdown, HTML, SARIF, GitHub summary, and annotations tests pass.
- [ ] Identical normalized results produce deterministic output.
- [ ] Score calculation remains killed / (killed + survived), excluding UNVIABLE and TIMEOUT.

## Runner and isolation — Required

- [ ] A failing baseline prevents classification.
- [ ] Baseline artifacts cannot be contaminated by mutant builds.
- [ ] Scratch worktrees are cleaned after success and failure.
- [ ] `clean --dry-run` is non-destructive; `clean` removes only managed state.
- [ ] Timeout and shard determinism tests pass.
- [ ] Opt-in result-cache hits occur only with a fully matching identity; `--no-cache` and `--clear-cache` behave as documented ([tests](result-cache.md#regression-coverage)).
- [ ] Benchmark matrix refuses external targets without `--allow-external` and never scores a failed baseline ([docs](benchmark-matrix.md#safety-model)).

## Security — Required

- [ ] `SECURITY.md` is current.
- [ ] Normal user-controlled invalid input returns errors, not panics.
- [ ] Managed cleanup paths cannot be redirected through symlinks.
- [ ] Report output does not overwrite a final-path symlink target.
- [ ] Dependency advisory review has no unreviewed critical/high finding; the independent RustSec audit job passes (see [policy](dependency-auditing.md)).
- [ ] Environment credential redaction regression tests pass for errors, metadata, and child stderr.
- [ ] Package contains no credentials, local secrets, or private fixture data.

## Compatibility — Required

- [ ] Cargo.toml declares the minimum Rust version and CI verifies it.
- [ ] Current stable Rust CI passes.
- [ ] Supported operating systems are documented in the versioned [v1.0 platform policy](supported-platforms.md) and `check-platform --json` agrees on a primary host.
- [ ] Any v0.1.x incompatibility has a migration note.
- [ ] v1 JSON compatibility rules are documented.

## Packaging — Required

Run `cargo package --locked`, then verify:

- [ ] Package succeeds from a clean checkout.
- [ ] README, LICENSE, CONTRIBUTING, SECURITY, and docs are included.
- [ ] Scratch/cache/target/report artifacts are excluded.
- [ ] `cargo install --path . --locked` works.
- [ ] `cargo soro-mutants --version` reports the intended version.

## Documentation — Required

- [ ] README install and quick-start commands work.
- [ ] Every public CLI subcommand is documented.
- [ ] Operator, reporting, changed-selection, nextest, troubleshooting, architecture, result-cache, benchmark-matrix, and supported-platforms docs match release behavior.
- [ ] Known limitations distinguish non-goals from missing release requirements.

## Release notes — Required

- [ ] Release notes cover features, compatibility changes, security hardening, and known limitations.
- [ ] Notes link the exact tag/commit.
- [ ] Pre-release wording is removed for final v1.
- [ ] Changed defaults, schemas, or exit behavior have upgrade notes.

## Final sign-off — Required

Record in the release PR:

- Release commit SHA
- CI run URL
- MSRV job
- Stable Rust job
- `cargo package --locked` result
- Security/advisory review
- Documentation review
- Known limitations review
- Maintainer sign-off

A release is blocked by an unchecked Required item or release-blocking defect, not merely by unrelated future-feature issues.
