#!/usr/bin/env python3
"""Module-size gate for tui-translator.

WP-25.01 (#759): the audit found 4 files in `src/` that vastly
exceed the documented 1000-LOC module gate:

```
src/main.rs          6959
src/tui/mod.rs       5241
src/config/mod.rs    5165
src/pipeline/mod.rs  4285
```

The fix is a pure refactor that splits each into submodules. The
gate is the regression catcher: this script walks `src/**/*.rs`,
filters to "oversize-amenable" files (i.e. files in `mod.rs`
contexts where splitting is plausible), and asserts each is at or
below the threshold.

# Why a script, not a Rust lint

The `clippy::module_size` lint is per-file, not per-`mod`. The
refactor strategy is to move parts of `mod.rs` into siblings
(`src/tui/app_state.rs`, `src/tui/subtitle_pane.rs`, etc.) and
keep `mod.rs` as the orchestrator. A Rust lint cannot distinguish
"this file is the orchestrator for sibling submodules" from
"this file is monolithic code that should be split". The
heuristic the script uses is:

- `src/<dir>/mod.rs` files are flagged if their `wc -l` exceeds
  the threshold.
- `src/<dir>/<name>.rs` (non-mod) files are flagged if they
  exceed the threshold.
- `src/main.rs` is flagged if it exceeds the threshold (this
  is the binary's entry point; we want to be able to ship a
  refactored binary too).

# Exit codes

- 0 — all in-scope files are at or below the threshold
- 1 — at least one file exceeds the threshold
- 2 — script misconfiguration (e.g. missing src/ directory)

# Usage

    python3 scripts/ci/check_module_sizes.py
    python3 scripts/ci/check_module_sizes.py --threshold 1000 --src src
    python3 scripts/ci/check_module_sizes.py --list

The --list flag prints the table and exits 0 (informational
only).  This is useful for spot-checks before a refactor PR.

# Why we excluded `src/bin/*.rs`

The bin/ directory holds long-running benchmark and probe
binaries that are intentionally procedural.  Splitting them is
not on the v1-readiness roadmap; the module-size gate is for
the production library / binary entry points only.
"""

from __future__ import annotations

import argparse
import sys
from dataclasses import dataclass
from pathlib import Path

# Default threshold matches CONTRIBUTING.md / AGENTS.md: a module
# file should not exceed 1000 lines of code.  The audit found 4
# files in `src/` that violate this; the script is the gate.
DEFAULT_THRESHOLD_LOC: int = 1000


@dataclass
class ModuleSize:
    path: Path
    loc: int

    def rel(self, root: Path) -> str:
        return str(self.path.relative_to(root))


def iter_source_files(src_root: Path) -> list[Path]:
    """Yield all `*.rs` files under `src_root`, sorted by path.

    Excludes `target/` build artefacts by virtue of starting
    under `src/`.  Excludes `src/bin/` per the module comment;
    the bin/ directory is intentionally long-running
    procedural code, not subject to the 1000-LOC gate.
    """
    if not src_root.is_dir():
        raise FileNotFoundError(f"src root not found: {src_root}")
    files: list[Path] = []
    for p in src_root.rglob("*.rs"):
        if not p.is_file():
            continue
        rel = p.relative_to(src_root)
        # Skip the bin/ directory.  We still want src/main.rs
        # and the library's mod.rs files in scope.
        if rel.parts and rel.parts[0] == "bin":
            continue
        files.append(p)
    files.sort()
    return files


def line_count(path: Path) -> int:
    """Return the line count of `path` (blank lines included).

    `wc -l` is the reference; this counts newline characters.
    We open the file as text (utf-8 with replacement) so a
    stray non-UTF-8 byte does not crash the gate.
    """
    try:
        with path.open("r", encoding="utf-8", errors="replace") as fp:
            return sum(1 for _ in fp)
    except OSError as e:
        print(f"::warning::could not read {path}: {e}", file=sys.stderr)
        return 0


def collect_sizes(src_root: Path) -> list[ModuleSize]:
    files = iter_source_files(src_root)
    out: list[ModuleSize] = []
    for p in files:
        out.append(ModuleSize(path=p, loc=line_count(p)))
    return out


def format_table(sizes: list[ModuleSize], root: Path) -> str:
    if not sizes:
        return "(no source files found)"
    # Print the top-20 by LOC so the table fits in a CI log.
    biggest = sorted(sizes, key=lambda s: s.loc, reverse=True)[:20]
    pad = max((len(s.rel(root)) for s in biggest), default=60)
    lines: list[str] = []
    lines.append(f"{'file'.ljust(pad)}  {'loc':>8}")
    lines.append("-" * (pad + 11))
    for s in biggest:
        lines.append(f"{s.rel(root).ljust(pad)}  {s.loc:>8}")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Module-size gate (WP-25.01 #759)")
    parser.add_argument(
        "--src",
        type=Path,
        default=Path("src"),
        help="path to src/ (default: ./src)",
    )
    parser.add_argument(
        "--threshold",
        type=int,
        default=DEFAULT_THRESHOLD_LOC,
        help=f"max LOC per file (default: {DEFAULT_THRESHOLD_LOC})",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="print the top-20 biggest files and exit (no gate)",
    )
    parser.add_argument(
        "--waivers",
        type=Path,
        default=None,
        help=(
            "path to a waiver file (one path per line, `# comments` "
            "ignored). Files in the waiver list are exempted from "
            "the threshold. Use sparingly; waivers are a regression "
            "catcher for the pre-existing baseline, not a permanent "
            "carve-out. Defaults to `.standards-waivers.txt` in the "
            "repo root."
        ),
    )
    args = parser.parse_args(argv)

    if not args.src.is_dir():
        print(f"::error::src/ directory not found: {args.src}", file=sys.stderr)
        return 2

    # Resolve the waiver list. Default to
    # `.standards-waivers.txt` in the repo root (the same file
    # the 600-LOC gate in `engineering-standards.md` uses).
    if args.waivers is None:
        default_waivers = Path(".standards-waivers.txt")
        if default_waivers.is_file():
            args.waivers = default_waivers
    waived_paths: set[str] = set()
    if args.waivers is not None and args.waivers.is_file():
        with args.waivers.open("r", encoding="utf-8") as fp:
            for raw in fp:
                line = raw.split("#", 1)[0].strip()
                if not line:
                    continue
                # Strip the `src/` prefix so the waiver list
                # matches the same path notation as
                # `.standards-waivers.txt` (which uses
                # `src/<path>`).
                if line.startswith("src/"):
                    line = line[len("src/") :]
                if line:
                    waived_paths.add(line)
        print(
            f"::notice::module-size gate: loaded {len(waived_paths)} "
            f"waivers from {args.waivers}: {sorted(waived_paths)}",
            file=sys.stderr,
        )

    sizes = collect_sizes(args.src)
    if args.list:
        print(format_table(sizes, args.src))
        return 0

    over: list[ModuleSize] = []
    for s in sizes:
        if s.loc <= args.threshold:
            continue
        # Match the file directly, or its parent dir (which
        # waives `src/<dir>/mod.rs`), or the directory
        # containing it (e.g. `src/tui/` waives
        # `src/tui/mod.rs`).
        rel = s.rel(args.src)
        parent_dir = str(Path(rel).parent)
        if parent_dir == ".":
            parent_prefix = ""
        else:
            parent_prefix = parent_dir + "/"
        if rel in waived_paths or parent_prefix in waived_paths:
            continue
        over.append(s)
    if over:
        print(
            f"::error::module-size gate FAILED — {len(over)} file(s) > {args.threshold} LOC",
            file=sys.stderr,
        )
        for s in sorted(over, key=lambda s: s.loc, reverse=True):
            print(
                f"::error::  {s.rel(args.src)}: {s.loc} LOC",
                file=sys.stderr,
            )
        return 1
    print(
        f"::notice::module-size gate PASS — all {len(sizes)} files <= {args.threshold} LOC"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
