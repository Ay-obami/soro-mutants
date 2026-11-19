# Soro Mutants v1.0 report consumer guide

Use the versioned JSON output for automated counts, dashboards, and score
gates. Choose Markdown or HTML for review and SARIF 2.1.0 for survivor
annotations in code-scanning systems. SARIF includes survivors only, so use the
JSON test report to calculate scores. See [SARIF reporting](sarif-report.md).

```bash
mkdir -p reports
cargo soro-mutants list . --json --output reports/mutants.json
cargo soro-mutants test . --json --output reports/results.json
```

## JSON contracts

The integer `schema_version` describes the report format independently of the
tool's release number. Check for version 1 before reading the corresponding
array. Successful empty runs still produce a versioned envelope with an empty
array.

| Report | Fields |
| --- | --- |
| List | `schema_version`, `mutants` |
| Test | `schema_version`, `results`; each result has `mutant` and `outcome` |
| Test run metadata | `metadata` contains nullable `rustc_version` and `cargo_version`, the configured `test_command`, and the effective `compile_timeout_seconds` and `test_timeout_seconds` |
| Suppression summary | Optional `suppression` with `count` and `details` when suppressions were recorded |

Each mutant includes `id`, `operator`, `file`, nullable `function`, `span`
(`line`, `column`, `end_line`, `end_column`), `original`, `replacement`,
and `description`. File paths are relative to the project argument. Resolve
them against that project root, which may differ from the CI checkout root or
the report's output directory. JSON line and column numbers start
at 1. Markdown source links assume the project root as their base.

Mutant IDs are deterministic for identical source and discovery settings. IDs
survive sharding, but can shift when the source or discovery filters change.
Store the revision, filters, and shard index alongside the report when
comparing runs.

## Reproduce the CLI score

Count `KILLED` and `SURVIVED` as viable. Exclude `UNVIABLE` and `TIMEOUT` from
the denominator. The score is `100 × killed / viable`, with 0% when there are
no viable results. For shards, combine counts from all successful reports
before dividing.

```bash
jq '
  if .schema_version != 1 or (.results | type) != "array"
  then error("expected a schema v1 test report")
  else
    [.results[].outcome] as $outcomes
    | ($outcomes | map(select(. == "KILLED")) | length) as $killed
    | ($outcomes | map(select(. == "KILLED" or . == "SURVIVED")) | length) as $viable
    | {killed: $killed, viable: $viable,
       score: (if $viable == 0 then 0 else 100 * $killed / $viable end)}
  end
' reports/results.json
```

Check the CLI exit status before ingestion. Exit 0 indicates successful
execution and exit 4 indicates a completed report below the requested threshold.
An operational, argument, or baseline failure can leave a missing, empty, or
incomplete output file. When piping output, enable your shell's pipeline
failure propagation or capture the command's status before parsing.

## Compatibility

Read named fields and tolerate unknown optional fields. Version 1 covers
field types and meanings, the envelope, and outcome labels. Breaking changes
require a new schema version. New operator IDs can appear within version 1.
Avoid relying on object key order or whitespace. Older unversioned reports
used root arrays and need an explicit legacy reader. See the
[JSON compatibility policy](json-output.md) and [example report](json-report-example.json).

Reports include original and replacement source snippets, and test metadata
contains the configured command verbatim. Review these fields before sharing
artifacts outside the project.
