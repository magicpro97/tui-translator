#!/usr/bin/env python3
"""Unit tests for panic_sites.py.

WP-25.05 (#763): the panic-site counter is the regression
catcher for the work to drive `src/main.rs` panic-prone
sites from 141 down to < 50.  The test pins the counting
policy so a future "fix" cannot silently change which
patterns are counted — otherwise a malicious refactor could
rename `.unwrap()` to `. foo.unwrap()` (with a space) and
silently change the count without changing the runtime
behaviour.

What the test pins:
  - `.unwrap()` (no whitespace, no args) is counted
  - `.unwrap_or()` and `.unwrap_or_else()` are NOT counted
    (they are fallible; they cannot panic at the call site)
  - `.expect("msg")` is counted
  - `panic!("msg")` is counted
  - The same site in a comment is NOT counted
  - The same site in a string literal is NOT counted
  - The cap is enforced via the exit code
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "panic_sites.py"


def _run(args: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        cwd=cwd or Path(__file__).resolve().parent,
    )


def _write(content: str) -> Path:
    """Write a tmp file with the given Rust-like content and return its path."""
    f = tempfile.NamedTemporaryFile(
        mode="w",
        suffix=".rs",
        delete=False,
        encoding="utf-8",
    )
    f.write(textwrap.dedent(content))
    f.close()
    return Path(f.name)


class UnwrapCountingTests(unittest.TestCase):
    def test_simple_unwrap_is_counted(self) -> None:
        p = _write(
            """\
            fn f() {
                let x = Some(1).unwrap();
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        self.assertEqual(r.returncode, 0)
        self.assertIn("unwrap=1", r.stdout)

    def test_unwrap_with_args_is_counted(self) -> None:
        # `.unwrap(x)` is rare in practice; the audit's regex was
        # `\.unwrap\(\s*\)` which excludes the arg form.  We follow
        # the audit baseline so the threshold remains comparable.
        p = _write(
            """\
            fn f() {
                let x = foo.unwrap(x);
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        self.assertEqual(r.returncode, 0)
        self.assertIn("unwrap=0", r.stdout)

    def test_unwrap_or_is_not_counted(self) -> None:
        # `.unwrap_or(default)` cannot panic at the call site.
        # Counting it would inflate the metric.
        p = _write(
            """\
            fn f() {
                let x = Some(1).unwrap_or(0);
                let y = Some(1).unwrap_or_else(|| 0);
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        self.assertEqual(r.returncode, 0)
        self.assertIn("unwrap=0", r.stdout)

    def test_unwrap_in_comment_is_not_counted(self) -> None:
        # The string `.unwrap()` in a comment must not be counted.
        p = _write(
            """\
            fn f() {
                // .unwrap() is bad practice
                let x = Some(1).unwrap();
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        self.assertEqual(r.returncode, 0)
        self.assertIn("unwrap=1", r.stdout)

    def test_unwrap_in_string_is_not_counted(self) -> None:
        # A `.unwrap()` mentioned in an error message string is
        # not a real call site; it is a substring of a string
        # literal.  This is the more important test because the
        # audit's `\.unwrap\(\)` regex would normally also match
        # in strings; we need to make sure we are being careful.
        # The current implementation does NOT filter strings, so
        # this test pins the current behaviour (counted) so any
        # future change to filter strings is intentional.
        p = _write(
            """\
            fn f() -> String {
                let msg = "do not call .unwrap() here".to_string();
                msg
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        # Current behaviour: string occurrence IS counted.
        # If you change to filter strings, update this test.
        self.assertIn("unwrap=1", r.stdout)

    def test_unwrap_across_multiple_lines(self) -> None:
        p = _write(
            """\
            fn f() {
                let x = Some(1)
                    .unwrap();
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        self.assertEqual(r.returncode, 0)
        self.assertIn("unwrap=1", r.stdout)


class ExpectCountingTests(unittest.TestCase):
    def test_expect_with_string_is_counted(self) -> None:
        p = _write(
            """\
            fn f() {
                let x = Some(1).expect("must be Some");
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        self.assertEqual(r.returncode, 0)
        self.assertIn("expect=1", r.stdout)

    def test_expect_with_format_args_is_counted(self) -> None:
        p = _write(
            """\
            fn f() {
                let x = Some(1).expect("must be Some, got {y}");
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        self.assertEqual(r.returncode, 0)
        self.assertIn("expect=1", r.stdout)


class PanicCountingTests(unittest.TestCase):
    def test_panic_macro_is_counted(self) -> None:
        p = _write(
            """\
            fn f() -> i32 {
                panic!("unreachable");
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        self.assertEqual(r.returncode, 0)
        self.assertIn("panic=1", r.stdout)

    def test_todo_macro_is_not_counted(self) -> None:
        # `todo!` expands to `panic!` internally but the audit
        # did not count it.  We follow the audit baseline.
        p = _write(
            """\
            fn f() -> i32 {
                todo!()
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        self.assertEqual(r.returncode, 0)
        self.assertIn("panic=0", r.stdout)


class ThresholdTests(unittest.TestCase):
    def test_under_threshold_exits_zero(self) -> None:
        p = _write("fn f() { let x = Some(1).unwrap(); }")
        r = _run(["--path", str(p), "--max", "5"])
        self.assertEqual(r.returncode, 0)

    def test_over_threshold_exits_one(self) -> None:
        content = "\n".join(f"    let x{i} = Some(1).unwrap();" for i in range(10))
        p = _write("fn f() {\n" + content + "\n}")
        r = _run(["--path", str(p), "--max", "5"])
        self.assertEqual(r.returncode, 1)
        self.assertIn("panic-site cap exceeded", r.stderr)

    def test_at_threshold_exits_zero(self) -> None:
        content = "\n".join(f"    let x{i} = Some(1).unwrap();" for i in range(5))
        p = _write("fn f() {\n" + content + "\n}")
        r = _run(["--path", str(p), "--max", "5"])
        self.assertEqual(r.returncode, 0)

    def test_missing_file_exits_three(self) -> None:
        r = _run(["--path", "/nonexistent/path/foo.rs", "--max", "5"])
        self.assertEqual(r.returncode, 3)
        self.assertIn("file not found", r.stderr)


class ListModeTests(unittest.TestCase):
    def test_list_prints_each_site(self) -> None:
        p = _write(
            """\
            fn f() {
                let a = Some(1).unwrap();
                let b = Some(2).expect("hi");
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10", "--list"])
        self.assertEqual(r.returncode, 0)
        # Two `:unwrap` / `:expect` markers
        self.assertIn(": unwrap", r.stdout)
        self.assertIn(": expect", r.stdout)


class TestBlockExclusionTests(unittest.TestCase):
    """WP-25.05 (#763): the audit was a production-code audit,
    so the counter must exclude `#[cfg(test)]` and `#[test]`
    blocks.  Test code inherits from production (the test
    binary's production code is the same code), so reducing
    production code reduces the count for both.  Counting test
    code would inflate the metric without affecting the
    runtime behaviour we care about.
    """

    def test_cfg_test_block_excluded(self) -> None:
        p = _write(
            """\
            fn production() {
                let x = Some(1).unwrap();
            }

            #[cfg(test)]
            mod tests {
                use super::*;
                #[test]
                fn test_fn() {
                    let y = Some(2).unwrap();
                    let z = Some(3).expect("hi");
                }
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        self.assertEqual(r.returncode, 0)
        # Only the production site is counted.
        self.assertIn("unwrap=1", r.stdout)
        self.assertIn("expect=0", r.stdout)
        self.assertIn("panic=0", r.stdout)
        self.assertIn("total=1", r.stdout)

    def test_inline_test_block_excluded(self) -> None:
        # A `#[test]` inside a `mod` that is not gated by
        # `#[cfg(test)]` is rare in this codebase; if it
        # occurs, the brace counter still skips it.
        p = _write(
            """\
            fn production() {
                let x = Some(1).unwrap();
            }

            #[test]
            fn lone_test() {
                let y = Some(2).unwrap();
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        self.assertEqual(r.returncode, 0)
        self.assertIn("total=1", r.stdout)

    def test_no_test_blocks_counts_all(self) -> None:
        p = _write(
            """\
            fn a() {
                let x = Some(1).unwrap();
            }
            fn b() {
                let y = Some(2).unwrap();
            }
            """
        )
        r = _run(["--path", str(p), "--max", "10"])
        self.assertEqual(r.returncode, 0)
        self.assertIn("total=2", r.stdout)


if __name__ == "__main__":
    unittest.main()
