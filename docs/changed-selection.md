# Selecting changed files and functions

Soro Mutants can restrict mutation discovery and test execution to changed source files. These filters work with both `list` and `test` and intersect with existing `--operator`, `--file`, `--function`, exclusions, and sharding.

## Explicit changed-file set

Pass paths **relative to the project argument**:

```sh
cargo soro-mutants list . --changed-file src/lib.rs --changed-file src/admin.rs --json
cargo soro-mutants test . --changed-file src/lib.rs --test-command 'cargo test -q'
```

Paths are exact matches (not globs). Repeat `--changed-file` for multiple files. Only discovered mutants in matching files are selected; an empty selection skips baseline and mutant execution. The regular CLI filters are applied first, and mutant IDs remain deterministic.

## Git diff base

Choose a local Git revision, such as a mainline commit:

```sh
cargo soro-mutants list . --diff-base origin/main --json
cargo soro-mutants test . --diff-base HEAD~1 --changed-functions
```

`--diff-base` uses Git's tracked-file diff, including staged changes, changed tracked files, and detected renames. For renames, the **new destination path** is selected. Git-staged added files are included; **untracked files must first be added to Git** or passed via `--changed-file`. Deleted files do not contain source mutants and are ignored. Files outside the selected project subdirectory are ignored.

With `--changed-functions`, the tool compares zero-context Git diff hunks with parsed Rust function spans and selects all mutants in affected functions, not just mutations located on edited lines. The option requires `--diff-base` and cannot be combined with `--changed-file`.

Function spans may be ambiguous (for example, unusual quoted Git patch paths, deleted-only hunks, or malformed Rust). For those files the tool conservatively retains **file-level selection** rather than missing potential mutants. Edits outside all recognized functions likewise retain file-level selection.

## Requirements and scope

- `--diff-base` requires a Git repository, the `git` executable, and a valid local revision. A clear error is reported if Git metadata or the revision is unavailable.
- `--changed-file` works without Git metadata.
- This is source-local selection, not semantic dependency analysis. Code indirectly affected by a change in another file is not automatically included.
- The selection does not change the meaning of KILLED, SURVIVED, UNVIABLE, or TIMEOUT outcomes.
- Parallel execution and remote CI integration are not included.
