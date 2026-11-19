# Automated benchmark matrix (opt-in, schema version 1)

The `benchmark-matrix` subcommand reproduces the benchmark targets documented in
[`docs/benchmark.md`](benchmark.md) from a machine-readable manifest. It is an
**opt-in, local developer/CI-maintenance tool**. It is not part of normal unit CI
and does not publish or update security claims automatically.

- [Manifest](benchmark-matrix.json)
- [JSON Schema](benchmark-matrix.schema.json)

## Safety model

- Nothing outside the manifest is executed, and the manifest is only read when
  the subcommand is invoked explicitly.
- `kind=external` targets are **never** fetched or executed unless
  `--allow-external` is passed on the command line. `--local-only` skips every
  external target and is mutually exclusive with `--allow-external`.
- External targets require a `https://` repository URL and a pinned hexadecimal
  git revision (7–40 hex characters). Branch names, tags, and `HEAD` are
  rejected. The resolved full commit is recorded in the summary.
- An external checkout under `--checkout-dir` is reused only when its `HEAD`
  still matches the pinned revision; otherwise the command stops with an error
  instead of silently running a different revision.
- A target's baseline command must pass before any mutant is scored. If the
  baseline fails or times out, the target is marked `failed`, no mutation
  results are produced, and the overall command exits non-zero.
- Results are labeled "mutation-testing observation only; not a security
  finding about any third-party project." A surviving mutant is never converted
  into a vulnerability claim.

## Manifest fields

| Field | Meaning |
| --- | --- |
| `schema_version` | Integer major version; currently **1** |
| `description` | Optional matrix description copied into the summary |
| `targets[]` | Nonempty list of unique targets |
| `targets[].id` | Unique 1–64 character id (`A-Z`, `a-z`, `0-9`, `-`, `_`); also the checkout directory name |
| `targets[].kind` | `local` or `external` |
| `targets[].path` | Required for `local`: a directory whose canonical path is inside the repository root |
| `targets[].repository` | Required for `external`: public `https://` URL |
| `targets[].revision` | Required for `external`: pinned hexadecimal git commit (7–40 chars) |
| `targets[].operator` | Optional built-in operator filter |
| `targets[].file` | Optional project-relative file suffix filter |
| `targets[].function` | Optional function name filter |
| `targets[].test_command` | Nonempty command executed for the baseline and every mutant |
| `targets[].note` | Optional human-readable note |

Unknown fields are ignored. Required-field removal, renaming, or incompatible
interpretation changes require a new major `schema_version`.

## Usage

Run only the local fixtures (safe default; no network access):

```bash
cargo soro-mutants benchmark-matrix docs/benchmark-matrix.json --local-only
```

Run a single local target and write a Markdown summary suitable for reviewing
before updating `docs/benchmark.md`:

```bash
cargo soro-mutants benchmark-matrix docs/benchmark-matrix.json \
  --target local-auth-strong \
  --local-only \
  --output /tmp/benchmark-local-auth.md
```

Fetch and execute pinned public targets. This is the **only** way external
repositories are touched:

```bash
cargo soro-mutants benchmark-matrix docs/benchmark-matrix.json \
  --allow-external \
  --checkout-dir .soro-mutants-benchmarks \
  --json --output /tmp/benchmark-summary.json
```

Useful options:

- `--timeout SECONDS` — per-command wall-clock timeout (default 120). The
  aggregate mutant command receives a budget derived from this value and the
  number of discovered mutants.
- `--target ID` — run only one manifest target.
- `--json` — emit the structured summary on stdout.
- `--output PATH` — write the Markdown or JSON summary to a file.

The mutant phase invokes this same CLI's `test` subcommand with `--json` and
`--no-cache`, so interactive result-cache entries can never substitute for
benchmark evidence.

## Structured summary

The JSON summary contains:

- `schema_version`, `generated_by`, `description`, `notice`;
- execution settings: `allow_external`, `local_only`, `checkout_dir`,
  `timeout_seconds`, `rustc_version`, `cargo_version`;
- one entry per manifest target with `status` (`completed`, `skipped`, or
  `failed`), optional `reason`, local path, repository, requested and resolved
  revision, the `test_command`, baseline command record (`command`, `exit_code`,
  `outcome`, `duration_ms`, `cwd`, `target_dir`), mutant counts, score, and one
  record per mutant (`id`, `operator`, `file`, `line`, `description`, `original`,
  `replacement`, `outcome`).

The score is `killed / (killed + survived)`, excluding `UNVIABLE` and `TIMEOUT`,
matching the normal test report. A failed baseline produces `counts: null` and
an empty mutant list. The Markdown renderer prints the same information with a
prominent non-security-finding notice.

## Updating `docs/benchmark.md`

The generated summary is evidence for a manual documentation update. A
maintainer reviews baseline status, pinned revisions, and mutant outcomes, then
edits `docs/benchmark.md`. The runner never writes to that file and never
creates a release or security advisory.
