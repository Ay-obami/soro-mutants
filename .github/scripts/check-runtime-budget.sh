#!/usr/bin/env bash
set -euo pipefail

# check-runtime-budget.sh
#
# Enforces tolerant runtime budgets for CI test suite runs to catch accidental large
# performance regressions early without causing flaky false alarms on loaded runners.

show_usage() {
  cat << 'EOF' >&2
usage: check-runtime-budget.sh <budget-seconds> <command...>
       check-runtime-budget.sh --self-test

Examples:
  check-runtime-budget.sh 300 cargo test --all
  check-runtime-budget.sh --self-test
EOF
}

run_self_test() {
  echo "[runtime-budget] Running self-test and regression verification..."

  # 1. Fast command must succeed under generous budget
  local fast_start
  fast_start=$(date +%s)
  "$0" 10 bash -c "true"
  echo "[runtime-budget] Self-test passed: Fast command succeeded within budget."

  # 2. Regression seed: Simulated slow command exceeding budget must fail with exit code 1
  set +e
  local slow_out
  slow_out=$("$0" 1 bash -c "sleep 2" 2>&1)
  local slow_status=$?
  set -e

  if [[ $slow_status -ne 1 ]]; then
    echo "[runtime-budget] Self-test failed: expected exit code 1 on budget violation, got $slow_status" >&2
    exit 2
  fi

  if ! grep -q "ERROR: Runtime budget exceeded" <<< "$slow_out"; then
    echo "[runtime-budget] Self-test failed: missing expected budget violation message in: $slow_out" >&2
    exit 2
  fi

  echo "[runtime-budget] Self-test passed: Budget violation regression caught as expected."
  echo "[runtime-budget] All self-tests passed."
  exit 0
}

if [[ $# -lt 1 ]]; then
  show_usage
  exit 2
fi

if [[ "$1" == "--self-test" ]]; then
  run_self_test
fi

if [[ $# -lt 2 ]]; then
  show_usage
  exit 2
fi

BUDGET_SECONDS="$1"
shift
COMMAND=("$@")

if ! [[ "$BUDGET_SECONDS" =~ ^[0-9]+$ ]] || [[ "$BUDGET_SECONDS" -le 0 ]]; then
  echo "error: budget-seconds must be a positive integer, got '$BUDGET_SECONDS'" >&2
  exit 2
fi

echo "======================================================================"
echo "[runtime-budget] Target budget: ${BUDGET_SECONDS}s"
echo "[runtime-budget] Executing command: ${COMMAND[*]}"
echo "======================================================================"

START_TIME=$(date +%s)

set +e
"${COMMAND[@]}"
CMD_EXIT_CODE=$?
set -e

END_TIME=$(date +%s)
ELAPSED_SECONDS=$(( END_TIME - START_TIME ))

echo "======================================================================"
echo "[runtime-budget] Command exit status: ${CMD_EXIT_CODE}"
echo "[runtime-budget] Elapsed time:        ${ELAPSED_SECONDS}s"
echo "[runtime-budget] Permitted budget:   ${BUDGET_SECONDS}s"
echo "======================================================================"

if [[ $CMD_EXIT_CODE -ne 0 ]]; then
  echo "[runtime-budget] Command failed with exit status ${CMD_EXIT_CODE}" >&2
  exit "$CMD_EXIT_CODE"
fi

if [[ $ELAPSED_SECONDS -gt $BUDGET_SECONDS ]]; then
  echo "ERROR: Runtime budget exceeded!" >&2
  echo "The execution took ${ELAPSED_SECONDS}s, which exceeds the tolerant threshold of ${BUDGET_SECONDS}s." >&2
  echo "Please check if recent changes introduced unbounded loops, slow dependencies, or test performance regressions." >&2
  exit 1
fi

echo "[runtime-budget] SUCCESS: Runtime within acceptable budget (${ELAPSED_SECONDS}s <= ${BUDGET_SECONDS}s)"
exit 0
