#!/usr/bin/env python3
"""
preToolUse hook — block dangerous shell commands.

PURPOSE (plain language):
  This guard runs before every shell command the AI executes. It blocks
  operations that could destroy files, escalate privileges, or silently
  overwrite the remote repository history. Normal development commands
  (cargo build, cargo test, git add, git commit, git push --force-with-lease)
  are NOT blocked.

WHAT IS BLOCKED:
  - sudo / runas (privilege escalation)
  - rm -rf on filesystem root or the repo root directory
  - mkfs / diskpart / Windows format (disk wipe operations)
  - Remove-Item -Recurse -Force on drive root (Windows PowerShell equivalent)
  - curl/wget piped directly into a shell interpreter (supply-chain attack vector)
  - git push --force or -f (use --force-with-lease instead)
  - git push +refspec (force via refspec shorthand)
  - git reset --hard HEAD~2 or more (multi-commit hard history rewrite)
  - SQL DROP TABLE / DROP DATABASE

WHAT IS ALLOWED:
  - cargo clean (wipes only ./target, which is safe to regenerate)
  - git reset --hard HEAD~1 (single-commit undo is acceptable)
  - git push --force-with-lease (safe force push)
  - All normal cargo / rustup / git fetch / git pull operations
"""

import json
import os
import re
import sys

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


def recursive_force_rm_paths(command: str) -> list[str]:
    paths: list[str] = []
    for segment in re.split(r"[;&|]", command):
        tokens = segment.strip().split()
        if not tokens or tokens[0] != "rm":
            continue

        flags = [token for token in tokens[1:] if token.startswith("-")]
        has_recursive = any(
            token in {"-r", "-R", "--recursive"}
            or (token.startswith("-") and not token.startswith("--") and re.search(r"[rR]", token))
            for token in flags
        )
        has_force = any(
            token in {"-f", "--force"}
            or (token.startswith("-") and not token.startswith("--") and "f" in token.lower())
            for token in flags
        )
        if not (has_recursive and has_force):
            continue

        for token in tokens[1:]:
            if token.startswith("-"):
                continue
            paths.append(token.strip("\"'"))
    return paths


def main() -> int:
    try:
        data = json.loads(sys.stdin.read() or "{}")
    except json.JSONDecodeError:
        return 0
    if data.get("toolName") not in {"bash", "powershell"}:
        return 0

    command = str(get_args(data).get("command", ""))

    # ── Privilege escalation ────────────────────────────────────────────────
    if re.search(r"\b(sudo|runas)\b", command):
        deny("Privilege escalation (sudo/runas) blocked. Run commands as the current user.")

    # ── Filesystem destruction ──────────────────────────────────────────────
    for path in recursive_force_rm_paths(command):
        if path == "/":
            deny("Destructive rm -rf on filesystem root blocked.")

        if path in {".", "./"}:
            deny("rm -rf on repo root blocked. Use `cargo clean` to wipe ./target safely.")

        if path == "./target" or path.startswith("./target/"):
            continue
        if path == "./node_modules" or path.startswith("./node_modules/"):
            continue

    if re.search(r"\b(mkfs|diskpart)\b", command, re.IGNORECASE):
        deny("Disk format operations (mkfs/diskpart) blocked.")

    if re.search(r"\bformat\s+[A-Z]:\b", command, re.IGNORECASE):
        deny("Windows drive format command blocked.")

    # Windows PowerShell recursive force-delete on drive root
    if re.search(r"Remove-Item.*-Recurse.*-Force\s+(C:\\|/)", command, re.IGNORECASE):
        deny("Destructive Remove-Item on drive root blocked.")

    # ── Download-and-execute pipeline ───────────────────────────────────────
    # Piping downloaded content directly into an interpreter is a supply-chain risk
    if re.search(r"(curl|wget)\b.*\|\s*(bash|sh|python3?|pwsh|powershell)", command, re.IGNORECASE):
        deny(
            "Download-and-execute pipeline blocked. Download the script, inspect it, "
            "then run it separately."
        )

    # ── Force push (history rewrite without a safety lease) ─────────────────
    push_m = re.search(r"(?:^|[;&|\"'\s])git\s+push(?:\s+|$)(.*)", command, re.IGNORECASE)
    if push_m:
        push_args = push_m.group(1).replace('"', " ").replace("'", " ")
        push_args_without_lease = re.sub(
            r"(^|\s)--force-with-lease(?:=[^\s]+)?",
            " ",
            push_args,
            flags=re.IGNORECASE,
        )
        # --force or --force= (but NOT --force-with-lease)
        if re.search(r"(^|\s)--force([=\s;|&]|$)", push_args_without_lease):
            deny(
                "git push --force blocked. Use --force-with-lease to protect "
                "collaborators' commits."
            )
        if re.search(r"(^|\s)-[^-\s]*f[^\s]*([\s;|&]|$)", push_args):
            deny("git push -f blocked. Use --force-with-lease.")
        if re.search(r"(^|\s)\+[^\s]+", push_args):
            deny("git push via +refspec (force) blocked. Use --force-with-lease.")

    # ── Multi-commit hard reset ──────────────────────────────────────────────
    # HEAD~1 is acceptable; HEAD~2 and beyond silently discards reviewed history
    if re.search(r"git\s+reset\s+--hard\s+HEAD~(?:[2-9]|\d{2,})(\s|$)", command):
        deny(
            "git reset --hard HEAD~2+ blocked. Create a revert commit to preserve history, "
            "or use --soft/--mixed for single-commit adjustments."
        )

    # ── SQL destructive operations ───────────────────────────────────────────
    if re.search(r"\bDROP\s+(TABLE|DATABASE)\b", command, re.IGNORECASE):
        deny("SQL DROP TABLE / DROP DATABASE blocked.")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
