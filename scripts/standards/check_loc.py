#!/usr/bin/env python3
"""LOC gate for issue #483 (STD-01).

Fails when a non-waivered Rust source file exceeds 600 physical lines.
The waiver file `.standards-waivers.txt` lists exempt baseline files;
new files must NOT be added there.

Usage:
    python scripts/standards/check_loc.py [--root .] [--max-lines 600]
                                          [--waivers .standards-waivers.txt]
                                          [--only path1 path2 ...]

`--only` restricts the check to the given paths (used in PR diff mode).
"""
from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

DEFAULT_MAX = 600


def load_waivers(path: Path) -> set[str]:
    waived: set[str] = set()
    if not path.exists():
        return waived
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].strip()
        if line:
            waived.add(line.replace("\\", "/"))
    return waived


def iter_rs_files(root: Path):
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in {"target", ".git", "node_modules"}]
        for name in filenames:
            if name.endswith(".rs"):
                yield Path(dirpath) / name


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=".")
    ap.add_argument("--max-lines", type=int, default=DEFAULT_MAX)
    ap.add_argument("--waivers", default=".standards-waivers.txt")
    ap.add_argument("--only", nargs="*", default=None,
                    help="Restrict check to these paths (PR diff mode).")
    args = ap.parse_args()

    root = Path(args.root).resolve()
    waivers = load_waivers(Path(args.waivers))

    if args.only:
        candidates = [Path(p) for p in args.only if p.endswith(".rs")]
    else:
        candidates = list(iter_rs_files(root))

    failures: list[str] = []
    for f in candidates:
        if not f.exists():
            continue
        try:
            rel = f.resolve().relative_to(root).as_posix()
        except ValueError:
            rel = f.as_posix()
        try:
            count = sum(1 for _ in f.open(encoding="utf-8", errors="replace"))
        except OSError as e:
            print(f"warning: cannot read {rel}: {e}", file=sys.stderr)
            continue
        if count > args.max_lines and rel not in waivers:
            failures.append(f"{rel}: {count} lines (limit {args.max_lines})")

    if failures:
        print("LOC gate failed for the following files:")
        for line in failures:
            print(f"  - {line}")
        print("\nFix: split the file into smaller modules, or add a waiver with a "
              "linked refactor issue to .standards-waivers.txt (discouraged).")
        return 1
    print(f"LOC gate OK ({len(candidates)} file(s) checked, limit {args.max_lines}).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
