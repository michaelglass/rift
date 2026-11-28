#!/usr/bin/env bash
# The check runs. `roc/check.sh` (`mise run check`) is the full run, the publish gate: fmt,
# `cargo check --locked` with and without `roc-core`, every test (the workspace, the lib with
# `roc-core`, the Roc crate's differentials, rift-core), the drag fuzzer at 2000 x 40 both ways,
# and clippy with `roc-core`. `roc/check.sh fast` (`mise run check-fast`) is the inner loop: fmt,
# the `roc-core` check, the lib with `roc-core`, rift-core-roc and rift-core, each drawn test cut
# to the counts in FAST_CUTS; it is not a publish gate.
#
# Everything is built first; then the test runs, which share no build, run side by side: the two
# lib runs and the fuzzers through libtest (one process: nextest's process per test costs about
# 0.4 s of system time each for the lib's 950 tests), the crates with few long tests through
# cargo-nextest (the project's pinned version, mise.toml), across binaries. One line per check;
# each check's output in target/check-logs/<check>.log. Exits 1 if any check failed. First, a
# warning (never a failure) when roc/NIGHTLY is older than the newest Roc nightly; nothing when
# offline.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
mode=${1:-full}
logs=target/check-logs
mkdir -p "$logs"
rm -f "$logs"/*.log

# What check-fast cuts: each drawn test's count (its default in the comment).
FAST_CUTS=(
    RIFT_FOREST_CASES=300      # rift lib: drawn traditional and master-stack systems (3000)
    RIFT_SCROLLING_CASES=300   # rift lib: drawn scrolling strips (3000; 600 arranged)
    RIFT_FUZZ_CASES=40         # rift lib: the reactor fuzzer's cases (200)
    RIFT_FUZZ_STEPS=20         # rift lib: the reactor fuzzer's steps (30)
    RIFT_LAYOUT_CASES=2000     # rift-core-roc: the layout differentials (20,000)
    RIFT_BSP_TREE_RUNS=800     # rift-core-roc: the BSP tree (8000)
    RIFT_STACK_TREE_RUNS=800   # rift-core-roc: the stacks (8000)
    RIFT_GROUP_FOREST_RUNS=300 # rift-core-roc: the group forest (per test)
    RIFT_COLUMNS_RUNS=400      # rift-core-roc: the scrolling columns (4000)
    RIFT_WORKSPACES_CASES=2000 # rift-core-roc: the workspaces (20,000)
    RIFT_MAIN_WINDOW_RUNS=100  # rift-core-roc: the main window (1000)
    RIFT_CONSTRAINTS_RUNS=200  # rift-core-roc: the window constraints (2000)
    RIFT_FLOATS_RUNS=200       # rift-core-roc: the floating state (2000)
    RIFT_DISPLAYS_RUNS=200     # rift-core-roc: the display memory (2000)
    RIFT_EVICTIONS_RUNS=200    # rift-core-roc: the evictions (2000)
    RIFT_CAMERAS_RUNS=200      # rift-core-roc: the cameras (2000)
    RIFT_SESSION_ROUNDS=40     # rift-core-roc: the session (200)
)
# The full run's fuzzer budget.
FUZZ=(RIFT_FUZZ_CASES=2000 RIFT_FUZZ_STEPS=40)

roc/bump.sh --warn-if-stale

failed=0
started=$(date +%s)

# `report NAME STATUS`: the check's line, its counts read from its log (libtest's result lines
# or nextest's summary).
report() {
    local name=$1 status=$2 counts
    counts=$(grep -E '^test result' "$logs/$name.log" |
        awk '{p+=$4; f+=$6; i+=$8} END {if (NR) printf "%d passed %d failed %d ignored", p, f, i}')
    if [ -z "$counts" ]; then
        counts=$(sed -n 's/.*Summary \[.*\] *[0-9]* tests* run: \(.*\)/\1/p' "$logs/$name.log" | tail -1)
    fi
    echo "$name: exit $status $counts"
    [ "$status" -eq 0 ] || failed=1
}

# `run NAME CMD...`: a check run in the foreground.
run() {
    local name=$1
    shift
    "$@" >"$logs/$name.log" 2>&1
    report "$name" $?
}

# `spawn NAME CMD...` starts a check in the background; `collect` waits for them in order.
names=()
pids=()
spawn() {
    local name=$1
    shift
    "$@" >"$logs/$name.log" 2>&1 &
    names+=("$name")
    pids+=($!)
}
collect() {
    local index
    for index in "${!pids[@]}"; do
        wait "${pids[$index]}"
        report "${names[$index]}" $?
    done
    names=()
    pids=()
}

# `executable LOG`: the lib's test binary a `cargo test --no-run --message-format=json` built.
executable() {
    python3 -c 'import json, sys
for line in open(sys.argv[1]):
    try:
        message = json.loads(line)
    except ValueError:
        continue
    if message.get("executable") and message.get("target", {}).get("kind") == ["lib"]:
        print(message["executable"])' "$1" | tail -1
}

run fmt cargo +nightly fmt --all --check
if [ "$mode" = full ]; then
    run check-plain cargo check --locked --workspace --all-targets
fi
run check-roc cargo check --locked --workspace --all-targets --features roc-core

# The builds.
run build-lib-roc cargo test --lib --features roc-core --no-run --message-format=json
run build-core-roc cargo nextest run -p rift-core-roc --features roc --no-run
run build-core cargo nextest run -p rift-core --no-run
if [ "$mode" = full ]; then
    run build-lib cargo test --lib --no-run --message-format=json
    run build-workspace cargo nextest run --workspace --no-run
    cargo clippy --all-targets --features roc-core >"$logs/clippy.log" 2>&1
    status=$?
    warnings=$(grep -E '^(warning|error)' "$logs/clippy.log" | grep -cv generated)
    echo "clippy: exit $status $warnings (errors $(grep -c '^error' "$logs/clippy.log"))"
    [ "$status" -eq 0 ] || failed=1
fi
lib_roc=$(executable "$logs/build-lib-roc.log")

# The test runs, side by side.
if [ "$mode" = full ]; then
    lib=$(executable "$logs/build-lib.log")
    spawn lib "$lib"
    spawn lib-roc "$lib_roc"
    spawn core-roc cargo nextest run -p rift-core-roc --features roc --no-fail-fast
    # The rest of the workspace (rift-core, rift-protocol, the binaries, settings_native).
    spawn workspace cargo nextest run --workspace --no-fail-fast -E 'not (package(rift-wm) & kind(lib))'
    spawn fuzz-plain env "${FUZZ[@]}" "$lib" drag_invariants_hold_across_interleavings
    spawn fuzz-roc env "${FUZZ[@]}" "$lib_roc" drag_invariants_hold_across_interleavings
else
    spawn lib-roc env "${FAST_CUTS[@]}" "$lib_roc"
    spawn core-roc env "${FAST_CUTS[@]}" cargo nextest run -p rift-core-roc --features roc --no-fail-fast
    spawn core cargo nextest run -p rift-core --no-fail-fast
fi
collect

echo "total: $(($(date +%s) - started)) s"
if [ "$mode" != full ]; then
    echo "check-fast is not a publish gate: run mise run check before publishing"
fi
exit "$failed"
