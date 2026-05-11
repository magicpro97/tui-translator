#!/usr/bin/env python3
"""
preToolUse hook — gate git commit operations for this Rust project.

PURPOSE (plain language):
  This guard runs before every `git commit` command. It inspects the files
  that are staged (about to be committed) and blocks the commit if any of
  the following problems are found. It does NOT slow down normal development —
  the check only runs at commit time, not on every file edit.

WHAT IS CHECKED:

  1. Rust debug macros in staged .rs files  [BLOCKED]
     dbg!() prints debug output to stderr and must be removed before committing.
     Use the `tracing` or `log` crate for any logging you want to keep.

  2. Credentials and secrets files staged for commit  [BLOCKED]
     Files named .env, google-credentials*.json, service-account*.json, or
     with the extensions .pem / .key / .p12 / .pfx must never be committed.
     These files contain API keys or private keys that would be exposed
     publicly if the repository is ever shared or made public.
     Add these files to .gitignore before staging them.

  3. Binary files over 1 MB  [BLOCKED]
     Large binary assets inflate the git repository size permanently and slow
     down clones and CI. Use Git LFS (`git lfs track "*.wav"`) for audio
     samples, recordings, and other large binary files.

  4. todo!() macro with no linked issue  [BLOCKED]
     A bare `todo!()` or `todo!("message without an issue number")` will panic
     if that code path is reached in a real meeting. Link every todo! to a
     GitHub issue number so it is tracked: `todo!("#42: implement retry")`.
     Or open a GitHub issue and remove the todo!() entirely.

WHAT IS NOT BLOCKED:
  - Cargo.lock (auto-managed by cargo, committing it is correct for binaries)
  - Normal .rs file changes without the above patterns
  - .env.example or .env.template files (they contain only placeholder values)
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

if os.name == "nt":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")


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


def staged_blob_text(rel: str) -> str:
    try:
        result = subprocess.run(
            ["git", "show", f":{rel}"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="ignore",
            timeout=10,
        )
        return result.stdout if result.returncode == 0 else ""
    except Exception:
        return ""


def staged_blob_size(rel: str) -> int:
    try:
        result = subprocess.run(
            ["git", "cat-file", "-s", f":{rel}"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        return int(result.stdout.strip()) if result.returncode == 0 else 0
    except Exception:
        return 0


def main() -> int:
    try:
        data = json.loads(sys.stdin.read() or "{}")
    except json.JSONDecodeError:
        return 0
    if data.get("toolName") not in {"bash", "powershell"}:
        return 0

    command = str(get_args(data).get("command", ""))
    # Only run for git commit commands
    if not re.search(r"\bgit\s+commit\b", command):
        return 0

    repo_root_str = git_output("rev-parse", "--show-toplevel").strip()
    repo_root = Path(repo_root_str) if repo_root_str else Path.cwd()

    staged = [line for line in git_output("diff", "--cached", "--name-only").splitlines() if line.strip()]

    for rel in staged:
        full_path = repo_root / rel

        # ── Credentials and secrets files ──────────────────────────────────
        # Match .env (but not .env.example or .env.template)
        if re.search(r"(^|[/\\])\.env$", rel):
            deny(
                f"COMMIT BLOCKED: {rel} is an environment file that may contain real secrets. "
                "Add `.env` to .gitignore. Commit `.env.example` with placeholder values instead."
            )

        # Google credential / service account JSON files
        if re.search(
            r"(google[_-]credentials?|service[_-]account|gcp[_-]key)[^/\\]*\.json$",
            rel,
            re.IGNORECASE,
        ):
            deny(
                f"COMMIT BLOCKED: {rel} looks like a Google service-account or credentials file. "
                "Never commit API keys or service-account files. "
                "Keep this file local and gitignored, or load credentials from environment variables."
            )

        # Certificate and private-key files
        if re.search(r"\.(pem|key|p12|pfx|jks|crt)$", rel, re.IGNORECASE):
            deny(
                f"COMMIT BLOCKED: {rel} is a certificate or private-key file. "
                "Private keys must never be committed to version control."
            )

        # ── Large binary files ──────────────────────────────────────────────
        blob_size = staged_blob_size(rel)
        if blob_size > 1_048_576:
            size_kb = blob_size // 1024
            deny(
                f"COMMIT BLOCKED: {rel} is {size_kb} KB (limit: 1024 KB). "
                "Track large files with Git LFS: `git lfs track \"*.wav\"` then re-stage."
            )

        # ── Rust source file checks ─────────────────────────────────────────
        if rel.endswith(".rs"):
            content = staged_blob_text(rel)
            if not content:
                continue

            for line in content.splitlines():
                stripped = line.strip()
                if stripped.startswith("//"):
                    continue

                code_only = line.split("//", 1)[0]

                # dbg!() — always a debug artifact
                if re.search(r"\bdbg!\s*\(", code_only):
                    deny(
                        f"COMMIT BLOCKED: `dbg!()` found in {rel}. "
                        "Remove debug macros before committing. "
                        "Use `tracing::debug!()` or `log::debug!()` for structured logging."
                    )

                # todo!() without a linked GitHub issue number
                # Match: todo!() with no args OR todo!(\"message without #NNN\")
                for m in re.finditer(r"\btodo!\s*\(([^)]*)\)", code_only):
                    inner = m.group(1).strip().strip('"').strip("'")
                    # Allow if the message contains a GitHub issue reference (#42) or keyword
                    if inner and re.search(r"#\d+|issue\s*\d+|ticket|GH-\d+", inner, re.IGNORECASE):
                        continue
                    deny(
                        f"COMMIT BLOCKED: `todo!({inner[:60]!r})` in {rel} has no issue reference. "
                        "A bare todo!() panics if reached in a live meeting. "
                        "Either link it to a GitHub issue — todo!(\"#42: description\") — "
                        "or open an issue and remove the todo!() macro."
                    )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
