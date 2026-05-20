#!/usr/bin/env python3
"""
preToolUse hook — block commits that stage real session or audio archive artifacts.

PURPOSE (plain language):
  This guard runs before every `git commit` command and inspects the staged
  file list.  It blocks the commit when real measurement or privacy artifacts
  are about to be committed.  These files may contain meeting transcripts,
  audio recordings of all call participants, or API-key-bearing eval reports.

WHAT IS BLOCKED:
  - Any *.jsonl file outside the committed test-fixture directories
    (tests/fixtures/ and tests/soak/).  JSONL files are the live session
    transcript log format written by session_store.
  - Any *.wav file outside the committed test-fixture directories.
    WAV files are the raw audio archive written when audio_archive.store_audio
    is enabled.
  - Any file inside a sessions/, audio-archive/, or eval-session/ directory.
    These directories store runtime measurement and evaluation artifacts.
  - *.session.jsonl files — the explicit session-log name pattern.

WHAT IS NOT BLOCKED:
  - tests/fixtures/*.jsonl  (committed test fixtures — sample session logs)
  - tests/fixtures/*.wav    (committed test fixtures — sample audio for STT tests)
  - tests/soak/soak_audio.wav (committed soak test WAV)
  - Normal source files, documentation, and configuration

WHY THIS MATTERS:
  Session JSONL logs contain transcript text of every utterance heard during a
  meeting, timestamps, speaker costs, and session metadata.  Audio archive WAV
  files contain the raw audio of every meeting participant.  Committing either
  type of file is a privacy violation and may expose real conversation content
  or API billing data in the public repository.
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import NamedTuple

if os.name == "nt":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

# Directories that contain COMMITTED test fixture JSONL/WAV files.
# Files under these paths are allowed even if they match the blocked patterns.
FIXTURE_PREFIXES = (
    "tests/fixtures/",
    "tests\\fixtures\\",
    "tests/soak/",
    "tests\\soak\\",
)

# Blocked directory names — any file under these dirs is rejected.
BLOCKED_DIRS = re.compile(
    r"(^|[/\\])(sessions|audio-archive|eval-session)[/\\]",
    re.IGNORECASE,
)

# Blocked file extensions and name patterns outside fixture dirs.
BLOCKED_EXTENSIONS = re.compile(r"\.(jsonl|wav)$", re.IGNORECASE)
BLOCKED_NAME_PATTERN = re.compile(r"\.session\.jsonl$", re.IGNORECASE)
GIT_COMMIT_COMMAND = re.compile(
    r"\bgit(?:\.exe)?"
    r"(?:\s+(?:"
    r"--no-pager|--paginate|--bare|--literal-pathspecs|--no-optional-locks|"
    r"--git-dir=\S+|--work-tree=\S+|--namespace=\S+|"
    r"(?:-C|-c|--git-dir|--work-tree|--namespace)\s+\S+"
    r"))*"
    r"\s+commit(?=\s|$)",
    re.IGNORECASE,
)


class ArtifactDecision(NamedTuple):
    blocked: bool
    reason: str


def deny(reason: str) -> None:
    print(json.dumps({"permissionDecision": "deny", "permissionDecisionReason": reason}))
    raise SystemExit(0)


def get_args(data: dict) -> dict:
    args = data.get("toolArgs", {})
    if isinstance(args, str):
        try:
            return json.loads(args)
        except json.JSONDecodeError:
            return {}
    return args if isinstance(args, dict) else {}


def git_output_or_deny(*args: str) -> str:
    try:
        result = subprocess.run(["git", *args], capture_output=True, text=True, timeout=10)
    except FileNotFoundError:
        deny("COMMIT BLOCKED: git executable was not found, so staged privacy artifacts could not be inspected.")
    except subprocess.TimeoutExpired:
        deny("COMMIT BLOCKED: git staged-file inspection timed out; refusing to commit privacy artifacts blindly.")

    if result.returncode != 0:
        stderr = (result.stderr or "").strip()
        detail = f" Git error: {stderr[:240]}" if stderr else ""
        deny(f"COMMIT BLOCKED: git staged-file inspection failed.{detail}")

    return result.stdout


def is_fixture(rel: str) -> bool:
    """Return True if rel is inside a committed test-fixture directory."""
    # Normalise to forward slashes for prefix matching
    norm = rel.replace("\\", "/")
    return any(norm.startswith(prefix.replace("\\", "/")) for prefix in FIXTURE_PREFIXES)


def is_git_commit_command(command: str) -> bool:
    """Return True for common git commit invocations, including global git options."""
    return bool(GIT_COMMIT_COMMAND.search(command))


def artifact_decision(rel: str) -> ArtifactDecision:
    """Return whether a staged relative path is a blocked privacy artifact."""
    if is_fixture(rel):
        return ArtifactDecision(False, "")

    if BLOCKED_DIRS.search(rel):
        return ArtifactDecision(
            True,
            f"COMMIT BLOCKED: {rel} is inside a runtime artifact directory "
            "(sessions/, audio-archive/, or eval-session/).  "
            "These directories hold real meeting transcripts, audio recordings, "
            "or evaluation reports that must never enter version control.  "
            "Add the directory to .gitignore and un-stage the file: "
            "`git restore --staged <file>`",
        )

    if BLOCKED_EXTENSIONS.search(rel) or BLOCKED_NAME_PATTERN.search(rel):
        ext = Path(rel).suffix.upper()
        return ArtifactDecision(
            True,
            f"COMMIT BLOCKED: {rel} is a {ext} file that may contain "
            "real session transcript data or recorded audio.  "
            "Only test fixtures under tests/fixtures/ and tests/soak/ are allowed.  "
            "If this is a new test fixture, move it under one of those directories.  "
            "If this is a real session or archive file, delete it and add the "
            "pattern to .gitignore.",
        )

    return ArtifactDecision(False, "")


def self_test() -> int:
    artifact_cases = {
        "tests/fixtures/session_log_v1.jsonl": False,
        "tests\\fixtures\\sample.wav": False,
        "tests/soak/soak_audio.wav": False,
        "session.session.jsonl": True,
        "captures/live.jsonl": True,
        "captures/live.wav": True,
        "sessions/live.txt": True,
        "audio-archive/live.txt": True,
        "eval-session/report.json": True,
        "src/main.rs": False,
    }
    command_cases = {
        "git commit -m probe": True,
        "git --no-pager commit -m probe": True,
        "git -C . commit -m probe": True,
        "git -c user.name=bot commit -m probe": True,
        "git.exe --work-tree=. commit": True,
        "git status": False,
        "git commit-tree HEAD": False,
    }

    failures = [
        (path, expected, artifact_decision(path).blocked)
        for path, expected in artifact_cases.items()
        if artifact_decision(path).blocked != expected
    ]
    failures.extend(
        (command, expected, is_git_commit_command(command))
        for command, expected in command_cases.items()
        if is_git_commit_command(command) != expected
    )
    if failures:
        for item, expected, actual in failures:
            print(f"FAIL {item}: expected {expected}, got {actual}")
        return 1

    print(f"artifact-guard self-test PASS ({len(artifact_cases) + len(command_cases)} cases)")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    try:
        data = json.loads(sys.stdin.read() or "{}")
    except json.JSONDecodeError:
        return 0
    if data.get("toolName") not in {"bash", "powershell"}:
        return 0

    command = str(get_args(data).get("command", ""))
    if not is_git_commit_command(command):
        return 0

    staged = [
        line for line in git_output_or_deny("diff", "--cached", "--name-only").splitlines() if line.strip()
    ]

    for rel in staged:
        decision = artifact_decision(rel)
        if decision.blocked:
            deny(decision.reason)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
