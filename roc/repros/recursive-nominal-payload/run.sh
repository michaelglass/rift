#!/usr/bin/env bash
# usage: ./run.sh speed|size|dev
set -u
cd "$(dirname "$0")"
${ROC:-roc} build --target=arm64mac --output=app.a --opt="${1:-dev}" main.roc >/dev/null || exit 1
cc host.c app.a -o host -Wl,-no_fixup_chains 2>/dev/null && ./host
echo "exit $?"
rm -f host app.a
