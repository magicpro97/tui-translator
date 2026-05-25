#!/usr/bin/env python3
"""`unwrap`/`expect`/`panic!` gate for non-test Rust code (issue #483).

Scans only NEW or MODIFIED lines (via `--added-lines-file`) so existing
debt is not retroactively blocked. In PR mode, the workflow extracts the
added-lines list from `git diff`.

Without `--added-lines-file`, runs in advisory mode and reports counts
only.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

PATTERNS = re.compile(r"\b(unwrap\(\)|expect\([^)]*\)|panic!\s*\()")
TEST_HINTS = ("/tests/", "tests/", "/benches/", "benches/")


def is_test_path(path: str) -> bool:
    p = path.replace("\\", "/")
    return any(h in p for h in TEST_HINTS)


def is_test_line_context(path: Path, lineno: int) -> bool:
    try:
        text = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return False
    start = max(0, lineno - 300)
    window = text[start:lineno]
    block = "\n".join(window)
    return ("#[cfg(test)]" in block) or ("#[test]" in block)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--added-lines-file", default=None,
                    help="File with `path:lineno` entries from PR diff.")
    ap.add_argument("--root", default=".")
    args = ap.parse_args()

    if not args.added_lines_file:
        print("check_unwrap: advisory mode (no diff supplied); skipping.")
        return 0

    failures: list[str] = []
    root = Path(args.root)
    try:
        entries = Path(args.added_lines_file).read_text(encoding="utf-8").splitlines()
    except OSError as e:
        print(f"error: cannot read added-lines file: {e}", file=sys.stderr)
        return 2

    for entry in entries:
        entry = entry.strip()
        if not entry or ":" not in entry:
            continue
        path_str, _, lineno_str = entry.rpartition(":")
        if not path_str.endswith(".rs"):
            continue
        if is_test_path(path_str):
            continue
        try:
            lineno = int(lineno_str)
        except ValueError:
            continue
        f = root / path_str
        if not f.exists():
            continue
        try:
            lines = f.read_text(encoding="utf-8", errors="replace").splitlines()
            line = lines[lineno - 1]
        except (OSError, IndexError):
            continue
        if PATTERNS.search(line) and not is_test_line_context(f, lineno):
            if "allow-unwrap" in line and re.search(r"#\d+", line):
                continue
            failures.append(f"{path_str}:{lineno}: {line.strip()}")

    if failures:
        print("unwrap/expect/panic! gate failed on NEW non-test code:")
        for line in failures:
            print(f"  - {line}")
        print("\nFix: return `Result`/`anyhow::Error`, or annotate the line "
              "with `// allow-unwrap: #NNN` referencing a tracking issue.")
        return 1
    print("unwrap/expect/panic! gate OK.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
