#!/usr/bin/env python3
"""Conventional Commits + DCO sign-off gate (issue #483).

Validates commit messages on the current PR range. Expects to be invoked
with `--base <sha>` and `--head <sha>`; in CI these come from the GitHub
event payload.

A title is accepted when it matches:
    type(scope?)!?: short summary

with `type` in {feat, fix, docs, style, refactor, perf, test, build,
ci, chore, revert} and total length <= 100 chars (90 for the summary).

Each commit body must contain a `Signed-off-by:` trailer (DCO).

Merge commits (commit with >1 parents) are skipped.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys

TYPES = "feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert"
TITLE_RE = re.compile(rf"^({TYPES})(\([^)]+\))?!?: .{{1,90}}$")


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], encoding="utf-8", errors="replace")


def commits(base: str, head: str) -> list[str]:
    out = git("rev-list", "--no-merges", f"{base}..{head}")
    return [c for c in out.splitlines() if c]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", required=True)
    ap.add_argument("--head", required=True)
    ap.add_argument("--no-signoff", action="store_true",
                    help="Skip DCO sign-off requirement.")
    args = ap.parse_args()

    failures: list[str] = []
    for sha in commits(args.base, args.head):
        title = git("log", "-1", "--format=%s", sha).strip()
        body = git("log", "-1", "--format=%B", sha)
        short = sha[:8]
        if not TITLE_RE.match(title):
            failures.append(f"{short}: title does not match Conventional Commits: {title!r}")
        if not args.no_signoff and "Signed-off-by:" not in body:
            failures.append(f"{short}: missing `Signed-off-by:` trailer (use `git commit -s`)")

    if failures:
        print("Commit policy gate failed:")
        for line in failures:
            print(f"  - {line}")
        print("\nFix: rewrite with `git commit --amend -s` or `git rebase -i` to "
              "use Conventional Commits and add a DCO sign-off.")
        return 1
    print("Commit policy gate OK.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
