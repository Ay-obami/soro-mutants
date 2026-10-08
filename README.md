# Soro Mutants

[![CI](https://github.com/Ay-obami/soro-mutants/actions/workflows/ci.yml/badge.svg)](https://github.com/Ay-obami/soro-mutants/actions/workflows/ci.yml)

Soroban-aware semantic mutation testing for Stellar smart contracts.

Soro Mutants injects realistic Soroban faults into Rust contract source and runs the existing test suite against each mutant. The goal is not to find vulnerabilities in the current source. The goal is to answer a different question:

> Would your tests notice if a security-critical Soroban behavior were accidentally changed or removed?

## Why this exists

Generic Rust mutation tools are useful for language-level mutations such as arithmetic and boolean changes. Soroban contracts also rely on semantics that generic Rust tooling does not understand:

- which `Address` must authorize an operation;
- whether state TTL extension is part of the intended lifecycle;
- whether contract events are part of the observable API;
- whether value moves from the correct sender to the correct recipient.

Soro Mutants targets those semantics.

## Current operators

| Operator | Mutation |
| --- | --- |
| `AUTH-001` | Remove `require_auth()` |
| `AUTH-002` | Authenticate a different in-scope `Address` |
| `TTL-001` | Remove `extend_ttl()` |
| `EVENT-001` | Remove `env.events().publish(...)` |
| `TOKEN-001` | Swap sender and recipient on a direct three-argument `.transfer(...)` call |

The prototype intentionally excludes event helper calls such as `events().transfer(...)` from `TOKEN-001`.

## Install and CLI

From a local checkout:

```bash
cargo install --path .
```

The installed binary follows Cargo's subcommand convention, so it can be invoked as `cargo soro-mutants`.

List semantic mutants without executing them:

```bash
cargo soro-mutants list /path/to/project
```

Filter by operator, file, or function:

```bash
cargo soro-mutants list /path/to/project \
  --operator AUTH-001 \
  --file contracts/pool/src/contract.rs \
  --function propose_admin
```

Execute matching mutants:

```bash
cargo soro-mutants test /path/to/project \
  --file contracts/pool/src/contract.rs \
  --function propose_admin \
  --test-command 'cargo test -q -p phoenix-pool admin_change'
```

Machine-readable output is available on both commands:

```bash
cargo soro-mutants list /path/to/project --json
cargo soro-mutants test /path/to/project --json
```

In JSON mode, stdout is reserved for JSON so it can be piped directly into `jq` or CI tooling. Compiler/test diagnostics may still appear on stderr.

Results are classified as:

- `KILLED`: at least one configured test failed after the mutation.
- `SURVIVED`: the configured tests still passed.
- `UNVIABLE`: the mutated source did not compile.
- `TIMEOUT`: compile or test execution exceeded the configured timeout.

A surviving mutant is **not automatically a vulnerability**. It means the configured tests did not distinguish the mutant from the original program and needs human review.

## Proof-of-concept result

The controlled fixtures demonstrate the intended behavior:

- weak auth fixture: `AUTH-001` and `AUTH-002` both survive;
- strong auth fixture: both mutants are killed by an explicit authorization assertion.

Real-repository benchmarks now show both sides of the signal:

- **Phoenix Protocol** (`aa9bfc0`): admin-change auth/event mutants survive, while explicit TTL and token-balance tests kill their corresponding mutants.
- **Soroswap Core** (`6eade00`): the focused token auth test kills both removal of `from.require_auth()` and authentication of `to` instead.
- **Stellar `soroban-examples`** (`03d42aa`): the official `single_offer` tests kill removal of `seller.require_auth()` and all three token-direction reversals in `trade()`.
- **RWA Toolkit Stellar contracts** (`a92ad6a`): sender-auth mutants are killed, while transfer-event deletion mutants survive.

Broken baselines are excluded rather than scored. Blend's integration baseline is therefore not counted as benchmark evidence in the current environment.

See [docs/benchmark.md](docs/benchmark.md) for exact commands, commit hashes, exclusions, and interpretation.

## Design principles

1. **Semantic over generic.** Do not duplicate arithmetic/comparison mutations already handled well by generic Rust mutation tools.
2. **Low false-positive pressure.** Prefer narrow recognizers and compile viability checks over broad regex mutation.
3. **Preserve source.** Parse with `syn`, but patch the original text so line numbers, formatting, and comments stay meaningful.
4. **Do not call survivors vulnerabilities.** Mutation testing evaluates test strength, not production exploitability.
5. **Respect the target repo.** Use its own Rust toolchain and configured test command.

## Project docs

- [Architecture](docs/architecture.md)
- [Operator specification](docs/operators.md)
- [Benchmark evidence](docs/benchmark.md)
- [Roadmap](docs/roadmap.md)
- [Drips / Stellar Wave maintainer brief](docs/drips-application.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)

## Status

`0.1.0` is a pre-release implementation. The core hypothesis has been validated against controlled fixtures and multiple public Soroban codebases; the current focus is keeping the five initial semantic operators narrow, reproducible, and low-noise before expanding the operator set.
