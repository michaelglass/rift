#!/usr/bin/env python3
"""The clippy gate: the workspace's warnings may only go down.

`roc/clippy-baseline.txt` lists the warnings the workspace has, each keyed by its lint, its
file and the item it is in (the nearest item declaration above it), never its line: an
unrelated edit does not churn the list. A count says how many such warnings there are.

    clippy_gate.py check  LOG BASELINE      exit 1 on a warning not in the baseline (listed),
                                            or on one the baseline lists that is gone (listed:
                                            shrink the baseline in the same commit)
    clippy_gate.py shrink LOG BASELINE      rewrite the baseline without the warnings that are
                                            gone; it never adds one
    clippy_gate.py write  LOG BASELINE      write the baseline from scratch (its first time)

LOG is `cargo clippy --message-format=json` output from a run that re-linted the workspace's own
crates (roc/check.sh passes clippy a new `--cfg clippy_gate` nonce each run, which re-lints
exactly them). Python 3's standard library only.
"""

import json
import os
import re
import sys
from collections import Counter

ITEM = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|unsafe|const|default|extern(?:\s+\"[^\"]*\")?)\s+)*"
    r"(fn|struct|enum|trait|impl|mod|type|static|const|union|macro_rules!)\b"
)


def item_of(path, line, cache={}):
    """The nearest item declaration at or above `line` in `path`, as written up to its body."""
    lines = cache.get(path)
    if lines is None:
        try:
            with open(path, encoding="utf-8") as f:
                lines = f.read().split("\n")
        except OSError:
            lines = []
        cache[path] = lines
    for text in reversed(lines[:line]):
        if ITEM.match(text):
            # The visibility's parentheses are not the item's: strip it before cutting at one.
            bare = re.sub(r"^\s*pub(?:\([^)]*\))?\s+", "", text)
            head = re.split(r"[{(=;]", bare.strip(), maxsplit=1)[0]
            return " ".join(head.split())[:100]
    return "(top)"


def current(log_path):
    """The warnings in a clippy JSON log, as a multiset of (lint, file, item)."""
    root = os.getcwd()
    seen = set()
    keys = Counter()
    with open(log_path, encoding="utf-8") as f:
        for raw in f:
            if not raw.startswith("{"):
                continue
            message = json.loads(raw)
            if message.get("reason") != "compiler-message":
                continue
            m = message["message"]
            if m.get("level") not in ("warning", "error") or not m.get("spans"):
                continue
            span = next((s for s in m["spans"] if s.get("is_primary")), m["spans"][0])
            lint = (m.get("code") or {}).get("code") or m["message"].split(":")[0]
            path = span["file_name"]
            absolute = path if os.path.isabs(path) else os.path.join(root, path)
            shown = os.path.relpath(absolute, root)
            if shown.startswith(".."):
                shown = os.path.basename(path)
            # One warning compiled into several targets (a lib and its tests) counts once.
            place = (lint, shown, span["line_start"], span["column_start"], m["message"])
            if place in seen:
                continue
            seen.add(place)
            keys[(lint, shown, item_of(absolute, span["line_start"]))] += 1
    return keys


def read(baseline_path):
    keys = Counter()
    with open(baseline_path, encoding="utf-8") as f:
        for line in f:
            if line.startswith("#") or not line.strip():
                continue
            count, lint, path, item = line.rstrip("\n").split("\t")
            keys[(lint, path, item)] = int(count)
    return keys


def write(baseline_path, keys):
    with open(baseline_path, "w", encoding="utf-8") as f:
        f.write("# The workspace's clippy warnings: count, lint, file, item. Only ever shrinks\n")
        f.write("# (roc/clippy_gate.py; roc/check.sh fails on anything not listed here).\n")
        for (lint, path, item), count in sorted(keys.items()):
            f.write(f"{count}\t{lint}\t{path}\t{item}\n")


def show(title, keys):
    print(title)
    for (lint, path, item), count in sorted(keys.items()):
        print(f"  {count} x {lint} in {path} ({item})")


def main(argv):
    command, log_path, baseline_path = argv
    now = current(log_path)
    if command == "write":
        write(baseline_path, now)
        return 0
    baseline = read(baseline_path)
    new = now - baseline
    gone = baseline - now
    if command == "shrink":
        write(baseline_path, baseline - gone)
        if new:
            show("not shrunk past these, which the baseline does not list:", new)
            return 1
        return 0
    if new:
        show("new clippy warnings (fix them; the baseline only shrinks):", new)
    if gone:
        show(
            "gone since the baseline (shrink it in this commit: roc/clippy_gate.py shrink):", gone
        )
    print(f"clippy: {sum(now.values())} warnings, baseline {sum(baseline.values())}")
    return 1 if new or gone else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
