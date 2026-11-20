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
