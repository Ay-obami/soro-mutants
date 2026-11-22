#!/usr/bin/env bash
set -euo pipefail

# check-dependency-graph.sh
#
# Audits the committed root Cargo.lock for duplicate crate names and the root
# Cargo.toml for direct dependencies no longer referenced under src/. Vendor-
# specific audit databases and network access are deliberately not required:
# the checks are pure functions of the checked-in files, so results are stable
# across machines and CI runners.

PREFIX="[dependency-graph]"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LOCKFILE="$ROOT/Cargo.lock"
MANIFEST="$ROOT/Cargo.toml"
POLICY="$ROOT/.cargo/dependency-hygiene.toml"

show_usage() {
  cat << 'EOF' >&2
usage: check-dependency-graph.sh
       check-dependency-graph.sh --self-test
EOF
}

# Emit "name version" pairs for every package in a Cargo.lock.
lock_pairs() {
  awk '
    /^\[\[package\]\]/ { in_package=1; next }
    /^\[\[/ { in_package=0 }
    in_package && /^name = / { name=$3; gsub(/"/, "", name) }
    in_package && /^version = / { version=$3; gsub(/"/, "", version); if (name != "" && version != "") print name " " version }
  ' "$1" | sort -u
}

# Emit "name|version1 version2" for each crate name appearing at >1 version.
duplicate_names() {
  awk '
    { count[$1]++; versions[$1] = versions[$1] " " $2 }
    END { for (name in count) if (count[name] > 1) print name "|" versions[name] }
  ' "$1" | sort
}

allowed_names() {
  if [[ ! -f "$POLICY" ]]; then
    echo "$PREFIX policy file missing: $POLICY" >&2
    exit 2
  fi
  awk '/^allow = \[/{flag=1} flag{print} /^\]/{flag=0}' "$POLICY" \
    | grep -o '"[^"]*"' | tr -d '"' | sort -u || true
}

direct_dependency_names() {
  awk '
    /^\[dependencies\]/ { in_section=1; next }
    /^\[/ { in_section=0 }
    in_section && /^[A-Za-z0-9_-]+[[:space:]]*=/ {
      name=$1; sub(/[[:space:]]*=.*/, "", name); print name
    }
  ' "$1" | sort -u
}

audit() {
  local lock="$1" manifest="$2"
  local status=0
  local pairs duplicates allowed line name versions dep crate

  pairs=$(mktemp)
  lock_pairs "$lock" > "$pairs"
  duplicates=$(duplicate_names "$pairs")
  allowed=$(allowed_names)

  if [[ -n "$duplicates" ]]; then
    while IFS= read -r line; do
      name="${line%%|*}"
      versions="${line#*|}"
      if grep -Fxq "$name" <<< "$allowed"; then
        echo "$PREFIX reviewed duplicate: $name versions:$versions (see .cargo/dependency-hygiene.toml)"
        continue
      fi
      echo "$PREFIX ERROR: duplicate crate $name at versions:$versions has no reviewed exception" >&2
      status=1
    done <<< "$duplicates"
  fi

  while IFS= read -r dep; do
    [[ -z "$dep" ]] && continue
    crate="${dep//-/_}"
    if ! grep -Rqs --include='*.rs' -E "(^|[^[:alnum:]_])${crate}([^[:alnum:]_]|$)" "$ROOT/src"; then
      echo "$PREFIX ERROR: direct dependency '$dep' is not referenced under src/" >&2
      status=1
    fi
  done < <(direct_dependency_names "$manifest")

  rm -f "$pairs"
  if [[ $status -eq 0 ]]; then
    echo "$PREFIX dependency graph is clean"
  fi
  return "$status"
}

run_self_test() {
  echo "$PREFIX running self-test and regression verification..."
  local work pairs detected
  work=$(mktemp -d)
  cat > "$work/duplicate.lock" << 'EOF'
[[package]]
name = "serde_json"
version = "1.0.1"

[[package]]
name = "serde_json"
version = "1.0.2"

[[package]]
name = "syn"
version = "2.0.119"

[[package]]
name = "syn"
version = "3.0.6"
EOF

  pairs=$(mktemp)
  lock_pairs "$work/duplicate.lock" > "$pairs"
  detected=$(duplicate_names "$pairs")
  if ! grep -q '^serde_json|' <<< "$detected" || ! grep -q '^syn|' <<< "$detected"; then
    echo "$PREFIX self-test failed: injected duplicates were not detected" >&2
    rm -rf "$work" "$pairs"
    exit 2
  fi
  echo "$PREFIX self-test passed: injected duplicate crate is detected"

  allowed=$(allowed_names)
  if ! grep -Fxq "syn" <<< "$allowed"; then
    echo "$PREFIX self-test failed: reviewed syn exception is missing from policy" >&2
    rm -rf "$work" "$pairs"
    exit 2
  fi
  echo "$PREFIX self-test passed: reviewed duplicate exception is honored"

  rm -rf "$work" "$pairs"
  echo "$PREFIX all self-tests passed"
  exit 0
}

case "${1:-}" in
  "") audit "$LOCKFILE" "$MANIFEST" ;;
  --self-test) run_self_test ;;
  *)
    echo "$PREFIX ERROR: unknown argument $1" >&2
    show_usage
    exit 2
    ;;
esac
