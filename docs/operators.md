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

Recognizes `extend_ttl(...)` only when it is called on a Soroban `instance`, `persistent`, or `temporary` storage accessor, including simple locals and explicitly typed locals initialized directly from those accessors, and replaces the call expression with `()`.

Local recognition is deliberately one hop: aliases of an already recognized local are ignored rather than guessed through dataflow. This keeps TTL discovery conservative and prevents unrelated same-named methods from being pulled in through ambiguous aliases.

Arbitrary methods named `extend_ttl` are ignored. The operator asks whether tests actually enforce the expected state-lifetime behavior.

## EVENT-001 — remove direct event publication

Recognizes direct Soroban event publication through `.events().publish(...)` when `.events()` is called on an explicit `Env` / `&Env` function parameter, and replaces the publication with `()`.

Generic objects that happen to expose `.events().publish(...)` are ignored. It does not yet cover every typed event abstraction or event accessors stored in locals/fields.

## EVENT-002 — suppress direct typed event publication

Matches a standalone statement in a function taking an explicitly typed Soroban `Env` (or `&Env`):

```rust
#[contractevent]
pub struct TransferNotice { /* typed fields */ }

// Inside a function with env: Env:
TransferNotice { recipient, amount }.publish(&env);
```

The struct must carry `#[contractevent]` in the same source file. The AST recognizer replaces the **publish expression only** with `()`, preserving the terminating semicolon and surrounding formatting. The mutation compiles while omitting the event, allowing tests that assert `env.events().all()` to kill it. A test observing only the function's return value may let it survive.

The initial supported shape is a direct `EventStruct { ... }.publish(&env);` statement. It intentionally excludes variables holding events, chained event constructors, event types declared elsewhere, publish calls embedded in other expressions, dynamic environment references, non-`Env` arguments, and arbitrary same-named methods. It does not inspect macro expansion, event schemas, or event listeners. Event helpers such as `events().transfer(...)` are never interpreted as token-client transfers.

The typed syntax above is supported by Soroban SDK 25.3.0's `#[contractevent]` macro.

## EVENT-003 — substitute an event actor Address

For a standalone typed `#[contractevent]` struct literal published directly with `.publish(&env)`, Soro Mutants can replace a simple `Address` field value with another explicit `Address` parameter from the same function. Shorthand fields are rewritten explicitly (for example, `actor` becomes `actor: alternate`) so the event schema itself is preserved.

The recognizer does not mutate arbitrary structs, helper-returned events, non-`Address` values, variables holding event instances, or values inferred through aliases. Survivors indicate that tests may assert event existence without checking the emitted actor.
## EVENT-004 — substitute a numeric event value

For a standalone typed `#[contractevent]` struct literal published directly with `.publish(&env)`, Soro Mutants replaces a simple primitive-integer function parameter used as an event field with `0`. Shorthand fields are expanded (for example, `amount` becomes `amount: 0`) so the event schema remains unchanged.

Supported numeric parameter types are Rust primitive signed and unsigned integers (`i8` through `i128`, `u8` through `u128`, plus `isize`/`usize`). Generic numbers, aliases, arithmetic expressions, and arbitrary structs are intentionally ignored. Zero is deterministic, type-inferred by the event field, and avoids introducing overflow or underflow. Survivors suggest tests may observe the event without enforcing its numeric payload.

## EVENT-005 — substitute a typed event topic Address

For a standalone \`#[contractevent]\` struct literal published directly with \`.publish(&env)\`, this operator changes a field explicitly marked \`#[topic]\` when that field is a simple \`Address\` parameter and another explicit \`Address\` parameter is available. Candidate addresses are ordered by their function-parameter order, making discovery deterministic.

The guard is intentionally strict: the struct must be declared with \`#[contractevent]\` in the same file, the field itself must carry \`#[topic]\`, publication must target an explicit \`Env\` parameter, and both original and replacement values must be explicit \`Address\` parameters. Arbitrary tuples, symbols, normal structs, data fields, aliases, and helper-generated events are ignored.

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

## TOKEN-003 — substitute the token transfer amount

For a recognized Soroban token-client `transfer(&from, &to, &amount)` call, this operator is emitted only when the third argument is a reference to a simple function parameter with a primitive integer type. It generates two deterministic replacements: `&0` and `&1`.

The narrow shape keeps mutations type-inferred by the token API and avoids arithmetic that could overflow or underflow. Computed expressions, aliases, generic number wrappers, and non-token `.transfer(...)` calls are ignored. A zero mutant asks whether tests reject no value movement; a one mutant asks whether they enforce the exact amount rather than merely some movement.

## TTL-002 / TTL-003 — lifecycle renewal policies

For recognized Soroban `instance().extend_ttl(threshold, target)` and keyed `persistent()/temporary().extend_ttl(&key, threshold, target)` calls, the following operators leave the call itself intact:

- **TTL-002:** change only the renewal threshold to `0` (renewal ordinarily won't occur while the entry is still live).
- **TTL-003:** change only the extension target to the original threshold (shortens the requested post-renewal lifetime).

Both substitutions are `u32`-compatible and preserve keys, source spans and deterministic IDs. Identical replacements are skipped. These are lifecycle-policy checks, not generic arithmetic mutations. Tests should control ledger minimum TTL and sequence and assert remaining TTL, not just a function's return value. Depending on the host's initial TTL and the chosen parameters, either candidate may be observationally equivalent; Soro Mutants does not claim to prove equivalence. Compile-invalid mutations are classified `UNVIABLE` by the standard separate compile gate.

## CALL-001 — direct cross-contract dependency address substitution

For a direct Soroban env.invoke_contract call where env is an explicit Env parameter and the first argument is a reference to an explicit Address parameter, substitute another explicitly typed Address parameter in the same function. Only the first argument changes; method, return type, function name and other arguments remain intact. Output identifies both original and substituted target names.

This scope intentionally excludes dynamic expressions, address locals, struct fields, contract-client helpers, non-Env receivers and arbitrary same-named methods. Dependencies that return identical values can let the mutant survive despite incorrect routing. Strong tests should assert which dependency's state changed rather than checking the return value alone.
