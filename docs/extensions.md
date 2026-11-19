# Custom semantic operator extension API (prototype)

Soro Mutants exposes a versioned compile-time prototype at `cargo_soro_mutants::extension::v1`.

An extension implements `SemanticOperator`, receives immutable `OperatorContext` plus the parsed `syn::File`, and returns `MutationCandidate` source edits. The boundary validates operator IDs, Rust parsing, byte ranges, and UTF-8 boundaries before returning candidates.

See `examples/custom_operator.rs` for a complete example.

## Failure isolation

Extensions never receive mutable access to the baseline project or runner outcome state. An extension error returns no candidates, and invalid ranges are rejected before any edit can be applied. Discovery failure therefore cannot be converted into `KILLED` or `SURVIVED` baseline evidence.

## Versioning

The prototype lives under `extension::v1`. Additive changes may be made within v1; breaking changes require a new versioned module. The core CLI's internal `Mutant` and `Outcome` representations remain private and are not part of this contract.

## Trust boundary

Custom operators are trusted local compile-time Rust code linked by the embedding program. This prototype does not download code, dynamically load libraries, execute remote plugins, or provide a plugin marketplace.
