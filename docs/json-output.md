# JSON output compatibility

Both `list --json` and `test --json` emit an object with the integer
`schema_version`, currently `1`. This version describes the JSON contract and is
independent of the package version. Text output is unchanged.

`list` returns `{"schema_version": 1, "mutants": [...]}`. Each mutant retains
`id`, `operator`, `file`, `function` (a string or null), `span` (`line`, `column`,
`end_line`, `end_column`), `original`, `replacement`, and `description`.
Internal byte offsets are not serialized.

`test` returns `{"schema_version": 1, "results": [...]}`. Each result contains
`mutant` with those same fields and `outcome`, one of `KILLED`, `SURVIVED`,
`UNVIABLE`, or `TIMEOUT`. See the [illustrative report](json-report-example.json).
Empty successful reports retain the envelope and an empty array. Baseline failures
still fail the command instead of emitting a successful report.

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
