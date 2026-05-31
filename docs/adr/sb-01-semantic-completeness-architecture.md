# ADR: SB-01 — Semantic Sentence Completeness Architecture

**Status:** Accepted  
**Issue:** [#663 (SB-WBS epic)](https://github.com/magicpro97/tui-translator/issues/663)  
**Date:** 2024-01-01  
**Revised:** 2026-05-31 (SB-05 Tier 3 section added)

---

## Context

The live STT → MT pipeline flushes fragments as soon as the speech window closes. For
Subject-Object-Verb languages (Japanese, Korean, etc.), the verb — and thus the meaning
— arrives at the end of the clause. Mid-clause flushes produce low-quality translations:
partial prepositional phrases or bare topic markers arrive at the MT model without the
predicate they belong to.

Goal: hold STT fragments until a clause is semantically complete, then forward to MT.

---

## Decision

Implement a **tiered completeness judge** injected into `SentenceAggregator`:

| Tier | Name | Description | Feature flag |
|------|------|-------------|--------------|
| 1 | `RuleBasedJudge` | Regex patterns on JA verbal endings (PLAIN_VERB, POLITE_VERB, COPULA, etc.) | Always-on when `semantic_buffering.enabled = true` |
| 2 | `ConfidenceGate` | Holds fragments where Whisper `avg_logprob < threshold` | Always-on when `semantic_buffering.enabled = true` |
| 3 | `WtpJudge` | ONNX neural boundary classifier (`wtp-bert-mini`) | Opt-in: `semantic-buffering-wtp` Cargo feature |

All tiers implement the `CompletenessJudge` trait:

```rust
pub trait CompletenessJudge: Send + Sync {
    fn judge(&self, text: &str, context: &SegmentContext) -> Completeness;
}
```

---

## Tier 1: Rule-Based Judge

- Pattern-matches Japanese verb forms, copulas, and incomplete-particle endings.
- Zero latency, zero memory overhead.
- Covers common formal/polite speech; misses informal conversational fragments.
- Source: `src/pipeline/completeness/rules.rs`

---

## Tier 2: Confidence Gate

- Guards against low-confidence STT output causing spurious flushes.
- Uses Whisper `avg_logprob`; no-op for non-Whisper STT providers.
- Configurable threshold via `pipeline.semantic_buffering.min_confidence_threshold`.
- Source: `src/pipeline/completeness/confidence.rs`

---

## Tier 3: WtpJudge — `wtp-bert-mini` ONNX Classifier

### Model provenance

| Field | Value |
|-------|-------|
| Model | `wtp-bert-mini` |
| Author | Benjamin Minixhofer, 2023 |
| Licence | **MIT** — permissive, no commercial restriction |
| Hub | <https://huggingface.co/benjamin/wtp-bert-mini> |
| Licence URL | <https://huggingface.co/benjamin/wtp-bert-mini/blob/main/LICENSE> |
| ONNX file | `model.onnx` (in HuggingFace repo siblings) |
| Architecture | `BertCharForTokenClassification` (`model_type: bert-char`) |
| Hidden layers | 4 |
| Hidden size | 256 |
| Multilingual | 85 languages including Japanese, Vietnamese, English |

A copy of the MIT licence text must be placed at
`assets/licenses/wtp-bert-mini-mit.txt` in the distribution bundle.

### Why Tier 3 is opt-in

- The model ONNX file is approximately 40–60 MB.
- Cold-start adds ~80–200 ms.
- Default builds and CI remain lightweight and unaffected.
- Pattern mirrors `local-stt-metal` and `local-tts` opt-in features.

### Character hash encoding

`wtp-bert-mini` uses character-level hash embeddings — no external tokenizer is needed.
Each Unicode scalar is mapped to 8 hash-bucket IDs using PRIMES-based modular arithmetic:

```python
PRIMES = [31, 43, 59, 61, 73, 97, 103, 113]
hash_ids[i] = ((ord(c) + 1) * PRIMES[i]) % 8192
```

The equivalent Rust implementation is in `src/pipeline/completeness/wtp.rs` and requires
no external crate beyond `half` (for f16 attention_mask) and `ort`.

### ONNX tensor specification

| Name | Shape | Dtype | Description |
|------|-------|-------|-------------|
| `hashed_ids` | `[1, seq_len, 8]` | `int64` | Character hash bucket IDs |
| `attention_mask` | `[1, seq_len]` | `float16` | Real-position mask (1.0 for all real chars) |
| `logits` (output) | `[1, seq_len, 127]` | `float32` | Per-character, per-class logits |

Label index 1 (`NEWLINE_INDEX + 1 = 1`) is the primary sentence-boundary indicator.
Boundary probability = `sigmoid(logits[0, last_pos, 1])`.

### Obtaining the model

```bash
# Download model.onnx from HuggingFace Hub
pip install huggingface_hub
python -c "from huggingface_hub import hf_hub_download; \
  hf_hub_download('benjamin/wtp-bert-mini', 'model.onnx', local_dir='./models/wtp')"

# Rename to match the expected file name
mv ./models/wtp/model.onnx ./models/wtp/wtp-bert-mini.onnx
```

Then set in `config.json`:

```json
{
  "pipeline": {
    "semantic_buffering": {
      "enabled": true,
      "tier3_enabled": true,
      "wtp_model_dir": "/path/to/models/wtp"
    }
  }
}
```

### Offline-only data-flow guarantee

- The model is loaded from disk at startup and cached in memory.
- No HTTP or HuggingFace API calls are made at inference time.
- The `ort` ONNX Runtime session runs entirely locally.
- Network access is only required once to download the model file (setup step).

---

## Alternatives considered

| Alternative | Reason rejected |
|-------------|----------------|
| Always-on neural judge | Adds cold-start and binary size to default builds |
| `sat-3l-sm` (newer SaT model) | 428 MB ONNX — far too large for this use case |
| Hand-rolled Viterbi on verb patterns | Doesn't handle code-switching or informal speech |
| Cloud completeness API | Violates offline-first principle |

---

## Consequences

- Default feature set is unchanged; existing tests and CI unaffected.
- Users who need Tier 3 must download ~50 MB model ONNX and set `tier3_enabled = true`.
- CI tests that require the model gate on `WTP_MODEL_PATH` env var and skip when absent.
- Licence: MIT — no legal blockers.
