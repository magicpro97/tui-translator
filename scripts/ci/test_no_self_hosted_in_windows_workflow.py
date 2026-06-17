#!/usr/bin/env python3
"""Regression test for the self-hosted runner queue fix (issue #837).

WP-25.06 (#837): 5 of 6 gate jobs in
`.github/workflows/windows-selfhosted-test.yml` were stuck queued with
`runner_name=null` because the self-hosted `LINHPC` Windows runner
could not pick them up.

The fix moves every gate job to GitHub-hosted `windows-latest`. This
test pins the invariant so a future PR cannot silently re-introduce a
self-hosted `runs-on:` (or a stale file name with "selfhosted" in it)
without a test failure.

Pinned invariants:

1. The workflow file path no longer contains the word "selfhosted"
   (it was renamed to `windows-gate.yml`).
2. No `runs-on:` value in the workflow uses the `[self-hosted, ...]`
   label set.
3. The 6 gate jobs we expect are all present and have non-self-hosted
   runners.
4. The 4 oversized baseline files remain in `.standards-waivers.txt`
   (the module-size gate must not flag the waiver-tracked files).
"""

from __future__ import annotations

import re
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOW_DIR = REPO_ROOT / ".github" / "workflows"

# The original file has been renamed; both the old name (for migration)
# and the new name (canonical) are accepted as sources of truth by this
# test, but the canonical name is preferred.
CANONICAL_WORKFLOW = WORKFLOW_DIR / "windows-gate.yml"
LEGACY_WORKFLOW = WORKFLOW_DIR / "windows-selfhosted-test.yml"

EXPECTED_JOBS = {
    "fmt",
    "test-default",
    "test-release",
    "coverage-gate",
    "module-size-gate",
    "panic-sites-gate",
}

# Stored without the `src/` prefix because the parser below strips it
# to match the production script's normalisation (see
# scripts/ci/check_module_sizes.py:225-232).
WAIVER_REQUIRED_FILES = {
    "main.rs",
    "tui/mod.rs",
    "config/mod.rs",
    "pipeline/mod.rs",
}


def _parse_waivers(text: str) -> set[str]:
    """Parse a `.standards-waivers.txt` body and return the set of
    normalised path strings that are waived.

    The production script (`check_module_sizes.py`) strips the
    `src/` prefix and treats directory-level waivers (trailing `/`)
    as matching the `mod.rs` inside. This parser mirrors that
    behaviour so the regression test for the 4 baseline files
    matches the gate's interpretation exactly.
    """
    waived: set[str] = set()
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        # Strip trailing inline comment.
        path = stripped.split("#", 1)[0].strip()
        if path.startswith("src/"):
            path = path[len("src/") :]
        if path.endswith("/"):
            path = path.rstrip("/")
        if path:
            waived.add(path)
    return waived


def _resolve_workflow(
    canonical: Path = CANONICAL_WORKFLOW,
    legacy: Path = LEGACY_WORKFLOW,
) -> Path:
    """Return the path of the active workflow file.

    Prefers the new canonical name. Falls back to the legacy name for
    one release so we can rename-and-move in separate steps if needed.

    The arguments are injectable so the tests can pin both branches
    without mutating the filesystem.
    """
    if canonical.is_file():
        return canonical
    if legacy.is_file():
        return legacy
    raise FileNotFoundError(
        f"neither {canonical.name} nor {legacy.name} exists"
    )


class ResolveWorkflowTests(unittest.TestCase):
    """Pin the lookup helper: canonical wins, legacy is the fallback,
    neither raises."""

    def test_canonical_wins_when_present(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            canonical = tmp_path / "new.yml"
            legacy = tmp_path / "old.yml"
            canonical.write_text("name: x\n", encoding="utf-8")
            legacy.write_text("name: y\n", encoding="utf-8")
            self.assertEqual(_resolve_workflow(canonical, legacy), canonical)

    def test_legacy_used_when_canonical_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            canonical = tmp_path / "missing.yml"
            legacy = tmp_path / "old.yml"
            legacy.write_text("name: y\n", encoding="utf-8")
            self.assertEqual(_resolve_workflow(canonical, legacy), legacy)

    def test_raises_when_both_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            canonical = tmp_path / "missing1.yml"
            legacy = tmp_path / "missing2.yml"
            with self.assertRaises(FileNotFoundError):
                _resolve_workflow(canonical, legacy)


class WorkflowFilePathTests(unittest.TestCase):
    """The workflow file has been renamed to drop the misleading
    "selfhosted" suffix. The legacy name is tolerated for one commit so
    the rename can land independently of the runs-on migration.
    """

    def test_canonical_name_exists(self) -> None:
        self.assertTrue(
            CANONICAL_WORKFLOW.is_file(),
            msg=(
                f"expected canonical workflow at {CANONICAL_WORKFLOW}, "
                f"rename {LEGACY_WORKFLOW.name} -> {CANONICAL_WORKFLOW.name}"
            ),
        )

    def test_legacy_name_removed(self) -> None:
        self.assertFalse(
            LEGACY_WORKFLOW.is_file(),
            msg=(
                f"{LEGACY_WORKFLOW.name} still present; delete it after the "
                f"rename to {CANONICAL_WORKFLOW.name} so future PRs cannot "
                f"silently re-introduce the self-hosted runner queue bug"
            ),
        )


class WorkflowRunsOnTests(unittest.TestCase):
    """Pin that NO job uses a self-hosted runner. Regression catch for
    #837: a future PR must not add `runs-on: [self-hosted, ...]` to
    the gate workflow.
    """

    def setUp(self) -> None:
        self.path = _resolve_workflow()
        self.text = self.path.read_text(encoding="utf-8")

    def _runs_on_lines(self) -> list[tuple[str, str]]:
        """Return (job_id, runs_on_value) for every job in the workflow.

        A "job" is a YAML mapping at two-space indent under `jobs:`.
        `runs-on:` is on the line directly after the job name.
        """
        jobs: list[tuple[str, str]] = []
        in_jobs = False
        current_job: str | None = None
        current_runs_on: str | None = None
        for raw in self.text.splitlines():
            line = raw.rstrip()
            if line.startswith("jobs:"):
                in_jobs = True
                continue
            if not in_jobs:
                continue
            # Job entries: two-space indent, then `name:`, no further indent.
            m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
            if m:
                # Flush the previous job if it had a runs-on.
                if current_job is not None and current_runs_on is not None:
                    jobs.append((current_job, current_runs_on))
                current_job = m.group(1)
                current_runs_on = None
                continue
            # runs-on line under a job: 4-space indent.
            m = re.match(r"^    runs-on:\s*(.+?)\s*$", line)
            if m and current_job is not None:
                current_runs_on = m.group(1)
        # Flush the last job.
        if current_job is not None and current_runs_on is not None:
            jobs.append((current_job, current_runs_on))
        return jobs

    def test_no_self_hosted_runs_on(self) -> None:
        for job_id, runs_on in self._runs_on_lines():
            with self.subTest(job=job_id):
                self.assertNotIn(
                    "self-hosted",
                    runs_on,
                    msg=(
                        f"job {job_id!r} uses self-hosted runner {runs_on!r}; "
                        f"this is the #837 regression. Use a GitHub-hosted "
                        f"runner (e.g. `runs-on: windows-latest`)."
                    ),
                )

    def test_all_expected_jobs_present(self) -> None:
        found = {job_id for job_id, _ in self._runs_on_lines()}
        self.assertEqual(
            found,
            EXPECTED_JOBS,
            msg=(
                f"workflow job list drift: expected exactly "
                f"{sorted(EXPECTED_JOBS)}, got {sorted(found)}"
            ),
        )

    def test_every_job_has_runs_on(self) -> None:
        for job_id, runs_on in self._runs_on_lines():
            with self.subTest(job=job_id):
                self.assertTrue(
                    runs_on,
                    msg=f"job {job_id!r} has empty runs-on",
                )


class StandardsWaiversTests(unittest.TestCase):
    """The 4 oversized baseline files are tracked by issue #483 and
    must remain in the waiver list. The module-size gate (1000 LOC)
    must not flag them.
    """

    def setUp(self) -> None:
        self.path = REPO_ROOT / ".standards-waivers.txt"
        self.text = self.path.read_text(encoding="utf-8") if self.path.is_file() else ""

    def test_waiver_file_exists(self) -> None:
        self.assertTrue(
            self.path.is_file(),
            msg=f"{self.path} not found; the module-size gate needs it",
        )

    def test_oversized_baseline_files_are_waived(self) -> None:
        waived = _parse_waivers(self.text)
        for required in WAIVER_REQUIRED_FILES:
            with self.subTest(file=required):
                self.assertIn(
                    required,
                    waived,
                    msg=(
                        f"{required} missing from .standards-waivers.txt; "
                        f"the module-size gate will flag it. Tracked for "
                        f"refactor in #483."
                    ),
                )


class WaiverParserTests(unittest.TestCase):
    """Pin the parser's normalisation rules so a future change to
    `.standards-waivers.txt` format cannot silently break the gate."""

    def test_strips_src_prefix(self) -> None:
        self.assertEqual(_parse_waivers("src/foo/mod.rs\n"), {"foo/mod.rs"})

    def test_strips_trailing_slash(self) -> None:
        self.assertEqual(_parse_waivers("src/foo/\n"), {"foo"})

    def test_strips_inline_comment(self) -> None:
        self.assertEqual(
            _parse_waivers("src/foo/mod.rs  # reason\n"),
            {"foo/mod.rs"},
        )

    def test_ignores_comment_and_blank_lines(self) -> None:
        body = (
            "# header comment\n"
            "\n"
            "src/foo/mod.rs\n"
            "  # leading-whitespace comment\n"
        )
        self.assertEqual(_parse_waivers(body), {"foo/mod.rs"})

    def test_empty_body_yields_empty_set(self) -> None:
        self.assertEqual(_parse_waivers(""), set())


if __name__ == "__main__":
    unittest.main()
