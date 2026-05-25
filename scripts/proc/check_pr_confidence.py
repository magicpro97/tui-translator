#!/usr/bin/env python3
"""PROC-01 Opus review gate (issue #465).

Parses a pull-request body and the list of files it touches, and decides
whether the PR satisfies the Opus review gate:

  - Body MUST contain a ``Confidence: <value>`` line whose value is one of
    the allowed confidence options.
  - When Confidence < 1.0 OR any touched file is under one of the sensitive
    path prefixes (``src/audio/``, ``src/providers/``, ``src/pipeline/``),
    the body MUST also contain an ``### Opus review evidence`` section with
    non-empty, non-placeholder content.

The script is invoked by ``.github/workflows/proc-opus-gate.yml`` and exits
non-zero on a gate failure so the workflow check turns red.

Usage::

    python scripts/proc/check_pr_confidence.py \
        --body-file pr_body.md \
        --files-file changed_files.txt

Exit codes:
    0 — gate satisfied (or override accepted)
    1 — gate failed (reason printed to stdout)
    2 — invalid invocation
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, List, Optional, Tuple

ALLOWED_CONFIDENCE = {"1.0", "0.9", "0.8", "0.7", "0.6", "<0.6"}
SENSITIVE_PREFIXES = ("src/audio/", "src/providers/", "src/pipeline/")
CONFIDENCE_RE = re.compile(
    r"^\s*Confidence\s*:\s*(?P<value>\S[^\n]*?)\s*$",
    re.IGNORECASE | re.MULTILINE,
)
EVIDENCE_HEADING_RE = re.compile(
    r"^#{2,4}\s*Opus review evidence\s*$",
    re.IGNORECASE | re.MULTILINE,
)
OVERRIDE_RE = re.compile(
    r"override\s*:\s*@?[A-Za-z0-9_\-]+\s*:\s*\S+",
    re.IGNORECASE,
)


@dataclass
class GateResult:
    ok: bool
    confidence: Optional[str]
    needs_evidence: bool
    sensitive_hits: List[str]
    reasons: List[str]

    def summary(self) -> str:
        lines = [
            f"confidence={self.confidence!r}",
            f"needs_evidence={self.needs_evidence}",
            f"sensitive_hits={self.sensitive_hits}",
            f"ok={self.ok}",
        ]
        if self.reasons:
            lines.append("reasons:")
            lines.extend(f"  - {r}" for r in self.reasons)
        return "\n".join(lines)


def _normalise_confidence(raw: str) -> Optional[str]:
    s = raw.strip().strip("`").strip()
    # Accept "<0.6 (spike required)" by collapsing to "<0.6".
    if s.startswith("<"):
        return "<0.6" if s.startswith("<0.6") else None
    # Strip parenthetical clarifications.
    s = s.split()[0]
    return s if s in ALLOWED_CONFIDENCE else None


def parse_confidence(body: str) -> Optional[str]:
    """Return the normalised Confidence value or ``None`` if missing/invalid."""
    for m in CONFIDENCE_RE.finditer(body):
        normalised = _normalise_confidence(m.group("value"))
        if normalised is not None:
            return normalised
    return None


def has_evidence_section(body: str) -> bool:
    """Return True when the body has a non-empty Opus review evidence section."""
    match = EVIDENCE_HEADING_RE.search(body)
    if not match:
        return False
    tail = body[match.end():]
    # Stop at the next heading of equal-or-higher level.
    next_heading = re.search(r"^#{1,4}\s", tail, re.MULTILINE)
    section = tail if not next_heading else tail[: next_heading.start()]
    # Strip HTML comments before evaluating emptiness.
    section = re.sub(r"<!--.*?-->", "", section, flags=re.DOTALL)
    stripped = section.strip()
    if not stripped:
        return False
    # "N/A" answers are only acceptable when evidence is NOT required; callers
    # handle that separately. Here we only check that the section has real
    # content (more than a placeholder).
    if stripped.lower().startswith("n/a"):
        return True
    # Reject obvious placeholder leftovers.
    placeholders = ("tbd", "todo", "fill me", "<paste")
    return not any(stripped.lower().startswith(p) for p in placeholders)


def section_text(body: str, heading_re: re.Pattern) -> str:
    match = heading_re.search(body)
    if not match:
        return ""
    tail = body[match.end():]
    next_heading = re.search(r"^#{1,4}\s", tail, re.MULTILINE)
    section = tail if not next_heading else tail[: next_heading.start()]
    return re.sub(r"<!--.*?-->", "", section, flags=re.DOTALL).strip()


def sensitive_files(files: Iterable[str]) -> List[str]:
    return [f for f in files if any(f.startswith(p) for p in SENSITIVE_PREFIXES)]


def evaluate(body: str, files: Iterable[str]) -> GateResult:
    confidence = parse_confidence(body)
    hits = sensitive_files(files)
    reasons: List[str] = []

    if confidence is None:
        reasons.append(
            "PR body is missing a valid `Confidence:` line "
            f"(allowed values: {sorted(ALLOWED_CONFIDENCE)})."
        )

    needs_evidence = bool(hits) or (
        confidence is not None and confidence != "1.0"
    )

    if needs_evidence:
        evidence_text = section_text(body, EVIDENCE_HEADING_RE)
        if not has_evidence_section(body):
            reasons.append(
                "Opus review evidence section is required (sensitive paths or "
                "confidence < 1.0) but is missing or empty."
            )
        elif evidence_text.lower().startswith("n/a"):
            reasons.append(
                "Opus review evidence is required but body says 'N/A'. "
                "Provide reviewer + verdict + link, or an explicit "
                "'override: <handle>: <reason>' line."
            )
        elif confidence is not None and confidence != "1.0":
            # For confidence<1.0 require either a link or an override line.
            link_re = re.compile(r"https?://\S+")
            if not link_re.search(evidence_text) and not OVERRIDE_RE.search(
                evidence_text
            ):
                reasons.append(
                    "Confidence < 1.0 requires either a spike evidence URL "
                    "or a 'override: <user>: <reason>' line in the Opus "
                    "review evidence section."
                )

    return GateResult(
        ok=not reasons,
        confidence=confidence,
        needs_evidence=needs_evidence,
        sensitive_hits=hits,
        reasons=reasons,
    )


def _read(path: Optional[str]) -> str:
    if not path:
        return ""
    return Path(path).read_text(encoding="utf-8")


def _read_lines(path: Optional[str]) -> List[str]:
    if not path:
        return []
    return [
        line.strip()
        for line in Path(path).read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--body-file", required=True)
    parser.add_argument(
        "--files-file",
        required=True,
        help="Newline-separated list of paths changed in the PR.",
    )
    parser.add_argument(
        "--github-output",
        default=None,
        help="If provided, write key=value outputs for downstream steps.",
    )
    args = parser.parse_args(argv)

    body = _read(args.body_file)
    files = _read_lines(args.files_file)
    result = evaluate(body, files)

    print(result.summary())
    if args.github_output:
        with open(args.github_output, "a", encoding="utf-8") as fh:
            fh.write(f"confidence={result.confidence or ''}\n")
            fh.write(f"needs_evidence={'true' if result.needs_evidence else 'false'}\n")
            fh.write(f"ok={'true' if result.ok else 'false'}\n")

    return 0 if result.ok else 1


if __name__ == "__main__":
    sys.exit(main())
