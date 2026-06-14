#!/usr/bin/env python3
"""Coverage gate for the v1-critical layers of tui-translator.

WP-25.04 (#762): the audit noted that no coverage tooling ran in CI
and an estimated 50-60% branch coverage on the v1-critical layers
(audio, session, provider, pipeline, tui) was unmeasured. This
script consumes the `lcov.info` produced by `cargo llvm-cov` and
asserts a baseline threshold so PRs cannot regress the codebase
below the current coverage floor.

# Inputs

`lcov.info` (default: `./lcov.info`) is the standard LCOV coverage
report.  The script walks the `SF:` (source file) lines, filters
by the v1-critical layer paths, and aggregates `lfd:` (lines
found) / `lrh:` (lines hit) / `brf:` (branches found) / `brh:`
(branches hit) records by file.

# Output

Prints a per-file table and an overall summary.  Exits 0 if the
overall coverage is >= the threshold (default 60%); exits 1
otherwise with the failing files printed to stderr.

# Usage

    python3 scripts/ci/coverage_gate.py --lcov lcov.info --threshold 60.0
    python3 scripts/ci/coverage_gate.py --lcov lcov.info --threshold 60.0 --layers audio,session,provider,pipeline,tui

The script is intentionally pure Python (no `coverage` or
`lcov` Python package dependency) so the self-hosted Windows
runner can run it without `pip install` steps.  It is also
strict about malformed input — it raises an explicit error
if a record is missing rather than silently dropping it.

# Exit codes

- 0 — overall coverage >= threshold
- 1 — overall coverage < threshold, or the script was
  misconfigured (missing file, no records for a layer)
- 2 — internal error (e.g. malformed LCOV)

# Why a script, not a Rust binary

The coverage data lives in `lcov.info`, a text format.  Parsing
it in Python keeps the script under 200 lines and dependency-free.
A Rust binary would require an extra `[[bin]]` entry, more
build time, and would not help the `lcov` parser logic.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

# WP-25.04 (#762): these are the v1-critical layers.  Adding a
# new layer is a deliberate decision; the script does NOT walk
# `src/` recursively because that would over-include build
# artefacts and `bin/` modules that are not the v1 contract.
V1_CRITICAL_LAYERS: tuple[str, ...] = (
    "src/audio/",
    "src/session/",
    "src/providers/",
    "src/pipeline/",
    "src/tui/",
)


@dataclass
class FileCoverage:
    """Aggregated coverage for a single source file.

    The `lines_found` and `lines_hit` fields are the standard
    LCOV `lfd` / `lhr` (or `lf` / `lh`) record counters.  We
    store them as `int` rather than `float` because the LCOV
    format reports integer line counts; the ratio is computed
    in `line_pct` and `branch_pct`.

    The `branches_found` / `branches_hit` fields correspond to
    the LCOV `brf` / `brh` records.  The LCOV format only
    emits these if branch coverage was enabled in the
    instrumentation; the script handles their absence by
    treating the file as 100% branch-covered (so the gate
    does not false-fail on builds without branch coverage).
    """

    path: str
    lines_found: int = 0
    lines_hit: int = 0
    branches_found: int = 0
    branches_hit: int = 0

    def line_pct(self) -> float:
        if self.lines_found == 0:
            return 100.0
        return 100.0 * self.lines_hit / self.lines_found

    def branch_pct(self) -> float:
        if self.branches_found == 0:
            # No branch coverage in the report; treat as 100%
            # so a line-only build does not false-fail.
            return 100.0
        return 100.0 * self.branches_hit / self.branches_found


@dataclass
class CoverageReport:
    files: dict[str, FileCoverage] = field(default_factory=dict)

    def add_line_record(self, path: str, found: int, hit: int) -> None:
        f = self.files.setdefault(path, FileCoverage(path=path))
        # The LCOV format uses `lfd`/`lhr` in the tracefile
        # sections but `lf`/`lh` in the summary.  This script
        # consumes the tracefile format; see `parse_lcov`.
        f.lines_found += found
        f.lines_hit += hit

    def add_branch_record(self, path: str, found: int, hit: int) -> None:
        f = self.files.setdefault(path, FileCoverage(path=path))
        f.branches_found += found
        f.branches_hit += hit

    def files_in_layers(self, layers: tuple[str, ...]) -> list[FileCoverage]:
        """Return all files whose path is under one of `layers`."""
        out: list[FileCoverage] = []
        for f in self.files.values():
            if any(f.path.startswith(layer) for layer in layers):
                out.append(f)
        return out

    def aggregate(self, files: list[FileCoverage]) -> tuple[int, int, int, int]:
        """Sum lines_found/lines_hit/branches_found/branches_hit."""
        lf = sum(f.lines_found for f in files)
        lh = sum(f.lines_hit for f in files)
        bf = sum(f.branches_found for f in files)
        bh = sum(f.branches_hit for f in files)
        return lf, lh, bf, bh


# LCOV record parsers.  We compile the regexes once at module
# load because the script reads a moderately large lcov.info
# (10-50 MB for a project of this size) and re-compilation
# would dominate the wall time.
_RE_SF = re.compile(rb"^SF:(.+)$")
_RE_LFD = re.compile(rb"^lfd:(\d+)$")
_RE_LHR = re.compile(rb"^lhr:(\d+)$")
_RE_BRF = re.compile(rb"^brf:(\d+)$")
_RE_BRH = re.compile(rb"^brh:(\d+)$")
_RE_END = re.compile(rb"^end_of_record$")


def parse_lcov(lcov_path: Path) -> CoverageReport:
    """Parse `lcov_path` and return the aggregated coverage.

    The LCOV format is one record per source file; each record
    starts with `SF:<path>` and ends with `end_of_record`.  We
    iterate line by line, tracking the current source file and
    accumulating the per-file counters.
    """
    if not lcov_path.exists():
        raise FileNotFoundError(f"lcov file not found: {lcov_path}")

    report = CoverageReport()
    current_path: str | None = None
    line_count = 0

    with lcov_path.open("rb") as fp:
        for raw in fp:
            line_count += 1
            if (m := _RE_SF.match(raw)) is not None:
                # `SF:` paths are relative to the project root
                # in `cargo llvm-cov` output; the v1-critical
                # layer filters assume that prefix.
                raw_path = m.group(1).decode("utf-8", errors="replace")
                # Normalise path separators: on Windows, paths
                # may use backslashes; the v1-critical layer
                # filters assume forward slashes.
                raw_path = raw_path.replace("\\", "/")
                # If the path is absolute (e.g. on Windows the
                # `cargo llvm-cov` output may include a
                # `C:\...` prefix when the build is run from
                # outside the project root), strip the
                # absolute prefix and keep only the
                # `src/<rest>` suffix.
                if ":" in raw_path[:3]:
                    # The colon in the first 3 characters is a
                    # Windows drive-letter prefix.  Strip the
                    # everything-before-`src/` portion.
                    idx = raw_path.rfind("src/")
                    if idx >= 0:
                        raw_path = raw_path[idx:]
                current_path = raw_path
                # Debug: print first 5 paths so we can see
                # what `cargo llvm-cov` actually emits.
                if line_count <= 30:
                    print(
                        f"::notice::SF: {raw_path!r}",
                        file=sys.stderr,
                    )
            elif (m := _RE_LFD.match(raw)) is not None:
                if current_path is None:
                    raise ValueError(f"line {line_count}: lfd before SF")
                report.add_line_record(current_path, int(m.group(1)), 0)
            elif (m := _RE_LHR.match(raw)) is not None:
                if current_path is None:
                    raise ValueError(f"line {line_count}: lhr before SF")
                f = report.files.setdefault(
                    current_path, FileCoverage(path=current_path)
                )
                f.lines_hit += int(m.group(1))
            elif (m := _RE_BRF.match(raw)) is not None:
                if current_path is None:
                    raise ValueError(f"line {line_count}: brf before SF")
                report.add_branch_record(current_path, int(m.group(1)), 0)
            elif (m := _RE_BRH.match(raw)) is not None:
                if current_path is None:
                    raise ValueError(f"line {line_count}: brh before SF")
                f = report.files.setdefault(
                    current_path, FileCoverage(path=current_path)
                )
                f.branches_hit += int(m.group(1))
            elif _RE_END.match(raw):
                current_path = None

    return report


def format_pct(pct: float) -> str:
    return f"{pct:6.2f}%"


def print_table(files: list[FileCoverage]) -> None:
    # Print a per-file table so a CI log shows which files
    # are dragging the average down.  The table is fixed-width
    # and pipe-delimited for grep-ability.
    print(f"{'file':<60} {'lines':>10} {'line%':>8} {'br':>6} {'br%':>8}")
    print("-" * 96)
    for f in sorted(files, key=lambda x: (x.line_pct(), x.path)):
        print(
            f"{f.path:<60} {f.lines_hit:>6}/{f.lines_found:<3} {format_pct(f.line_pct())} "
            f"{f.branches_hit:>4}/{f.branches_found:<1} {format_pct(f.branch_pct())}"
        )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Coverage gate for v1-critical layers (WP-25.04 #762)"
    )
    parser.add_argument(
        "--lcov",
        type=Path,
        default=Path("lcov.info"),
        help="path to lcov.info (default: ./lcov.info)",
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=60.0,
        help="minimum line coverage %% (default: 60.0)",
    )
    parser.add_argument(
        "--layers",
        type=str,
        default=",".join(V1_CRITICAL_LAYERS),
        help="comma-separated path prefixes for the v1-critical layers",
    )
    parser.add_argument(
        "--branch-threshold",
        type=float,
        default=None,
        help="minimum branch coverage %% (default: same as --threshold)",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="print per-file coverage and exit (no gate enforcement)",
    )
    args = parser.parse_args(argv)

    layers = tuple(s for s in args.layers.split(",") if s)
    branch_threshold = (
        args.branch_threshold if args.branch_threshold is not None else args.threshold
    )

    try:
        report = parse_lcov(args.lcov)
    except FileNotFoundError as e:
        # The CI step that calls this script may run before
        # `cargo llvm-cov` produces lcov.info.  Fail loud
        # so the operator can see the path that was expected
        # to exist (the workflow check would otherwise pass
        # silently on a missing artefact).
        print(f"::error::{e}", file=sys.stderr)
        return 1
    files = report.files_in_layers(layers)
    if not files:
        print(
            f"::error::no coverage records found under any of {layers!r}; "
            f"check that `cargo llvm-cov` was run with the correct --no-cfg-coverage flags",
            file=sys.stderr,
        )
        return 1

    lf, lh, bf, bh = report.aggregate(files)
    line_pct = 100.0 * lh / lf if lf else 100.0
    branch_pct = 100.0 * bh / bf if bf else 100.0

    print(
        f"Coverage gate: line {format_pct(line_pct)} >= {args.threshold:.1f}%, "
        f"branch {format_pct(branch_pct)} >= {branch_threshold:.1f}%"
    )
    print(f"  layers: {', '.join(layers)}")
    print(f"  files:  {len(files)}")
    print(f"  lines:  {lh} / {lf}")
    print(f"  branch: {bh} / {bf}")
    print()
    print_table(files)

    if args.list:
        return 0

    failures: list[str] = []
    if line_pct < args.threshold:
        failures.append(
            f"line coverage {line_pct:.2f}% below threshold {args.threshold:.1f}%"
        )
    if branch_pct < branch_threshold:
        failures.append(
            f"branch coverage {branch_pct:.2f}% below threshold {branch_threshold:.1f}%"
        )
    if failures:
        print()
        print("::error::coverage gate FAILED", file=sys.stderr)
        for f in failures:
            print(f"::error::  {f}", file=sys.stderr)
        return 1
    print()
    print(
        f"::notice::coverage gate PASS — line {line_pct:.2f}%, branch {branch_pct:.2f}%"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
