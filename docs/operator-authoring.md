# v1.0 semantic operator authoring guide

Use this guide for **built-in Soroban semantic operators**. The
[compile-time extension prototype](extensions.md) is separate and does not
register operators in the CLI. Read [the operator catalog](operators.md) and
[maintainer architecture](v1-maintainer-architecture.md) first.

## 1. Define a specific semantic mistake

Choose a stable ID in an existing family (`AUTH`, `CALL`, `EVENT`,
`TOKEN`, `TTL`, `STORAGE`) or propose a documented new family. A new
operator should model a meaningful Soroban contract mistake that tests could
detect, **not** arbitrary arithmetic or Rust syntax changes.

Describe a positive source pattern *and* its false-positive boundary before
editing code. Example for authorization:

```rust
// Positive: clearly a Soroban Address authorization call.
pub fn send(from: soroban_sdk::Address) {
    from.require_auth();
}
```

A local `logger.require_auth()` on an unrelated Rust type must **not**
be mutated simply because the method name matches. The actual recognizer
must check compatible receiver type/context, not copy this illustrative
snippet as an exhaustive implementation.

## 2. Recognize conservatively with syn

Built-in discovery is implemented in `src/main.rs` using `syn` AST
visitors. Follow an existing operator from `BUILTIN_OPERATORS` to the
matching visitor and candidate construction code.

- Locate the narrowly supported AST shape and capture enough contextual
  evidence (function parameters, recognized Soroban types or client).
- Skip ambiguous or unresolved calls rather than guessing.
- Choose an explicit replacement with a predictable, syntax-valid outcome
  where feasible; avoid semantic expansion of older operators.
- Preserve source byte offsets, comments and formatting. Do not rewrite the
  whole file based on pretty-printed AST.
- Keep operator IDs, mutation ordering and filtering deterministic.

The public `extension::v1::MutationCandidate` illustrates the byte-span
idea but is **not** the CLI's internal mutant model. For the prototype, see
[`examples/custom_operator.rs`](../examples/custom_operator.rs);
its boolean flip is educational and *not* a production Soroban operator.

## 3. Add positive, negative and failure coverage

A focused operator change normally needs all of:

1. A **positive** unit test and a representative weak fixture that discovers
   the intended mutant, plus a strong fixture whose tests detect the fault.
2. A **false-positive** unit test and a non-Soroban or ambiguous-call fixture
   proving the same method spelling is ignored.
3. An **equivalence** and boundary check (e.g. identical replacement,
   nested call, multiline or Unicode offsets) where relevant.
4. Verification that `list` and `test` select the same candidate set and
   that invalid source returns a controlled failure.

Existing tests under `tests/*_cli.rs`, negative fixtures, and
`.github/scripts/check-semantic-fixture.sh` show current conventions.
Do not change existing IDs or golden output fields without an explicit
compatibility discussion.

## 4. Document and wire it

Update `BUILTIN_OPERATORS`, the corresponding recognizer,
[`docs/operators.md`](operators.md), representative fixtures and tests.
Update README's current-operators section only when the new operator is
actually implemented. Keep the CLI help/operator list aligned.

A user should be able to run the following in the repository checkout:

```bash
cargo run -- operators
cargo run -- list fixtures/weak-auth --operator AUTH-001 --json
cargo run -- test fixtures/weak-auth --operator AUTH-001
```

The `AUTH-001` example uses an existing shipped operator; replace the ID
and fixture path with your new operator's verified names when documenting
a new feature.

## 5. Verify before a PR

```bash
cargo fmt --all -- --check
cargo test --all
cargo clippy --all-targets --all-features -- -D warnings
cargo package --locked
```

Review the semantic-fixture CI job for required weak/strong coverage.
Check empty-selection JSON as well as a populated `list` result. Explain
any `UNVIABLE` outcome; do not count it as killed.

## Common pitfalls and limitations

- A method name is not evidence of Soroban semantics.
- Compile-invalid replacement candidates should remain `UNVIABLE`.
- Non-deterministic traversal or filesystem iteration changes public mutant
  ordering and breaks reproducibility.
- A passing test baseline is mandatory before classifying mutants.
- A surviving mutant is evidence about test sensitivity, not a security finding.
- The current CLI has no dynamic operator loading: modifying only
  `src/lib.rs` extension examples will not expose a new built-in operator.
