# Soro Mutants v1.0 performance tuning

Start with a small selection and compare runs with the same source revision,
filters, toolchain, and test command. Time the first run and an immediate repeat
to see how much time comes from cold builds.

```bash
cargo soro-mutants list . --operator AUTH-001 --count-only
time cargo soro-mutants test . --operator AUTH-001 \
  --compile-timeout 300 --test-timeout 60
```

## Reuse build artifacts and choose where to put them

The default cache lives at `.soro-mutants-target/` under the project root.
It holds `baseline/` for clean builds and `mutants-shared/` for incremental
mutant builds. Both persist between runs. The runner copies the project to
`.soro-mutants-worktree/` at the project root for each mutant and normally
removes that scratch copy after execution.

Use `--target-dir` to move build artifacts to a fast disk with enough space.
Relative cache paths are resolved against the project root. Prefer an absolute
location outside the checkout so a custom-named build cache is outside the
project copy. Scratch remains under the project root. To place scratch on a
different disk, put the checkout on that disk.

```bash
cache="$(mktemp -d)"
cargo soro-mutants test . --target-dir "$cache"
du -sh "$cache"
cargo soro-mutants clean . --target-dir "$cache" --dry-run
```

Reuse the same cache location on later runs. When reclaiming space, preview
cleanup first and run the same command without `--dry-run` after the mutation
process has stopped. See [troubleshooting](troubleshooting.md#disk-usage).

## Set budgets for compilation and tests

`--compile-timeout` applies to each mutant's separate compile gate.
`--test-timeout` applies to the baseline and each mutant test command.
`--timeout` supplies the fallback for either phase without an explicit budget
and defaults to 120 seconds. A custom command without a recognized compile gate
uses the test budget for its entire run, including any compilation.

Set the test budget high enough for a cold baseline build plus tests. Set the
compile budget using cold and warm mutant builds. Repeated `TIMEOUT` results
can indicate insufficient CPU or I/O capacity as well as tight budgets.

## Choose between shards and nextest

Mutants run serially within each invocation. Shards divide that work across
independent checkouts and cache directories. Each nonempty shard repeats the
baseline and may build dependencies, so more shards trade aggregate CPU and
disk use for a shorter wall-clock run. Start with two and compare elapsed
time and runner usage before increasing the count.

```bash
cargo soro-mutants list . --shard 1/2 --count-only
cargo soro-mutants list . --shard 2/2 --count-only
cargo soro-mutants test . --shard 1/2 --json --output shard-1.json
```

Run shard 2 in its own checkout with the same source, filters, and shard total.
See the [CI matrix example](ci-integration.md#shard-a-larger-run).

If your project already uses nextest, compare its runtime against the default
runner on the same selection. Nextest can run tests concurrently within each
mutant and still uses a Cargo compile gate. On constrained machines, concurrent
test execution can increase memory and CPU pressure. Nextest must already be
installed, and `--runner nextest` cannot be combined with `--test-command`.
See [nextest setup](nextest-runner.md).

```bash
cargo nextest --version
cargo soro-mutants test . --runner nextest --operator AUTH-001
```
