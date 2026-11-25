# Human report format versions and large-report behavior

Text and Markdown mutation reports include a deterministic banner:

```text
soro-mutants <tool-version> (report format 1)
```

The tool version comes from the packaged Cargo version. The report-format
version changes only when the human-readable text/Markdown layout makes a
breaking compatibility change. JSON continues to use its existing
`schema_version` contract independently.

The banner contains no timestamp, hostname, path, or environment data, so
identical inputs remain reproducible. Machine-readable JSON, HTML, and SARIF
contracts are not changed by this banner.

## Large reports

Report serializers are regression-tested with 5,000 synthetic mutation results.
The test verifies deterministic JSON, Markdown, and HTML output and places
generous output-size bounds on each format to catch accidental explosive
growth. This is a regression guard rather than a hard public maximum: projects
may contain more than 5,000 mutants.

## Mutant source-copy exclusions

Ordinary mutant source worktrees intentionally exclude Cargo's `target/`
directory plus Soro Mutants-managed `.soro-mutants-target/` and
`.soro-mutants-worktree/` trees. This prevents recursive copies and avoids
duplicating build artifacts into every mutant source tree.

Prebuilt WASM fixtures in recognized target subpaths are copied through the
existing narrow fixture path; other Cargo build artifacts remain excluded.
