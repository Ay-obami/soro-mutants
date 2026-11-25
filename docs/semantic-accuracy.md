# Semantic recognition, lexical scopes and source locations

Soro Mutants v1 discovers **semantic mutants** only where the syntax supplies
supported Soroban type and receiver evidence. This intentionally favors
precision over trying to infer every Rust type.

## Avoiding false-positive wrapper mutants (#95)

A method's name alone is not proof of Soroban semantics. An application type
may implement `require_auth`, `transfer`, `publish` or `extend_ttl`.
Calling those methods on unsupported wrappers produces **zero semantic mutants**
for the corresponding Soroban operators, even if a genuine `Env` or `Address`
appears elsewhere in the same function.

For example:

```rust
struct Wrapper<T>(T);
impl<T> Wrapper<T> {
    fn require_auth(&self) {}
}
fn mixed(env: Env, owner: Address, wrapper: Wrapper<u32>) {
    wrapper.require_auth(); // no AUTH-001 or AUTH-002
    owner.require_auth();   // genuine AUTH-001, with Soroban Address evidence
}
```

Direct Soroban `env.events().publish(...)`, recognized token-client
`transfer(...)`, and `env.storage().instance().extend_ttl(...)` remain
discoverable. A coincidentally named `wrapper.storage().instance().extend_ttl`
does not qualify: the storage chain must originate at a recognized `Env`
binding or use a previously proven storage accessor.

These are **syntactic guarantees for supported shapes**, not arbitrary
cross-crate type resolution. User-defined types intentionally imitating the
supported SDK shape cannot be distinguished without whole-program inference.

## Visible Address candidates and shadowing (#96)

`AUTH-002` substitutions use the binding visible at each mutation site.
Candidate order is deterministic: function parameters in signature order,
followed by supported local bindings in lexical declaration order. A nested
`let`, `for` pattern, `match` arm or closure parameter hides an outer
binding of the same name, **even if the new binding is not an Address**.
Bindings introduced within a scope are discarded when that scope ends.

```rust
fn update(owner: Address, backup: Address) {
    owner.require_auth(); // backup can replace owner
    {
        let backup: u32 = 1;
        owner.require_auth(); // backup cannot replace owner here
    }
    owner.require_auth(); // backup is visible again
}
```

Supported `Option<Address>` match-arm and explicitly typed
`Vec<Address>` iterator bindings are included only inside their corresponding
arm or loop body. A typed local `let alternate: Address = ...` can be a
candidate after its initializer. Analysis neither performs general type
inference nor crosses function boundaries to borrow another function's
bindings. A closure's own parameter bindings never leak into the caller.

## Direct Soroban SDK import aliases (#97)

Simple direct renamed imports in the parsed file or inline module are
recognized, including grouped `use` declarations:

```rust
use soroban_sdk::{Address as Account, Env as Runtime};
use soroban_sdk::token::Client as AssetClient;
use soroban_sdk::storage::Instance as LocalInstance;

fn transfer(env: Runtime, from: Account, to: Account, client: AssetClient) {
    from.require_auth();                  // AUTH-001 / AUTH-002
    client.transfer(&from, &to, &1);      // TOKEN-001
    env.events().publish(("payment",), ()); // EVENT-001
}
```

The supported direct origins are `soroban_sdk::Address`,
`soroban_sdk::Env`, `soroban_sdk::token::{Client, TokenClient}`,
and `soroban_sdk::storage::{Instance, Persistent, Temporary}`. Alias
recognition also applies inside supported `Option<...>` and `Vec<...>`
function-parameter shapes. Regular unaliased and fully qualified SDK types
continue to work. An unrelated `use other::Address as Account` **does not**
establish Soroban Address evidence.

Wildcard imports, `super::*` re-exports, local `use` statements and
arbitrary cross-module or cross-crate alias chains are out of scope.

## Multiline original-source spans (#88)

Mutants retain their **original source** start and end byte offsets before
rewriting. Source spans use 1-based line and column numbers; columns correspond
to UTF-8 **byte** positions (the underlying parser's 0-based byte columns are
converted for display). Byte spans use half-open intervals `[start, end)`.
A multiline mutation beginning at line 2, column 5 and ending immediately
after a closing parenthesis on line 3, column 6 has a location
`2:5-3:6`.

```rust
    admin.require_auth(
    ); // trailing comment is outside the replacement
```

The `list --json` / `test --json` `span` object includes
`line`, `column`, `end_line`, and `end_column`. Text, Markdown, and
HTML report the full range for multiline targets. Markdown deep links use
`#L2-L3`; SARIF supplies `startLine`, `startColumn`, `endLine`, and
`endColumn` for multiline regions. Existing single-line presentation and
SARIF output remain compatible.

A mutation changes only the selected UTF-8 byte range. It does **not** reflow
the original source or shift the location reported for that mutant. The
original snippet excludes unrelated indentation, trailing comments, and
surrounding statements.

Use `cargo test --test semantic_accuracy_regressions` to run the
mixed-wrapper, scope-shadowing, SDK-alias, and multiline-format regressions.
