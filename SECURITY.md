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
