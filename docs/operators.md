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

## AUTH-004 — helper-aware authorization

`AUTH-001`/`AUTH-002` only see authorization calls written directly in the current function. Real Soroban contracts often delegate authorization to a small local helper, so the entry point that *needs* authorization contains no `require_auth()` of its own. `AUTH-004` follows exactly one documented helper shape from the call site to its `require_auth()` sink. It is not a general interprocedural analyzer.

### Supported helper shape

The helper must be a free function declared at the **top level of the same source file** and must be:

- non-generic, non-`async`, non-`const`, non-`unsafe`, not `extern`, not variadic, and return `()`;
- a body made **only** of one or more `param.require_auth();` statements, where each `param` is a simple parameter explicitly typed `Address` or `&Address`.

```rust
fn require_owner(owner: &Address) {
    owner.require_auth();
}
```

Helpers with any other statement (storage writes, conditionals, `let` bindings, stored-admin lookups, `require_auth_for_args`, return values) are ignored, so a mutation at the call site changes authorization and nothing else.

### Supported call site

A standalone statement `helper(args...);` that:

- uses a plain, single-segment path (`require_owner(...)`), with the same number of arguments as the helper;
- appears in a function with explicit Soroban context (an `Env` / `&Env` or `Address` / `&Address` parameter);
- is in a function declared at the top level of the file or in a top-level `impl` block (not inside a nested `mod`);
- is not shadowed by a local binding or parameter of the same name.

### Mutations

The mutants are generated at the call site, because that is where the authorization sink is reached for *this* entry point; mutating the sink itself would change every caller at once (and `AUTH-001` already removes the sink directly). Descriptions name the sink they reach.

1. **Skip the helper authorization** — the call is replaced with `()`:

   ```rust
   require_owner(&from);   // becomes: ();
   ```

2. **Authenticate another address through the helper** — for each authorized argument that is an explicit `Address` parameter of the caller (written `&param`, `param`, or `param.clone()`), it is replaced with each other explicit `Address` parameter in signature order. A `&Address` helper parameter receives `&alternate`; an `Address` helper parameter receives `alternate.clone()`:

   ```rust
   require_owner(&from);   // becomes: require_owner(&to);
   ```

   Arguments that are not explicit caller `Address` parameters (locals, fields, computed values) are never redirected; only the skip mutant is generated for them.

### Out of scope / limitations

- crossing crate or module boundaries: helpers must be defined in the same file, and qualified paths (`Self::helper`, `crate::helper`, `module::helper`) are ignored;
- method calls, trait methods, closures, function pointers, and any other form of dynamic dispatch;
- helpers that call other helpers (only one hop is followed), macros, and macro expansion;
- helper calls embedded in larger expressions rather than written as statements;
- whole-program dataflow analysis.

A survivor means the configured tests did not distinguish removing or redirecting the authorization performed through the helper. As with `AUTH-001`/`AUTH-002`, tests that assert `env.auths()` for the expected `AuthorizedInvocation` should kill both mutants, while functional-only tests with `mock_all_auths()` may let them survive. A survivor is not by itself a vulnerability claim. Equivalent behavior is possible when the same address is authorized elsewhere in the invocation, or when tests pass the same address for both parameters.
## AUTH-005 — substitute a stored admin/owner authority

A contract can authenticate an address and still authenticate the **wrong privileged actor**. `AUTH-005` targets the authority value a function loads from contract state, rather than the authorization call itself.

### Supported retrieval pattern

All of the following must hold, inside one function:

1. The function has an explicit `Env` / `&Env` parameter and at least one explicit `Address` / `&Address` parameter.
2. A local is initialized **directly** from a storage lookup rooted at that `Env` parameter, unwrapped with `.unwrap()` or `.expect(..)`:

   ```rust
   env.storage().instance().get(&key).unwrap()
   env.storage().persistent().get(&key).expect("...")
   env.storage().temporary().get(&key).unwrap()
   ```

3. The local is explicitly typed `Address`, either by annotation (`let admin: Address = ...`) or by the lookup turbofish (`.get::<_, Address>(&key)`).
4. The local is the receiver of `require_auth()` or `require_auth_for_args(..)` in the same function. **This use as an authorization sink is what marks it as a privileged authority** — names such as `admin` or `owner` are never used as evidence.
5. The local name is bound exactly once in the function and does not shadow a parameter.

For each explicit `Address` parameter (in signature order), the loaded value is replaced with `<parameter>.clone()`:

```rust
pub fn set_admin(env: Env, new_admin: Address) {
    let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
    admin.require_auth();
    env.storage().instance().set(&DataKey::Admin, &new_admin);
}
```

becomes:

```rust
    let admin: Address = new_admin.clone();
```

Because the substitution happens at the load, every later use of the authority (the authorization sink, events, comparisons) observes the wrong actor. `.clone()` keeps the mutant type-compatible for both `Address` and `&Address` parameters without moving the parameter.

### Not treated as privileged authorities

- storage lookups whose result is never used as a `require_auth*` receiver (for example a stored `recipient` or treasury address);
- non-`Address` state (`i128`, `u32`, `Option<Address>` without unwrapping, and so on);
- `unwrap_or(..)`, `unwrap_or_else(..)`, `?`, `let ... else`, and other fallback forms;
- untyped locals, where the value type is not explicit;
- lookups not rooted directly at an explicit `Env` parameter: storage handles held in locals, helper functions such as `load_owner(&env)`, struct fields, or arbitrary `storage()` methods on non-`Env` receivers;
- shadowed or re-bound names, and authorization sinks in nested functions;
- functions without an alternate explicit `Address` parameter.

Role inference from names and whole-program authorization analysis are out of scope.

### Equivalent-mutant risks

A survivor means the tests did not distinguish the stored authority from another in-scope address. It is not a vulnerability claim, and some survivors are equivalent:

- a test passes the stored admin itself as the alternate parameter, so both values are equal;
- the authority is only used for the authorization sink and tests mock all authorizations without checking `env.auths()` (the weak fixture demonstrates this);
- `AUTH-002` may already substitute the `require_auth()` receiver at the same sink. When the stored local has no other use, the `AUTH-002` and `AUTH-005` mutants for that sink can be behaviorally equivalent to each other, although they edit different source spans.

Tests that assert the exact authorizing address — for example by comparing `env.auths()` with the expected `AuthorizedInvocation`, or by mocking authorization only for the stored admin — should kill it.

## STORAGE-001 — substitute a direct Soroban storage class

For a direct accessor rooted at an explicit `Env` parameter, `env.storage().persistent()`, `env.storage().temporary()`, or `env.storage().instance()` is replaced with each other storage class in deterministic `instance`, `persistent`, `temporary` order. The operation performed on the accessor (`set`, `get`, `remove`, `extend_ttl`, and so on) is left unchanged.

The guard intentionally excludes locals, aliases, helper-returned storage handles, and unrelated methods with the same names. Some substitutions are not type-compatible with the downstream operation; those mutants are intentionally generated and the normal compile-viability phase classifies them as `UNVIABLE` rather than treating them as test failures.

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

## CALL-002 — generated contract-client argument substitution

Recognizes a local generated Soroban contract client initialized directly as `SomeContractClient::new(&env, &dependency)`. On later method calls through that simple local variable, a referenced `Address` or primitive integer function parameter may be replaced with another explicit parameter of the same supported category. The client receiver itself is not changed.

The recognizer intentionally rejects token clients, helper-returned clients, struct fields, aliases, computed arguments, cloned expressions, and unknown argument types. Candidate ordering follows the enclosing function signature, so output is deterministic. This is not general Rust argument permutation or whole-program type inference.

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

## TOKEN-002 — swap allowance owner and spender

Token allowances separate *who owns the value* from *who may move it*. Confusing the two roles is still valid Rust, but it changes who controls value movement. `TOKEN-002` swaps the two leading `Address` role arguments of the following Soroban token-interface calls, and **only** these calls, matched by exact method name and argument count:

| Call | Swapped arguments |
| --- | --- |
| `approve(&from, &spender, &amount, &expiration_ledger)` | owner `from` ↔ `spender` |
| `allowance(&from, &spender)` | owner `from` ↔ `spender` |
| `transfer_from(&spender, &from, &to, &amount)` | `spender` ↔ owner `from` |
| `burn_from(&spender, &from, &amount)` | `spender` ↔ owner `from` |

Example:

```rust
fn grant(env: Env, token_id: Address, owner: Address, spender: Address, amount: i128, expiration_ledger: u32) {
    let client = token::Client::new(&env, &token_id);
    client.approve(&owner, &spender, &amount, &expiration_ledger);
}
```

becomes:

```rust
    client.approve(&spender, &owner, &amount, &expiration_ledger);
```

The receiver must be a recognized Soroban token client, using exactly the same recognizer as `TOKEN-001`: an explicit `TokenClient` / `token::Client` parameter, a local variable initialized directly from a token-client constructor, or a direct constructor chain. Both role arguments must be `&ident` references to **distinct** function parameters explicitly typed `Address` (or `&Address`). The edit covers only the text from the first role argument to the second; the separator (including comments), the receiver, and all later arguments (amount, recipient, expiration ledger) are preserved byte-for-byte.

Compile viability: because both swapped values are references to explicitly typed `Address` parameters, the swap is type-preserving for every supported call. Any mutant that still fails to compile is classified `UNVIABLE` by the normal compile gate.

False-positive boundaries — the following are intentionally ignored:

- same-named methods on receivers that are not recognized token clients (for example `registry.approve(...)`);
- event helpers such as `TokenUtils::new(&env).events().approve(...)` and `env.events().publish(...)`;
- calls with a different argument count than the supported signatures above;
- role arguments that are locals, aliases, clones, field accesses, or other computed expressions (no dataflow or type inference is attempted);
- identical role arguments (`approve(&owner, &owner, ...)`), where a swap would be a no-op;
- `transfer` and every other token method (`transfer` direction is `TOKEN-001`).

This is not generic method-argument permutation: only the owner/spender roles of the calls listed above are swapped.

A survivor means the configured tests did not distinguish *which* address owns the allowance from *which* address may spend it — for example, a test that only checks a function's return value. Tests that assert `allowance(&owner, &spender)` state, or that the spender can actually `transfer_from` the owner's balance, should kill it. A survivor is not by itself a vulnerability claim. Equivalent behavior is possible, for example when a test passes the same address as owner and spender, or when the mutated allowance is never read or spent.

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

## TOKEN-004 — substitute a recognized token client

For a recognized Soroban token-client `transfer(&from, &to, &amount)` whose receiver is a simple in-scope token client variable, this operator replaces that receiver with another recognized token client variable. Candidates are emitted in deterministic discovery order: typed token-client parameters first, then local variables initialized directly from recognized token-client constructors.

Direct constructor chains, arbitrary method receivers, helper-returned clients, struct fields, and unrecognized client types are intentionally ignored. The operator changes only which token client receives the transfer call; sender, recipient, and amount are preserved.

## TTL-002 / TTL-003 — lifecycle renewal policies

For recognized Soroban `instance().extend_ttl(threshold, target)` and keyed `persistent()/temporary().extend_ttl(&key, threshold, target)` calls, the following operators leave the call itself intact:

- **TTL-002:** change only the renewal threshold to `0` (renewal ordinarily won't occur while the entry is still live).
- **TTL-003:** change only the extension target to the original threshold (shortens the requested post-renewal lifetime).

Both substitutions are `u32`-compatible and preserve keys, source spans and deterministic IDs. Identical replacements are skipped. These are lifecycle-policy checks, not generic arithmetic mutations. Tests should control ledger minimum TTL and sequence and assert remaining TTL, not just a function's return value. Depending on the host's initial TTL and the chosen parameters, either candidate may be observationally equivalent; Soro Mutants does not claim to prove equivalence. Compile-invalid mutations are classified `UNVIABLE` by the standard separate compile gate.

## CALL-003 — bypass a generated contract-client return mismatch check

The initial supported shape is deliberately narrow: a local generated Soroban client created directly as `SomeContractClient::new(&env, &dependency)` is called inside the condition `if client.method(...) != expected { ... }`. `CALL-003` replaces that mismatch comparison with `false`, preserving the surrounding control flow while testing whether the suite enforces the rejected-return path.

Ordinary local function returns, arbitrary method receivers, equality checks, helper-returned clients, aliases, and more complex boolean expressions are out of scope. A surviving mutant does not prove the external call is wrong; equivalent behavior is possible when tests never exercise a mismatching return value or the branch has no observable effect.

## CALL-001 — direct cross-contract dependency address substitution

For a direct Soroban env.invoke_contract call where env is an explicit Env parameter and the first argument is a reference to an explicit Address parameter, substitute another explicitly typed Address parameter in the same function. Only the first argument changes; method, return type, function name and other arguments remain intact. Output identifies both original and substituted target names.

This scope intentionally excludes dynamic expressions, address locals, struct fields, contract-client helpers, non-Env receivers and arbitrary same-named methods. Dependencies that return identical values can let the mutant survive despite incorrect routing. Strong tests should assert which dependency's state changed rather than checking the return value alone.
