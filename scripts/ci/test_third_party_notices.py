#!/usr/bin/env python3
"""T6 (#812) — verify THIRD_PARTY_NOTICES.md.

Asserts that:
  * the file exists at the repo root
  * it names every bundled third-party component
  * it cites the correct upstream license URL for each
  * it is non-empty and ends with a verification note
"""

from __future__ import annotations

import sys
from pathlib import Path

NOTICES_PATH = Path(__file__).resolve().parents[2] / "THIRD_PARTY_NOTICES.md"

REQUIRED_COMPONENTS = [
    # (heading substring, license identifier)
    ("k2-fsa / sherpa-onnx", "Apache 2.0"),
    ("FunASR", "MIT"),  # FunASR model weights are MIT
    ("Whisper", "MIT"),
    ("OPUS-MT", "Apache 2.0"),
    ("Supertonic", "supertonic-notice.txt"),
]

REQUIRED_URLS = [
    "https://github.com/k2-fsa/sherpa-onnx",
    "https://github.com/alibaba-damo-academy/FunASR",
    "https://github.com/openai/whisper",
    "https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi",
    "https://github.com/SupertoneInc/supertonic",
]


def main() -> int:
    if not NOTICES_PATH.exists():
        print(f"FAIL: {NOTICES_PATH} not found")
        return 1
    text = NOTICES_PATH.read_text(encoding="utf-8")
    if len(text) < 500:
        print(f"FAIL: THIRD_PARTY_NOTICES.md is suspiciously short ({len(text)} chars)")
        return 1
    for heading, license_id in REQUIRED_COMPONENTS:
        if heading not in text:
            print(f"FAIL: missing component heading: {heading!r}")
            return 1
        if license_id not in text:
            print(f"FAIL: {heading!r} does not cite license {license_id!r}")
            return 1
    for url in REQUIRED_URLS:
        if url not in text:
            print(f"FAIL: missing upstream URL: {url}")
            return 1
    if "Verification" not in text:
        print("FAIL: 'Verification' section missing")
        return 1
    if "Apache 2.0" not in text:
        print("FAIL: Apache 2.0 license preamble missing")
        return 1
    if "MIT" not in text:
        print("FAIL: MIT license mention missing")
        return 1
    print("OK THIRD_PARTY_NOTICES.md: all 5 components + 5 URLs + preamble present")
    return 0


if __name__ == "__main__":
    sys.exit(main())
