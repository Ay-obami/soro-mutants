# SARIF 2.1.0 reporting

Use `cargo soro-mutants test PATH --sarif` to emit a SARIF 2.1.0 report. The `--sarif` reporter is mutually exclusive with `--json`, `--markdown`, and `--html`. Existing JSON output is unchanged.

Only **SURVIVED** mutants appear as code-scanning findings. A surviving mutant means that the test command did not detect the semantic change. This is **a test-quality signal, not evidence of an exploitable vulnerability**. KILLED, UNVIABLE, and TIMEOUT outcomes are excluded from SARIF findings. Each finding includes the stable operator rule ID (for example `AUTH-001`), source file and line/column, function, operator, outcome, mutation description, and original/replacement source.

The reporter emits deterministic SARIF (no timestamps); an empty selection produces a valid report with empty `rules` and `results` arrays. See [the example](sarif-report-example.json).

## GitHub code scanning

Generate the report in a CI job that already installs Soro Mutants:

```sh
cargo soro-mutants test . --operator AUTH-001 --sarif --output soro-mutants.sarif
```

Upload the resulting file using `github/codeql-action/upload-sarif@v3`:

```yaml
permissions:
  contents: read
  security-events: write

steps:
  # Checkout, install toolchain and Soro Mutants, and generate the SARIF file.
  - uses: github/codeql-action/upload-sarif@v3
    with:
      sarif_file: soro-mutants.sarif
```

This is an opt-in integration example; Soro Mutants does not upload automatically. Code-scanning findings should be interpreted as surviving mutation tests, **not security alerts**.
