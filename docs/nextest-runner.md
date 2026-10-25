# Optional cargo-nextest runner

Soro Mutants runs `cargo test -q` by default. Workspaces already using [cargo-nextest](https://nexte.st/) can select an alternative runner:

```sh
cargo install cargo-nextest --locked
cargo soro-mutants test . --runner nextest
```

The opt-in flag executes `cargo nextest run` for the clean baseline and each viable mutant. The default `--runner cargo-test` retains the existing `--test-command` behavior. The runner is never downloaded or installed automatically.

Before any nextest run, Soro Mutants checks that `cargo nextest --version` works and reports a clear installation error otherwise. A failing or timed-out clean baseline stops the run without reporting mutants. For each mutant, a separate `cargo test --no-run` compilation gate first distinguishes **UNVIABLE** from **KILLED**. The nextest exit status then determines **SURVIVED** (tests pass) or **KILLED** (tests fail); compile and test timeouts continue to yield **TIMEOUT**.

## Compatibility and limitations

- `--runner nextest` and a custom `--test-command` cannot be combined. Choose `--runner cargo-test --test-command '...'` for custom commands. Extra nextest arguments are not yet exposed by this mode.
- The nextest executable and its dependencies must be present on the machine and in Cargo's subcommand search path. The default runner does **not** need nextest.
- This does not add parallel mutant execution; mutant runs remain sequential, though nextest can run tests concurrently within a single mutant.
- The compile gate uses Cargo's test builder, and the baseline/mutant runs use the same runner and isolated target paths.
- Output formats, mutation-score rules, and baseline gating are unchanged.

To confirm nextest is installed before opting in, run `cargo nextest --version`.
