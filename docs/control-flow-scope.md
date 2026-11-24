# Control-flow binding scope

Soro Mutants keeps semantic discovery conservative around Rust control-flow patterns.

- Calls inside `if let` and `while let` bodies are visited without leaking pattern bindings outside their lexical scope.
- For `for binding in addresses.iter()`, when `addresses` is an explicitly typed `Vec<Address>` function parameter and the loop pattern is a simple identifier, the loop-local binding is treated as an in-scope Address candidate only for the loop body. This allows authorization substitutions such as `admin.require_auth()` → `item.require_auth()` while preventing the loop-local name from leaking after the loop.
- Tuple and struct destructuring whose element types cannot be proven from the syntax are intentionally not guessed. Their method calls may be ignored by type-sensitive operators rather than creating false-positive substitutions.

The analysis does not infer arbitrary iterator item types, follow aliases through user-defined containers, or broaden Soroban type recognition beyond explicit supported shapes. This keeps discovery deterministic and avoids unsafe semantic guesses.

## Match arms

For a function parameter explicitly typed as `Option<Address>`, a simple `Some(binding)` match pattern introduces `binding` as an Address candidate only for that arm. The candidate is available to the arm guard and body, then discarded before the next arm. Nested matches follow the same lexical rule, so a binding from one arm is never proposed as an `AUTH-002` replacement in a sibling arm or after the match.

Soro Mutants does not guess types for arbitrary destructuring patterns. For example, `Option<u32>` bindings are never treated as Address candidates merely because they appear in a match.
