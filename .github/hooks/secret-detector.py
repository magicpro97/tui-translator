#!/usr/bin/env python3
"""
preToolUse hook — detect and block hardcoded secrets in file edits and creates.

PURPOSE (plain language):
  This guard inspects every file the AI writes or modifies. It looks for
  patterns that look like real API keys, tokens, or private keys and blocks
  the write if any are found. This protects against accidentally committing
  credentials to the git repository.

  This project uses Google Cloud APIs (Speech-to-Text, Cloud Translation,
  Text-to-Speech). A leaked Google API key results in unexpected billing
  charges on the project owner's account. Google service-account JSON files
  contain a private key that grants full API access.

WHAT IS BLOCKED:
  - Google API keys   (AIza...)
  - Google OAuth client secrets in JSON
  - Google service-account private keys in JSON
  - AWS Access Key IDs (AKIA...)
  - AWS Secret Access Keys
  - GitHub personal access tokens (ghp_, gho_, ghu_, ghs_, ghr_)
  - PEM private key blocks (-----BEGIN PRIVATE KEY / RSA PRIVATE KEY / etc.)
  - JWT tokens (three-part base64url format)
  - Generic high-entropy bearer / API key assignments

WHAT IS ALLOWED:
  - Placeholder strings ("YOUR_API_KEY_HERE", "<INSERT_KEY>", "REPLACE_ME", etc.)
  - Example / documentation values that contain the words "example", "dummy", "fake", "test"
  - Environment-variable references (e.g. std::env::var("GOOGLE_API_KEY"))

CORRECT PATTERN FOR THIS PROJECT:
  Keep real credentials out of tracked repository files.
  For this project, store the real Google API key in a local, gitignored
  `config.json` file or load it from an environment variable during startup.
  Placeholder values are allowed in `config.example.json` and docs.
"""

import json
import os
import re
import sys

if os.name == "nt":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

# Strings that indicate a value is intentionally a placeholder
PLACEHOLDER_RE = re.compile(
    r"(example|placeholder|todo|replace[_-]?me|your[_-]?|xxx+|000+|<[^>]+>|dummy|fake|test)",
    re.IGNORECASE,
)

# Each entry: (human-readable name, compiled regex)
SECRET_PATTERNS = (
    # Google API keys used by Speech / Translate / TTS in this project
    ("Google API Key", re.compile(r"AIza[0-9A-Za-z_-]{35}")),
    # Google OAuth2 client secret JSON field
    ("Google OAuth Client Secret", re.compile(r'"client_secret"\s*:\s*"[A-Za-z0-9_\-]{20,}"')),
    # Google service-account private key JSON field (grants full API access)
    (
        "Google Service Account Private Key",
        re.compile(r'"private_key"\s*:\s*"-----BEGIN (RSA )?PRIVATE KEY'),
    ),
    # AWS access key ID
    ("AWS Access Key ID", re.compile(r"\bAKIA[0-9A-Z]{16}\b")),
    # AWS secret access key assignment
    (
        "AWS Secret Access Key",
        re.compile(
            r"(AWS_SECRET_ACCESS_KEY|aws_secret_access_key|secretAccessKey|secret_access_key)"
            r"\s*[=:]\s*[A-Za-z0-9/+]{40}"
        ),
    ),
    # GitHub tokens (fine-grained and classic)
    ("GitHub Token", re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}")),
    # PEM private key blocks (TLS, SSH, service accounts)
    (
        "Private Key (PEM block)",
        re.compile(r"-----BEGIN\s+(RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----"),
    ),
    # JWT tokens — three base64url segments
    ("JWT Token", re.compile(r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.")),
    # Generic bearer/access/api_key literal assignment with a long value
    (
        "Hardcoded Bearer/Access/API Token",
        re.compile(
            r'(bearer[_-]?token|access[_-]?token|api[_-]?key)\s*=\s*["\'][A-Za-z0-9_\-\.]{30,}["\']',
            re.IGNORECASE,
        ),
    ),
)


def deny(secret_name: str) -> None:
    print(
        json.dumps(
            {
                "permissionDecision": "deny",
                "permissionDecisionReason": (
                    f"Potential secret detected: {secret_name}. "
                    "Do not hardcode credentials in tracked repository files. "
                    "For this project, keep real keys in local gitignored "
                    "`config.json` or load them from environment variables."
                ),
            }
        )
    )
    raise SystemExit(0)


def get_args(data: dict) -> dict:
    args = data.get("toolArgs", {})
    if isinstance(args, str):
        try:
            return json.loads(args)
        except json.JSONDecodeError:
            return {}
    return args if isinstance(args, dict) else {}


def main() -> int:
    try:
        data = json.loads(sys.stdin.read() or "{}")
    except json.JSONDecodeError:
        return 0
    if data.get("toolName") not in {"edit", "create"}:
        return 0

    args = get_args(data)
    content = str(args.get("file_text") or args.get("new_str") or "")
    if not content:
        return 0

    for name, pattern in SECRET_PATTERNS:
        for match in pattern.finditer(content):
            matched_text = match.group(0)
            # Allow if the matched text itself looks like a placeholder
            if not PLACEHOLDER_RE.search(matched_text):
                deny(name)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
