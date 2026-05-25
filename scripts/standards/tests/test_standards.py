"""Unit tests for the STD-01 engineering-standards gate scripts (#483).

Run with:
    python -m unittest scripts.standards.tests.test_standards
"""
from __future__ import annotations

import os
import sys
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SCRIPTS = ROOT / "scripts" / "standards"


def run(script: str, *args: str, cwd: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPTS / script), *args],
        cwd=cwd, capture_output=True, text=True,
    )


class LocGateTests(unittest.TestCase):
    def test_over_budget_fails(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            (tdp / "src").mkdir()
            big = tdp / "src" / "big.rs"
            big.write_text("\n".join(f"// line {i}" for i in range(700)), encoding="utf-8")
            (tdp / ".standards-waivers.txt").write_text("", encoding="utf-8")
            res = run("check_loc.py", "--root", str(tdp),
                     "--waivers", str(tdp / ".standards-waivers.txt"),
                     cwd=tdp)
            self.assertEqual(res.returncode, 1, res.stdout + res.stderr)
            self.assertIn("big.rs", res.stdout)

    def test_waiver_passes(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            (tdp / "src").mkdir()
            big = tdp / "src" / "big.rs"
            big.write_text("\n".join(f"// line {i}" for i in range(700)), encoding="utf-8")
            (tdp / ".standards-waivers.txt").write_text("src/big.rs\n", encoding="utf-8")
            res = run("check_loc.py", "--root", str(tdp),
                     "--waivers", str(tdp / ".standards-waivers.txt"),
                     cwd=tdp)
            self.assertEqual(res.returncode, 0, res.stdout + res.stderr)

    def test_under_budget_passes(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            (tdp / "src").mkdir()
            small = tdp / "src" / "small.rs"
            small.write_text("// only one line\n", encoding="utf-8")
            (tdp / ".standards-waivers.txt").write_text("", encoding="utf-8")
            res = run("check_loc.py", "--root", str(tdp),
                     "--waivers", str(tdp / ".standards-waivers.txt"),
                     cwd=tdp)
            self.assertEqual(res.returncode, 0, res.stdout + res.stderr)


class TodoGateTests(unittest.TestCase):
    def _scratch(self, content: str) -> Path:
        td = Path(tempfile.mkdtemp())
        (td / "src").mkdir()
        f = td / "src" / "x.rs"
        f.write_text(content, encoding="utf-8")
        return td

    def test_todo_without_issue_fails(self) -> None:
        td = self._scratch("// TODO: implement\nfn main() {}\n")
        res = run("check_todo_refs.py", str(td / "src"), cwd=td)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("TODO", res.stdout)

    def test_todo_with_issue_passes(self) -> None:
        td = self._scratch("// TODO(#42): implement\nfn main() {}\n")
        res = run("check_todo_refs.py", str(td / "src"), cwd=td)
        self.assertEqual(res.returncode, 0, res.stdout)

    def test_no_todo_passes(self) -> None:
        td = self._scratch("fn main() { let x = 1; }\n")
        res = run("check_todo_refs.py", str(td / "src"), cwd=td)
        self.assertEqual(res.returncode, 0, res.stdout)


class UnwrapGateTests(unittest.TestCase):
    def test_added_unwrap_fails(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            (tdp / "src").mkdir()
            src = tdp / "src" / "x.rs"
            src.write_text("fn run() { let _ = foo.unwrap(); }\n", encoding="utf-8")
            (tdp / "added.txt").write_text("src/x.rs:1\n", encoding="utf-8")
            res = run("check_unwrap.py", "--root", str(tdp),
                     "--added-lines-file", str(tdp / "added.txt"),
                     cwd=tdp)
            self.assertEqual(res.returncode, 1, res.stdout)

    def test_added_unwrap_in_test_module_passes(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            (tdp / "src").mkdir()
            src = tdp / "src" / "x.rs"
            src.write_text(
                "#[cfg(test)]\nmod tests {\n  #[test]\n  fn t() { foo.unwrap(); }\n}\n",
                encoding="utf-8")
            (tdp / "added.txt").write_text("src/x.rs:4\n", encoding="utf-8")
            res = run("check_unwrap.py", "--root", str(tdp),
                     "--added-lines-file", str(tdp / "added.txt"),
                     cwd=tdp)
            self.assertEqual(res.returncode, 0, res.stdout)

    def test_allow_marker_with_issue_passes(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            (tdp / "src").mkdir()
            src = tdp / "src" / "x.rs"
            src.write_text(
                "fn run() { let _ = foo.unwrap(); // allow-unwrap: #1\n}\n",
                encoding="utf-8")
            (tdp / "added.txt").write_text("src/x.rs:1\n", encoding="utf-8")
            res = run("check_unwrap.py", "--root", str(tdp),
                     "--added-lines-file", str(tdp / "added.txt"),
                     cwd=tdp)
            self.assertEqual(res.returncode, 0, res.stdout)


if __name__ == "__main__":
    unittest.main()
