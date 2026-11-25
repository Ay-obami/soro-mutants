# Stable semantic discovery: overlap, Unicode, and local SDK re-exports

These guarantees apply to both `list` and `test` for built-in semantic
operators. They do not introduce higher-order mutants or change the meanings
of AUTH, EVENT, TTL, TOKEN, CALL, or STORAGE operators.

## Overlapping source ranges (#89)

A mutant replaces one contiguous source span per run. Candidate discovery is
sorted by project-relative file, starting byte offset, operator ID, replacement,
end offset, original source, function and description. Within a single
operator, the first recognized range wins against an intersecting range
with different boundaries. Subsequent contained or partially overlapping
ranges from that operator are suppressed with reason `overlapping source span`.
Ranges that only touch at a boundary do not overlap.

Mutants from **different semantic operator IDs** are also independent: for
example, TTL-001 removes an `extend_ttl()` call, while TTL-002 mutates its
threshold argument. Both remain discoverable despite overlapping spans,
preserving established operator behavior. Mutants with an **identical start
and end offset** but distinct replacements are likewise valid alternatives
because they execute in separate test runs.
Unchanged replacements are suppressed as `unchanged replacement`, and
identical file/range/source/replacement edits as `duplicate source edit`.

For example, a recognized local helper call
`authorize(&admin)` can produce one edit replacing the whole call and
another editing only `&admin`. When those ranges conflict, only the
deterministically preferred range for that operator survives. No conflicting
partial ranges from a single operator reach the normal mutant list. Suppression reasons use the existing summary
output and optional JSON `suppression.details` field; the report schema and
result statuses are unchanged. Mutant IDs are generated **after** suppression,
in stable `M0001`, `M0002`, ... order.

## UTF-8 safe source edits and positions (#90)

Mutation offsets are **UTF-8 byte offsets**, checked against valid character
boundaries before slicing. Bytes outside the selected span, including
whitespace, indentation, CRLF, and Unicode content, are unchanged.

For example:

```rust
// 🦀 Preserve this comment.
fn guard(user: Address) {
    let café = "日本語";
    user.require_auth();
}
```

An AUTH-001 replacement changes only `user.require_auth()` to `()`.
The comment, non-ASCII identifier, and multi-byte string are preserved exactly.
Source reports use 1-based **line and UTF-8 byte column** coordinates.
Columns count UTF-8 bytes, so columns after a non-ASCII character are larger
than their Unicode character index. Invalid spans (including those splitting
a multi-byte character) cannot be applied. Supported Rust mutations are
reparsed in regression tests.

Non-UTF-8 source and changes to output encoding are out of scope.

## Explicit local SDK re-exports (#98)

Only public, explicit `pub use` declarations directly naming these SDK
items are recognized inside the current parsed Rust file and inline modules:

- `soroban_sdk::Address`, including `as` renames.
- `soroban_sdk::Env`, including `as` renames.
- `soroban_sdk::token::Client`, including `as` renames.

Example:

```rust
pub use soroban_sdk::{Address as Account, Env as HostEnv};

fn authorize(_env: HostEnv, user: Account) {
    user.require_auth();
}
```

The explicit alias `Account` is treated as an SDK `Address` in the
existing AUTH recognizers. Qualified references into a local inline module
also work, for example
`mod sdk { pub use soroban_sdk::Address as Account; }` followed by
`crate::sdk::Account`.

The following are **not** inferred: `pub use soroban_sdk::*`,
`pub use third_party::Address as Account`, transitive imports through
other crates, `mod other_file;` contents, macro expansion, and arbitrary
`type` aliases. Module-scoped declarations do not leak into sibling modules.
Unsupported forms are ignored rather than guessed; a simple name that was
already supported by existing built-in recognizers is otherwise unchanged.

Run the regression tests with `cargo test --all`, and inspect the example
fixture in `fixtures/equivalence-semantic-edge-cases/src/lib.rs`.
