#!/usr/bin/env bash
# Rift tracks the newest Roc nightly (roc-lang/nightlies; the owner's policy). `mise run
# roc-bump`: find the newest nightly, install it (the compiler, its `-safe` build, and its source
# for the glue spec) beside the others under $ROC_NIGHTLY_HOME (default
# ~/.local/share/roc-nightly), point `current`, `safe` and `src-current` at it, write
# roc/NIGHTLY, and run the check run (roc/check.sh). A failed check run puts the old pin and links
# back and names the checks that broke (their logs in target/check-logs); the newest nightly
# stays installed for a repro. `--warn-if-stale` only warns when roc/NIGHTLY is older than the
# newest nightly, and says nothing when the newest cannot be read (offline).
set -euo pipefail
cd "$(dirname "$0")/.."
home=${ROC_NIGHTLY_HOME:-$HOME/.local/share/roc-nightly}
pinned=$(cat roc/NIGHTLY)

# The newest nightly's tag (`nightly-YYYY-MM-DD-<commit>`), or nothing when it cannot be read.
newest() {
    curl -fsS --max-time 10 https://api.github.com/repos/roc-lang/nightlies/releases/latest 2>/dev/null |
        sed -n 's/^ *"tag_name": *"\(nightly-[0-9-]*-[0-9a-f]*\)".*/\1/p' | head -1
}

if [ "${1:-}" = "--warn-if-stale" ]; then
    latest=$(newest || true)
    if [ -n "$latest" ] && [[ "$latest" > "$pinned" ]]; then
        echo "warning: roc/NIGHTLY is $pinned; the newest Roc nightly is $latest (mise run roc-bump)"
    fi
    exit 0
fi

latest=$(newest || true)
[ -n "$latest" ] || { echo "roc-bump: cannot read the newest nightly (offline?)" >&2; exit 1; }
if ! [[ "$latest" > "$pinned" ]]; then
    echo "roc-bump: $pinned is the newest nightly"
    exit 0
fi
stamp=${latest#nightly-}

# Downloads `asset` of release `tag` and unpacks it into `dir` (its top directory is the
# asset's name without .tar.gz), unless already there.
fetch() {
    local tag=$1 asset=$2 dir=$3
    local name=${asset%.tar.gz}
    mkdir -p "$dir"
    if [ ! -d "$dir/$name" ]; then
        gh release download -R roc-lang/nightlies "$tag" -p "$asset" -D "$dir" --clobber
        tar -xzf "$dir/$asset" -C "$dir"
    fi
    echo "$dir/$name"
}

compiler=$(fetch "$latest" "roc_nightly-macos_apple_silicon-$stamp.tar.gz" "$home/dl")
safe=$(fetch "$latest-safe" "roc_nightly-macos_apple_silicon-$stamp-safe.tar.gz" "$home/dl-safe")
source=$(fetch "$latest" "roc_nightly-source-$stamp.tar.gz" "$home/src-dl")
[ -x "$compiler/roc" ] || { echo "roc-bump: no roc in $compiler" >&2; exit 1; }
[ -f "$source/src/glue/src/RustGlue.roc" ] || { echo "roc-bump: no RustGlue.roc in $source" >&2; exit 1; }

# The links as they are, to put back on a failure.
link() { readlink "$home/$1" 2>/dev/null || true; }
old_current=$(link current) old_safe=$(link safe) old_src=$(link src-current)
point() { ln -sfn "$2" "$home/$1"; }
restore() {
    for pair in "current:$old_current" "safe:$old_safe" "src-current:$old_src"; do
        if [ -n "${pair#*:}" ]; then point "${pair%%:*}" "${pair#*:}"; else rm -f "$home/${pair%%:*}"; fi
    done
    echo "$pinned" >roc/NIGHTLY
}

point current "$compiler"
point safe "$safe"
point src-current "$source"
echo "$latest" >roc/NIGHTLY
echo "roc-bump: $pinned -> $latest; running the check run"
# The dev backend is not used (docs/roc-fork.md, "The dev backend"); each bump says whether
# rift-core-roc's layout tests, where it crashed, now pass under it. Never a gate. Its own target
# directory: the Roc archive is bundled into rift-core-roc's rlib.
if ROC_OPT=dev CARGO_TARGET_DIR=target/roc-dev cargo test -q -p rift-core-roc --features roc --test layout >target/roc-dev.log 2>&1; then
    echo "roc-bump: rift-core-roc's layout tests pass under --opt=dev at $latest; the dev backend may be worth retrying (docs/roc-fork.md)"
else
    echo "roc-bump: rift-core-roc's layout tests still fail under --opt=dev at $latest (target/roc-dev.log)"
fi
# The same crash reduced (roc/repros/recursive-nominal-payload): a nominal record as a recursive
# union's variant payload, which Boxy lowering (--specialize=no, the default for --opt=size and
# --opt=dev) misreads; rift builds with --specialize=yes. Never a gate: each probe says whether
# Boxy still has the bug.
for probe in "dev" "size" "speed no"; do
    set -- $probe
    if ROC="$home/current/roc" roc/repros/recursive-nominal-payload/run.sh "$@" 2>/dev/null | grep -qx 7; then
        echo "roc-bump: the recursive-nominal-payload repro passes under --opt=$1${2:+ --specialize=$2} at $latest (report it)"
    else
        echo "roc-bump: the recursive-nominal-payload repro still crashes under --opt=$1${2:+ --specialize=$2} at $latest"
    fi
done
if roc/check.sh; then
    echo "roc-bump: $latest is green; commit roc/NIGHTLY"
else
    restore
    echo "roc-bump: $latest broke the checks above (logs in target/check-logs); the pin stays $pinned" >&2
    exit 1
fi
