# Mutation Fixture Naming and Organization Convention

This document specifies the standardized layout, naming conventions, and structure for test fixtures in Soro Mutants.

## Principles

1. **Auditable Pairs**: Every semantic mutation operator should have paired weak and strong test suites demonstrating both survival (`SURVIVED`) and detection (`KILLED`).
2. **Deterministic Layout**: Fixture names follow predictable kebab-case patterns encoding their purpose and operator family.
3. **Workspace Isolation**: Fixtures must never pollute the root Cargo workspace or leak intermediate build artifacts.
4. **Minimal Dependencies**: Fixtures use lightweight, minimal Soroban contract declarations without heavy external dependencies.

## Directory Structure and Naming Rules

All fixtures reside directly under the `fixtures/` root directory.

### 1. Weak / Strong Test Pairs
Used to verify mutation discovery, survival on weak tests, and killing on strong tests.

- Format: `fixtures/{strength}-{family}-{variant}`
  - `{strength}`: `weak` or `strong`
  - `{family}`: Operator family (`auth`, `call`, `event`, `storage`, `token`, `ttl`)
  - `{variant}`: Specific operator aspect or sub-behavior (e.g. `args`, `admin`, `helper`, `actor`, `topic`, `value`, `allowance`, `amount`, `client`, `lifecycle`)

Examples:
- `fixtures/weak-auth` / `fixtures/strong-auth` (for AUTH-001, AUTH-002)
- `fixtures/weak-auth-args` / `fixtures/strong-auth-args` (for AUTH-003)
- `fixtures/weak-token-amount` / `fixtures/strong-token-amount` (for TOKEN-003)
- `fixtures/weak-ttl-lifecycle` / `fixtures/strong-ttl-lifecycle` (for TTL-002, TTL-003)

### 2. Equivalence / False-Positive Suppression Fixtures
Used to verify that semantic suppressions and equivalence recognizers avoid generating no-op or invalid mutants.

- `fixtures/equivalence-positive`: Contains syntax that represents an observational no-op (e.g., swapping identical sender/recipient); must produce 0 mutants.
- `fixtures/equivalence-negative`: Contains syntax where swap is semantically meaningful; must produce the expected candidate mutant.

### 3. Compile-Only Viability Fixtures
Used to verify that representative mutants pass the compile-only gate without emitting invalid Rust.

- Format: `fixtures/compile-only/{family}`
  - `fixtures/compile-only/auth`
  - `fixtures/compile-only/token`
  - `fixtures/compile-only/event`
  - `fixtures/compile-only/storage`

### 4. Workspace / CLI Topology Fixtures
Used to verify multi-package discovery and workspace scoping.

- Format: `fixtures/workspace-{variant}`
  - Example: `fixtures/workspace-multi-package`

## Fixture Project Structure

Each fixture directory should contain:

```
fixtures/weak-auth-example/
├── Cargo.toml
└── src/
    └── lib.rs
```

### `Cargo.toml` Guidelines
- Minimal package definition with `edition = "2021"`.
- Clean dependencies; avoid downloading non-standard or heavy dependencies during test runs.
- If the fixture is excluded from the root workspace, ensure it compiles independently.

### `src/lib.rs` Guidelines
- Contract logic should be minimal and focused strictly on the tested semantic pattern.
- Functions must declare explicit Soroban types (`Env`, `Address`, `Symbol`, `Val`, etc.) required by operator recognizers.
- In `weak-*` fixtures: `#[test]` assertions pass on the original code and also pass when the mutant is applied (`SURVIVED`).
- In `strong-*` fixtures: `#[test]` assertions pass on the original code and fail when the mutant is applied (`KILLED`).

## Auditing and CI Verification

Automated checks in `tests/fixture_conventions.rs` enforce:
- Naming conformity against regex `^(weak|strong|equivalence|compile-only|workspace)-.*`.
- Existence of valid `src/lib.rs` and `Cargo.toml` in contract fixtures.
- Symmetrical pairing of weak and strong fixtures.
