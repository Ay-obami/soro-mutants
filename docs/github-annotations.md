# GitHub Actions annotations for surviving mutants

Use `--github-annotations` with `cargo soro-mutants test` to emit [GitHub workflow-command annotations](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-commands#setting-a-warning-message) for **SURVIVED** semantic mutants.

```sh
cargo soro-mutants test . --github-annotations --json --output results.json
```

Each unique surviving mutant emits one `::warning` command with its relative file, line, column, operator, and description. Annotations go to **stderr**, preserving stdout/JSON, Markdown, HTML and SARIF output formats. Mutants killed by tests, unviable mutants, and timeouts are not annotated.

These warnings indicate **test-quality gaps**, not proven security vulnerabilities. The reporter never assumes files must appear in the PR diff: annotations for untouched files are still emitted safely, although GitHub's PR interface may not display inline markers outside changed lines. Inspect the workflow run's annotations for those cases.

## CI setup

Use the flag inside an ordinary GitHub Actions job that checks out the repository and installs the Rust toolchain and Soro Mutants:

```yaml
name: Mutation annotations
on: [pull_request]
permissions:
  contents: read
jobs:
  annotate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      # Install the Rust toolchain and cargo-soro-mutants for your project.
      - name: Run semantic mutation tests and annotate survivors
        run: cargo soro-mutants test . --github-annotations --json --output results.json
```

No write token or GitHub API credentials are required for workflow-command warnings. For repository-wide code-scanning SARIF uploads, use [the separate SARIF reporter](sarif-report.md), which requires the `security-events: write` permission. This option does not automatically create GitHub issues or enforce a review policy.
