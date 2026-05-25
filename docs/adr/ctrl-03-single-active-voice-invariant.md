# CTRL-03 — Single active voice invariant across single and dual modes

* **Status:** Accepted
* **Date:** 2026-05-27
* **Issues:** [#456](https://github.com/magicpro97/tui-translator/issues/456)
* **Builds on:** DM-06 / [#382](https://github.com/magicpro97/tui-translator/issues/382)
  (`tts_source` slot selection), DM-01 / [#377](https://github.com/magicpro97/tui-translator/issues/377)
  (slot mode), [#490](https://github.com/magicpro97/tui-translator/issues/490) /
  [#491](https://github.com/magicpro97/tui-translator/issues/491) /
  [#531](https://github.com/magicpro97/tui-translator/pull/531) (TTS streaming
  + cloud-fallback contract), [#457](https://github.com/magicpro97/tui-translator/issues/457) /
  [#420](https://github.com/magicpro97/tui-translator/issues/420) /
  [#532](https://github.com/magicpro97/tui-translator/pull/532) (backend
  selection contract).

## Context

The user mandate is unambiguous: **dual-slot and single-slot modes must
never play more than one TTS voice at a time.**  Allowing two voices to
overlap would defeat the live-bilingual-subtitle UX (the listener cannot
follow two concurrent streams) and would violate the deterministic
playback contract assumed by the [DM-06 policy
tests](../../tests/dm06_dual_tts_policy.rs).

Pieces of this invariant already existed before #456:

* `TtsSource` (`config::TtsSource`) is a closed enum (`Off | A | B`) that
  selects which slot synthesises in dual mode.
* `TtsSource::is_active_for_slot(slot_is_a, is_dual)` returns the boolean
  gate consumed by each orchestrator.
* `OrchestratorContext::tts_active_for_slot` is set **once** at orchestrator
  construction from the call above and is `false` for the non-selected
  slot.
* The pipeline gates every TTS provider call with
  `tts_enabled AND tts_active_for_slot` (see `src/pipeline/mod.rs`).
* `AppConfig::requires_restart` classifies `tts_source` mutations as
  `true`, so the captured slot identity cannot drift mid-session.
* `SharedPlaybackService = Arc<Mutex<Option<PlaybackService>>>` (in
  `src/main.rs`) is a process-wide singleton shared between both slots,
  so even a defective second synthesiser would serialise through one
  playback owner.
* In dual mode, slot B is wired with `RuntimeTtsProvider::Disabled`
  (`src/main.rs`), which structurally prevents a second concurrent
  provider instance from being constructed.

CTRL-03 turns the collection above into a **named, tested invariant**
with a typed witness on the configuration type itself.

## Decision

The single-active-voice invariant is:

> For every legal `(SlotMode, TtsSource, tts_enabled)` triple, the number
> of orchestrator slots whose effective TTS gate
> (`tts_enabled AND tts_active_for_slot`) evaluates to `true` is `<= 1`.

It is enforced by the following five layers, listed from the most-typed
("compile-time witness") to the most operational ("runtime gate"):

1. **`TtsSource` is a closed enum.**  No third "both" variant can be
   added without a deliberate code change.
2. **`TtsSource::active_slot_count(is_dual) -> u8`** (added in this PR) is
   the typed witness of the invariant.  The function returns `0` only
   when `(is_dual, src) == (true, Off)`; otherwise it returns `1`.  Tests
   exhaustively assert `active_slot_count(..) <= 1`.
3. **`TtsSource::is_active_for_slot(slot_is_a, is_dual)`** is the per-slot
   boolean gate.  In dual mode, the two calls for slot A and slot B are
   provably mutually exclusive; their sum equals `active_slot_count`.
4. **Orchestrator capture.**  `OrchestratorContext::tts_active_for_slot`
   is set once from (3) at construction and never mutated thereafter.
   `AppConfig::requires_restart` returns `true` for any `tts_source`
   change so the capture stays valid for the lifetime of the orchestrator.
5. **Single playback owner.**  `SharedPlaybackService` is a singleton
   shared by every orchestrator slot; only one `PlaybackService` exists
   per process.  In dual mode, slot B's TTS provider slot is filled with
   `RuntimeTtsProvider::Disabled`, so even an unintended
   `tts_active_for_slot = true` on slot B would be a no-op rather than a
   second concurrent voice.

### What this ADR does **not** decide

* It does not introduce a voice catalog or hot-swap UX — those belong to
  CTRL-02 / [#455](https://github.com/magicpro97/tui-translator/issues/455).
* It does not select or implement any new TTS provider — that belongs to
  [#493](https://github.com/magicpro97/tui-translator/issues/493) and
  [#494](https://github.com/magicpro97/tui-translator/issues/494).
* It does not change which slot may speak in dual mode; `tts_source`
  semantics are unchanged from DM-06.

## Consequences

* **Verification.**  `tests/ctrl03_single_active_voice.rs` (added in this
  PR) walks the full Cartesian product
  `{Off, A, B} × {single, dual} × {tts_enabled=false, true}` and asserts
  the gate sum is `<= 1`.  The existing DM-06 policy tests in
  `tests/dm06_dual_tts_policy.rs` are unchanged.
* **Voice swap mid-utterance.**  Because `tts_source` changes require a
  restart (layer 4), a voice swap cannot interrupt a currently-playing
  utterance — the next utterance will be the first one routed under the
  new gate.  The user-visible behaviour described in #456 ("finish the
  current utterance and apply to the next") follows directly from this
  classification.  Lifting the restart requirement in a future change
  would require re-deriving the invariant under the new live-reload
  semantics.
* **Future TTS providers** (Azure, Ollama, Supertonic, etc.) inherit the
  invariant automatically: they plug into the existing gate at layer (3)
  and the singleton playback owner at layer (5).  Adding a provider does
  not require revisiting CTRL-03.
* **No behaviour change** for existing single-slot users: layer (2)
  returns `1` for every `TtsSource` in single mode, matching the
  pre-existing semantics where TTS is controlled exclusively by
  `tts_enabled`.

## Operator-visible summary

The Settings panel and `config.json` field `tts_source` continue to
accept `"off" | "a" | "b"` and only have effect in dual-slot mode.
Mutating the field still triggers the standard restart-required banner.
No new keyboard shortcuts, no new config fields, and no migration are
introduced by this ADR.
