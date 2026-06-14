#!/usr/bin/env python3
"""Per-file coverage gate for v1-critical layers.

WP-25.05 (coverage-100% follow-up): the user asked for 100%
branch coverage on the v1-critical layers.  The project
has 92 production files across 5 layers; getting every
file to 100% in one PR is not realistic.  This gate instead
enforces the per-PR discipline: a PR that adds or modifies
a file in a v1-critical layer must bring that file to 100%
line+branch coverage.  Other files may be below 100%; the
total may be below 100% — the gate is per-file, not aggregate.

How it works:

1. The caller (CI step) passes the list of files touched by
   the PR (`--changed-files`).
2. For each touched file that is under a v1-critical prefix,
   the gate reads the lcov entry and asserts line + branch
   coverage is 100%.
3. If any file is below 100%, the gate fails with a per-file
   breakdown.

Usage::

    python scripts/ci/per_file_coverage_gate.py \\
        --lcov target/evidence/lcov.info \\
        --changed-files changed_files.txt \\
        --layers src/audio/,src/session/,src/providers/,src/pipeline/,src/tui/

Exit codes:
    0 — all touched v1-critical files are at 100%
    1 — at least one touched file is below 100%
    2 — invalid invocation
    3 — lcov file not found
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import List, Optional


@dataclass
class FileCoverage:
    path: Path
    lines_found: int
    lines_hit: int
    branches_found: int
    branches_hit: int

    @property
    def line_pct(self) -> float:
        return 100.0 * self.lines_hit / self.lines_found if self.lines_found else 100.0

    @property
    def branch_pct(self) -> float:
        return (
            100.0 * self.branches_hit / self.branches_found
            if self.branches_found
            else 100.0
        )

    def at_100(self) -> bool:
        return self.line_pct >= 100.0 and self.branch_pct >= 100.0


_SF_RE = re.compile(r"^SF:(.+)$")
_LF_RE = re.compile(r"^LF:(\d+)$")
_LH_RE = re.compile(r"^LH:(\d+)$")
_BRF_RE = re.compile(r"^BRF:(\d+)$")
_BRH_RE = re.compile(r"^BRH:(\d+)$")
_END_RE = re.compile(r"^end_of_record$")


def parse_lcov(path: Path) -> dict[Path, FileCoverage]:
    """Parse an lcov.info file.  Returns a dict from absolute
    path to FileCoverage.
    """
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError as e:
        raise FileNotFoundError(f"lcov file not found: {path}") from e

    result: dict[Path, FileCoverage] = {}
    cur: Optional[FileCoverage] = None
    cur_path: Optional[Path] = None
    for line in text.splitlines():
        if m := _SF_RE.match(line):
            cur_path = Path(m.group(1).strip()).resolve()
            cur = FileCoverage(
                path=cur_path,
                lines_found=0,
                lines_hit=0,
                branches_found=0,
                branches_hit=0,
            )
            continue
        if cur is None:
            continue
        if m := _LF_RE.match(line):
            cur.lines_found = int(m.group(1))
        elif m := _LH_RE.match(line):
            cur.lines_hit = int(m.group(1))
        elif m := _BRF_RE.match(line):
            cur.branches_found = int(m.group(1))
        elif m := _BRH_RE.match(line):
            cur.branches_hit = int(m.group(1))
        elif _END_RE.match(line):
            assert cur_path is not None and cur is not None
            result[cur_path] = cur
            cur = None
            cur_path = None
    return result


def _read_changed_files(path: Path) -> List[Path]:
    """Read a file of changed paths, one per line.  Empty
    lines and lines starting with `#` are ignored.
    """
    out: list[Path] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        out.append(Path(s))
    return out


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lcov", type=Path, required=True)
    parser.add_argument("--changed-files", type=Path, required=True)
    parser.add_argument(
        "--layers",
        type=str,
        default="src/audio/,src/session/,src/providers/,src/pipeline/,src/tui/",
        help="comma-separated v1-critical layer prefixes",
    )
    args = parser.parse_args(argv)

    layers = [layer.strip() for layer in args.layers.split(",") if layer.strip()]
    try:
        coverage_map = parse_lcov(args.lcov)
    except FileNotFoundError as e:
        print(f"::error::{e}", file=sys.stderr)
        return 3

    # Resolve all keys in the coverage map to canonical paths
    # so we can match against the absolute path of the changed
    # file (which on macOS may live under /private/tmp, while
    # the change-file list contains the un-privated /tmp path).
    coverage_map_canon: dict[str, FileCoverage] = {}
    for k, v in coverage_map.items():
        try:
            coverage_map_canon[k.resolve().as_posix()] = v
        except OSError:
            coverage_map_canon[k.as_posix()] = v

    changed = _read_changed_files(args.changed_files)
    cwd = Path.cwd()
    in_scope: list[Path] = []
    for p in changed:
        p_abs = p if p.is_absolute() else (cwd / p).resolve()
        for layer in layers:
            if (
                str(p_abs)
                .replace("\\", "/")
                .startswith((cwd / layer).resolve().as_posix())
            ):
                in_scope.append(p_abs)
                break
    if not in_scope:
        print("No changed files in v1-critical layers; gate is a no-op.")
        return 0

    print(f"Per-file 100% coverage gate ({len(in_scope)} file(s) in scope):")
    fail: list[tuple[Path, FileCoverage]] = []
    for p in in_scope:
        cov = coverage_map_canon.get(p.resolve().as_posix())
        if cov is None:
            print(f"::error::no lcov data for {p}", file=sys.stderr)
            return 1
        marker = "OK" if cov.at_100() else "FAIL"
        print(
            f"  [{marker}] {p}: lines {cov.lines_hit}/{cov.lines_found} "
            f"({cov.line_pct:.1f}%)  branches {cov.branches_hit}/{cov.branches_found} "
            f"({cov.branch_pct:.1f}%)"
        )
        if not cov.at_100():
            fail.append((p, cov))

    if fail:
        n = len(fail)
        print(
            f"::error::per-file coverage gate failed: {n} file(s) below 100%",
            file=sys.stderr,
        )
        for p, cov in fail:
            print(
                f"  ::error::  {p}: lines {cov.line_pct:.1f}% branches {cov.branch_pct:.1f}%",
                file=sys.stderr,
            )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
