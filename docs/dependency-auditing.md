# Dependency vulnerability scanning and exception policy

CI runs **RustSec cargo-audit** in an independent `dependency-audit` job for
pull requests and pushes to `main`. It analyzes the committed root
`Cargo.lock` against the advisory database:

```bash
cargo install cargo-audit --locked
cargo audit --file Cargo.lock
```

The separate job does not change MSRV verification or mutation-test behavior.
It fails on active vulnerability advisories; an unavailable advisory database
or failed tool installation also fails the job, rather than silently passing.
A clean result applies to the **root lockfile**, not every independently
versioned fixture lockfile or deployed downstream contract.

## Responding to findings

1. Read the advisory ID, affected crate/version and any patched versions.
2. Update the dependency and lockfile where a compatible fix exists; run
   `cargo test --all`, `cargo clippy --all-targets --all-features -- -D warnings`,
   and `cargo audit --file Cargo.lock`.
3. If there is no immediate fix, propose a *time-bounded* exception in
   `.cargo/audit.toml`. The change must identify the advisory, affected
   dependency, impact, why the fix is unavailable, compensating controls,
   an owner and a review/removal date in the pull request and in a TOML
   comment next to the entry. Seek maintainer/security review.
4. Never add a blanket ignore or suppress a new finding solely to turn CI green.
   Remove the exception as soon as a patched dependency is available.

Example of an **illustrative** exception (not an approved exception):

```toml
[advisories]
# RUSTSEC-YYYY-NNNN: <affected crate>; <risk and mitigation>;
# owner: <maintainer>; remove/review by YYYY-MM-DD.
ignore = ["RUSTSEC-YYYY-NNNN"]
```

The committed configuration contains **no** exceptions. An advisory DB
update may change audit results without a source change. Reproduce with the
advisory ID and database revision and investigate the report; do not automatically
ignore new advisories. A clean audit is not a guarantee that dependencies are
vulnerability-free.

## Duplicate and unnecessary crates

The independent `dependency-hygiene` CI job complements the advisory scan by
checking the dependency graph for avoidable bloat:

- **Duplicate crate names** — more than one version of the same crate in the
  committed root `Cargo.lock` (identical versions in different graph branches
  are not duplicates).
- **Unused direct dependencies** — a `[dependencies]` entry in `Cargo.toml`
  that is no longer referenced anywhere under `src/`.

The check is a pure function of the checked-in files, so it needs neither
network access nor an advisory database:

```bash
.github/scripts/check-dependency-graph.sh --self-test
.github/scripts/check-dependency-graph.sh
cargo test --test dependency_graph_cli
```

The `--self-test` mode is the regression seed: it injects a synthetic
`serde_json` duplicate into a sample lockfile and verifies the detector both
reports it and honors a reviewed exception. The Rust test target performs the
same assertions over the real lockfile and CI workflow, so deleting the guard
from CI fails a normal `cargo test`.

### Current reviewed exception

`syn` appears at two majors: `2.0.119` is our direct AST/span-locations API,
while `3.0.6` is pulled in by `clap_derive 4.6.7` and `serde_derive 1.0.229`.
That duplication is reviewed and time-bounded in
`.cargo/dependency-hygiene.toml` with an owner and review date. To introduce a
new exception, document the same fields in that file and in the pull request;
blanket allowlists are not accepted. Removing a duplicate by upgrading a
direct dependency is always preferred over adding an exception.

Only the root `Cargo.lock` is audited. Fixture lockfiles under `fixtures/`
are independently versioned and excluded, and the `fuzz/` crate has its own
manifest outside the workspace. Disabling this job or downgrading its failures
to warnings is a policy change that requires maintainer review.
