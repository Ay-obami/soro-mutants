# Mutation corpus manifest (schema version 1)

Soro Mutants uses a portable JSON manifest to catalog generalized Soroban bug patterns and the mutations expected to expose them. A manifest is descriptive: validating it does **not** execute mutants or download external repositories.

- [JSON Schema](corpus-manifest.schema.json)
- [Two-entry example](corpus-example.json)
- [Generalized Soroban Bug Corpus](soroban-bug-corpus.json)

Validate a manifest:

```bash
cargo soro-mutants validate-corpus docs/corpus-example.json
cargo soro-mutants validate-corpus docs/soroban-bug-corpus.json
```

The command reports the number of entries on success. Malformed JSON, missing fields, unsupported versions, and semantic validation failures produce exit code 1 with a field-specific error.

## Required fields

| Field | Meaning |
| --- | --- |
| `schema_version` | Integer major version; currently **1** |
| `entries` | Nonempty list of unique corpus entries |
| `entries[].id` | Stable unique, nonempty corpus identifier |
| `entries[].operator` | Operator ID in `FAMILY-000` format, including planned operators |
| `entries[].source_pattern.language` | `rust` for Soroban source |
| `entries[].source_pattern.code` | Representative nonempty source pattern |
| `entries[].mutation.original` | Original nonempty source snippet |
| `entries[].mutation.replacement` | Nonempty injected snippet, different from original |
| `entries[].expectations.weak` | Expected mutant outcome under a weak test suite |
| `entries[].expectations.strong` | Expected mutant outcome under a strong test suite |
| `entries[].provenance.kind` | Source classification, such as `repository-fixture` or `operator-specification` |
| `entries[].provenance.reference` | Traceable local path, repository link, or paper reference |

Accepted expected outcomes are `KILLED`, `SURVIVED`, `UNVIABLE`, and `TIMEOUT`. These are expectations rather than test-run results. The manifest validator deliberately does not require an operator to be enabled in the current CLI because the corpus can describe planned mutations.

## Compatibility policy

- Version **1** readers must recognize all required fields and their documented meanings.
- Producers may add optional fields (such as provenance notes or tags) without changing the major version. Readers ignore unknown fields; the JSON Schema permits them.
- Required-field removal, renaming, type changes, or incompatible interpretation changes require a **new major schema_version** and a corresponding validator update.
- A reader must reject unsupported major versions instead of silently interpreting them as version 1.
- IDs should be persistent across corpus revisions. Duplicate IDs in one document are invalid.
- Entries and their order are authored data; the validation command does not reorder, normalize, or rewrite them.

Additional semantic checks by the CLI enforce nonblank values, a three-digit operator suffix, unique IDs, supported outcome labels, and genuinely changed source snippets. The JSON Schema covers shape and basic field constraints; use `validate-corpus` for the complete validation policy.

## Review guidelines for future corpus contributions

All additions to the Soroban semantic bug corpus must adhere to the following standards:

1. **Proven Semantic Bug Classes**: The bug pattern must represent a genuine Soroban smart contract fault class (e.g. authorization bypass, storage lifetime expiration, inverted token transfer, wrong storage type, or dependency substitution).
2. **Public Provenance**: Every entry must include a verifiable public provenance reference link (such as official Stellar documentation, SEPs, public GitHub issues, security advisories, or audited incident postmortems).
3. **Minimal, License-Safe Examples**: Source code patterns must be concise, standalone, illustrative snippets authored specifically for the corpus. Do not copy proprietary or third-party code.
4. **Outcome Classifications**: Entries must specify realistic expected outcomes for weak test suites (`SURVIVED`) versus strong semantic test assertions (`KILLED`).
5. **Schema and Semantic Validation**: Contributions must pass validation by `cargo soro-mutants validate-corpus <path>` before inclusion.
6. **No Unsupported Claims**: Do not speculate or make unverified claims regarding historical security incidents; rely strictly on documented facts in the cited references.

## Future integration

The format is intended for future benchmark-matrix automation and a curated Soroban fault corpus. It does not automatically execute the benchmarks, assert that examples compile, or import external source repositories.
