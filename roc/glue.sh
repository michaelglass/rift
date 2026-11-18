#!/usr/bin/env bash
# Regenerates the Rust glue for roc/platform into crates/rift-core-roc/src/roc_platform_abi.rs.
# Run after every Roc nightly bump: the glue ABI is compiler-defined and changes between
# nightlies. ROC names the compiler (default: the pinned nightly under ~/.local/share);
# ROC_SRC a checkout of roc-lang/roc at the same commit, for the glue spec (RustGlue.roc).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
roc="${ROC:-$HOME/.local/share/roc-nightly/current/roc}"
src="${ROC_SRC:-$HOME/.local/share/roc-nightly/src}"
pinned="$(cat "$here/NIGHTLY")"
"$roc" version | grep -q "${pinned#nightly-}" || {
  echo "glue.sh: $roc is not the pinned $pinned" >&2
  exit 1
}
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
"$roc" glue "$src/src/glue/src/RustGlue.roc" "$out" "$here/platform/main.roc"
cp "$out/roc_platform_abi.rs" "$here/../crates/rift-core-roc/src/roc_platform_abi.rs"
echo "glue.sh: regenerated crates/rift-core-roc/src/roc_platform_abi.rs"
