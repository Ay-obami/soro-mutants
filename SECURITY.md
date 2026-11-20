# Security Policy

Soro Mutants is testing infrastructure. A surviving mutant means the configured test suite did not distinguish an injected semantic change from the original source. It is not, by itself, evidence that a deployed contract is vulnerable.

## Reporting a security issue

Please do not publish exploit details for a vulnerability in Soro Mutants or in a benchmarked third-party project before the affected maintainers have had a reasonable opportunity to investigate.

For issues in Soro Mutants itself, open a minimal private security report through GitHub's security advisory flow once the public repository is available. Include:

- the affected version or commit;
- a minimal reproducer;
- expected and actual behavior;
- whether the issue can corrupt mutation classification or modify source outside the scratch workspace.

For a vulnerability discovered in a third-party contract while using Soro Mutants, report it to that project's maintainers using their security policy. Do not use the Soro Mutants issue tracker to publish third-party exploit details.

## Benchmark policy

Public benchmark notes report mutation-testing behavior only. They must:

- use a green unmodified baseline;
- record the exact test scope used;
- distinguish `SURVIVED` from a vulnerability finding;
- exclude results whose environment or baseline is not reproducible.

## Filesystem cleanup boundary

The `clean` command removes only Soro Mutants-managed scratch and build-cache paths. It refuses to clean when the managed scratch path, cache root, or managed cache directories are symbolic links. This prevents an attacker-controlled link from redirecting cleanup into an unrelated directory. The command does not attempt to clean arbitrary user directories or global Cargo caches.

## Panic-free input handling

Normal user-controlled CLI arguments, project contents, and environment failures should return structured errors rather than trigger `unwrap`/`expect` panics. CI includes an audit guard over production code plus adversarial CLI coverage for invalid diff-base input.

## Report output boundary

Soro Mutants refuses to open a report destination when the destination itself is a symbolic link. This prevents `--output` from silently following a pre-existing link and overwriting its target. Parent-directory topology remains under the caller's control; use a trusted report directory in hostile multi-user environments.
## Secret redaction in diagnostics

Test subprocess stderr and actionable CLI error messages are sanitized for values of environment variables with credential-like names (such as `*_TOKEN`, `*_SECRET`, `*_PASSWORD`, `*_API_KEY`, or `*_CREDENTIAL*`). Nonempty values of four bytes or more are replaced with `[REDACTED]`. The `test` JSON report's `metadata.test_command` and the text baseline command are sanitized as well. This protection does not deliberately alter original/replacement Rust source excerpts: users must not put credentials into source fixtures. Environment values with non-credential variable names, very short values, and multiline output that splits a secret across lines are outside the automated filter; scrub these before using the tool and avoid passing secrets on the command line.

Secret-bearing command lines should not be used: shell history, process listings and internal resumable-run identity files can retain a literal command. Prefer injecting secret values through protected CI environment variables rather than interpolating them into `--test-command`. This redaction boundary is defense in depth, not a complete secret-handling system.

## Dependency scanning

CI checks the root Rust dependency lockfile against the RustSec advisory database. See [dependency scanning policy](docs/dependency-auditing.md) for investigation, review and time-bounded exceptions.
