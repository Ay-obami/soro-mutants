# CLI Filter Parsing Fuzz Target

This document explains the fuzzing target and fuzz-testing harnesses implemented for Soro Mutants CLI filter parsing (`--operator`, `--exclude-path`, `--shard`, `--file`, `--function`).

## Objective

Catch panics, unexpected crashes, infinite loops, and pathological behavior when parsing CLI filters across:
- **Operator IDs** (`--operator`): unknown IDs, non-ASCII characters, null bytes, unbounded strings.
- **Path globs** (`--exclude-path`): unbalanced brackets, malformed regex/glob syntax, pathological recursion.
- **Shard specifiers** (`--shard`): invalid delimiters, 0-division, index exceeding total, integer overflows, non-numeric strings.
- **File and function filters** (`--file`, `--function`): null bytes, control characters, path-traversal strings, unbounded identifiers.

## Implementation Structure

- Core parsing and validation logic resides in `src/filters.rs`.
- Structured error enum `FilterParseError` returns actionable diagnostic errors with zero panics.
- Continuous automated property and randomized stress tests are run in `tests/filter_fuzz_cli.rs`.
- LLVM LibFuzzer integration target resides in `fuzz/fuzz_targets/filter_parser.rs`.

## Running Locally

### Standard Regression and Property Test (Fast, bounded)
Run the automated fuzz harness using standard cargo:

```bash
cargo test --test filter_fuzz_cli
```

This runs:
1. Determinism verification on all valid inputs.
2. Boundary and pathological regression suites.
3. 5,000 randomized pseudo-fuzz strings covering edge cases.
4. CLI subprocess execution verifying code 2 exit status on malformed arguments.

### Running with `cargo-fuzz` (LibFuzzer)
To run continuous fuzzing with LLVM LibFuzzer:

1. Install `cargo-fuzz`:
   ```bash
   cargo install cargo-fuzz
   ```

2. Run the `filter_parser` fuzz target:
   ```bash
   cargo fuzz run filter_parser -- -max_total_time=60
   ```

## Seed Corpus and Regression Seeds

Seeds that test boundary conditions are embedded in `tests/filter_fuzz_cli.rs` and cover:
- Empty strings `""`
- Null bytes `"\0"`
- Trailing slashes, missing slashes, extra slashes (`"1/"`, `"/1"`, `"1/2/3"`)
- Zero indices and totals (`"0/0"`, `"0/1"`, `"1/0"`)
- Overflowing integers (`"1/999999999999999999999999999999999999"`)
- Pathological globs (`"["`, `"**/**/[a-"`)
- Unbounded character repetitions
