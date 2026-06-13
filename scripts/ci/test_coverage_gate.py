#!/usr/bin/env python3
"""Unit tests for coverage_gate.py.

WP-25.04 (#762): test the gate script as a library, not just
as a CLI tool.  Each test pins a property the gate relies on
for correct behaviour:

- File-filter scope: only files under the requested layer
  prefixes count toward the gate.
- Line-coverage aggregation: line hits / line found.
- Branch-coverage aggregation: branch hits / branch found.
- Missing branch records: treated as 100% (no false fail).
- Empty input: clear error, exit 1.
- Missing file: clear error, exit 1.
- Threshold fail: exit 1, error message names the threshold.
- --list: prints but does not gate.

These tests can be run with `python3 -m unittest
scripts.ci.test_coverage_gate` (when the directory is on
PYTHONPATH) or with `python3 scripts/ci/test_coverage_gate.py`
directly.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

# Add the scripts/ci/ directory to sys.path so we can import
# `coverage_gate` as a module without packaging it.  The test
# runs from the repo root by default.
SCRIPTS_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS_DIR))

import coverage_gate  # type: ignore[import-not-found]  # noqa: E402


def _write_lcov(content: str) -> Path:
    """Write `content` to a temp file and return the path."""
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".info", delete=False, encoding="utf-8"
    ) as fp:
        fp.write(content)
        return Path(fp.name)


# ── parser tests ──────────────────────────────────────────────────


class ParseLcovTests(unittest.TestCase):
    def test_parses_one_record_with_line_and_branch(self) -> None:
        path = _write_lcov(
            "TN:test\nSF:src/audio/foo.rs\nlfd:10\nlhr:8\nbrf:4\nbrh:3\nend_of_record\n"
        )
        try:
            report = coverage_gate.parse_lcov(path)
            self.assertEqual(len(report.files), 1)
            f = report.files["src/audio/foo.rs"]
            self.assertEqual(f.lines_found, 10)
            self.assertEqual(f.lines_hit, 8)
            self.assertEqual(f.branches_found, 4)
            self.assertEqual(f.branches_hit, 3)
        finally:
            path.unlink()

    def test_parses_two_records(self) -> None:
        path = _write_lcov(
            "SF:src/audio/a.rs\nlfd:5\nlhr:5\nend_of_record\n"
            "SF:src/session/b.rs\nlfd:10\nlhr:3\nend_of_record\n"
        )
        try:
            report = coverage_gate.parse_lcov(path)
            self.assertEqual(len(report.files), 2)
            self.assertEqual(report.files["src/audio/a.rs"].line_pct(), 100.0)
            self.assertEqual(report.files["src/session/b.rs"].line_pct(), 30.0)
        finally:
            path.unlink()

    def test_missing_branch_records_treated_as_full(self) -> None:
        path = _write_lcov("SF:src/audio/c.rs\nlfd:10\nlhr:7\nend_of_record\n")
        try:
            report = coverage_gate.parse_lcov(path)
            f = report.files["src/audio/c.rs"]
            self.assertEqual(f.branches_found, 0)
            # No branch records → 100% (we cannot fail a build
            # for absent data; that would be a false negative).
            self.assertEqual(f.branch_pct(), 100.0)
        finally:
            path.unlink()

    def test_missing_file_raises(self) -> None:
        with self.assertRaises(FileNotFoundError):
            coverage_gate.parse_lcov(Path("/tmp/definitely-not-here-12345.info"))

    def test_malformed_lfd_before_sf_raises(self) -> None:
        path = _write_lcov("lfd:1\n")
        try:
            with self.assertRaises(ValueError):
                coverage_gate.parse_lcov(path)
        finally:
            path.unlink()

    def test_decode_handles_non_utf8_paths_gracefully(self) -> None:
        # The SF: line may contain a path with non-UTF-8 bytes
        # on Windows (CP1252, etc.).  The parser must not crash.
        path = Path(tempfile.mkstemp(suffix=".info")[1])
        try:
            path.write_bytes(
                b"SF:src/audio/\xe4\xf6\xfc.rs\nlfd:1\nlhr:1\nend_of_record\n"
            )
            report = coverage_gate.parse_lcov(path)
            self.assertEqual(len(report.files), 1)
            # Key decodes with replacement; the bytes survive.
            self.assertIn("src/audio/", next(iter(report.files)))
        finally:
            path.unlink()


# ── layer filter tests ────────────────────────────────────────────


class LayerFilterTests(unittest.TestCase):
    def test_files_in_layers_filters_correctly(self) -> None:
        report = coverage_gate.CoverageReport()
        report.add_line_record("src/audio/foo.rs", 10, 8)
        report.add_line_record("src/session/bar.rs", 5, 2)
        report.add_line_record("src/unrelated/baz.rs", 1, 1)
        files = report.files_in_layers(("src/audio/", "src/session/"))
        paths = sorted(f.path for f in files)
        self.assertEqual(paths, ["src/audio/foo.rs", "src/session/bar.rs"])

    def test_files_in_layers_empty_for_no_match(self) -> None:
        report = coverage_gate.CoverageReport()
        report.add_line_record("src/unrelated.rs", 1, 1)
        files = report.files_in_layers(("src/audio/",))
        self.assertEqual(files, [])

    def test_files_in_layers_substring_is_not_a_match(self) -> None:
        # `src/audio_extra/` is NOT under `src/audio/` per the
        # prefix-with-trailing-slash rule.  This guards against
        # `startswith` matching too eagerly.
        report = coverage_gate.CoverageReport()
        report.add_line_record("src/audio_extra/x.rs", 1, 1)
        files = report.files_in_layers(("src/audio/",))
        self.assertEqual(files, [])


# ── aggregate tests ───────────────────────────────────────────────


class AggregateTests(unittest.TestCase):
    def test_aggregate_sums_lines_and_branches(self) -> None:
        report = coverage_gate.CoverageReport()
        report.add_line_record("src/audio/a.rs", 10, 8)
        report.add_line_record("src/audio/b.rs", 20, 12)
        report.add_branch_record("src/audio/a.rs", 4, 3)
        report.add_branch_record("src/audio/b.rs", 6, 4)
        files = report.files_in_layers(("src/audio/",))
        lf, lh, bf, bh = report.aggregate(files)
        self.assertEqual((lf, lh, bf, bh), (30, 20, 10, 7))

    def test_aggregate_empty_files(self) -> None:
        report = coverage_gate.CoverageReport()
        lf, lh, bf, bh = report.aggregate([])
        self.assertEqual((lf, lh, bf, bh), (0, 0, 0, 0))


# ── CLI integration tests ─────────────────────────────────────────


class CliTests(unittest.TestCase):
    def _run_cli(self, *args: str) -> subprocess.CompletedProcess[str]:
        """Run the script as a subprocess and return the result."""
        cmd = [sys.executable, str(SCRIPTS_DIR / "coverage_gate.py"), *args]
        env = {**os.environ, "PYTHONPATH": str(SCRIPTS_DIR)}
        return subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=60)

    def test_cli_missing_file_exits_1(self) -> None:
        result = self._run_cli("--lcov", "/tmp/nope-12345.info")
        self.assertEqual(result.returncode, 1)
        self.assertIn("not found", result.stderr)

    def test_cli_empty_lcov_exits_1(self) -> None:
        path = _write_lcov("")
        try:
            result = self._run_cli("--lcov", str(path))
            self.assertEqual(result.returncode, 1)
            self.assertIn("::error::", result.stderr)
        finally:
            path.unlink()

    def test_cli_passes_above_threshold(self) -> None:
        path = _write_lcov("SF:src/audio/foo.rs\nlfd:10\nlhr:9\nend_of_record\n")
        try:
            result = self._run_cli("--lcov", str(path), "--threshold", "60.0")
            self.assertEqual(result.returncode, 0)
            self.assertIn("::notice::coverage gate PASS", result.stdout)
        finally:
            path.unlink()

    def test_cli_fails_below_threshold(self) -> None:
        path = _write_lcov("SF:src/audio/foo.rs\nlfd:10\nlhr:1\nend_of_record\n")
        try:
            result = self._run_cli("--lcov", str(path), "--threshold", "60.0")
            self.assertEqual(result.returncode, 1)
            self.assertIn("::error::coverage gate FAILED", result.stderr)
            self.assertIn("10.00%", result.stderr)  # actual coverage printed
        finally:
            path.unlink()

    def test_cli_list_does_not_gate(self) -> None:
        path = _write_lcov("SF:src/audio/foo.rs\nlfd:10\nlhr:1\nend_of_record\n")
        try:
            result = self._run_cli("--lcov", str(path), "--list")
            # --list bypasses the gate; exit 0 even though 10% < 60%.
            # Also: the "FAILED" notice MUST NOT appear because
            # --list is purely informational.
            self.assertEqual(result.returncode, 0)
            self.assertNotIn("FAILED", result.stdout)
            self.assertNotIn("FAILED", result.stderr)
            # The header is always printed; the FAILED/notice
            # summary is suppressed in --list mode.
            self.assertIn("Coverage gate:", result.stdout)
        finally:
            path.unlink()

    def test_cli_filters_by_layer(self) -> None:
        # `src/unrelated.rs` is full-coverage but should be ignored
        # because it is not in any v1-critical layer.  Only the
        # audio file counts; with 5/5 covered the overall is 100%.
        path = _write_lcov(
            "SF:src/unrelated/x.rs\nlfd:1000\nlhr:0\nend_of_record\n"
            "SF:src/audio/y.rs\nlfd:5\nlhr:5\nend_of_record\n"
        )
        try:
            result = self._run_cli("--lcov", str(path), "--threshold", "60.0")
            self.assertEqual(result.returncode, 0)
            self.assertIn("100.00%", result.stdout)  # only audio file counted
        finally:
            path.unlink()

    def test_cli_custom_branch_threshold(self) -> None:
        path = _write_lcov(
            "SF:src/audio/foo.rs\nlfd:10\nlhr:10\nbrf:10\nbrh:5\nend_of_record\n"
        )
        try:
            # Line 100% passes, but branch 50% < 80% branch threshold.
            result = self._run_cli(
                "--lcov", str(path), "--threshold", "60.0", "--branch-threshold", "80.0"
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("branch coverage", result.stderr)
        finally:
            path.unlink()


if __name__ == "__main__":
    unittest.main()
