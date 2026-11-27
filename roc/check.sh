#!/usr/bin/env bash
# The full check run (`mise run check`): fmt, `cargo check --locked` with and without
# `roc-core`, the workspace tests, the lib with `roc-core`, the Roc crate's differentials,
# rift-core, the drag fuzzer both ways, and clippy with `roc-core`. One line per check; each
# check's output in target/check-logs/<check>.log. Exits 1 if any check failed. First, a warning
# (never a failure) when roc/NIGHTLY is older than the newest Roc nightly; nothing when offline.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
logs=target/check-logs
mkdir -p "$logs"

roc/bump.sh --warn-if-stale

failed=0
run() {
    local name=$1
    shift
    "$@" >"$logs/$name.log" 2>&1
    local status=$?
    local results
    results=$(grep -E '^test result' "$logs/$name.log" | awk '{p+=$4; f+=$6; i+=$8} END {if (NR) printf "%d passed %d failed %d ignored", p, f, i}')
    echo "$name: exit $status $results"
    [ "$status" -eq 0 ] || failed=1
}

run fmt cargo +nightly fmt --all --check
run check-plain cargo check --locked --workspace --all-targets
run check-roc cargo check --locked --workspace --all-targets --features roc-core
run workspace cargo test --workspace
run lib-roc cargo test --lib --features roc-core
run core-roc cargo test -p rift-core-roc --features roc
run core cargo test -p rift-core
run fuzz-plain env RIFT_FUZZ_CASES=2000 RIFT_FUZZ_STEPS=40 cargo test --lib drag_invariants_hold_across_interleavings
run fuzz-roc env RIFT_FUZZ_CASES=2000 RIFT_FUZZ_STEPS=40 cargo test --lib --features roc-core drag_invariants_hold_across_interleavings
cargo clippy --all-targets --features roc-core >"$logs/clippy.log" 2>&1
status=$?
warnings=$(grep -E '^(warning|error)' "$logs/clippy.log" | grep -cv generated)
echo "clippy: exit $status $warnings (errors $(grep -c '^error' "$logs/clippy.log"))"
[ "$status" -eq 0 ] || failed=1

exit "$failed"
