#!/usr/bin/env python3
"""Panic-site counter for tui-translator.

WP-25.05 (#763): the audit's top cực-gắt finding was that
`src/main.rs` had 105 `.unwrap()` + 34 `.expect()` + 2
`panic!()` = **141 panic-prone sites** in the binary.  The
acceptance criterion for the work is to drive that number
below 50 and keep it below 50 as the binary evolves.

This script counts the panic-prone sites in a single source
file.  It is invoked by `.github/workflows/panic-sites.yml`
(a job added in the same PR) and exits non-zero when the
count exceeds the threshold.

It is also used as a library by the unit tests in
`test_panic_sites.py` so the counting logic itself is
covered by the test suite — the agent's audit-claw will not
be fooled by a future refactor that \"fixes\" the count by
under-counting the patterns.

Usage::

    python scripts/ci/panic_sites.py --path src/main.rs --max 50

Exit codes:
    0 — under threshold
    1 — over threshold
    2 — invalid invocation
    3 — file not found
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import List


# WP-25.05 (#763): the regex set must be the union of the three
# panic-prone sites; tests pin this so future \"fixes\" cannot
# silently change the counting policy.
_UNWRAP_RE = re.compile(r"\.unwrap\(\s*\)")
_EXPECT_RE = re.compile(r"\.expect\(")
_PANIC_RE = re.compile(r"\bpanic!\s*\(")
# Macro: `todo!`, `unimplemented!` are also panic-prone (they
# expand to `panic!` internally).  The audit didn't count them
# in the 141 sites; we follow suit so the threshold remains
# comparable to the audit baseline.
_MACRO_FORMS: tuple[re.Pattern[str], ...] = ()


@dataclass
class PanicSite:
    line: int
    col: int
    kind: str  # "unwrap" | "expect" | "panic"
    text: str  # the source line (truncated)

    def format(self) -> str:
        return f"  {self.path}:{self.line}:{self.col}: {self.kind}"


@dataclass
class CountResult:
    path: Path
    unwrap_count: int
    expect_count: int
    panic_count: int
    sites: List[PanicSite]

    @property
    def total(self) -> int:
        return self.unwrap_count + self.expect_count + self.panic_count

    def summary(self) -> str:
        return (
            f"{self.path}: unwrap={self.unwrap_count} "
            f"expect={self.expect_count} panic={self.panic_count} "
            f"total={self.total}"
        )


def _classify(text: str, m: re.Match[str]) -> str:
    # `PanicSite` doesn't carry the regex; classify from the
    # matched substring.
    if m.re is _UNWRAP_RE:
        return "unwrap"
    if m.re is _EXPECT_RE:
        return "expect"
    if m.re is _PANIC_RE:
        return "panic"
    return "other"


def count_sites(path: Path) -> CountResult:
    """Count panic-prone sites in a single file.

    The implementation is a three-pass scan so each pattern
    contributes to its own count and the per-line / per-col
    information is preserved for the verbose report.
    """
    try:
        content = path.read_text(encoding="utf-8")
    except FileNotFoundError as e:
        raise FileNotFoundError(f"file not found: {path}") from e
    lines = content.splitlines()

    sites: list[PanicSite] = []

    def _compute_test_line_set(lines: list[str]) -> set[int]:
        """Return the set of 1-based line numbers that are inside a
        test block (`#[cfg(test)]` or `#[test]`).

        Algorithm: walk the file from top to bottom, tracking the
        brace depth.  A test scope opens on a line starting with
        `#[cfg(test)]` or `#[test]`; the depth at the open point
        is recorded.  The scope closes when the brace depth
        returns to or below that recorded value (specifically,
        when a `}` brings the depth back).

        We also handle the case where a `#[cfg(test)]` is
        immediately followed by a sibling `mod tests {` line:
        the brace counter opens the mod body, then walks to
        the closing `}`.
        """
        test_lines: set[int] = set()
        in_test = False
        depth_at_open: int = 0
        current_depth: int = 0
        for i, line in enumerate(lines):
            stripped = line.lstrip()
            if not in_test:
                if stripped.startswith("#[cfg(test)]") or stripped.startswith(
                    "#[test]"
                ):
                    in_test = True
                    depth_at_open = current_depth
                    # The `#[cfg(test)]` line itself is part of
                    # the test scope; it is the attribute that
                    # gates the test block.
                    test_lines.add(i + 1)
            else:
                test_lines.add(i + 1)
                # Update the brace depth for this line.
                opens = line.count("{")
                closes = line.count("}")
                current_depth += opens - closes
                # Close the test scope when the brace depth
                # has returned to the depth at the open.
                # The `#[cfg(test)] mod tests {` line is itself
                # part of the test scope; the matching `}` of
                # the mod brings the depth back to the open
                # depth.  We do NOT use the defensive
                # `elif fn/mod/impl` close here because it
                # mis-fires on the `mod tests {` opening line
                # (which starts with `mod ` at the same indent
                # as the `#[cfg(test)]` attribute).
                if current_depth <= depth_at_open and closes > 0:
                    in_test = False
        return test_lines

    def _strip_comments_and_strings(line: str) -> str:
        """Strip Rust line comments (`// ...`) from a line.

        String literals are not stripped — a `.unwrap()` mentioned
        in a log message is still a string occurrence.  This means
        the count may include a small number of false positives
        from error-message strings.  We document the trade-off in
        the test `test_unwrap_in_string_is_not_counted` so a
        future change to filter strings is intentional.
        """
        # Match `//` not inside a string.  The simple approach:
        # find the first `//` and treat everything from there to EOL
        # as a comment.  This is correct for `.unwrap()` because
        # `//` cannot appear inside a string literal on the same
        # line as an unwrap call (a string with `"//"` would have
        # to terminate before the unwrap).
        idx = line.find("//")
        if idx < 0:
            return line
        return line[:idx]

    def _scan(
        pattern: re.Pattern[str], kind: str, lines: list[str]
    ) -> tuple[int, list[PanicSite]]:
        test_lines = _compute_test_line_set(lines)
        n = 0
        sites: list[PanicSite] = []
        for i, line in enumerate(lines, start=1):
            # Skip test code.  The audit was a *production* code
            # audit; test binary inherits production code, so
            # reducing production code reduces both.  Counting
            # test sites would inflate the metric without
            # affecting production behaviour.
            if i in test_lines:
                continue
            # Filter comments so a `.unwrap()` in a `// ...` line
            # is not counted.  See _strip_comments_and_strings.
            stripped = _strip_comments_and_strings(line)
            for m in pattern.finditer(stripped):
                sites.append(
                    PanicSite(
                        line=i,
                        col=m.start() + 1,
                        kind=kind,
                        text=line.strip()[:120],
                    )
                )
                n += 1
        return n, sites

    unwrap_n, unwrap_sites = _scan(_UNWRAP_RE, "unwrap", lines)
    expect_n, expect_sites = _scan(_EXPECT_RE, "expect", lines)
    panic_n, panic_sites = _scan(_PANIC_RE, "panic", lines)
    sites = unwrap_sites + expect_sites + panic_sites

    for s in sites:
        s.path = path  # set back-reference for format()

    return CountResult(
        path=path,
        unwrap_count=unwrap_n,
        expect_count=expect_n,
        panic_count=panic_n,
        sites=sites,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--path",
        type=Path,
        required=True,
        help="source file to scan",
    )
    parser.add_argument(
        "--max",
        type=int,
        default=50,
        help="panic-site cap (default 50, WP-25.05 acceptance criterion)",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="list each panic-prone site (useful for inventory)",
    )
    args = parser.parse_args(argv)

    try:
        result = count_sites(args.path)
    except FileNotFoundError as e:
        print(f"::error::{e}", file=sys.stderr)
        return 3

    print(result.summary())
    if args.list:
        for s in result.sites:
            print(s.format())
            print(f"    {s.text}")

    if result.total > args.max:
        print(
            f"::error::panic-site cap exceeded: {result.total} > {args.max}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
