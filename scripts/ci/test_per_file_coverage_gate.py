#!/usr/bin/env python3
"""Unit tests for per_file_coverage_gate.py.

WP-25.05 (coverage-100% follow-up): the per-file 100% gate
is the regression-catcher.  These tests pin:
  - lcov parsing (LF/LH/BRF/BRH per file)
  - the layer filter (only files under v1-critical prefixes)
  - the threshold behaviour (100% required)
  - the missing-file failure mode
  - the no-op behaviour (no changed files in scope)

The test fixtures live in ``scripts/ci/fixtures/*.info`` so
the lcov path-rewriting step (replace ``PLACEHOLDER_*`` with
the real absolute path of the file under test) is
deterministic and survives `textwrap.dedent` bugs.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "per_file_coverage_gate.py"
FIXTURES = Path(__file__).resolve().parent / "fixtures"


def _rewrite(content: str) -> str:
    """Replace the ``PLACEHOLDER_*`` markers in a fixture with
    the real absolute path of the file under test.
    """
    cwd = Path.cwd()
    replacements = {
        "PLACEHOLDER_SRC_AUDIO_PCM_FORMAT_RS": str(
            cwd / "src" / "audio" / "pcm_format.rs"
        ),
        "PLACEHOLDER_SRC_AUDIO_EMPTY_RS": str(cwd / "src" / "audio" / "empty.rs"),
        "PLACEHOLDER_SRC_AUDIO_FOO_RS": str(cwd / "src" / "audio" / "foo.rs"),
        "PLACEHOLDER_SRC_BIN_FOO_RS": str(cwd / "src" / "bin" / "foo.rs"),
        "PLACEHOLDER_SRC_CUSTOM_FOO_RS": str(cwd / "src" / "custom" / "foo.rs"),
        "PLACEHOLDER_SRC_AUDIO_A_RS": str(cwd / "src" / "audio" / "a.rs"),
        "PLACEHOLDER_SRC_AUDIO_B_RS": str(cwd / "src" / "audio" / "b.rs"),
    }
    for marker, real in replacements.items():
        content = content.replace(marker, real)
    return content


def _write_lcov(fixture: str) -> Path:
    content = _rewrite((FIXTURES / fixture).read_text(encoding="utf-8"))
    f = tempfile.NamedTemporaryFile(
        mode="w", suffix=".info", delete=False, encoding="utf-8"
    )
    f.write(content)
    f.close()
    return Path(f.name)


def _write_changed_files(paths: list[str]) -> Path:
    f = tempfile.NamedTemporaryFile(
        mode="w", suffix=".txt", delete=False, encoding="utf-8"
    )
    f.write("\n".join(paths))
    f.close()
    return Path(f.name)


def _run(args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        # Run the gate from the repository root so that the
        # `src/audio/...` paths in the changed-files list
        # resolve correctly.  The script's logic depends on
        # `Path.cwd()` matching the repo root.
        cwd=Path(__file__).resolve().parent.parent.parent,
    )


class LcovParsingTests(unittest.TestCase):
    def test_single_file_full_line(self) -> None:
        p = _write_lcov("lcov_full_100.info")
        cf = _write_changed_files(["src/audio/pcm_format.rs"])
        r = _run(["--lcov", str(p), "--changed-files", str(cf)])
        self.assertEqual(r.returncode, 0, msg=r.stdout + r.stderr)
        self.assertIn("lines 100/100 (100.0%)", r.stdout)

    def test_single_file_partial_line_fails(self) -> None:
        p = _write_lcov("lcov_partial_line.info")
        cf = _write_changed_files(["src/audio/pcm_format.rs"])
        r = _run(["--lcov", str(p), "--changed-files", str(cf)])
        self.assertEqual(r.returncode, 1, msg=r.stdout + r.stderr)
        self.assertIn("FAIL", r.stdout)
        self.assertIn("50.0%", r.stdout)

    def test_single_file_partial_branch_fails(self) -> None:
        p = _write_lcov("lcov_partial_branch.info")
        cf = _write_changed_files(["src/audio/pcm_format.rs"])
        r = _run(["--lcov", str(p), "--changed-files", str(cf)])
        self.assertEqual(r.returncode, 1, msg=r.stdout + r.stderr)
        self.assertIn("FAIL", r.stdout)

    def test_zero_branches_is_100_percent_branch(self) -> None:
        p = _write_lcov("lcov_no_branches.info")
        cf = _write_changed_files(["src/audio/pcm_format.rs"])
        r = _run(["--lcov", str(p), "--changed-files", str(cf)])
        self.assertEqual(r.returncode, 0, msg=r.stdout + r.stderr)

    def test_zero_lines_is_vacuously_ok(self) -> None:
        p = _write_lcov("lcov_empty_file.info")
        cf = _write_changed_files(["src/audio/empty.rs"])
        r = _run(["--lcov", str(p), "--changed-files", str(cf)])
        self.assertEqual(r.returncode, 0, msg=r.stdout + r.stderr)


class LayerFilterTests(unittest.TestCase):
    def test_audio_file_in_scope(self) -> None:
        p = _write_lcov("lcov_audio_ok.info")
        cf = _write_changed_files(["src/audio/foo.rs"])
        r = _run(["--lcov", str(p), "--changed-files", str(cf)])
        self.assertEqual(r.returncode, 0, msg=r.stdout + r.stderr)
        self.assertIn("1 file(s) in scope", r.stdout)

    def test_non_layer_file_out_of_scope(self) -> None:
        p = _write_lcov("lcov_bin_layer.info")
        cf = _write_changed_files(["src/bin/foo.rs"])
        r = _run(["--lcov", str(p), "--changed-files", str(cf)])
        self.assertEqual(r.returncode, 0, msg=r.stdout + r.stderr)
        self.assertIn("no-op", r.stdout)

    def test_custom_layers(self) -> None:
        p = _write_lcov("lcov_custom_layer.info")
        cf = _write_changed_files(["src/custom/foo.rs"])
        r = _run(
            [
                "--lcov",
                str(p),
                "--changed-files",
                str(cf),
                "--layers",
                "src/custom/",
            ]
        )
        self.assertEqual(r.returncode, 0, msg=r.stdout + r.stderr)


class ChangedFilesTests(unittest.TestCase):
    def test_empty_changed_file_is_noop(self) -> None:
        p = _write_lcov("lcov_audio_ok.info")
        cf = _write_changed_files([])
        r = _run(["--lcov", str(p), "--changed-files", str(cf)])
        self.assertEqual(r.returncode, 0, msg=r.stdout + r.stderr)
        self.assertIn("no-op", r.stdout)

    def test_comments_in_changed_file_ignored(self) -> None:
        # `#`-prefixed lines in the changed-file list are
        # silently ignored, so the file we care about is in
        # scope.
        p = _write_lcov("lcov_audio_ok.info")
        cf = _write_changed_files(["# header comment", "src/audio/foo.rs", ""])
        r = _run(["--lcov", str(p), "--changed-files", str(cf)])
        self.assertEqual(r.returncode, 0, msg=r.stdout + r.stderr)

    def test_missing_lcov_exits_three(self) -> None:
        cf = _write_changed_files(["src/audio/foo.rs"])
        r = _run(
            [
                "--lcov",
                "/nonexistent/path/lcov.info",
                "--changed-files",
                str(cf),
            ]
        )
        self.assertEqual(r.returncode, 3, msg=r.stdout + r.stderr)
        self.assertIn("lcov file not found", r.stderr)

    def test_changed_file_with_no_lcov_entry_fails(self) -> None:
        p = _write_lcov("lcov_audio_ok.info")
        cf = _write_changed_files(["src/audio/missing.rs"])
        r = _run(["--lcov", str(p), "--changed-files", str(cf)])
        self.assertEqual(r.returncode, 1, msg=r.stdout + r.stderr)


class MultiFileTests(unittest.TestCase):
    def test_two_files_one_fails(self) -> None:
        p = _write_lcov("lcov_two_files.info")
        cf = _write_changed_files(["src/audio/a.rs", "src/audio/b.rs"])
        r = _run(["--lcov", str(p), "--changed-files", str(cf)])
        self.assertEqual(r.returncode, 1, msg=r.stdout + r.stderr)
        self.assertIn("2 file(s) in scope", r.stdout)
        self.assertIn("[OK]", r.stdout)
        self.assertIn("[FAIL]", r.stdout)


if __name__ == "__main__":
    unittest.main()
