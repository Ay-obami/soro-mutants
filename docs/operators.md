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

## AUTH-003 — change an explicit authorized argument set

Recognizes a direct `address.require_auth_for_args(args)` call only when `address` is a simple identifier explicitly typed `Address` (or `&Address`) in the enclosing function signature, and the function also has an explicit `Env` (or `&Env`) parameter. It replaces only the `args` expression with an empty Soroban argument vector; **the authorization call itself remains**.

For example:

```rust
fn authorize(env: Env, admin: Address, recipient: Address) {
    admin.require_auth_for_args((recipient.clone(),).into_val(&env));
}
```

becomes:

```rust
fn authorize(env: Env, admin: Address, recipient: Address) {
    admin.require_auth_for_args(soroban_sdk::Vec::new(&env));
}
```

The mutation asks whether the tests enforce the exact signed argument set. A functional-only test with mocked authorization may let it survive, whereas a test checking `env.auths()` for the expected `AuthorizedInvocation` should kill it.

This intentionally does **not** recognize unrelated same-named methods on arbitrary receivers, chained receiver expressions, calls outside a function with an explicit `Env` and `Address` parameter, or helper-mediated authorization. It does not attempt to prove the semantic type of arbitrary locals.

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

## Conservative equivalent-mutant suppression

Before numbering and executing mutants, Soro Mutants suppresses two **provably identical source-edit patterns**:

1. **Unchanged replacement:** the replacement string exactly matches the original source span. Applying the mutant would leave the source unchanged.
2. **Duplicate source edit:** another candidate already replaces the same byte span of the same file, with the same original and replacement strings. Executing both would produce identical source files.

This deliberately does **not** simplify algebra, compare runtime results, or treat edits in different source spans/files as equivalent. Even whitespace differences remain separate: comparing expressions through formatting normalization could hide meaningful changes to literals, macros, or generated code.

The normal `list` and `test` text output gives the total suppressed count and the count for each reason. JSON reports include a `suppression` object with `count` and per-candidate `details` when something was suppressed; the existing JSON shape remains unchanged when no candidates are suppressed. `--count-only` continues to print only the retained-mutant count.

**These conservative heuristics are not a proof of general program equivalence.** Many context-dependent equivalent mutants remain intentionally unsuppressed rather than risk hiding a meaningful test gap.
