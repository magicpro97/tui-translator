# `tests/fixtures/jv02/` — JV-02 corpus seed

This directory contains the **synthetic** seed fixture used by:

* JV-03 corpus validator tests
* JV-04 offline chrF++/BLEU sample run

It is **not** the 300-row benchmark corpus.  The full corpus is assembled at
build time from licensed public sources (FLORES-200, ALT, Tatoeba) plus this
synthetic supplement; see
[`docs/evidence/ja-vi-benchmark-corpus-plan.md`](../../../docs/evidence/ja-vi-benchmark-corpus-plan.md).

## Files

| File | Purpose |
|---|---|
| `synthetic_seed.jsonl` | 20 hand-written `ja→vi` rows, CC0-1.0, no PII. Covers all seven row categories. |
| `synthetic_seed_manifest.json` | Schema version, row count, SHA-256 of `synthetic_seed.jsonl`, seed, source attribution. |
| `generate_synthetic_seed.py` | Reproducible generator for the two files above. Pure stdlib. |
| `verify_synthetic_seed.py` | Reference implementation of the JV-03 validator rules (V1..V10) + JV-02 redaction scan (R1..R11) over the seed. Run `python verify_synthetic_seed.py` and expect `OK`. |
| `pii_denylist.txt` | Project-specific personal/company name deny-list; consumed by the JV-02 redaction scanner (rule R11). Empty in V1. |

## Privacy and licence

All rows in `synthetic_seed.jsonl` are **synthetic**, authored for this
project, and released under **CC0-1.0** (public domain dedication).  No real
meeting content, no real names, no real organisations, no secrets, no PII.

## Determinism

Rows are sorted by `id` ascending, UTF-8 NFC-normalised, separated by `\n`
(no `\r`), with a single trailing `\n`.  Run:

```powershell
# Verify hash (PowerShell, no Python)
$bytes = [System.IO.File]::ReadAllBytes("tests\fixtures\jv02\synthetic_seed.jsonl")
$sha   = [System.Security.Cryptography.SHA256]::Create().ComputeHash($bytes)
($sha | ForEach-Object { $_.ToString("x2") }) -join ""
```

The output must equal `corpus_sha256` in `synthetic_seed_manifest.json`.
