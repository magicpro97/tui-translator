#!/usr/bin/env python3
"""TODO/FIXME issue-reference gate for issue #483 (STD-01).

Every TODO/FIXME/HACK/XXX comment in tracked source must reference an
issue number using `#NNN` syntax, e.g.:

    // TODO(#123): wire up retry budget

Usage:
    python scripts/standards/check_todo_refs.py [paths...]

If no paths are supplied, scans the working tree (src/, tests/, scripts/).
"""
from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

MARKERS = re.compile(r"\b(TODO|FIXME|HACK|XXX)\b")
ISSUE_REF = re.compile(r"#\d{1,6}")
EXTS = {".rs", ".py", ".ps1", ".sh", ".toml", ".md", ".yml", ".yaml"}
SKIP_DIRS = {"target", ".git", "node_modules", "dist", "build"}

SELF_PATHS = {
    "scripts/standards/check_todo_refs.py",
    "scripts/standards/tests/test_standards.py",
    "docs/engineering-standards.md",
    ".github/workflows/standards.yml",
    ".github/pull_request_template.md",
}


def iter_files(roots):
    for root in roots:
        p = Path(root)
        if p.is_file():
            if p.suffix in EXTS:
                yield p
            continue
        for dirpath, dirnames, filenames in os.walk(p):
            dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
            for n in filenames:
                f = Path(dirpath) / n
                if f.suffix in EXTS:
                    yield f


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("paths", nargs="*", default=None)
    args = ap.parse_args()

    roots = args.paths or ["src", "tests", "scripts"]
    failures: list[str] = []
    checked = 0

    for f in iter_files(roots):
        rel = f.as_posix()
        if rel in SELF_PATHS or rel.endswith(tuple(SELF_PATHS)):
            continue
        checked += 1
        try:
            for lineno, line in enumerate(f.open(encoding="utf-8", errors="replace"), 1):
                if MARKERS.search(line) and not ISSUE_REF.search(line):
                    failures.append(f"{rel}:{lineno}: {line.rstrip()}")
        except OSError as e:
            print(f"warning: cannot read {rel}: {e}", file=sys.stderr)

    if failures:
        print("TODO/FIXME issue-reference gate failed:")
        for line in failures[:200]:
            print(f"  - {line}")
        if len(failures) > 200:
            print(f"  ...and {len(failures) - 200} more")
        print("\nFix: rewrite the marker to include an issue, e.g. "
              "`TODO(#123): explain why`.")
        return 1
    print(f"TODO/FIXME gate OK ({checked} file(s) scanned).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
