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

The report is written before an unmet threshold exits with code 4. A baseline
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