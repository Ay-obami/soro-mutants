# Soro Mutants v1.0 CI integration

Run mutation tests from the Soroban project's checkout, using the project's Rust
toolchain and the same test selection as its regular CI. The examples assume a
committed Cargo.lock and use main to pick up the reporting features. For a
reproducible pipeline, replace `--branch main` with `--rev REVIEWED_COMMIT_SHA`
after validating that revision on your project.

## GitHub Actions job with a score threshold

Save this as `.github/workflows/mutation.yml`. Replace stable with the project's
pinned toolchain if needed, and add any project-specific build or fixture setup
before the mutation step. The minimum score of 80 is an example policy.

```yaml
name: Soroban mutation tests
on: [pull_request, workflow_dispatch]
permissions:
  contents: read
jobs:
  mutation:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install Soro Mutants
        run: cargo install --git https://github.com/Ay-obami/soro-mutants --branch main --locked
      - name: Run mutation tests
        run: |
          cargo soro-mutants test . --test-command 'cargo test -q --locked' \
            --min-score 80 --github-summary --github-annotations \
            --json --output mutation-results.json
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: mutation-results
          path: mutation-results.json
```

Add `--no-progress` to keep the job log to the final summary, and see
[monorepo invocation patterns](monorepo.md) when the repository holds several
projects. The report is written before an unmet threshold exits with code 4. A baseline
failure exits with code 3 and may leave an empty report file. Check the run
status before consuming an artifact. With no viable results, the CLI score is
0%, so a positive threshold fails. The job summary shows counts and surviving
mutants. Warning annotations are emitted on stderr for survivors and need only
read permissions. For files outside the PR diff, view annotations on the
workflow run. See [annotations](github-annotations.md) and [report consumption](report-consumer.md).

## Shard a larger run

Use the same checkout revision and filters in every job. Each hosted runner has
its own checkout and build caches. This example uses one artifact per shard and
keeps all shard jobs running when a peer fails.

```yaml
name: Sharded Soroban mutation tests
on: [pull_request, workflow_dispatch]
permissions:
  contents: read
jobs:
  mutation:
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix:
        shard: [1, 2, 3, 4]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install Soro Mutants
        run: cargo install --git https://github.com/Ay-obami/soro-mutants --branch main --locked
      - name: Run shard
        env:
          SHARD: ${{ matrix.shard }}
        run: |
          cargo soro-mutants test . --test-command 'cargo test -q --locked' \
            --shard "$SHARD/4" --github-summary --github-annotations \
            --json --output "shard-$SHARD.json"
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: mutation-shard-${{ matrix.shard }}
          path: shard-${{ matrix.shard }}.json
  score:
    needs: mutation
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
        with:
          pattern: mutation-shard-*
          merge-multiple: true
      - name: Enforce whole-run score
        run: |
          jq -es --argjson minimum 80 '
            if length != 4 or any(.[]; .schema_version != 1 or (.results | type) != "array")
            then error("expected four schema v1 test reports")
            else
              [.[].results[].outcome] as $outcomes
              | ($outcomes | map(select(. == "KILLED")) | length) as $killed
              | ($outcomes | map(select(. == "KILLED" or . == "SURVIVED")) | length) as $viable
              | (if $viable == 0 then 0 else 100 * $killed / $viable end) as $score
              | ("Whole-run mutation score: \($score)%" | debug)
              | $score >= $minimum
            end
          ' shard-{1,2,3,4}.json
```

The score job runs only after all shards succeed. It computes the threshold
from combined outcome counts, since shards can have different numbers of viable
mutants. Each nonempty shard repeats the clean baseline, and empty shards still
produce valid reports. See [performance tuning](performance-tuning.md) for the
trade-offs of increasing the shard count.

## CI test-suite runtime budget monitoring

To detect accidental performance regressions and unbounded test suite execution times early, CI enforces a tolerant runtime budget via `.github/scripts/check-runtime-budget.sh`.

- **Tolerant threshold**: A budget of 480 seconds (8 minutes) is allocated for `cargo test --all` to accommodate CI virtualization variances while strictly preventing pathological regressions from exceeding the job timeout.
- **Diagnostics**: If the test suite exceeds the allocated threshold, the script outputs detailed metadata including total elapsed time, budget ceiling, and actionable troubleshooting guidance.

### Running locally
Contributors can verify runtime budget compliance locally:
```bash
# Verify with custom budget (e.g. 300 seconds)
.github/scripts/check-runtime-budget.sh 300 cargo test --all

# Run self-test regression suite
.github/scripts/check-runtime-budget.sh --self-test
```
Or run the dedicated test target:
```bash
cargo test --test runtime_budget_cli
```

## Latest stable Rust check

The `stable` job in `.github/workflows/msrv.yml` is the latest-stable gate. It is intentionally independent of the
pinned `1.85.0` MSRV job: it uses `dtolnay/rust-toolchain@stable` with no `toolchain:` pin, keeps a 15-minute job
bound, writes the detected toolchain versions to `$GITHUB_STEP_SUMMARY`, and runs the `latest_stable_ci` contract
test with a 30-second runtime budget. The regression seed in that test rejects a stable job that has been pinned,
unbounded, or stripped of its verification steps.

Run the check locally before opening a release-affecting pull request:

```bash
cargo +stable check --locked --all-targets --all-features
cargo +stable test --locked --test check_platform_cli
cargo +stable test --locked --test latest_stable_ci
cargo +stable test --locked --test parser_stable_syntax_corpus_cli
cargo +stable run --locked -- check-platform --json
```

See [supported platforms](supported-platforms.md#latest-stable-gate) for the full toolchain policy.
