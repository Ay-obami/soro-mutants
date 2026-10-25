# Mutation corpus manifest (schema version 1)

Soro Mutants uses a portable JSON manifest to catalog generalized Soroban bug patterns and the mutations expected to expose them. A manifest is descriptive: validating it does **not** execute mutants or download external repositories.

- [JSON Schema](corpus-manifest.schema.json)
- [Two-entry example](corpus-example.json)

Validate a manifest:

```bash
cargo soro-mutants validate-corpus docs/corpus-example.json
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

## Future integration

The format is intended for future benchmark-matrix automation and a curated Soroban fault corpus. It does not automatically execute the benchmarks, assert that examples compile, or import external source repositories.
