# SUPERTONIC-01 — Supertonic TTS feasibility, vendor evidence & integration shape (Wave-1 T0 spike)

> Issue: [#486](https://github.com/magicpro97/tui-translator/issues/486)
> Wave: 1 · Tier: A · Red mode: `evidence_first`
> Status: **DRAFT — evidence-first / RED**. The "no hardware available" path
> is documented first (§0) per the wave envelope; vendor-document analysis
> follows. Empirical measurements are explicitly recorded as **deferred
> blockers** until a Supertonic build is reproducible in this repo.
> Decision confidence at close of this spike: **0.85** (down from the 1.0
> ceiling because cold-start, RTF, RSS, time-to-first-audio, and
> shutdown-cleanness numbers are not yet observed locally; see §0 and §8).

---

## 0. Hardware / vendor-access limitations (evidence_first / RED skeleton)

This spike was executed **without** a working Supertonic build. The wave
envelope explicitly authorises that path (`acceptance-matrix.md` row #486:
"Measurement test cases need a working Supertonic build; without one, the
report must record limitations and an explicit follow-up blocker.").

### 0.1 What this spike could NOT do in Wave 1

| Required test case (issue body §Test cases) | Status | Reason |
|---|---|---|
| Run vendor Rust example offline | ❌ Not executed | No upstream binary or model weights vendored into this repo; no `Cargo.toml` edits permitted in Wave 1 (cargo-policy.md). Building the vendor example requires a new crate dependency and onnxruntime DLL placement that is out of scope. |
| Run Python `supertonic serve` HTTP path | ❌ Not executed | Python sidecar is research-only and requires a separately-managed Python env + model download (≥ several hundred MB) outside the agent's sandbox. Also explicitly NOT a shippable path per acceptance criterion. |
| Synthesize 1× `en`, 1× `ja`, 1× `vi` utterance | ❌ Not executed | Depends on either path above. |
| Measure **cold start** | ❌ Deferred | Requires running build + warm-up loop. |
| Measure **warm synthesis latency** | ❌ Deferred | Requires running build. |
| Measure **time-to-first-audio (TTFA) proxy** | ❌ Deferred | Requires running build + streaming hook. |
| Measure **RTF** (real-time factor) | ❌ Deferred | Requires running build. |
| Measure **RSS** (resident set size) | ❌ Deferred | Requires running build + `Process` probe (which itself is a downgraded Wave-1 deliverable, #502). |
| Verify **shutdown is clean without `_exit`** | ❌ Deferred | Requires running build; this is the single highest-risk question (see §6.4). |

### 0.2 What this spike DID do

- Vendor-document analysis of the upstream `supertone-inc/supertonic`
  repository (Rust example layout, ONNX model card, license, CLI usage)
  and the Supertonic 3 model card on Hugging Face (sources cited inline in
  §1).
- Mapping of those facts onto this repository's existing provider trait
  surface (`src/providers/mod.rs`) and Google TTS implementation
  (`src/providers/google/tts.rs`) to derive **three concrete integration
  shapes** (§3) and their costs.
- Comparison against this repo's existing local-ONNX precedent — the
  `local_mt` ORT/ONNX runtime path documented in `docs/` and visible in
  `src/providers/` — so the "native Rust ONNX" option is not greenfield.
- An explicit follow-up-blocker list (§8) so the empirical numbers are
  pinned and cannot silently fall out of the project.

### 0.3 What must happen before this report can flip to GREEN (decision = 1.0)

A successor spike SUPERTONIC-01b (or an in-place amend of this report)
must, at minimum:

1. Vendor-or-download a Supertonic model and run the upstream Rust
   example on Windows 10/11, recording cold-start ms, warm ms, TTFA proxy
   ms, RTF, peak RSS MB, and a thread/handle/process dump captured **after
   normal `Drop` only** (no `_exit`).
2. Repeat the same three measurements through the Python HTTP sidecar on
   the same machine and confirm whether the Python path's shutdown
   leaks threads / GPU contexts.
3. File the numbers under a new evidence path (e.g.
   `verification-evidence/supertonic/SUPERTONIC-01b-bench.{json,md}`)
   added to a future wave's `files_allowed.txt`. **No edits to allow-list
   in Wave 1.**

Until those numbers exist, the decision in §7 is best treated as a
**provisional integration shape** rather than a closed contract.

---

## 1. Vendor evidence (citations)

All upstream artifacts cited below were referenced by URL only; nothing
was downloaded into the repo. Quotations are paraphrased; consult the
upstream sources for verbatim text.

| Ref | Source | What it tells us |
|-----|--------|------------------|
| **V-1** | `supertone-inc/supertonic` GitHub repository (upstream) | Reference implementation is C++/ONNX with a Rust example under `examples/rust/` that wraps `onnxruntime` via the `ort` crate ecosystem; model is loaded from local `.onnx` file(s); inference is synchronous; the example does not appear to expose an explicit streaming API. |
| **V-2** | Supertonic 3 model card (Hugging Face: `supertone-inc/supertonic-3`) | Multilingual TTS (en/ja/vi among supported locales); designed for offline CPU inference; model artifact is ONNX; weights are released under the upstream's stated license. |
| **V-3** | `supertonic serve` (Python) | A Python HTTP front-end shipped by the upstream that exposes synthesis over `localhost`. Documented by the upstream as a research/dev convenience; not positioned as a production deployment shape. |
| **V-4** | Issue #486 body | Acceptance criterion: *"Python sidecar accepted only for research or explicitly rejected for shipping"*. This pins the policy regardless of whether Python is faster in benchmarks. |
| **V-5** | This repo: `src/providers/mod.rs`, `src/providers/google/tts.rs` | Existing `TtsProvider` trait and an async Google implementation set the integration contract this work must conform to. |
| **V-6** | This repo: local MT (ONNX/ORT) integration | Pattern precedent — the project already loads ONNX models via `ort` for translation; the same crate / runtime version can be reused for TTS without a new top-level dependency family. |

> **Note on V-1 / V-2 / V-3:** the exact upstream commit / file SHAs and
> the exact HF model revision are intentionally not pinned in this
> document because doing so would imply they had been fetched and
> verified locally, which §0 rules out. The successor spike must pin
> them.

---

## 2. Constraints from this project (binding)

| # | Constraint | Source | Implication for integration shape |
|---|------------|--------|------------------------------------|
| C-1 | **Single Windows `.exe`** — no installer, no service, no extra runtime | `AGENTS.md`, `docs/01-product-vision.md` | Python sidecar **cannot** be the shipping deployment shape. |
| C-2 | **Async Tokio everywhere; no `std::thread::spawn` except for WASAPI** | this repo's Copilot instructions | Synthesis call must be `async fn` and must not block the runtime. CPU-bound ONNX inference must run on `spawn_blocking`. |
| C-3 | **No `unwrap`/`expect` outside tests/`main`** | repo conventions | Provider must surface ORT/IO errors via `thiserror` per `providers/mod.rs` style. |
| C-4 | **Tracing only, no `println!`** | repo conventions | First-audio / warm-up timings must use `tracing` spans. |
| C-5 | **Phase-gate stubs** — TTS is Phase 4 work; this report belongs to the Phase 4 substrate | `AGENTS.md` table | Decision here pins the trait impl shape **before** anyone writes the impl. Aligns with acceptance criterion *"No `src/providers/` implementation starts until this closes"*. |
| C-6 | **No `Cargo.toml` / `Cargo.lock` edits in Wave 1** | `cargo-policy.md`, this tentacle's envelope | This spike cannot itself add `ort` or pin a Supertonic crate. Any dep need triggers a `dep-request.md`; the Wave-1 envelope says none is expected for downgraded scopes. |
| C-7 | **No edits outside `verification-evidence/supertonic/SUPERTONIC-01-spike.md`** | final-dispatch-authorization §1 | This report is the only artifact for this issue. |
| C-8 | **Decision confidence must reach 1.0** for issue closure | issue #486 acceptance | Empirical numbers from §0 are gating; this Wave-1 deliverable is "shape decided, numbers deferred". |

---

## 3. Three integration shapes (compared)

The issue body explicitly names three architectures: native Rust ONNX,
Python HTTP sidecar, and CLI subprocess. Each is summarised below
against the constraints in §2 and the required test cases in §0.

### 3.1 Option A — **Native Rust ONNX via `ort`** (in-process)

**Shape:** A new module `src/providers/supertonic/` containing
`tts.rs` implementing `TtsProvider`. The model `.onnx` file(s) are
loaded from a configurable path (default beside the `.exe`); inference
runs on `tokio::task::spawn_blocking` with the existing `ort` runtime
(reused from local MT, V-6).

| Dimension | Assessment |
|-----------|------------|
| Single-`.exe` constraint (C-1) | ✅ Inference is in-process; only the `.onnx` weights ship alongside (or are downloaded on first use). |
| Async fit (C-2) | ✅ via `spawn_blocking`. Streaming TTFA achievable if upstream model exposes chunked outputs; otherwise TTFA proxy = full synthesis latency. |
| Error surface (C-3) | ✅ `thiserror` wrapping `ort::OrtError`. |
| Shutdown cleanness (issue test case) | ✅ **Cleanest path**: ORT session is RAII; no child process, no socket, no Python interpreter. Eliminates the `_exit` risk that motivates issue #486. (Empirical confirmation deferred per §0.) |
| Cold start | ❓ Deferred. Expected to be dominated by ORT session creation + first-token warmup (precedent from local MT). |
| Warm latency / RTF | ❓ Deferred. Vendor docs (V-2) suggest CPU-friendly; numbers required before commit. |
| RSS | ❓ Deferred. ORT session + model weights resident; precedent from local MT suggests acceptable. |
| Dep policy (C-6) | 🟡 Requires `ort` (already in repo for local MT — no new top-level family) and possibly a small Supertonic-specific token/feature crate. Net new crates likely **0–2**; needs verification when implementation starts. |
| Implementation effort | **3–5 dev-days** for a Phase-4 `TtsProvider` impl + integration tests + model-path config + warmup span. |
| Risk | Low–medium. ONNX op-set coverage in `ort`'s bundled runtime version may need updating; mitigated by reusing the local-MT-validated version. |

### 3.2 Option B — **Python HTTP sidecar (`supertonic serve`)**

**Shape:** This repo's `TtsProvider` impl POSTs synthesis requests to a
locally-running Python process started independently of the `.exe`.

| Dimension | Assessment |
|-----------|------------|
| Single-`.exe` (C-1) | ❌ **Disqualifies for shipping.** Requires Python install, model download, port management. |
| Acceptance criterion match | Allowed *only* as a "research" path per issue #486; not allowed as production. |
| TTFA / RTF | Likely competitive (vendor-supported), but irrelevant to shipping decision. |
| Shutdown cleanness | ❌ Highest risk: Python interpreters with native extensions are the documented source of `_exit`-on-shutdown workarounds. This is precisely the failure mode the issue is trying to avoid. |
| Dep policy (C-6) | N/A (out-of-process). |
| Implementation effort | Smallest *to prototype* (~1 day for a thin HTTP client), but useless as a shipping path; would be discarded code. |
| Verdict | **Research / benchmarking only.** Use it (if used at all) as a *reference oracle* for what numbers Option A should match, then delete. |

### 3.3 Option C — **CLI subprocess** (`supertonic` Rust example invoked per utterance)

**Shape:** The `TtsProvider` impl spawns the upstream Rust example
binary (vendored or built once at install time) per synthesis call,
piping text in and PCM/WAV out.

| Dimension | Assessment |
|-----------|------------|
| Single-`.exe` (C-1) | 🟡 Possible if the example binary is shipped alongside the main `.exe`, but contradicts the "single .exe" spirit and complicates packaging. |
| Async fit (C-2) | 🟡 `tokio::process::Command` works but adds per-call process spawn overhead (Windows process creation is notoriously expensive — tens to hundreds of ms). |
| TTFA | ❌ **Cold-start re-paid every utterance** unless the subprocess is kept alive, at which point it becomes a sidecar (Option B in disguise). |
| RTF | Likely uncompetitive with Option A for short utterances because of spawn overhead. |
| Shutdown cleanness | 🟡 Per-call subprocess exits naturally; main `.exe` shutdown is clean. But this trades shutdown safety for runtime cost. |
| Dep policy (C-6) | Adds a second build artifact rather than a new crate dep. |
| Implementation effort | ~2 dev-days for a robust spawn/pipe wrapper; ongoing maintenance cost for two binaries. |
| Verdict | **Rejected for production** — see §4. Retained here only as a fallback if Option A's ORT op-set support turns out to be unworkable. |

### 3.4 Side-by-side summary

| Criterion | A — Native ORT | B — Py HTTP sidecar | C — CLI subprocess |
|-----------|----------------|---------------------|---------------------|
| Single .exe ship | ✅ | ❌ | 🟡 (extra binary) |
| Shutdown without `_exit` | ✅ (expected) | ❌ (high risk) | 🟡 |
| TTFA potential | Best (in-proc, streaming if supported) | Mid (HTTP RTT) | Worst (process spawn) |
| Effort to ship | 3–5 d | N/A (research only) | 2 d + ongoing |
| Wave-4 Phase fit | ✅ Phase-4 native | ❌ | 🟡 |
| Decision pick | **PICK** | Research-only oracle | Fallback only |

---

## 4. Rejected alternatives

The following were considered and **explicitly rejected** during scoping
of this spike. Each rejection is recorded so future contributors do not
re-litigate them silently.

| # | Alternative | Why rejected |
|---|-------------|--------------|
| R-1 | **Adopt `supertonic serve` Python sidecar as the production path** | Violates C-1 (single .exe) and the explicit acceptance criterion in #486. |
| R-2 | **Ship `supertonic` CLI as a bundled second binary (Option C as production)** | Per-call process spawn destroys TTFA on Windows; doubles release surface. Retained only as fallback (§3.3). |
| R-3 | **Use a non-Supertonic alternative TTS** (e.g. Piper, RHVoice, Edge-TTS) | Out of scope for this issue. #486 is specifically a Supertonic feasibility spike; switching engines is a separate decision (and a separate spike). |
| R-4 | **Generate audio in a worker `std::thread::spawn`** | Violates C-2; `spawn_blocking` exists for this. |
| R-5 | **Stream audio via a custom IPC protocol to a long-running native subprocess** | Sidecar-with-different-clothes; reintroduces the shutdown-cleanness risk that motivates the whole issue. |
| R-6 | **Wait for upstream to publish a crate on crates.io and depend on it directly** | Cannot block on upstream packaging timeline; we control our own integration via `ort`. |
| R-7 | **Run Supertonic on GPU via DirectML/CUDA execution provider** | Out of scope for v1; CPU first per the model card guidance (V-2). Reconsider if RTF on CPU fails to meet the SLO. |
| R-8 | **Embed model weights into the `.exe`** | Weights are likely too large; ship as a separate file beside the binary (precedent: local MT). |

---

## 5. Recommended integration shape (provisional, pending §8)

**Pick Option A** (Native Rust ONNX via `ort`, in-process,
`spawn_blocking`), with Option B retained **only** as a benchmarking
reference during the implementation phase and deleted before merge.

### 5.1 Trait surface (no code in this report; this is the contract)

The existing `TtsProvider` trait in `src/providers/mod.rs` is the
contract. The Supertonic impl will:

- Construct an ORT session from a configurable model path (default:
  `./models/supertonic-*.onnx` resolved relative to the executable).
- Expose `async fn synth(...) -> Result<AudioBuf, TtsError>` matching
  the trait (exact signature inherited from `mod.rs` at impl time).
- Run the actual inference inside `tokio::task::spawn_blocking`.
- Emit a `tracing::instrument`'d span around session creation
  ("cold start") and each `synth` call (TTFA + total).
- Surface errors with `thiserror` (`SupertonicError::Ort`,
  `SupertonicError::ModelMissing`, `SupertonicError::Locale`).
- Implement `Drop` only via ORT's own RAII; no custom `unsafe` teardown,
  no atexit hook, no `_exit`.

### 5.2 Config surface (additive)

A future PR (out of scope here) will extend `config.json` with:

```jsonc
{
  "tts": {
    "provider": "supertonic",
    "supertonic": {
      "model_path": "models/supertonic-3.onnx",
      "max_concurrent": 1
    }
  }
}
```

This document does NOT add that key; it only records the shape.

### 5.3 Test layout (for the successor implementation issue)

- Unit tests in `src/providers/supertonic/tts.rs` for error mapping and
  config parsing.
- Integration test (gated by `cfg(feature = "supertonic-model-present")`
  or an `#[ignore]` attribute) that loads a tiny model + synthesises an
  `en` utterance and asserts non-empty PCM + clean session drop.
- A bench harness under `tests/` (or under the existing soak-proof bin)
  that records cold-start / warm / TTFA / RTF / RSS into the
  Wave-1-approved evidence schema (or a successor schema).

---

## 6. Risks (ranked)

| # | Risk | Likelihood | Impact | Mitigation |
|---|------|-----------|--------|------------|
| 6.1 | **Shutdown not clean without `_exit`** — Option A turns out to leak ORT threads on `Drop` | Low–Med | High (the headline risk in #486) | Mandatory test: spawn process, synth one utterance, observe `Drop`, assert clean exit; do *not* paper over with `_exit`. |
| 6.2 | **TTFA exceeds budget** for streaming-style UX | Med | Med | If model has a streaming/chunked output, surface it; otherwise document the latency floor and accept it as a Phase-4 limitation. |
| 6.3 | **ORT op-set mismatch** between the version this repo uses for local MT and Supertonic-3 model | Low | Med | Pin the model revision the project commits to; re-spike if upstream bumps the op-set beyond our `ort` version. Triggers a `dep-request.md` for the `ort` bump if so. |
| 6.4 | **Model size / RSS** unacceptable for the `.exe`-beside-weights deployment | Low | Med | Quantised variants from the model card (V-2) reduce RAM at small quality cost; record the tradeoff. |
| 6.5 | **Locale coverage** of the chosen model for `vi` is weaker than `en`/`ja` | Med | Low | Acceptance criterion only requires *one* utterance per locale to be synthesised; subjective quality is out of scope for this spike. |
| 6.6 | **Upstream license drift** between vendor evidence date and integration date | Low | High | Re-confirm the upstream license at PR time; record SHA in successor spike. |
| 6.7 | **No upstream Rust example update** for newer model revisions | Low | Low | We do not depend on the example binary; we own the integration. |

---

## 7. Effort estimate for native provider (Option A)

Assumes a single engineer with prior `ort` experience (precedent: local
MT), Windows 10/11 dev machine, Supertonic-3 model artifact reachable.

| Workstream | Estimate | Notes |
|------------|----------|-------|
| Reproduce upstream Rust example offline (vendor evidence GREEN) | 0.5 d | Includes model download + first synth. |
| Skeleton `src/providers/supertonic/` + trait impl with `bail!("not yet implemented (Phase 4)")` stubs | 0.5 d | Matches existing phase-stub pattern. |
| Full `synth` implementation incl. `spawn_blocking` + tracing spans | 1.0–1.5 d | |
| Config plumbing + error type | 0.5 d | |
| Unit + integration tests | 0.5–1.0 d | |
| Bench harness + first numbers recorded into evidence schema | 1.0 d | Aligns with #460 / QA8 schema work. |
| Code review + fixups | 0.5 d | |
| **Total** | **~3–5 dev-days** | + 0.5 d buffer per risk in §6 that fires. |

This estimate **excludes** model packaging / distribution policy
decisions and any GPU/DirectML follow-up (R-7).

---

## 8. Unresolved blockers (open this set before declaring decision = 1.0)

These are the explicit follow-up blockers required by acceptance-matrix
row #486 ("the report must record limitations and an explicit follow-up
blocker"). Each should become its own tracked issue in a future wave;
this document does not open issues itself (orchestrator owns issue
creation).

| ID | Blocker | What unblocks it |
|----|---------|------------------|
| B-1 | No measured **cold-start ms** on Windows for Supertonic | A successor that runs the vendor Rust example and records the number. |
| B-2 | No measured **warm synthesis latency ms** per locale | Same as B-1, ×3 locales. |
| B-3 | No measured **time-to-first-audio proxy ms** | Requires either a streaming-aware harness or a documented "no streaming → TTFA == warm latency" finding. |
| B-4 | No measured **RTF** | Same harness as B-1/B-2. |
| B-5 | No measured **RSS peak MB** | Probe via the `process.rs` work landing in #502 (T2). Cross-depends on QA8. |
| B-6 | **Shutdown cleanness** unverified — is `_exit` actually avoidable on Option A? | Forced-shutdown integration test, captured under the panic-hook / crash schema landing in #506 (T2). |
| B-7 | No pinned **upstream commit / model revision SHA** | Successor must pin and add to `verification-evidence/supertonic/`. |
| B-8 | **`ort` version compatibility** with the chosen model revision unverified | Reproduce locally with this repo's pinned `ort`; if mismatch, file `dep-request.md`. |
| B-9 | **Locale `vi` quality** unassessed (acceptance only requires one utterance; ship-quality is separate) | Out-of-scope for #486; track separately in a Phase-4 follow-up. |
| B-10 | **Streaming API availability** in the upstream model | Read upstream Rust example carefully; if no streaming, decide whether to ship a non-streaming TTS for v1. |

All ten blockers are **document-only** at this stage — none requires a
code edit in Wave 1. The successor spike (SUPERTONIC-01b) is expected
to close B-1 through B-6 in a single session; B-7/B-8 are pinning
exercises; B-9/B-10 are scope flags for Phase-4 planning.

---

## 9. Acceptance criteria — self-assessment

| Criterion (verbatim from #486) | Status in this report |
|---|---|
| "Decision confidence reaches 1.0 for production integration shape" | **0.85** — shape is decided (Option A), but the empirical numbers required for full 1.0 are deferred per §0 and §8. The wave envelope explicitly authorises this state ("record limitations and an explicit follow-up blocker"). |
| "Python sidecar accepted only for research or explicitly rejected for shipping" | ✅ §3.2, §4 R-1 — Python sidecar **rejected for shipping**, retained only as a benchmarking oracle during implementation and to be removed before merge. |
| "No `src/providers/` implementation starts until this closes" | ✅ This document is the gating artifact. No `src/providers/supertonic/` files exist. Allow-list is closed to this single markdown. |

---

## 10. Closing notes

- **No code or config was modified by this spike.** The single
  allow-listed file is this report.
- **No new dependency was requested.** Option A reuses the existing
  `ort` family; if a future implementation requires a version bump, that
  decision triggers a `dep-request.md` and is *not* implicit in this
  spike's recommendation.
- **No sub-agents were dispatched.**
- This document is intentionally written so that a future RED→GREEN
  amend simply replaces the "❌ Deferred" cells in §0.1 with measured
  values and bumps the confidence in §9 to 1.0; the shape decision in
  §5 is not expected to change.

---

*Authored under tentacle `w1-t0-486-supertonic-spike` (Wave 1, T0,
evidence_first). Reviewer: code-review agent (Sonnet-4.6) per
`final-dispatch-authorization.md §3 Tier A`.*
