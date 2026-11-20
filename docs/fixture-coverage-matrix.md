# Fixture Coverage Matrix

This document tracks fixture test coverage for all 23 built-in semantic mutation operators in Soro Mutants.

## Matrix Overview

Coverage dimensions tracked:
- **Positive Fixture**: Demonstrates discovery of the intended mutation.
- **Negative / Suppression Fixture**: Demonstrates that non-Soroban or false-positive / observational no-op candidates are cleanly excluded.
- **Weak Test Fixture**: Demonstrates that the mutant survives (`SURVIVED`) under under-specified tests.
- **Strong Test Fixture**: Demonstrates that the mutant is killed (`KILLED`) under comprehensive tests.
- **Compile Viability Gate**: Validates that generated mutants compile cleanly or are properly gated as `UNVIABLE`.
- **CI Automated**: Verified automatically in GitHub Actions CI (`.github/workflows/ci.yml`).

## Operator Coverage Table

| Operator ID | Description | Positive Fixture | Negative / Suppression Fixture | Weak Fixture (`SURVIVED`) | Strong Fixture (`KILLED`) | Compile Viability | CI Matrix |
|-------------|-------------|------------------|--------------------------------|---------------------------|---------------------------|-------------------|-----------|
| **AUTH-001** | Remove a `require_auth()` call | `fixtures/weak-auth` | `fixtures/equivalence-positive` | `fixtures/weak-auth` | `fixtures/strong-auth` | Viable | Yes |
| **AUTH-002** | Replace `require_auth()` receiver with another Address parameter | `fixtures/weak-auth` | Non-Address receiver filtering | `fixtures/weak-auth` | `fixtures/strong-auth` | Viable | Yes |
| **AUTH-003** | Replace `require_auth_for_args(...)` argument set with empty Vec | `fixtures/weak-auth-args` | Non-Soroban args check | `fixtures/weak-auth-args` | `fixtures/strong-auth-args` | Viable | Yes |
| **AUTH-004** | Skip or redirect auth through local `require_auth()` helper | `fixtures/weak-auth-helper` | Complex/external helpers | `fixtures/weak-auth-helper` | `fixtures/strong-auth-helper` | Viable | Yes |
| **AUTH-005** | Replace stored admin/owner authority with another Address parameter | `fixtures/weak-auth-admin` | Non-admin storage keys | `fixtures/weak-auth-admin` | `fixtures/strong-auth-admin` | Viable | Yes |
| **AUTH-006** | Remove or redirect authorization in recognized initialization entrypoints | `fixtures/weak-init-auth` | Generic authorization outside initialization context | `fixtures/weak-init-auth` | `fixtures/strong-init-auth` | Viable | Yes |
| **AUTH-007** | Remove or redirect authorization in recognized contract-upgrade entrypoints | `fixtures/weak-upgrade-auth` | Generic authorization outside upgrade context | `fixtures/weak-upgrade-auth` | `fixtures/strong-upgrade-auth` | Viable | Yes |
| **CALL-001** | Replace `Env::invoke_contract` dependency address with another Address | `fixtures/weak-call-target` | Non-client target addresses | `fixtures/weak-call-target` | `fixtures/strong-call-target` | Viable | Yes |
| **CALL-002** | Replace contract-client call argument with another typed parameter | `fixtures/weak-call-args` | Non-matching argument types | `fixtures/weak-call-args` | `fixtures/strong-call-args` | Viable | Yes |
| **CALL-003** | Remove supported contract-client return mismatch check | `fixtures/weak-call-return` | Non-contract-client returns | `fixtures/weak-call-return` | `fixtures/strong-call-return` | Viable | Yes |
| **EVENT-001** | Remove direct `env.events().publish(...)` publication | `fixtures/weak-typed-events` | Non-Soroban publish methods | `fixtures/weak-typed-events` | `fixtures/strong-typed-events` | Viable | Yes |
| **EVENT-002** | Remove standalone `#[contractevent]` typed event publication | `fixtures/weak-typed-events` | Non-contractevent attributes | `fixtures/weak-typed-events` | `fixtures/strong-typed-events` | Viable | Yes |
| **EVENT-003** | Replace Address field in typed event with another Address | `fixtures/weak-event-actor` | Single-address event structs | `fixtures/weak-event-actor` | `fixtures/strong-event-actor` | Viable | Yes |
| **EVENT-004** | Replace numeric field in typed event with zero | `fixtures/weak-event-value` | Non-numeric event fields | `fixtures/weak-event-value` | `fixtures/strong-event-value` | Viable | Yes |
| **EVENT-005** | Replace typed event topic Address with another Address | `fixtures/weak-event-topic` | Non-topic struct fields | `fixtures/weak-event-topic` | `fixtures/strong-event-topic` | Viable | Yes |
| **STORAGE-001** | Replace direct storage class accessor with another storage class | `fixtures/weak-storage-class` | Non-storage instance calls | `fixtures/weak-storage-class` | `fixtures/strong-storage-class` | Viable | Yes |
| **TOKEN-001** | Swap sender and recipient of token transfer | `fixtures/equivalence-negative` | `fixtures/equivalence-positive` | `fixtures/weak-token-amount` | `fixtures/strong-token-amount` | Viable | Yes |
| **TOKEN-002** | Swap owner and spender Address of token allowance call | `fixtures/weak-token-allowance` | Identical owner/spender | `fixtures/weak-token-allowance` | `fixtures/strong-token-allowance` | Viable | Yes |
| **TOKEN-003** | Replace typed token transfer amount with zero or one | `fixtures/weak-token-amount` | Non-transfer amount calls | `fixtures/weak-token-amount` | `fixtures/strong-token-amount` | Viable | Yes |
| **TOKEN-004** | Replace token client receiver with another in-scope token client | `fixtures/weak-token-client` | Single-client scopes | `fixtures/weak-token-client` | `fixtures/strong-token-client` | Viable | Yes |
| **TTL-001** | Remove an `extend_ttl()` call on storage accessor | `fixtures/weak-ttl-lifecycle` | Generic non-Soroban methods | `fixtures/weak-ttl-lifecycle` | `fixtures/strong-ttl-lifecycle` | Viable | Yes |
| **TTL-002** | Renew storage TTL even when remaining lifetime exceeds threshold | `fixtures/weak-ttl-lifecycle` | Unrelated numeric bounds | `fixtures/weak-ttl-lifecycle` | `fixtures/strong-ttl-lifecycle` | Viable | Yes |
| **TTL-003** | Shorten storage TTL extension target to its threshold | `fixtures/weak-ttl-lifecycle` | Inverted threshold/target | `fixtures/weak-ttl-lifecycle` | `fixtures/strong-ttl-lifecycle` | Viable | Yes |

## Running Matrix Verification Locally

To verify that all operators are documented and their referenced fixtures exist:

```bash
cargo test --test fixture_coverage_matrix
```

## Adding a New Operator

When introducing a new operator to Soro Mutants:
1. Create a `weak-<family>-<name>` fixture with minimal tests where the mutant survives.
2. Create a paired `strong-<family>-<name>` fixture where the mutant is killed.
3. If the operator performs equivalence suppression, add cases to `fixtures/equivalence-positive` or negative.
4. Add a row to the table above.
5. Add the matrix configuration entry to `.github/workflows/ci.yml`.
