"""Verification script for the JV-02 synthetic seed fixture.

Implements the JV-03 validator rules (V1..V10) and the JV-02 redaction scan
(R1..R11) against `synthetic_seed.jsonl`.  Pure stdlib, no network.

Exit code 0 = all checks pass.  Non-zero = at least one failure printed.
"""
from __future__ import annotations

import hashlib
import json
import re
import sys
import unicodedata
from pathlib import Path

HERE = Path(__file__).resolve().parent
JSONL = HERE / "synthetic_seed.jsonl"
MANIFEST = HERE / "synthetic_seed_manifest.json"
DENYLIST = HERE / "pii_denylist.txt"

ALLOWED_CATEGORIES = {
    "short", "medium", "long", "honorific", "disfluent", "technical", "named-entity",
}
ALLOWED_SOURCES = {"flores200", "alt", "tatoeba", "synthetic"}
ALLOWED_LICENSES = {"CC0-1.0", "CC-BY-4.0", "CC-BY-SA-4.0", "CC-BY-2.0-FR"}
ID_RE = re.compile(r"^jv-(flores200|alt|tatoeba|syn)-[0-9]{6}$")

REDACTION = [
    ("R1-google-api-key",  re.compile(r"AIza[0-9A-Za-z_\-]{35}")),
    ("R2-openai-key",      re.compile(r"sk-[A-Za-z0-9]{20,}")),
    ("R3-slack-token",     re.compile(r"xox[abprs]-[A-Za-z0-9\-]{10,}")),
    ("R4-github-pat",      re.compile(r"gh[oprsu]_[A-Za-z0-9]{30,}")),
    ("R5-auth-header",     re.compile(r"(?i)(authorization|bearer|basic)\s+[A-Za-z0-9+/=._\-]{8,}")),
    ("R6-email",           re.compile(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}")),
    ("R7-phone-generic",   re.compile(r"\+?\d[\d\s\-]{8,}\d")),
    ("R8-jp-phone",        re.compile(r"0\d{1,4}-\d{1,4}-\d{3,4}")),
    ("R9-credit-card",     re.compile(r"\b(?:\d[ \-]?){13,19}\b")),
    ("R10-meeting-url",    re.compile(r"(?i)(zoom\.us/j/|teams\.microsoft\.com/l/meetup-join/)\S+")),
]


def fail(msg: str, errors: list[str]) -> None:
    errors.append(msg)


def luhn_valid(value: str) -> bool:
    digits = [int(ch) for ch in re.sub(r"\D", "", value)]
    if len(digits) < 13:
        return False
    checksum = 0
    parity = len(digits) % 2
    for idx, digit in enumerate(digits):
        if idx % 2 == parity:
            digit *= 2
            if digit > 9:
                digit -= 9
        checksum += digit
    return checksum % 10 == 0


def load_denylist() -> list[str]:
    terms = []
    for line in DENYLIST.read_text(encoding="utf-8").splitlines():
        term = line.strip()
        if term and not term.startswith("#"):
            terms.append(term)
    return terms


def main() -> int:
    errors: list[str] = []
    denylist_terms = load_denylist()

    raw = JSONL.read_bytes()

    # Line endings: \r\n forbidden (V6).
    if b"\r\n" in raw:
        fail("V6: \\r\\n line endings detected", errors)

    # Trailing single newline, no double-trailing newline (V6).
    if not raw.endswith(b"\n"):
        fail("V6: missing trailing newline", errors)
    if raw.endswith(b"\n\n"):
        fail("V6: extra trailing newline", errors)

    # NFC normalisation (V6).
    text = raw.decode("utf-8")
    if unicodedata.normalize("NFC", text) != text:
        fail("V6: file is not NFC-normalised", errors)

    rows = []
    for ln, line in enumerate(raw.splitlines(), start=1):
        s = line.decode("utf-8")
        try:
            row = json.loads(s)
        except json.JSONDecodeError as e:
            fail(f"line {ln}: invalid JSON: {e}", errors)
            continue
        rows.append((ln, row))

    # Ordering: ascending by id (V6).
    ids = [r["id"] for _, r in rows if "id" in r]
    if ids != sorted(ids):
        fail("V6: rows not sorted by id ascending", errors)

    seen_ids: set[str] = set()
    for ln, row in rows:
        rid = row.get("id", "<missing>")

        # V1 duplicate
        if rid in seen_ids:
            fail(f"V1: duplicate id {rid} at line {ln}", errors)
        seen_ids.add(rid)

        # V5 id format
        if not ID_RE.match(rid):
            fail(f"V5: invalid id format {rid!r} at line {ln}", errors)

        # V2 empty ja
        ja = row.get("ja", "")
        if not isinstance(ja, str) or not ja.strip():
            fail(f"V2: empty ja at {rid}", errors)

        # V3 refs
        refs = row.get("vi_refs")
        if not isinstance(refs, list) or len(refs) == 0:
            fail(f"V3: missing or empty vi_refs at {rid}", errors)
        else:
            for i, r in enumerate(refs):
                if not isinstance(r, str) or not r.strip():
                    fail(f"V2: empty vi_refs[{i}] at {rid}", errors)

        # V4 language tags
        if row.get("lang_src") != "ja-JP":
            fail(f"V4: lang_src must be ja-JP at {rid}", errors)
        if row.get("lang_tgt") != "vi-VN":
            fail(f"V4: lang_tgt must be vi-VN at {rid}", errors)

        # V8 category
        if row.get("category") not in ALLOWED_CATEGORIES:
            fail(f"V8: invalid category {row.get('category')!r} at {rid}", errors)

        # V9 source
        if row.get("source") not in ALLOWED_SOURCES:
            fail(f"V9: unknown source {row.get('source')!r} at {rid}", errors)

        # V10 license
        if row.get("license") not in ALLOWED_LICENSES:
            fail(f"V10: disallowed license {row.get('license')!r} at {rid}", errors)

        # Redaction scan over ja and vi_refs joined.
        haystack = "\n".join(
            [ja] + (refs if isinstance(refs, list) else [])
        )
        for name, pat in REDACTION:
            m = pat.search(haystack)
            if m:
                if name == "R9-credit-card" and not luhn_valid(m.group(0)):
                    continue
                fail(f"REDACTION {name}: matched {m.group(0)!r} at {rid}", errors)
        for term in denylist_terms:
            if term in haystack:
                fail(f"REDACTION R11-denylist: matched {term!r} at {rid}", errors)

    # V7 hash check.
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    expected = manifest.get("corpus_sha256", "")
    actual = hashlib.sha256(raw).hexdigest()
    if expected != actual:
        fail(f"V7: corpus_sha256 mismatch: manifest={expected} actual={actual}", errors)

    # Coverage: synthetic seed must touch every allowed category.
    found_cats = {row.get("category") for _, row in rows}
    missing = ALLOWED_CATEGORIES - found_cats
    if missing:
        fail(f"coverage: seed missing categories {sorted(missing)}", errors)

    if errors:
        print("FAIL")
        for e in errors:
            print("  -", e)
        return 1

    print(f"OK  rows={len(rows)}  sha256={actual}  categories={sorted(found_cats)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
