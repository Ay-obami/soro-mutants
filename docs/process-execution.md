# Test-process execution

Soro Mutants runs the clean baseline and every mutant command with the same execution contract.

## Working directory

The baseline command runs from the canonical target project root, or from
`PATH/--test-dir` when `--test-dir` is supplied. Mutant commands run from
the equivalent path inside that mutant's copied scratch worktree. Relative
paths in the user command therefore resolve against the same project-relative
directory for the baseline and mutants.

This applies whether the CLI input `PATH` was absolute or relative. Soro
Mutants canonicalizes the project before execution.

Example:

```bash
cargo soro-mutants test ./contract \
  --test-dir integration \
  --test-command 'test "$(cat config.txt)" = staging'
```

Both the clean baseline and each mutant read
`integration/config.txt` from their respective project copies.

## Failure classification

A failed baseline is classified before any mutant executes:

- `compile_failure`: Cargo reported that the target could not compile.
- `test_failure`: the command ran but returned a non-zero test/command status.
- `command_not_found`: the shell could not find the requested command.
- `spawn_failure`: Soro Mutants could not start the command shell itself.
- `timeout`: the baseline exceeded the effective test timeout.

Baseline failures continue to use exit code **3**. The diagnostic includes the
effective command, execution directory, exit code when available, and bounded
stdout/stderr excerpts. Secret-shaped values are passed through the existing
diagnostic redaction before printing. No JSON or mutation results are created
for a failed baseline.

Launch failures during mutant execution are fatal runner errors rather than
being counted as `KILLED`, because no meaningful test result exists.

## Output bounds

Soro Mutants drains child stdout and stderr continuously to avoid pipe
deadlocks, but retains at most 8 KiB from each stream for diagnostics. This
keeps baseline-failure messages bounded even when a command is very noisy.

## Timeout cleanup

On Unix, each test shell is placed in its own process group. On timeout Soro
Mutants sends `TERM` to that group, waits for a short bounded grace period,
then sends `KILL` if the group is still running. This covers descendants that
remain in the spawned process group.

On Windows and other non-Unix platforms, the standard-library fallback
terminates the immediate spawned child. Descendants that detach into a
separate process tree may require operating-system-specific cleanup outside
Soro Mutants. Processes not started by Soro Mutants are never targeted.
