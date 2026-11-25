# Source preservation during mutation

Soro Mutants applies a mutant by replacing only the selected UTF-8 byte range. Bytes before and after that range are copied unchanged. This means leading and trailing comments, indentation, line endings (including CRLF), and unrelated formatting remain exactly as the target project wrote them.

For example, mutating `admin.require_auth()` in:

```rust
call(/* before */ admin.require_auth() /* after */); // tail
```

changes only the selected expression. Soro Mutants does not run `rustfmt` and does not reprint the surrounding syntax tree. Formatting inside the replacement text itself is operator-defined; whole-file formatting is intentionally out of scope.

All offset operations are checked against UTF-8 character boundaries.
Mutation locations are reported as 1-based line and 1-based **byte** column
positions (matching the parser's source spans), not Unicode code-point columns.
See [semantic discovery guarantees](semantic-discovery-quality.md).
