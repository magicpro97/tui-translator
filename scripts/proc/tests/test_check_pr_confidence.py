"""Unit tests for scripts.proc.check_pr_confidence (PROC-01, #465)."""

from __future__ import annotations

import unittest

from scripts.proc.check_pr_confidence import (
    ALLOWED_CONFIDENCE,
    evaluate,
    has_evidence_section,
    parse_confidence,
    sensitive_files,
)


BASE_BODY_TEMPLATE = """## Summary
Closes #999

## Verification
ran cargo test

## PROC-01 Opus review gate (#465)

Confidence: {conf}

### Opus review evidence

{evidence}
"""


def body(conf: str, evidence: str = "N/A — confidence 1.0, no sensitive paths touched") -> str:
    return BASE_BODY_TEMPLATE.format(conf=conf, evidence=evidence)


class ConfidenceParsingTests(unittest.TestCase):
    def test_parses_each_allowed_value(self) -> None:
        for v in ALLOWED_CONFIDENCE:
            self.assertEqual(parse_confidence(f"Confidence: {v}"), v, msg=v)

    def test_accepts_parenthetical_lt06(self) -> None:
        self.assertEqual(
            parse_confidence("Confidence: <0.6 (spike required)"),
            "<0.6",
        )

    def test_rejects_unknown_value(self) -> None:
        self.assertIsNone(parse_confidence("Confidence: 0.55"))
        self.assertIsNone(parse_confidence("Confidence: high"))

    def test_missing_line_returns_none(self) -> None:
        self.assertIsNone(parse_confidence("body without the field"))

    def test_placeholder_value_is_rejected(self) -> None:
        # Template default like "<!-- one of: 1.0, ... -->" is HTML-commented
        # away; if a developer leaves "<value>" literally it must NOT pass.
        self.assertIsNone(parse_confidence("Confidence: <value>"))


class EvidenceSectionTests(unittest.TestCase):
    def test_section_with_content_recognised(self) -> None:
        b = "### Opus review evidence\n\ntui-rust-code-reviewer: CLEAN\nhttps://x/1\n"
        self.assertTrue(has_evidence_section(b))

    def test_section_with_only_comment_is_empty(self) -> None:
        b = "### Opus review evidence\n\n<!-- placeholder -->\n"
        self.assertFalse(has_evidence_section(b))

    def test_missing_heading(self) -> None:
        self.assertFalse(has_evidence_section("nothing here"))


class SensitivePathTests(unittest.TestCase):
    def test_detects_each_prefix(self) -> None:
        files = [
            "src/audio/wasapi.rs",
            "src/providers/google/stt.rs",
            "src/pipeline/orchestrator.rs",
            "src/tui/layout.rs",
            "docs/foo.md",
        ]
        self.assertEqual(
            sensitive_files(files),
            [
                "src/audio/wasapi.rs",
                "src/providers/google/stt.rs",
                "src/pipeline/orchestrator.rs",
            ],
        )


class GateEvaluationTests(unittest.TestCase):
    def test_confidence_one_no_sensitive_paths_passes(self) -> None:
        res = evaluate(body("1.0"), ["src/tui/layout.rs"])
        self.assertTrue(res.ok, res.summary())
        self.assertFalse(res.needs_evidence)

    def test_missing_confidence_fails(self) -> None:
        res = evaluate("no field at all", ["docs/x.md"])
        self.assertFalse(res.ok)
        self.assertIn("Confidence:", " ".join(res.reasons))

    def test_sensitive_path_requires_evidence(self) -> None:
        # Confidence 1.0 but PR touches src/audio → evidence required.
        b = body("1.0", evidence="<!-- empty -->")
        res = evaluate(b, ["src/audio/wasapi.rs"])
        self.assertFalse(res.ok)
        self.assertTrue(res.needs_evidence)
        self.assertIn("evidence", " ".join(res.reasons).lower())

    def test_sensitive_path_with_evidence_passes(self) -> None:
        b = body("1.0", evidence="tui-rust-code-reviewer: CLEAN — https://github.com/x/y/issues/1")
        res = evaluate(b, ["src/audio/wasapi.rs"])
        self.assertTrue(res.ok, res.summary())

    def test_low_confidence_requires_link_or_override(self) -> None:
        b = body("0.8", evidence="reviewer said it was fine")
        res = evaluate(b, ["docs/x.md"])
        self.assertFalse(res.ok)
        self.assertIn("spike", " ".join(res.reasons).lower())

    def test_low_confidence_with_override_passes(self) -> None:
        b = body(
            "0.8",
            evidence="override: @magicpro97: accepted residual risk per #465",
        )
        res = evaluate(b, ["docs/x.md"])
        self.assertTrue(res.ok, res.summary())

    def test_low_confidence_with_link_passes(self) -> None:
        b = body("0.9", evidence="spike notes: https://example.com/spike")
        res = evaluate(b, ["docs/x.md"])
        self.assertTrue(res.ok, res.summary())

    def test_na_is_rejected_when_evidence_required(self) -> None:
        b = body("0.9", evidence="N/A")
        res = evaluate(b, ["docs/x.md"])
        self.assertFalse(res.ok)


if __name__ == "__main__":
    unittest.main()
