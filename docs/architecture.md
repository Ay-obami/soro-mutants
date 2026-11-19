# Architecture

Soro Mutants separates semantic mutation discovery from test execution.

```text
Rust source
   |
   v
syn parser
   |
   v
Soroban semantic recognizers
   |
   v
structured Mutant
   |
   v
fresh scratch worktree
   |
   +--> compile-only viability gate
   |
   v
configured Cargo tests
   |
   v
KILLED / SURVIVED / UNVIABLE / TIMEOUT
```

## Discovery

The engine parses Rust source with `syn` and visits functions and method calls. Each operator recognizes a narrow Soroban-specific pattern and records a source span, original text, replacement text, enclosing function, and operator ID.

The AST is used for recognition only. Mutations are applied to the original source text so formatting, comments, and useful line locations remain intact.

## Execution isolation

The target repository is never mutated in place. Each mutant is applied to a copied scratch worktree under `.soro-mutants-worktree`. The scratch worktree is removed after each mutant, including compile-invalid and timed-out outcomes.

The clean baseline uses:

```text
.soro-mutants-target/baseline
```

Mutated runs use a separate cache:

```text
.soro-mutants-target/mutants-shared
```

This prevents mutant build artifacts from contaminating the baseline while still allowing dependencies to be reused between mutant runs.

With `--jobs N` (N > 1), mutants run concurrently. Each worker has its own scratch worktree (`.soro-mutants-worktree/worker-N`) and its own Cargo cache (`.soro-mutants-target/mutants-parallel/worker-N`), so concurrent mutants never share source files or build output and never write to the baseline cache. Results are reported in mutant ID order regardless of completion order. See [parallel mutant execution](#parallel-mutant-execution) below. Build caches persist deliberately for performance and can be removed with `cargo soro-mutants clean`; cleanup never removes the target repository's normal Cargo cache or source files.

## Classification

A mutant must first pass a compile-only Cargo test command. Compile failures are `UNVIABLE`.

For viable mutants:

- configured tests fail -> `KILLED`;
- configured tests pass -> `SURVIVED`;
- execution exceeds the timeout -> `TIMEOUT`.

Only killed and survived mutants are included in the semantic mutation score.

## Boundaries

The engine does not infer that every survivor is exploitable. It measures whether a configured test suite distinguishes a deliberately injected Soroban semantic change.

Generic arithmetic and boolean mutations remain the job of tools such as `cargo-mutants`. Soro Mutants focuses on semantics that require Stellar/Soroban context.

## Parallel mutant execution

By default Soro Mutants executes viable mutants one at a time. Large Soroban workspaces can produce enough mutants that serial execution becomes slow, so `cargo soro-mutants test` accepts a worker count:

```bash
cargo soro-mutants test /path/to/project --jobs 4
# or
cargo soro-mutants test /path/to/project -j 4
```

`--jobs` must be a positive integer and defaults to `1`. `--jobs 1` is exactly the historical serial behavior and directory layout. The effective worker count is never larger than the number of selected mutants.

### Isolation guarantees

The clean baseline always runs first, once, against the unmodified project with its own Cargo target directory. Mutants are only executed after the baseline passes.

With `--jobs N` (N > 1), each worker owns two directories for the whole run:

```text
<project>/.soro-mutants-worktree/
├── worker-1/          # worker 1's scratch copy of the project
├── worker-2/
└── ...

<cache>/                # .soro-mutants-target by default, or --target-dir
├── baseline/           # clean baseline only — never used by a mutant
├── mutants-shared/     # serial (--jobs 1) mutant cache
└── mutants-parallel/
    ├── worker-1/       # CARGO_TARGET_DIR for worker 1
    ├── worker-2/
    └── ...
```

- **Source isolation.** A worker copies the project into its own scratch worktree, applies exactly one mutant, runs the compile gate and tests, and deletes the worktree before taking the next mutant. Two workers never share a source tree, so one mutant cannot leak into another. The target repository itself is never modified.
- **Baseline isolation.** Worker caches live under `mutants-parallel/`, a sibling of `baseline/`. The runner refuses to start if a worker cache would resolve inside the baseline cache.
- **Cargo state.** Each worker has its own `CARGO_TARGET_DIR`, so concurrent builds never write the same build artifacts, fingerprints, or lock files. The global Cargo home (registry and git caches) is shared and protected by Cargo's own package-cache locking, which is designed for concurrent Cargo processes. Each worktree has its own copy of `Cargo.lock`.
- **Copy safety.** Scratch copies skip Soro Mutants' own scratch and cache directories (including a custom `--target-dir` inside the project), so a worker never copies another worker's in-progress output.
- **Stable paths.** A worker keeps the same scratch path and target directory for every mutant it runs, so Cargo reuses dependency builds between those mutants and rebuilds only the mutated crate.

### Deterministic results

Mutants are assigned to workers dynamically, so they *finish* in a non-deterministic order. Results are nevertheless reported in mutant ID order: text progress lines are printed as soon as the next mutant in sequence is complete, and JSON, Markdown, HTML, SARIF, GitHub annotations, and summaries are produced from results ordered by mutant ID. For the same source, filters, and shard, a parallel report is identical to a serial one. Mutant IDs and shard assignment do not depend on `--jobs`.

If a mutant cannot be executed (for example an I/O error while copying the project), no new mutants are started, in-flight mutants are allowed to finish, scratch worktrees are removed, and the error with the lowest mutant ID is reported.

### Trade-offs

**Disk space.** Every worker has its own complete Cargo target directory. For Soroban contracts this is typically on the order of 1 GB or more per worker (about 1.3 GB per worker for this repository's token fixtures), so `--jobs 4` needs roughly four times the serial mutant cache, plus one scratch copy of the project per worker. Use `cargo soro-mutants clean` (with the same `--target-dir`, if any) to remove all worker caches.

**CPU and cold caches.** The first mutant on each worker compiles the full dependency graph into that worker's empty cache. On a cold cache, `--jobs N` therefore performs N dependency builds concurrently, and can be *slower* than serial execution on machines with few cores. The benefit appears once caches are warm and there are many mutants: on a 4-core machine, the 8-mutant `weak-token-client` fixture went from 10 s serially to 6 s with `--jobs 3` on warm caches, while the cold first run was slower than serial.

Cargo already parallelizes each individual build across all cores, and the test harness runs tests concurrently. Running N mutants at once multiplies that load. Reasonable starting points:

- choose `--jobs` at or below the number of physical cores, and lower it if memory is limited;
- optionally cap per-build parallelism, for example `CARGO_BUILD_JOBS=2 cargo soro-mutants test . --jobs 4`;
- increase `--compile-timeout` / `--test-timeout` if timeouts appear only under parallel load, since contention can stretch individual mutant runs.

**Interaction with sharding.** `--shard` and `--jobs` compose: a shard can execute its own mutants in parallel. Concurrent *shards* (separate `cargo soro-mutants` processes) still need separate project checkouts and cache directories, because all processes for one checkout share `.soro-mutants-worktree`.

### Out of scope

Distributed execution across machines and cloud build infrastructure are not provided. Use CI sharding to spread work across machines.
