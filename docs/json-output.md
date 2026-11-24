# JSON output compatibility

Both `list --json` and `test --json` emit an object with the integer
`schema_version`, currently `1`. This version describes the JSON contract and is
independent of the package version. Text output is unchanged.

`list` returns `{"schema_version": 1, "mutants": [...]}` and, when any of `--operator`, `--file`, `--function`, or `--shard` is active, an additive `metadata` object containing the selected filters. Each mutant retains
`id`, `operator`, `file`, `function` (a string or null), `span` (`line`, `column`,
`end_line`, `end_column`), `original`, `replacement`, and `description`.
Internal byte offsets are not serialized.

`test` returns `{"schema_version": 1, "results": [...], "metadata": {...}}`. Each result contains
`mutant` with those same fields and `outcome`, one of `KILLED`, `SURVIVED`,
`UNVIABLE`, or `TIMEOUT`. See the [illustrative legacy report](json-report-example.json).
The additional `metadata` object records `rustc_version` and `cargo_version` (strings,
or null when unavailable), `test_command` (the configured command after environment-secret redaction),
`compile_timeout_seconds`, and `test_timeout_seconds` (the effective budgets after
legacy `--timeout` fallback), plus `operator_filter`, `file_filter`, `function_filter`,
`shard_index`, and `shard_total` (null when the corresponding selector is unset). This makes
selection settings observable when reproducing a run. The metadata is also present for successful runs
with no matching mutants. Empty successful reports retain an empty results array.
Baseline failures still fail the command instead of emitting a successful report.

Privacy: reports intentionally exclude environment-variable dumps, host names,
user names, absolute working directories, and Cargo target-cache paths. The
configured `test_command` is included verbatim for reproducibility, so avoid
embedding passwords or tokens in that command and review reports before sharing
them publicly. Toolchain versions are queried via `rustc --version` and
`cargo --version`; failures to query either tool do not fail a mutation run.

## Compatibility policy

Version 1 introduces an envelope around the previous unversioned arrays. Existing
consumers must read `.mutants` for `list` and `.results` for `test` instead of
iterating the root array. Historical arrays have no schema version; no automatic
migration is provided.

Increment `schema_version` for incompatible changes, including removal or renaming
of fields, changes to field types or meanings, changes to the envelope, and changes
to the set or meaning of outcome labels. Adding optional fields may keep the same
version: consumers must ignore unknown fields and check the version before reading
a report. JSON whitespace and object key ordering are not part of the contract.
New operator IDs may be added without a schema bump; existing IDs must never be
repurposed. Report values and the number of discovered mutants can change as
recognizers improve without changing the schema.

## Field nullability, optionality, and zero-value semantics

To enable robust client-side deserialization, the following field guarantees apply to schema version 1:

| Field | Location | Presence | Type | Semantics & Nullability |
| --- | --- | --- | --- | --- |
| `schema_version` | Root | Required | Integer | Always positive integer (`1`). Never `null` or omitted. |
| `mutants` | `list --json` root | Required | Array | Array of mutant objects. Present and empty `[]` when no mutants are discovered; never `null` or omitted. |
| `results` | `test --json` root | Required | Array | Array of test result objects. Present and empty `[]` when no mutants are tested; never `null` or omitted. |
| `metadata` | `test --json` root | Required | Object | Present for all successful test runs, including zero-mutant runs. |
| `metadata.rustc_version` | `metadata` | Present | String or `null` | Non-empty string when `rustc --version` succeeds; `null` if the binary is missing or query fails. Never omitted. |
| `metadata.cargo_version` | `metadata` | Present | String or `null` | Non-empty string when `cargo --version` succeeds; `null` if the binary is missing or query fails. Never omitted. |
| `metadata.test_command` | `metadata` | Required | String | Configured test command after environment-secret redaction. Never `null`. |
| `metadata.compile_timeout_seconds` | `metadata` | Required | Integer | Non-negative effective compilation timeout in seconds. Never `null`. |
| `metadata.test_timeout_seconds` | `metadata` | Required | Integer | Non-negative effective test execution timeout in seconds. Never `null`. |
| `metadata.operator_filter` | `metadata` | Required for test; conditional for list | String or `null` | Active `--operator` filter, or `null` when unset. |
| `metadata.file_filter` | `metadata` | Required for test; conditional for list | String or `null` | Active `--file` suffix filter, or `null` when unset. |
| `metadata.function_filter` | `metadata` | Required for test; conditional for list | String or `null` | Active `--function` filter, or `null` when unset. |
| `metadata.shard_index` | `metadata` | Required for test; conditional for list | Integer or `null` | One-based active shard index, or `null` when `--shard` is unset. |
| `metadata.shard_total` | `metadata` | Required for test; conditional for list | Integer or `null` | Active shard total, or `null` when `--shard` is unset. |
| `suppression` | `test --json` root | Optional | Object | Omitted / absent when no mutants are suppressed. Contains `count` (integer) and `details` (array) when present. |
| `mutant.id` | Mutant object | Required | String | Non-empty deterministic identifier (e.g. `"M0001"`). Never `null`. |
| `mutant.operator` | Mutant object | Required | String | Non-empty operator code (e.g. `"AUTH-001"`). Never `null`. |
| `mutant.file` | Mutant object | Required | String | Project-relative file path using forward slashes. Never `null`. |
| `mutant.function` | Mutant object | Required | String or `null` | Name of enclosing function if discovered; `null` if the mutant is at module or const scope. |
| `mutant.span.line` | Mutant object | Required | Integer | 1-based start line. Never `null`. |
| `mutant.span.column` | Mutant object | Required | Integer | 1-based start column. Never `null`. |
| `mutant.original` | Mutant object | Required | String | Original source code snippet replaced. Never `null`. |
| `mutant.replacement` | Mutant object | Required | String | Mutant replacement code snippet. Never `null`. |
| `mutant.description` | Mutant object | Required | String | Human-readable explanation of the mutation. Never `null`. |
| `outcome` | Result object | Required | String | Enum string: `"KILLED"`, `"SURVIVED"`, `"UNVIABLE"`, or `"TIMEOUT"`. Never `null`. |

### Absence vs. Null vs. Empty Array vs. Zero
- **Absent / Omitted**: Used exclusively for optional features that were not triggered (such as `suppression` when zero mutants were suppressed).
- **`null`**: Used only when a property is semantically applicable but unresolvable (e.g., `function: null` for mutations outside function bodies, or toolchain versions when querying fails).
- **Empty Array `[]`**: Used when a collection has zero elements (e.g., `mutants: []` or `results: []`). Collections are never serialized as `null`.
- **Zero `0`**: Used for numeric quantities where zero is meaningful (for example, zero result counts). Timeout CLI limits must be positive and reject zero before execution. Numeric report fields are never serialized as `null`.

### Concrete Example: Zero-Mutant Test Report

When a test run matches no mutants, an empty results array is emitted with complete execution metadata:

```json
{
  "schema_version": 1,
  "results": [],
  "metadata": {
    "rustc_version": "rustc 1.85.0 (4d91de4e4 2025-02-17)",
    "cargo_version": "cargo 1.85.0 (d49bee01b 2025-02-14)",
    "test_command": "cargo test -q",
    "compile_timeout_seconds": 60,
    "test_timeout_seconds": 60
  }
}
```

## Timing fields

When `test --timing` is set, the test report adds optional timing data without changing `schema_version: 1`:

- `metadata.timing.discovery_ms`, `baseline_ms`, and `mutant_wall_ms` are wall-clock durations in milliseconds.
- `metadata.timing.compile_ms` and `test_ms` are sums across mutants that actually executed those phases. With `--jobs > 1`, either sum can be larger than `mutant_wall_ms`.
- Each executed result adds `timing.total_ms`, plus `compile_ms` and `test_ms` when those phases ran.
- Resumed and persistent-cache results have no per-mutant `timing` because no command ran in the current process.

Without `--timing`, both timing objects are omitted, preserving the existing JSON shape for consumers that require byte-stable default output. Timing values themselves are intentionally nondeterministic wall-clock observations; field presence and types are the stable contract.
