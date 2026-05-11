#!/usr/bin/env python3
"""
preToolUse hook — gate task_complete until build and test evidence exists.

PURPOSE (plain language):
  This guard runs before the AI declares a task "complete". It checks whether
  there is recent evidence that the Rust project actually builds and passes
  tests. Declaring a task complete without running `cargo test` means the next
  developer (or CI pipeline) may discover broken code after the fact.

  The verification plan for this project (docs/04-verification-plan.md) requires:
    Layer 1: cargo build succeeds — zero compile errors, zero warnings-as-errors
    Layer 1: cargo test passes — all unit tests green, zero skips
    Layer 1: cargo clippy passes — no lint errors

HOW THIS WORKS:
  After running `cargo test` (or `cargo build`) successfully, the AI or
  developer should touch the evidence marker:

    Windows (PowerShell):
      New-Item -Force ".copilot-state" -ItemType Directory | Out-Null
      [System.DateTimeOffset]::UtcNow.ToUnixTimeSeconds() |
        Set-Content ".copilot-state\\cargo-test-pass"

    Unix (bash):
      mkdir -p .copilot-state
      date +%s > .copilot-state/cargo-test-pass

  The marker expires after 2 hours. After that, run cargo test again.

WHEN THIS HOOK TRIGGERS:
  - Only on task_complete
  - Only when Rust source files (.rs) or Cargo.toml exist in the repo
    (if the project has no Rust files yet, the gate is skipped)
  - Only when there are uncommitted changes to .rs or Cargo.toml files,
    OR when the marker file is absent / expired

WHEN THIS HOOK DOES NOT TRIGGER:
  - Documentation-only changes (no .rs or Cargo.toml modifications)
  - The evidence marker exists and is less than 2 hours old
"""

import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

if os.name == "nt":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

# Where the evidence marker lives relative to the repository root.
EVIDENCE_MARKER_RELATIVE = Path(".copilot-state") / "cargo-test-pass"

# How long the marker is considered fresh (2 hours)
MAX_EVIDENCE_AGE_SECONDS = 2 * 3600


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


def git_output(*args: str) -> str:
    try:
        result = subprocess.run(["git", *args], capture_output=True, text=True, timeout=10)
        return result.stdout if result.returncode == 0 else ""
    except Exception:
        return ""


def repo_root() -> Path:
    root_str = git_output("rev-parse", "--show-toplevel").strip()
    return Path(root_str) if root_str else Path.cwd()


def repo_has_rust(root: Path) -> bool:
    """Return True if the project has any Rust source files."""
    # Quick scan: look for any .rs file or Cargo.toml
    for pattern in ("*.rs", "Cargo.toml"):
        if any(root.rglob(pattern)):
            return True
    return False


def has_rust_changes() -> bool:
    """
    Return True if there are staged or unstaged modifications to Rust source
    or Cargo manifest files. This is what makes verification mandatory.
    """
    # Staged changes (ready to commit)
    staged = git_output("diff", "--cached", "--name-only")
    # Unstaged tracked changes
    unstaged = git_output("diff", "--name-only")
    # Untracked files are invisible to `git diff`, so include them explicitly.
    untracked = git_output("ls-files", "--others", "--exclude-standard")
    all_changed = "\n".join(part for part in (staged, unstaged, untracked) if part)
    return bool(re.search(r"\.(rs|toml)$", all_changed, re.MULTILINE))


def evidence_marker(root: Path) -> Path:
    return root / EVIDENCE_MARKER_RELATIVE


def read_marker(marker: Path) -> float:
    """
    Return the Unix timestamp stored in the evidence marker, or 0.0 if the
    marker does not exist or cannot be parsed.
    """
    try:
        return float(marker.read_text(encoding="utf-8").strip())
    except Exception:
        return 0.0


def main() -> int:
    try:
        data = json.loads(sys.stdin.read() or "{}")
    except json.JSONDecodeError:
        return 0
    if data.get("toolName") != "task_complete":
        return 0

    root = repo_root()

    # Skip if the project has no Rust files yet (early scaffolding phase)
    if not repo_has_rust(root):
        return 0

    # Skip if there are no Rust-related changes (pure doc / config task)
    if not has_rust_changes():
        return 0

    # Read the evidence marker
    marker = evidence_marker(root)
    marker_ts = read_marker(marker)
    now = time.time()
    age_seconds = now - marker_ts if marker_ts > 0 else float("inf")

    if marker_ts == 0.0:
        deny(
            "TASK_COMPLETE BLOCKED: No `cargo test` evidence found. "
            "Run `cargo test` (and optionally `cargo clippy`) to verify the build. "
            "Then create the evidence marker so this gate passes:\n"
            "  PowerShell: "
            "New-Item -Force .copilot-state -ItemType Directory | Out-Null; "
            "[System.DateTimeOffset]::UtcNow.ToUnixTimeSeconds() | "
            "Set-Content .copilot-state\\cargo-test-pass\n"
            "  Bash: mkdir -p .copilot-state && date +%s > .copilot-state/cargo-test-pass\n"
            "The marker expires after 2 hours."
        )

    if age_seconds > MAX_EVIDENCE_AGE_SECONDS:
        age_hours = age_seconds / 3600
        deny(
            f"TASK_COMPLETE BLOCKED: `cargo test` evidence is {age_hours:.1f}h old "
            f"(max {MAX_EVIDENCE_AGE_SECONDS // 3600}h). "
            "Re-run `cargo test` and refresh the evidence marker before completing the task."
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
