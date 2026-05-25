# SUPERTONIC-02 — License, model distribution, consent, and privacy audit (DRAFT)

> Issue: [#487](https://github.com/magicpro97/tui-translator/issues/487)
> Parent: [#485](https://github.com/magicpro97/tui-translator/issues/485)
> Status: **DRAFT — docs-only, no provider code merged**.
> This memo is a policy substrate written **before** any
> `src/providers/supertonic/` code lands. It pins the obligations a future
> implementation PR must honour. It does NOT itself ship a provider, a
> downloader, a EULA dialog, or a model cache.
> Routing decision confidence: **0.85** — policy decisions are committed;
> empirical packaging-dry-run and runtime egress evidence is **deferred**
> to the implementation PR (see §8).

---

## 0. Scope guard (what this document is and is not)

This memo lives in `verification-evidence/` because it is the policy
input for SUPERTONIC-11 (#496) and SUPERTONIC-12 (#497). It is the
**only** SUPERTONIC-02 artifact in this PR.

In scope:

- License classification of the upstream Supertonic code and weights.
- Distribution / NOTICE / EULA obligations they create for our build.
- Consent UX obligations for first-time model download.
- Voice-cloning policy.
- Network-egress policy ("no silent network").

Out of scope (tracked by other issues):

- The actual `TtsProvider` impl (#490, #491, #493).
- The model cache implementation (#492).
- The consent dialog code path.
- The release packaging script changes.
- Empirical packaging-dry-run output (deferred, §8).

---

## 1. License facts (vendor evidence, by reference)

All citations are by URL; nothing was downloaded or vendored into this
repo. The implementation PR must re-confirm each fact at PR time and
record the upstream commit SHA / HF revision in an addendum.

| Ref | Source (URL) | Fact |
|-----|--------------|------|
| L-1 | Upstream `supertone-inc/supertonic` repository `LICENSE` file — https://github.com/supertone-inc/supertonic/blob/main/LICENSE | Code is **MIT-licensed**. |
| L-2 | Hugging Face model card `supertone-inc/supertonic` — https://huggingface.co/supertone-inc/supertonic | Weights are **OpenRAIL-M** (Responsible AI License — Model) with use-based restrictions. |
| L-3 | OpenRAIL-M reference text (BigScience) — https://www.licenses.ai/ai-licenses (canonical RAIL hub) and https://huggingface.co/spaces/bigscience/license (BigScience OpenRAIL-M reference) | Imposes **downstream propagation** of use restrictions to any redistributed model and to derived models/services. |
| L-4 | This repo `LICENSE` — https://github.com/magicpro97/tui-translator/blob/main/LICENSE | Project itself is MIT (Copyright 2026 magicpro97). |
| L-5 | This repo `PRIVACY.md` — https://github.com/magicpro97/tui-translator/blob/main/PRIVACY.md | Local-first by default; cloud egress is opt-in per provider toggle. Pattern precedent for any new provider. |

> URLs above are non-authoritative pointers for reviewer convenience.
> The implementation PR must re-resolve each URL at PR time, pin the
> exact upstream commit SHA / HF revision, and record the resolved
> license text in `verification-evidence/supertonic/` as a binding
> addendum to this memo (see §8 B-1).

> The two licenses (MIT for code, OpenRAIL-M for weights) live at
> different layers and must be treated separately. The MIT license does
> NOT cover the model weights, and the OpenRAIL-M weights are NOT
> "open-source" in the OSI sense.

---

## 2. License decision

**Adopt Supertonic under a dual-licence handling regime:**

1. **Code (MIT, L-1)** — compatible with our MIT project (L-4). Vendor
   any reused source under our top-level `LICENSE` aggregation rules; a
   per-file or per-module attribution header is sufficient.
2. **Weights (OpenRAIL-M, L-2)** — NOT bundled into the `.exe`. Weights
   are downloaded on first use, with the OpenRAIL-M text presented to
   the user **before** download begins.
3. **Use restrictions (L-3)** — propagated verbatim into our EULA /
   NOTICE / on-screen consent text. Any future redistribution of derived
   weights or fine-tunes must carry the same restrictions forward.

This regime is recorded so a future contributor cannot quietly bundle
weights into the installer.

---

## 3. Distribution strategy

| Component | Distribution path | Why |
|-----------|-------------------|-----|
| Supertonic source we reuse | Vendored under MIT attribution in `LICENSE` / `NOTICE` | Trivial — MIT permits redistribution with notice. |
| Supertonic model weights (`.onnx`) | **On-demand download** to `%LOCALAPPDATA%\tui-translator\models\tts\` (Windows) | OpenRAIL-M propagation is easier when the user fetches once, locally, with consent acknowledged. Avoids redistributing weights inside the `.exe`. |
| OpenRAIL-M restriction text | Shipped as `NOTICE-OpenRAIL-M.txt` and surfaced in consent dialog | Required propagation (L-3). |
| Voice clone artefacts (if any) | **Not bundled, not redistributed** by us | See §5. |

**The `.exe` ships zero weights and zero personally-trained voices.**
This is a hard constraint, not a preference.

---

## 4. NOTICE / EULA requirements (must-haves for implementation PR)

The implementation PR must add or extend the following files. This memo
does **not** add them — it only specifies their required content.

1. `NOTICE` (top-level) — append:
   - Supertonic source attribution (MIT, L-1).
   - Pointer to `NOTICE-OpenRAIL-M.txt` for weights.
2. `NOTICE-OpenRAIL-M.txt` (new) — full OpenRAIL-M text covering the
   weights, with the use-restrictions list verbatim.
3. `PRIVACY.md` — append a Supertonic section that:
   - Records first-use model download as a **user-initiated network
     event** (consistent with §6 below).
   - Confirms inference runs entirely offline once weights are present.
   - States that voice cloning is disabled by default (§5).
4. `docs/supertonic-user-guide.md` (created in this PR — see #497) —
   surfaces the same restrictions in plain English with a link to the
   full NOTICE.
5. In-app consent dialog string table — the user must explicitly accept
   OpenRAIL-M restrictions before the first download begins. Implicit
   acceptance (e.g. "downloading because the config flag is on") is
   prohibited; see §6.

---

## 5. Voice-clone policy

The Supertonic model family supports speaker-conditioning. Voice
cloning of real human speakers without their consent is one of the
OpenRAIL-M use restrictions (L-3) and is independently a privacy
risk under our PRIVACY policy (L-5).

Decision:

- **Voice cloning is OFF by default.** The shipped config exposes only
  the built-in voice presets that come with the official model card.
- Custom speaker conditioning (uploading a reference voice sample) is
  **not implemented in v1**. If a future PR adds it, it must:
  1. Require an explicit per-session opt-in (separate from the model
     download consent).
  2. Display a non-dismissible reminder that the user must have the
     consent of the speaker being cloned.
  3. Never auto-persist reference samples beyond the session unless the
     user explicitly saves them to a named voice profile.
  4. Refuse to clone from `audio_archive` recordings without a second
     consent prompt (the `audio_archive.consent_given` flag covers
     recording, not cloning).

---

## 6. Consent UX & no-silent-network statement

This section formalises the "no silent network" criterion in #487.

### 6.1 First-time model download
- Trigger: user opens TTS settings and selects a Supertonic voice for
  the first time.
- Required UI: a blocking dialog (TUI modal) showing
  - the model name and revision,
  - the download size and target path,
  - the OpenRAIL-M restriction summary + link to full text,
  - explicit `Accept` / `Decline` actions; `Decline` aborts the
    selection and leaves the previous TTS provider unchanged.
- The flag controlling this is per-install, not per-launch, but
  re-prompted on model revision change.

### 6.2 Runtime egress
After successful install, the Supertonic provider must perform **zero
network egress** during synthesis. The only allowed network actions are:

1. User-initiated re-download or version check, gated by the same
   consent flow as §6.1.
2. The existing `tts_cloud_fallback` path (Google TTS), which is itself
   **null by default** and only fires when the user has explicitly set
   the field in `config.json`. This memo does NOT loosen that default.

### 6.3 Local HTTP rejection
The Python `supertonic serve` sidecar (rejected for shipping in
SUPERTONIC-01 §3.2) implies a local HTTP socket. If a future operator
attempts to point the provider at a non-loopback URL, the provider
**must reject** the URL at config-load time. Default-deny is the
behaviour to implement, not a runtime warning.

### 6.4 Audio-archive interaction
`audio_archive.store_audio` and `audio_archive.consent_given` already
gate raw audio capture (precedent in `PRIVACY.md`). Supertonic
synthesis output is **not** subject to these flags — it is
locally-generated speech, not captured speaker audio — but the
voice-cloning path (§5) is gated separately as described.

---

## 7. Acceptance-criterion mapping

| Criterion from #487 | Where addressed in this memo |
|---|---|
| Security/privacy review CLEAN | This is the substrate; the actual CLEAN verdict is recorded by a `tui-security-auditor` pass on the implementation PR, not by this doc. |
| OpenRAIL-M strategy accepted | §2, §3 |
| OpenRAIL-M use restrictions propagated into EULA/NOTICE/release artifacts | §4 |
| No model redistribution without propagated restrictions | §3 (download-on-demand, not bundled); §4 (NOTICE-OpenRAIL-M.txt required) |
| Cloud fallback requires explicit consent | §6.2 — `tts_cloud_fallback` stays null by default |
| Package dry-run includes license/notice requirements | **Deferred to implementation PR**, §8 B-1 |
| Missing consent blocks model download or voice clone | §6.1, §5 — design pinned; runtime check **deferred**, §8 B-2 |
| Network-off runtime → zero egress except explicit fallback | §6.2 — policy pinned; runtime evidence **deferred**, §8 B-3 |
| Non-loopback local HTTP URL rejected by default | §6.3 — policy pinned; check **deferred**, §8 B-4 |

---

## 8. Deferred blockers (open before SUPERTONIC-11 default-flip)

| ID | Blocker | What unblocks it |
|----|---------|------------------|
| B-1 | Package dry-run not executed; we have no artifact proving the release ZIP/MSI carries NOTICE-OpenRAIL-M.txt | Implementation PR adds packaging step + dry-run output in `verification-evidence/supertonic/` |
| B-2 | Consent-blocking is design-only; no runtime test yet | Implementation PR adds the dialog + an integration test that simulates `Decline` and asserts no file is fetched |
| B-3 | No network-off runtime evidence (e.g. Wireshark / Windows Firewall log) showing zero egress during synthesis | Implementation PR runs the soak + records evidence |
| B-4 | No automated rejection test for non-loopback URLs | Implementation PR adds config-load unit test |
| B-5 | OpenRAIL-M text not yet vendored into the repo | Implementation PR adds `NOTICE-OpenRAIL-M.txt` |
| B-6 | Upstream license drift between this memo's date and PR date | Re-confirm at PR time; pin upstream SHA + HF revision in an addendum |

---

## 9. Decision-impact statement

This memo is **binding policy input** for:

- #496 (SUPERTONIC-11 default-readiness ADR) — default cannot flip
  unless §4 NOTICE files exist, §6 consent flow is implemented and
  tested, and B-1 through B-5 are closed.
- #497 (SUPERTONIC-12 user docs) — user-facing copy must mirror §6
  language verbatim where possible to avoid drift.

No `src/`, no `Cargo.toml`, no `config.example.json` is modified by this
document.
