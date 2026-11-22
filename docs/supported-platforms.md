# Soro Mutants v1.0 supported-platform policy (policy version 1.0)

This document is the **versioned, independently implementable** platform policy
for v1.0. It is self-contained: it does not depend on any unfinished roadmap
item. The `check-platform` subcommand implements exactly the checks listed
here, and its JSON report is the machine-readable verification record.

The policy preserves v0.1.x behavior. No mutation semantics, operator rules,
default flags, or report schemas are changed by this document. All checks are
deterministic observations of the current environment; expected failures are
reported as structured data, not panics.

## Supported operating systems

| Environment | Tier | Rules |
| --- | --- | --- |
| Linux (glibc, x86_64 or aarch64) | Supported | CI primary target; `bash`, `git`, `rustc`, `cargo` on `PATH` |
| macOS (x86_64 or Apple Silicon aarch64) | Supported | Same tool requirements as Linux |
| Windows 10/11 via WSL2 Linux distribution | Best-effort | Run all commands inside WSL2; native paths and shells are not supported |
| Windows native (MSVC/GNU shell) | Best-effort / not recommended | `bash -lc` invocation and Unix path assumptions may fail; use WSL2 |
| FreeBSD and other Unix-like hosts | Best-effort | No CI coverage; behavior depends on local toolchain |
| WASI, embedded/no-std targets, 32-bit hosts | Unsupported | Host-side test execution and Soroban host testutils require a 64-bit host |

## Rust and Cargo versions

- **MSRV:** Rust `1.85.0` (`rust-version = "1.85"` in `Cargo.toml`).
- **Supported Rust:** any stable Rust `>= 1.85.0`. The dedicated `.github/workflows/msrv.yml` workflow has two
  independent jobs: `msrv` verifies the pinned `1.85.0` baseline, while `stable` is the **latest stable Rust gate**
  and deliberately uses the unpinned `dtolnay/rust-toolchain@stable` action. Both run on every pull request and
  main push. The semantic fixture matrix separately pins `1.91.0`; that pin is *not* the latest-stable check.
- **Cargo:** Cargo `>= 1.85.0`, installed from the same toolchain as `rustc`.
  Cargo lockfile version 4 and `--locked` builds are assumed.
- **Compatibility assumption:** `rustc` and `cargo` are taken from one
  installed toolchain series. A mismatched pair is reported as a warning
  (`best-effort`) rather than silently assumed compatible.
- Nightly Rust and older stable releases below the MSRV are unsupported for
  v1.0 release verification.

## Primary supported development environments

- Linux x86_64, stable Rust (current CI toolchain `1.91.0`), `bash`, `git`.
- Linux x86_64 with Rust `1.85.0` for MSRV verification.
- macOS arm64/x86_64 with the same toolchain versions for local development.

## Best-effort environments

- Linux aarch64 (no dedicated CI runner).
- macOS on either architecture (no dedicated CI runner).
- Windows via WSL2, provided `bash`, `git`, and Rust are installed inside WSL2.
- Other Unix-like hosts with a compatible toolchain.

Best-effort environments are useful, but a failure there is not automatically a
v1.0 release blocker unless it reproduces on a primary environment.

## Unsupported environments

- Rust or Cargo older than `1.85.0`.
- Hosts without `bash` on `PATH` (the mutation runner executes commands with
  `bash -lc`).
- WASI, embedded targets, and 32-bit hosts.
- `kind=external` benchmark targets without network access, `git`, or explicit
  `--allow-external` opt-in.

## Platform-specific limitations and known restrictions

- Test commands run through `bash -lc`; shell-specific syntax from other shells
  is not normalized. `RUSTUP_TOOLCHAIN` inherited from the parent process is
  removed so that commands must pin a toolchain themselves.
- `RUSTUP_TOOLCHAIN=...` inside a manifest `test_command` continues to work and
  is recorded as part of the command string in benchmark summaries.
- Soroban host tests require the SDK `testutils` feature and a toolchain the
  pinned SDK version supports. A baseline failure is always reported as a
  `BaselineFailure` (exit code 3), never as a mutation score.
- Non-UTF-8 paths are accepted by the CLI where the OS permits them; JSON
  reports represent them with lossy string conversion.
- External benchmark execution additionally requires network access and a
  trusted pinned revision. It is never part of normal unit CI.
- Persistent result caches are local to a `--target-dir`; there is no remote or
  cross-repository cache reuse.

## Automated MSRV regression gate

The `Minimum supported Rust version` workflow runs independently of the semantic fixture matrix. Both toolchain jobs print `rustc` and `cargo` versions, compile all targets and features using the committed lockfile, run `check_platform_cli` regression tests, and execute `check-platform --json`. A dependency that silently raises the Rust requirement therefore fails the 1.85.0 job rather than being hidden by a newer toolchain. Jobs have a 15-minute limit. The workflow is also manually dispatchable.

To reproduce locally:

```bash
rustup toolchain install 1.85.0
cargo +1.85.0 check --locked --all-targets --all-features
cargo +1.85.0 test --locked --test check_platform_cli
cargo +1.85.0 run --locked -- check-platform --json
```

A failure at `cargo check --locked` may indicate a dependency has increased its Rust requirement; inspect `Cargo.lock` and the dependency's declared `rust-version` before changing the project's MSRV. The platform check requires `bash` and `git` on the host.

### Latest stable gate

The `stable` job in the same workflow tracks current stable Rust independently of the MSRV pin. It has a
15-minute bound, reports the exact `rustc`/`cargo` versions both in the log and in the GitHub step summary, and
runs the `latest_stable_ci` contract test under a 30-second runtime budget so a silently pinned, unbounded, or
truncated stable job fails the check. It also runs the modern stable-syntax parser corpus.

To reproduce the latest-stable gate locally:

```bash
rustup toolchain install stable
cargo +stable check --locked --all-targets --all-features
cargo +stable test --locked --test check_platform_cli
cargo +stable test --locked --test latest_stable_ci
cargo +stable test --locked --test parser_stable_syntax_corpus_cli
cargo +stable run --locked -- check-platform --json
```

A failure only in the `stable` job (while `msrv` passes) usually means a dependency or syntax feature requires a
newer compiler than the declared MSRV; update the MSRV deliberately rather than loosening the stable gate.

## Verification commands

Run these from the repository root:

```bash
# Machine-readable policy report (schema_version 1, policy_version 1.0).
cargo soro-mutants check-platform --json

# Human-readable report and remediation hints.
cargo soro-mutants check-platform

# Toolchain and host facts used by the check.
rustc --version
cargo --version
rustc -Vv | grep '^host:'
bash --version | head -1
git --version

# Build and packaging verifications in the v1 release checklist.
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo package --locked
```

The `check-platform` command emits these check ids: `operating-system`,
`architecture`, `rustc-msrv`, `cargo-version`, `toolchain-pair`, `shell`, and
`git`. Each has `status` of `pass`, `warning`, or `fail`, plus a `requirement`,
the `observed` value, and a `remediation` string when action is needed. Overall
`status` is `supported` (all pass), `best-effort` (warnings, no failures), or
`unsupported` (at least one failure). `unsupported` exits non-zero; the JSON
report is still printed.

Related tests and documentation:

- [`tests/check_platform_cli.rs`](../tests/check_platform_cli.rs)
- [`tests/panic_free_cli.rs`](../tests/panic_free_cli.rs)
- [`docs/v1-release-checklist.md`](v1-release-checklist.md)
- [`docs/troubleshooting.md`](troubleshooting.md)
- [`docs/benchmark-matrix.md`](benchmark-matrix.md)

## Concrete example

Given a Linux x86_64 host with:

```text
rustc 1.85.0 (msrv toolchain)
cargo 1.85.0
bash 5.2
git 2.45
host: x86_64-unknown-linux-gnu
```

the command:

```bash
cargo soro-mutants check-platform --json
```

returns `status: "supported"`; the `rustc-msrv`, `cargo-version`,
`toolchain-pair`, `shell`, `git`, `operating-system`, and `architecture` checks
are all `pass`. If the same host has `rustc 1.84.1`, `rustc-msrv` becomes `fail`,
the overall status becomes `unsupported`, the command exits non-zero, and the
`remediation` says to install Rust 1.85.0 or newer.

## Release-readiness checklist

- [ ] `docs/supported-platforms.md` policy version is linked from the release notes.
- [ ] `cargo soro-mutants check-platform --json` reports `supported` on the Linux x86_64 MSRV job.
- [ ] The same command reports `supported` on the current stable Linux x86_64 job.
- [ ] A best-effort host report (if checked) contains only warnings, not failures.
- [ ] `cargo fmt`, `cargo clippy`, `cargo test`, and `cargo package --locked` pass on a primary environment.
- [ ] Any v0.1.x incompatibility is called out in release notes; no silent default changes.
- [ ] External benchmark execution is confirmed absent from unit CI.

## Troubleshooting

| Symptom | Cause | Action |
| --- | --- | --- |
| `rustc-msrv` fails | `rustc` below 1.85.0 | `rustup toolchain install 1.85.0 && rustup default 1.85.0` |
| `cargo-version` fails | Cargo older than 1.85.0 | Install the matching Rust toolchain with `rustup` |
| `toolchain-pair` warns | `rustc` and `cargo` from different toolchains | Use `rustup default stable` or one pinned toolchain for both |
| `shell` fails | `bash` missing from `PATH` | Install bash or use a supported Linux/macOS/WSL2 host |
| `git` warns | git missing | Install git, or use `--local-only` for benchmarks |
| `architecture` fails | 32-bit or unusual host | Use a supported 64-bit x86_64/aarch64 host |
| `operating-system` fails | Unsupported host | Use Linux, macOS, or WSL2 |

For failures unrelated to the policy, see [`docs/troubleshooting.md`](troubleshooting.md).
