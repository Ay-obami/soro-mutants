#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 3 || $# -gt 4 ]]; then
  echo "usage: $0 <weak-fixture> <strong-fixture> <expected-operators-csv> [function-filter]" >&2
  exit 2
fi

weak_fixture=$1
strong_fixture=$2
IFS=',' read -r -a expected_operators <<< "$3"
function_filter=${4:-}

run_case() {
  local strength=$1
  local fixture=$2
  local expected_status=$3
  # Each matrix entry has its own runner. Sharing one target dir inside the
  # weak/strong pair reuses dependency builds without cross-family leakage.
  local target_dir="${RUNNER_TEMP:-/tmp}/soro-mutants-fixtures"

  for expected_operator in "${expected_operators[@]}"; do
    echo "::group::${strength}: ${fixture} / ${expected_operator}"
    args=(test "fixtures/$fixture" --operator "$expected_operator" --target-dir "$target_dir" --timeout 300 --test-command "cargo +1.91.0 test -q --locked")
    if [[ -n "$function_filter" ]]; then
      args+=(--function "$function_filter")
    fi
    output=$(./target/debug/cargo-soro-mutants "${args[@]}")
    printf '%s\n' "$output"

    if ! grep -Eq "^${expected_status}[[:space:]]+${expected_operator}([[:space:]]|$)" <<<"$output"; then
      echo "expected ${expected_status} result for ${expected_operator} in ${fixture}" >&2
      exit 1
    fi

    if grep -Eq '^(UNVIABLE|TIMEOUT)([[:space:]]|$)' <<<"$output"; then
      echo "unexpected UNVIABLE/TIMEOUT result for ${expected_operator} in ${fixture}" >&2
      exit 1
    fi
    echo "::endgroup::"
  done
}

run_case weak "$weak_fixture" SURVIVED
run_case strong "$strong_fixture" KILLED
