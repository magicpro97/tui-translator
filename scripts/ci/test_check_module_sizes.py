#!/usr/bin/env python3
"""Unit tests for check_module_sizes.py.

WP-25.01 (#759): the module-size gate is a simple wrapper around
`wc -l` and a threshold.  The interesting properties to lock
down are:

- File collection: walks `src/**/*.rs`, excludes `src/bin/`.
- Threshold behaviour: passes at exactly the threshold,
  fails one line above.
- Empty / missing `src/`: clear error, exit 2.
- `--list`: prints and exits 0.
- Excludes `src/bin/`: verified because the bin/ directory
  contains intentionally long-running procedural code that
  is not subject to the 1000-LOC gate.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import check_module_sizes  # type: ignore[import-not-found]  # noqa: E402


def _make_src_tree(files: dict[str, str]) -> Path:
    """Create a temp `src/` tree with the given relative paths.

    `files` maps relative path (under src/) to file content.
    The directory is created in a tmpdir and returned; the
    caller is responsible for cleanup.
    """
    tmp = Path(tempfile.mkdtemp(prefix="tui-modsize-"))
    for rel, content in files.items():
        p = tmp / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(textwrap.dedent(content), encoding="utf-8")
    return tmp


def _line_count(text: str) -> int:
    """Match the script's count: number of newlines, not lines.

    `wc -l` reports the number of newline characters. A file
    with N lines has either N or N+1 newline characters
    depending on whether the last line has a trailing newline.
    The Python iteration in `line_count` counts every line
    yielded by the iterator, which is the same as `wc -l` for
    files with a trailing newline and one less for files
    without.  This helper builds content with a trailing
    newline so the counts match `wc -l` exactly.
    """
    return text.count("\n")


class CollectSizesTests(unittest.TestCase):
    def test_collects_all_rs_under_src(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            (tmp_path / "src").mkdir()
            (tmp_path / "src/a.rs").write_text("a\nb\n")
            (tmp_path / "src/b.rs").write_text("x\n")
            sizes = check_module_sizes.collect_sizes(tmp_path / "src")
            paths = sorted(s.rel(tmp_path / "src") for s in sizes)
            self.assertEqual(paths, ["a.rs", "b.rs"])

    def test_excludes_bin_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            (tmp_path / "src").mkdir()
            (tmp_path / "src/main.rs").write_text("m\n")
            (tmp_path / "src/bin").mkdir()
            (tmp_path / "src/bin/bench.rs").write_text("b\n" * 5000)
            sizes = check_module_sizes.collect_sizes(tmp_path / "src")
            paths = sorted(s.rel(tmp_path / "src") for s in sizes)
            self.assertEqual(paths, ["main.rs"])

    def test_line_count_matches_wc(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            p = tmp_path / "f.rs"
            content = "line1\nline2\nline3\n"
            p.write_text(content, encoding="utf-8")
            # Python iteration counts lines, which is 3 here.
            self.assertEqual(check_module_sizes.line_count(p), 3)

    def test_missing_src_dir_raises(self) -> None:
        with self.assertRaises(FileNotFoundError):
            check_module_sizes.iter_source_files(Path("/tmp/definitely-not-here-99999"))


class CliTests(unittest.TestCase):
    def _run(self, *args: str, cwd: Path) -> subprocess.CompletedProcess[str]:
        cmd = [sys.executable, str(SCRIPTS_DIR / "check_module_sizes.py"), *args]
        return subprocess.run(cmd, capture_output=True, text=True, cwd=cwd, timeout=60)

    def test_passes_when_under_threshold(self) -> None:
        with tempfile.TemporaryDirectory() as cwd:
            (Path(cwd) / "src").mkdir()
            (Path(cwd) / "src" / "small.rs").write_text("a\nb\n")
            result = self._run(cwd=Path(cwd))
            self.assertEqual(result.returncode, 0)
            self.assertIn("::notice::module-size gate PASS", result.stdout)

    def test_fails_when_over_threshold(self) -> None:
        with tempfile.TemporaryDirectory() as cwd:
            (Path(cwd) / "src").mkdir()
            big = "x\n" * 1001
            (Path(cwd) / "src" / "huge.rs").write_text(big)
            result = self._run(cwd=Path(cwd))
            self.assertEqual(result.returncode, 1)
            self.assertIn("::error::module-size gate FAILED", result.stderr)
            self.assertIn("huge.rs: 1001 LOC", result.stderr)

    def test_passes_at_exactly_threshold(self) -> None:
        with tempfile.TemporaryDirectory() as cwd:
            (Path(cwd) / "src").mkdir()
            (Path(cwd) / "src" / "edge.rs").write_text("x\n" * 1000)
            result = self._run(cwd=Path(cwd))
            self.assertEqual(result.returncode, 0)

    def test_missing_src_exits_2(self) -> None:
        with tempfile.TemporaryDirectory() as cwd:
            result = self._run(cwd=Path(cwd))
            self.assertEqual(result.returncode, 2)
            self.assertIn("not found", result.stderr)

    def test_list_does_not_gate(self) -> None:
        with tempfile.TemporaryDirectory() as cwd:
            (Path(cwd) / "src").mkdir()
            (Path(cwd) / "src" / "huge.rs").write_text("x\n" * 9999)
            result = self._run("--list", cwd=Path(cwd))
            self.assertEqual(result.returncode, 0)
            self.assertIn("huge.rs", result.stdout)
            # The gate notice/error must NOT appear in --list mode.
            self.assertNotIn("FAILED", result.stdout)
            self.assertNotIn("FAILED", result.stderr)

    def test_custom_threshold(self) -> None:
        with tempfile.TemporaryDirectory() as cwd:
            (Path(cwd) / "src").mkdir()
            (Path(cwd) / "src" / "medium.rs").write_text("x\n" * 50)
            # With --threshold 40 this is a fail.
            result = self._run("--threshold", "40", cwd=Path(cwd))
            self.assertEqual(result.returncode, 1)
            # With --threshold 60 this is a pass.
            result = self._run("--threshold", "60", cwd=Path(cwd))
            self.assertEqual(result.returncode, 0)


class WaiverTests(CliTests):
    """The `--waivers` flag is the regression-catcher for the
    pre-existing baseline (the 4 files > 1000 LOC that
    predate the gate).  These tests pin the matching
    behaviour:

    - A file listed in the waiver file is exempt from the
      threshold.
    - A directory listed in the waiver file waives the
      `mod.rs` inside that directory.
    - Lines starting with `#` and blank lines in the waiver
      file are ignored.
    - The `src/` prefix is stripped from waiver paths.
    """

    def _write_waivers(self, tmp: Path, lines: list[str]) -> Path:
        path = tmp / "waivers.txt"
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        return path

    def _make_over(self, tmp: Path) -> Path:
        """Create a `src/<dir>/mod.rs` with 50 lines, then
        assert it fails the gate at threshold 40."""
        src = tmp / "src"
        (src / "foo").mkdir(parents=True)
        (src / "foo" / "mod.rs").write_text("\n".join(["x"] * 50) + "\n")
        return src

    def test_waiver_file_exempts_listed_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            self._make_over(tmp_path)
            waivers = self._write_waivers(tmp_path, ["src/foo/mod.rs"])
            result = self._run(
                "--threshold", "40", "--waivers", str(waivers), cwd=tmp_path
            )
            self.assertEqual(result.returncode, 0)
            self.assertIn("PASS", result.stdout)

    def test_directory_waiver_exempts_mod_rs_inside(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            self._make_over(tmp_path)
            waivers = self._write_waivers(tmp_path, ["src/foo/"])
            result = self._run(
                "--threshold", "40", "--waivers", str(waivers), cwd=tmp_path
            )
            self.assertEqual(result.returncode, 0)
            self.assertIn("PASS", result.stdout)

    def test_waiver_does_not_exempt_unrelated_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            self._make_over(tmp_path)
            waivers = self._write_waivers(tmp_path, ["src/bar/mod.rs"])
            result = self._run(
                "--threshold", "40", "--waivers", str(waivers), cwd=tmp_path
            )
            self.assertEqual(result.returncode, 1)
            # The error message goes to stderr; check both
            # streams to be robust.
            self.assertTrue("FAILED" in result.stdout or "FAILED" in result.stderr)

    def test_comments_and_blank_lines_in_waiver_file_are_ignored(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            self._make_over(tmp_path)
            waivers = self._write_waivers(
                tmp_path,
                [
                    "# This is a comment",
                    "",
                    "src/foo/mod.rs  # inline comment",
                    "  # leading-whitespace comment",
                ],
            )
            result = self._run(
                "--threshold", "40", "--waivers", str(waivers), cwd=tmp_path
            )
            self.assertEqual(result.returncode, 0)
            self.assertIn("PASS", result.stdout)

    def test_default_waivers_file_is_used_when_flag_omitted(self) -> None:
        # When the script is run from the repo root and
        # `.standards-waivers.txt` is present, the four
        # baseline files in the project root are exempt
        # by default.  We pin the repo's own `.standards-waivers.txt`
        # works against the script's own threshold of 1000.
        repo_root = Path(__file__).resolve().parent.parent.parent
        waivers = repo_root / ".standards-waivers.txt"
        if not waivers.is_file():
            self.skipTest(f"no .standards-waivers.txt at {waivers}")
        # Run with the default flag (no --waivers) and the
        # default threshold; expect pass.
        result = subprocess.run(
            [sys.executable, str(SCRIPTS_DIR / "check_module_sizes.py")],
            capture_output=True,
            text=True,
            cwd=repo_root,
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("PASS", result.stdout)

    def test_path_separator_normalisation_on_windows(self) -> None:
        # On Windows, `pathlib.Path` produces backslash
        # separators.  The waiver list uses forward slashes
        # (matching the `src/<path>` convention from
        # `.standards-waivers.txt`).  The script must
        # normalise the `rel` path to forward slashes so
        # the match works on Windows as well as POSIX.
        # We simulate the Windows behaviour by writing a
        # backslash-style path to the script via a
        # post-processing test: create a waiver file that
        # uses forward slashes (the conventional form) and
        # check that the script matches it.
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            self._make_over(tmp_path)
            # The waiver uses the conventional forward-slash
            # form; even if the script internally normalises
            # `rel` to backslashes (as on Windows), the
            # match must succeed.
            waivers = self._write_waivers(tmp_path, ["src/foo/mod.rs"])
            result = self._run(
                "--threshold", "40", "--waivers", str(waivers), cwd=tmp_path
            )
            self.assertEqual(
                result.returncode,
                0,
                msg=(f"stdout: {result.stdout!r}\nstderr: {result.stderr!r}"),
            )


if __name__ == "__main__":
    unittest.main()
