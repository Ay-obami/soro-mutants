# Test command environment

Baseline and mutant test commands inherit the caller's environment by default. Values containing spaces or Unicode are passed through unchanged, and variables that are absent remain absent. The same inheritance behavior is used for the clean baseline and every mutant command.

Soro Mutants intentionally controls only variables required for isolation: it sets `CARGO_TARGET_DIR` to the selected baseline or mutant cache and removes `RUSTUP_TOOLCHAIN` so the target repository's own toolchain configuration can take effect. Other caller-provided variables are not rewritten by Soro Mutants.

For example:

```bash
SORO_API_MODE='preview λ' cargo soro-mutants test . \
  --test-command 'test "$SORO_API_MODE" = "preview λ" && cargo test -q'
```

Do not print secret environment values from custom test commands; Soro Mutants redacts known secret-shaped diagnostics, but secret management itself is outside its scope.
