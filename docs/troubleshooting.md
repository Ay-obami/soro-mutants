# Soro Mutants v1.0 troubleshooting

## Diagnose by symptom

Start with the command's exit status and its stderr. Run these checks from
the target project root. If your invocation uses `--test-dir`, reproduce the
underlying test command from that project subdirectory.

| Symptom | Check | Next step |
| --- | --- | --- |
| Invalid argument (exit 2) | Run `cargo soro-mutants test --help`. | Check operator IDs, glob syntax, mutually exclusive output modes, and one-based shard numbers. |
| Project path cannot be resolved (exit 1) | Check the path from the shell's working directory and confirm it points to the intended Cargo project or workspace. | Use an absolute path if the caller's working directory varies. Check that the project and test subdirectory are readable. |
| Filesystem path too long (exit 1) | Check the path printed in the copy or directory creation diagnostic. | Move the checkout nearer the filesystem root or choose a shorter `--target-dir` (where applicable); keep directory nesting and filenames short. |
| No matching mutants (usually exit 0) | Read the `note:` and `warning:` lines on stderr, then run `cargo soro-mutants list . --count-only` and add your filters one at a time. | See [empty selections](#empty-selections). An empty selection skips the baseline. A positive score threshold on an empty selection exits 4. |
| Mutants from the wrong project, or every mutant `UNVIABLE` in a repository with several projects | Check which directory you passed as `PATH`. | See [monorepo invocation patterns](monorepo.md). |
| Too much progress output in CI logs | The text report prints one line per mutant. | Pass `--no-progress` to `test`. See [quiet CI output](#quiet-ci-output). |
| Baseline failed or timed out (exit 3) | Run the exact configured test command on the clean project under the same toolchain. | Fix dependency, setup, or test failures first. If the clean run passes slowly, increase `--test-timeout` to cover cold compilation and tests. |
| A mutant is `UNVIABLE` | Check whether the clean baseline passed and the separate mutant compile gate failed. | Review compiler diagnostics and the mutation location. Unviable outcomes are excluded from the score. With a custom command that lacks a compile gate, a compiler failure can be classified as `KILLED`. |
| A mutant is `TIMEOUT` | Compare cold-build and test timings against the two configured budgets. | Increase the budget for the affected phase and rerun with the same filters. Custom commands without a separate compile gate use the test budget. |
| Report is missing, empty, or unwritable | Check the exit status, output parent, write permissions, and free disk space. For a job summary, also check `GITHUB_STEP_SUMMARY`. | Create the output parent and retry. A failed baseline can leave an empty report. Exit 4 means the threshold failed after the report was written. |
| Score threshold failed (exit 4) | Read the report's killed and survived counts and confirm which shard or filters were used. | Inspect survivors and check for a zero viable count. See the [score calculation](report-consumer.md#reproduce-the-cli-score). |
| A run unexpectedly reuses cached outcomes | The invocation passed `--cache` and the full identity matched. | Compare stderr's cache diagnostics, then use `--no-cache` or `--clear-cache`. See [result cache](result-cache.md#invalidation-rules). |
| `check-platform` reports `unsupported` | Read the failing check id and `remediation` in the JSON report. | Follow the [platform troubleshooting table](supported-platforms.md#troubleshooting). |
| Benchmark matrix refuses external targets | External targets need an explicit `--allow-external` and a pinned revision. | Use `--local-only` for local fixtures, or opt in deliberately. See [benchmark safety](benchmark-matrix.md#safety-model). |

## Message format

Human-readable diagnostics go to stderr; reports go to stdout or `--output`.
Every diagnostic line starts with one of three lowercase prefixes:

| Prefix | Meaning | Effect on the exit status |
| --- | --- | --- |
| `error: ` | A fatal condition. The command stops. | Non-zero: 1 for an operational error, 3 for a failed baseline, 4 for a missed `--min-score`. Invalid arguments exit 2 with Clap's own `error: ` line. |
| `warning: ` | A problem you may want to act on, such as a skipped unparsable file or a discarded result cache. | None. |
| `note: ` | Neutral context, such as cache activity or why a selection is empty. | None. |

Message text starts with a lowercase letter, except for flags, identifiers, and
paths, and does not end with a period. A fatal error with a cause chain is one
line, with causes joined by `: `. For example:

```text
error: cannot resolve /no/such/project: No such file or directory (os error 2)
```

Scripts should match the prefix and the exit status rather than the wording of
the rest of the line. GitHub annotations written by `--github-annotations` use
GitHub's own `::warning` workflow command and are unaffected. The stderr output
of a benchmark command is passed through unchanged.

Previous releases printed fatal errors as `Error: ...`. Scripts that matched
that capitalized prefix should match `error: ` instead.

## Empty selections

A run whose selection is empty exits 0, skips the baseline, and prints the
usual empty report on stdout (`No matching Soroban semantic mutants found.`,
`Discovered 0 semantic mutants`, `0` for `--count-only`, or an empty JSON,
Markdown, HTML, or SARIF report). Stderr then explains why, and distinguishes a
valid empty result from a discovery problem:

| Stderr line | Meaning |
| --- | --- |
| `note: no supported Soroban mutation sites were found in N scanned Rust source file(s); this is a valid empty result, not a discovery failure` | Discovery worked. The project simply has nothing the operators recognize. |
| `note: N candidate mutant(s) were discovered but none matched the discovery filters: ...` | `--package`, `--operator`, `--file`, `--function`, or `--exclude-path` removed every candidate. |
| `note: N mutant(s) matched the discovery filters but none are in the changed-code selection: ...` | `--changed-file`, `--diff-base`, or `--changed-functions` removed the rest. |
| `note: N mutant(s) were selected before sharding but --shard I/T contains none of them` | The shard has no mutants; this is normal when shards outnumber mutants. |
| `note: all N candidate mutant(s) were suppressed as equivalent ...` | See the suppression summary. |
| `warning: no Rust source files were found under PATH; ...` | Probably the wrong `PATH` or an over-broad `--exclude-path`. Not a valid empty result. |
| `warning: N Rust source file(s) could not be parsed and were skipped: ...` | Fix the syntax error or exclude the file. Skipped files can hide mutants. |

The last line is always `note: active selection filters: ...` (listing each
active flag) or `note: no selection filters were active`. Discovery that cannot
run at all, such as an unreadable project, is a fatal `error: ` with exit 1 and
nothing on stdout. A positive `--min-score` on an empty selection still exits 4
after these lines.

```bash
$ cargo soro-mutants list . --operator TTL-001 --count-only
0
note: 4 candidate mutant(s) were discovered but none matched the discovery filters: --operator TTL-001
note: active selection filters: --operator TTL-001
```

## Quiet CI output

By default `test` prints `Baseline:` and `Baseline PASS` lines and one line per
mutant as it finishes. Pass `--no-progress` to drop all of those and keep only
the final summary:

```bash
cargo soro-mutants test . --no-progress --min-score 80
```

`--no-progress` only affects the text report. JSON, Markdown, HTML, and SARIF
output never contained progress lines and are unchanged, as are exit codes,
`--output`, `--min-score`, and the stderr diagnostics described above. It does
not affect `--resume` or `--progress-file`, which concern the on-disk progress
file. `list` has no progress output and does not accept the flag.

For example, compare discovery and execution with the same operator filter,
then save the report to a directory that already exists.

```bash
cargo soro-mutants list . --operator AUTH-001 --count-only
cargo test -q
mkdir -p reports
cargo soro-mutants test . --operator AUTH-001 \
  --compile-timeout 300 --test-timeout 180 \
  --json --output reports/mutations.json
```

## Disk usage

Mutation testing compiles a clean baseline and one or more mutated copies of a project. Cargo build artifacts can therefore become much larger than the Soro Mutants source checkout itself.

By default, Soro Mutants keeps generated build caches under the target project:

```text
.soro-mutants-target/
├── baseline/
├── mutants-shared/
└── mutants-parallel/   # only after runs with --jobs > 1, one cache per worker
```

Parallel runs (`--jobs N`) keep a full Cargo cache per worker, so they can use roughly N times the space of `mutants-shared`. See [parallel mutant execution](architecture.md#parallel-mutant-execution).

Temporary source worktrees are removed automatically after each mutant run. The baseline and shared mutant Cargo caches are intentionally retained so later runs can reuse dependencies.

To see how much space Soro Mutants-managed state is using:

```bash
du -sh .soro-mutants-target 2>/dev/null
du -sh .soro-mutants-worktree 2>/dev/null
```

If you used `--target-dir`, inspect that directory instead.

Remove only Soro Mutants-managed generated state with:

```bash
cargo soro-mutants clean /path/to/project
```

If mutation runs used a custom target directory, pass the same value to `clean`:

```bash
cargo soro-mutants clean /path/to/project --target-dir /path/to/cache
```

Avoid deleting the entire global Cargo registry or Cargo git cache just to reclaim mutation-test space. Those caches are shared by unrelated Rust projects and deleting them usually causes large dependency downloads on the next build.

## Respect the target project's lockfile and toolchain

Soro Mutants runs the target project's own test command. A reproducible mutation run should therefore respect the project's existing dependency lockfile and Rust toolchain.

When a project checks in `Cargo.lock`, prefer a test command that keeps it locked:

```bash
cargo soro-mutants test /path/to/project \
  --test-command 'cargo test -q --locked'
```

If the project pins Rust with `rust-toolchain.toml` or an equivalent repository configuration, use that toolchain rather than changing the project's source to match the machine's global default.

Before investigating a mutation-specific failure, verify the project baseline directly:

```bash
cd /path/to/project
cargo test --locked
```

If the unmodified project does not pass, Soro Mutants will refuse to score the mutation run because killed/survived classifications would not be trustworthy.

## Dependency-resolution failures

Soroban projects can occasionally encounter dependency-resolution or compiler errors caused by a transitive dependency version that is incompatible with the project's SDK or Rust toolchain.

Typical symptoms include:

- a crate requiring a newer Rust compiler than the project currently pins;
- trait/version mismatches between two transitive dependencies;
- a project that previously built successfully resolving a newly published dependency version after its lockfile was removed or regenerated.

When this happens:

1. Reproduce the failure on the unmodified project with its normal build/test command.
2. Confirm whether the repository has a committed `Cargo.lock` and `rust-toolchain.toml`.
3. Avoid deleting or regenerating the lockfile unless the project intentionally wants to update dependencies.
4. Compare the resolved dependency versions with a known-good checkout or CI run.
5. Treat any temporary dependency pin as a project-specific workaround, not a permanent Soro Mutants requirement.

Soro Mutants should not automatically rewrite a target project's lockfile or dependency versions. Dependency repair belongs to the target project because changing dependency resolution can change the program being tested.

## Is the failure caused by Soro Mutants?

A useful rule is to separate baseline failures from mutant failures:

- **Baseline fails:** fix or reproduce the target project first. Do not interpret mutation results.
- **Baseline passes, mutant does not compile:** the mutant is classified `UNVIABLE`.
- **Baseline passes, mutant tests fail:** the mutant is `KILLED`.
- **Baseline passes, mutant tests pass:** the mutant `SURVIVED` and needs human review.
- **Compile or tests exceed the configured timeout:** the mutant is `TIMEOUT`.

When reporting a problem, include the Soro Mutants version, target-project commit, Rust toolchain, exact test command, and whether the clean baseline passes.

## Deeply nested Rust syntax

Before parsing a Rust file, Soro Mutants applies a structural-depth guard. Source with more than **256** nested delimiters is rejected with an actionable error. Delimiters in ordinary strings and comments do not count. This is a denial-of-service safety boundary for pathological source, not a normal contract restriction.

## Oversized Rust source files

Soro Mutants intentionally refuses to parse an individual Rust source file larger than **16 MiB**. The limit is checked from filesystem metadata before the file is read into memory. This keeps discovery memory use predictable for generated or adversarial inputs. Split generated source into modules or exclude the generated path from mutation discovery when a file exceeds this limit.

## Non-Unicode filesystem paths

On Unix, the project root may contain bytes that are not valid UTF-8. Soro Mutants passes filesystem paths to child processes as native OS strings and does not replace an unrepresentable project root with another directory. Human diagnostics use the platform's display representation; project-relative report paths must still be serializable by the selected report format.
## Timeout argument validation

The `test` command accepts positive timeout durations for `--timeout`,
`--compile-timeout`, and `--test-timeout`. Bare integers remain seconds for
backward compatibility, and `s`, `m`, `h`, and `d` suffixes are supported. For example,
`cargo soro-mutants test fixtures/weak-auth --timeout 2m --compile-timeout 30s --test-timeout 1m`.
Negative values and zero are usage errors (exit code 2), reported on stderr
without producing a mutation report. The benchmark matrix `--timeout` uses the
same validation.

The `--version` and `-V` flags print `cargo-soro-mutants <version>` followed
by a newline on stdout, with exit code 0 and no stderr output.

After successful and failed mutation runs, the runner removes the
`.soro-mutants-worktree` scratch directory; reusable Cargo caches under
`.soro-mutants-target` are intentionally retained. Use
`cargo soro-mutants clean` to remove managed caches when needed.

## Runner environment parity and HOME isolation

The clean baseline, mutant compile checks, and mutant test commands inherit the same
parent process environment. For every command, the runner deliberately removes
`RUSTUP_TOOLCHAIN` and sets `CARGO_TARGET_DIR` to the corresponding isolated
build-cache directory. The latter differs between baseline and mutant runs by
design; other inherited variables, including `HOME`, are not modified by the
production runner. Run-to-run determinism requires using consistent toolchain
and environment values.

Tests that invoke tools which create home-directory caches should explicitly
pass temporary `HOME` and `XDG_CONFIG_HOME` values to their spawned CLI,
**not** modify the test process's global environment. For example, create a
scratch directory and pass `.env("HOME", &scratch_home)` to
`Command::new(env!("CARGO_BIN_EXE_cargo-soro-mutants"))`. This ensures all
baseline/mutant child processes share the same isolated test HOME without
writing into your actual user profile. Production commands retain normal HOME
behavior for backwards compatibility.

Scratch copies preserve the source tree's file and directory permissions,
including executable bits used by shell scripts. Source trees with intentionally
read-only files may prevent mutation of those files; make the source writable
before running mutation tests if appropriate. If filesystem path length limits
are exceeded while preparing a scratch copy, the error includes the failing
path and suggests a shorter checkout or target directory. No fixed artificial
path-length cutoff is imposed.

## Custom scratch directory is rejected

If `--scratch-dir` reports that the directory is the project root or one of its parents, choose a separate directory such as `/tmp/soro-mutants`, a fast local disk, or a dedicated project subdirectory. Soro Mutants appends `.soro-mutants-worktree` beneath the selected directory and refuses layouts that could make cleanup encompass the project itself.

If a custom scratch path is a symbolic link at cleanup time, the run fails safely rather than following the link. Remove the link or choose a normal directory and retry.
