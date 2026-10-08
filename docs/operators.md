# Operator Specification

This document defines the semantic contract of the v0.1 mutation operators.

## AUTH-001 — remove authorization

Recognizes a Rust method call named `require_auth` only inside a function with explicit Soroban context: at least one `Env` / `&Env` or `Address` / `&Address` parameter. The call expression is replaced with `()`.

This intentionally skips ambiguous `require_auth` methods in generic Rust code instead of assuming every same-named method is Soroban authorization.

Example:

```rust
from.require_auth();
```

becomes:

```rust
();
```

A survivor means the configured tests did not distinguish removal of that authorization call.

## AUTH-002 — authenticate another address

When a `require_auth()` receiver is a simple identifier and the enclosing function has another parameter explicitly typed `Address`, Soro Mutants substitutes that parameter as the authorization receiver.

Example:

```rust
fn transfer(env: Env, from: Address, to: Address) {
    from.require_auth();
}
```

may become:

```rust
fn transfer(env: Env, from: Address, to: Address) {
    to.require_auth();
}
```

This operator deliberately avoids broad type inference. Candidate replacement receivers must be explicit `Address` parameters.

## TTL-001 — remove TTL extension

Recognizes `extend_ttl(...)` only when it is called on a Soroban `instance`, `persistent`, or `temporary` storage accessor, including simple locals initialized from those accessors, and replaces the call expression with `()`.

Arbitrary methods named `extend_ttl` are ignored. The operator asks whether tests actually enforce the expected state-lifetime behavior.

## EVENT-001 — remove direct event publication

Recognizes direct Soroban event publication through `.events().publish(...)` when `.events()` is called on an explicit `Env` / `&Env` function parameter, and replaces the publication with `()`.

Generic objects that happen to expose `.events().publish(...)` are ignored. It does not yet cover every typed event abstraction or event accessors stored in locals/fields.

## TOKEN-001 — reverse direct transfer direction

Recognizes a direct three-argument method call named `transfer` only when the receiver is a known Soroban token client. v0.1 recognizes explicit `TokenClient` / `token::Client` parameters, local variables initialized from token-client constructors, and direct token-client constructor chains. It then swaps the first two arguments while preserving the third.

Example:

```rust
token.transfer(&from, &to, &amount)
```

becomes:

```rust
token.transfer(&to, &from, &amount)
```

The recognizer is deliberately conservative: arbitrary three-argument `.transfer(...)` methods and event helper transfers are ignored. This reduces false positives at the cost of not yet following token clients through struct fields, aliases, or helper-returned values.

## Classification

- `KILLED`: configured tests failed after applying the viable mutant.
- `SURVIVED`: configured tests still passed.
- `UNVIABLE`: the mutant failed the compile-only gate.
- `TIMEOUT`: compile or test execution exceeded the configured timeout.

A survivor is evidence about the configured tests, not by itself a vulnerability finding.
