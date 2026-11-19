# Soro Mutants v1.0 troubleshooting

## Diagnose by symptom

Start with the command's exit status and its stderr. Run these checks from
the target project root. If your invocation uses `--test-dir`, reproduce the
underlying test command from that project subdirectory.

| Symptom | Check | Next step |
| --- | --- | --- |
| Invalid argument (exit 2) | Run `cargo soro-mutants test --help`. | Check operator IDs, glob syntax, mutually exclusive output modes, and one-based shard numbers. |
| Project path cannot be resolved (exit 1) | Check the path from the shell's working directory and confirm it points to the intended Cargo project or workspace. | Use an absolute path if the caller's working directory varies. Check that the project and test subdirectory are readable. |
| No matching mutants (usually exit 0) | Run `cargo soro-mutants list . --count-only`, then add your filters one at a time. | Check the project-relative paths, function names, exclusions, suppressions, and shard. An empty selection skips the baseline. A positive score threshold on an empty selection exits 4. |
| Baseline failed or timed out (exit 3) | Run the exact configured test command on the clean project under the same toolchain. | Fix dependency, setup, or test failures first. If the clean run passes slowly, increase `--test-timeout` to cover cold compilation and tests. |
| A mutant is `UNVIABLE` | Check whether the clean baseline passed and the separate mutant compile gate failed. | Review compiler diagnostics and the mutation location. Unviable outcomes are excluded from the score. With a custom command that lacks a compile gate, a compiler failure can be classified as `KILLED`. |
| A mutant is `TIMEOUT` | Compare cold-build and test timings against the two configured budgets. | Increase the budget for the affected phase and rerun with the same filters. Custom commands without a separate compile gate use the test budget. |
| Report is missing, empty, or unwritable | Check the exit status, output parent, write permissions, and free disk space. For a job summary, also check `GITHUB_STEP_SUMMARY`. | Create the output parent and retry. A failed baseline can leave an empty report. Exit 4 means the threshold failed after the report was written. |
| Score threshold failed (exit 4) | Read the report's killed and survived counts and confirm which shard or filters were used. | Inspect survivors and check for a zero viable count. See the [score calculation](report-consumer.md#reproduce-the-cli-score). |

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