# Persistent mutation-result cache (opt-in)

The result cache stores mutation outcomes between runs so unchanged mutants are
not re-executed. It is **off by default**: pass `--cache` to `test` to enable
it. The cache is local to one `--target-dir`; there is no remote/shared cache
and no cross-repository reuse.

## Storage location

```text
<target-dir>/result-cache.json
```

The default `<target-dir>` is `<project>/.soro-mutants-target`. `clean` removes
`result-cache.json` together with the other managed state, and `clean --dry-run`
lists it without deleting it.

The file contains a format `version`, the complete run `identity`, an
`entries_hash` integrity digest, and `entries` (one record per mutant with
`id`, `operator`, `file`, `line`, `original`, `replacement`, and `outcome`).
Writes are atomic (write a temporary file, then rename). Any out-of-band edit
that changes `entries` without recomputing `entries_hash` is detected and
discarded.

## Cache-key generation

A stored result is reused only when **all** of the following identity fields
match the current run. These are the complete inputs to the key:

| Input | Why it is in the key |
| --- | --- |
| `workspace_root` | Canonical project path; prevents reuse from a different checkout |
| `source_hash` | Content hash of selected `.rs`, `Cargo.toml`, and `Cargo.lock` files |
| `workspace_source_hash` | Content hash of the entire workspace, including path dependencies outside `--package` selection |
| `lockfile_hash` | Direct hash of `Cargo.lock`; dependency resolution must be identical |
| `toolchain_file_hash` | Hash of `rust-toolchain.toml` or `rust-toolchain`, if present |
| `test_command` | Resolved runner command (after `--runner` and `--package` normalization), including all arguments |
| `rustc_version` | Toolchain identity reported by `rustc --version` |
| `cargo_version` | Cargo identity reported by `cargo --version` |
| `packages` | `--package` selection |
| `operator` | `--operator` filter |
| `file`, `function` | Source filters |
| `test_dir` | Working directory for the test command |
| `exclude_globs` | `--exclude-path` patterns |
| `shard` | `--shard INDEX/TOTAL` selection |
| `changed_file`, `diff_base`, `changed_functions` | Changed-selection configuration |
| `jobs` | Execution concurrency configuration |
| `execution_env_hash` | Hash of `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_BUILD_RUSTFLAGS`, `CARGO_BUILD_TARGET`, `RUSTC_WRAPPER`, `RUST_TEST_THREADS`, `RUSTUP_TOOLCHAIN` (values are hashed, never logged) |
| `compile_timeout_seconds`, `test_timeout_seconds` | Execution budgets |
| format `version` | Cache schema version; unknown versions are discarded |

Within a matching identity, an individual mutant is matched by
`operator + file + line + original + replacement`. The displayed `id` is
refreshed from the current discovery pass. A result with outcome `UNVIABLE` is
cacheable; `TIMEOUT` is never cached because it depends on host load.

## Hit and miss behavior

- The baseline command always runs before the cache is consulted. A failed or
  timed-out baseline aborts the run with exit code 3; no cached result is used
  and no result is scored.
- With `--resume`, the local progress file takes precedence over the result
  cache and its entries are reported as `(resumed)`.
- On a matching entry, the run prints `... (cache hit)` in text mode and the
  stderr summary reports `N hit(s)`.
- On a missing entry, the run prints `cache miss ...` in text mode, executes
  the mutant, stores the outcome, and increments the miss count.
- When the identity, version, or integrity digest does not match, the whole
  cache is discarded: stderr reports a specific `result cache miss: ...`
  reason, and a fresh cache is written at the end of the run.
- `--clear-cache` removes the stored file before the run starts.

## Invalidation rules

Any change to an identity input above invalidates the cache. In particular:

- source code, `Cargo.lock`, a `rust-toolchain*` file, or a path dependency
  changes;
- the test command, runner, package, operator, file/function filter,
  exclusion, shard, changed-selection setting, or `test_dir` changes;
- the rustc/cargo version or relevant execution environment variables change;
- compile/test timeouts or concurrency change;
- the cache format version changes, the JSON is malformed, or the entries
  digest does not match.

Because invalidation is conservative, a change to an unrelated field causes
extra executions rather than a stale reuse. This is intentional.

## Disabling and clearing

```bash
# Enable reuse for this run (default is a fresh execution every time).
cargo soro-mutants test /path/to/project --cache

# Explicitly disable reads and writes even if a cache file exists.
cargo soro-mutants test /path/to/project --no-cache

# Delete cached results before this run.
cargo soro-mutants test /path/to/project --cache --clear-cache

# Remove all managed state, including result-cache.json.
cargo soro-mutants clean /path/to/project
```

`--cache` and `--no-cache` conflict. Benchmark runs invoke `test` with
`--no-cache`, so benchmark evidence never comes from this cache.

## Concrete example

```bash
# First run: executes all mutants and populates the cache.
cargo soro-mutants test fixtures/weak-auth \
  --target-dir /tmp/soro-target --test-command 'true' \
  --operator AUTH-001 --cache

# Second run with identical inputs: no mutant commands are re-executed.
cargo soro-mutants test fixtures/weak-auth \
  --target-dir /tmp/soro-target --test-command 'true' \
  --operator AUTH-001 --cache
# stderr: result cache: 1 hit(s), 0 miss(es), 1 stored

# Editing a source file or changing --test-command forces fresh execution.
```

## Regression coverage

- [`tests/result_cache_cli.rs`](../tests/result_cache_cli.rs) covers repeated
  hits, invalidation on test-command and source changes, `--no-cache`,
  `--clear-cache`, and tampered-entry (contamination) detection.
- [`tests/cache_isolation_cli.rs`](../tests/cache_isolation_cli.rs) covers the
  separate baseline/mutant build-cache isolation rules.

A cache hit is not a new measurement; it is the reuse of an earlier result
whose complete identity matched. It must never be presented as a fresh
verification of a changed environment.
