# Runner path safety and semantic scope boundaries

## Scratch-root validation

Before the clean baseline starts, Soro Mutants verifies that the selected
scratch root can be created, written to, and cleaned up. A temporary probe file
is created inside the managed `.soro-mutants-worktree` root and removed
immediately. If creation, writing, or deletion fails, the command exits with an
actionable error before running the baseline.

For a custom location:

```bash
cargo soro-mutants test . --scratch-dir /fast/local/scratch
```

Soro Mutants still owns only the nested
`/fast/local/scratch/.soro-mutants-worktree` directory.

## Symlinked project roots

The CLI canonicalizes the project path before discovery and execution. When the
user passes a symbolic link to a project, reporting remains project-relative
(for example `src/lib.rs`), while scratch/cache ownership checks operate on the
resolved project root. The user-provided symlink itself is never removed by
normal mutation cleanup.

## Closure scope

Closure parameters are lexical bindings. A closure parameter that shadows an
outer `Address` removes that outer name from the substitution candidate set
inside the closure. A closure parameter explicitly typed as `Address` is an
eligible candidate only inside that closure. Captured outer addresses that are
not shadowed remain available, and closure-local candidates never leak after
the closure. This applies to move and non-move closures.

## Trait and UFCS false-positive guard

Method names alone are never sufficient evidence for Soroban semantics.
User-defined traits may define names such as `require_auth`, `transfer`,
`publish`, and `extend_ttl`; calls on unrelated trait receivers are ignored.
UFCS calls such as `MyTrait::transfer(...)` are also outside the supported
semantic method-call shapes. Direct Soroban calls in the same source remain
discoverable when the existing receiver/type guards are satisfied.
