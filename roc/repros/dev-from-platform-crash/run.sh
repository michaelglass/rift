#!/usr/bin/env bash
# usage: ./run.sh dev|speed
set -u
cd "$(dirname "$0")"
${ROC:-roc} build --target=arm64mac --output=app.a --opt="${1:-dev}" main.roc >/dev/null || exit 1
otool -l app.a | grep -A3 'sectname __const' | grep -A2 'segname __DATA' | sed -n 's/.*addr /__DATA,__const at /p'
cc host.c app.a -o host && ./host
status=$?
rm -f host app.a
exit "$status"
