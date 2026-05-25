# ADR SUPERTONIC-05 — TTS streaming and non-blocking provider contract

> **Issue:** [#490](https://github.com/magicpro97/tui-translator/issues/490)
> **Parent:** [#485](https://github.com/magicpro97/tui-translator/issues/485)
> **Status:** **Accepted** — additive contract, no breaking change.
> **Date:** 2026-05-26
> **Contract decision confidence:** 1.0

## Context

The existing [`TtsProvider`](../../src/providers/mod.rs) trait returns the
full synthesised audio buffer in one shot:

```rust
async fn synthesise(&self, text: &str, language_code: &str)
    -> Result<TtsResult, ProviderError>;
```

This shape is fine for the Google REST adapter — Google's `text:synthesize`
endpoint returns the complete MP3 in a single response — but it has two
risks as new providers join:

1. **First-audio latency = full synthesis latency.** A local neural TTS
   (Supertonic, issue #493) can begin producing PCM frames hundreds of
   milliseconds before the utterance completes; the current trait forces
   the pipeline to wait for the whole buffer before any playback can start.
2. **Blocking risk on the render thread.** Although `synthesise` is
   `async`, an unwary caller could `await` it from a path coupled to the
   TUI render loop or the audio capture callback, stalling either.

## Decision

Adopt an **additive, non-breaking** contract change:

1. Keep [`TtsProvider::synthesise`] as the canonical "give me the whole
   utterance" contract. It is the simplest shape and is what the Google
   REST API actually returns.
2. Add a new extension trait
   [`TtsStreamProvider`](../../src/providers/mod.rs) that extends
   [`TtsProvider`] with `synthesise_stream`, which pushes one or more
   `TtsAudioChunk` values through a `tokio::sync::mpsc::Sender`. The
   default implementation calls `synthesise` once and emits the whole
   buffer as a single chunk with `is_final = true`.
3. Document the **non-blocking guarantee** at the trait level: callers
   MUST schedule synthesis on a dedicated Tokio task and drain the
   receiver on the playback side; the TUI render thread MUST NOT await
   `synthesise` or `synthesise_stream` directly.
4. Document the **cancellation contract**: dropping the receiver causes
   `sink.send` to return `Err`; providers MUST exit the synthesis loop
   within 50 ms.

## Why not a single streaming-only trait?

A single streaming trait would force every provider — including Google's
buffer-only REST adapter — to fabricate chunking that does not exist.
That adds glue without value and makes contract tests harder to read.
The extension-trait approach lets each provider opt in when its own API
actually supports incremental output (Supertonic) while preserving the
existing implementation and tests for buffer-only providers (Google).

## Migration plan

| Step | Owner | What changes |
|---|---|---|
| 1 (this PR) | wave4/tts-contract-extension | Add `TtsAudioChunk` + `TtsStreamProvider` with default impl. Opt Google TTS in via `impl TtsStreamProvider for GoogleTtsProvider {}` (no behavioural change). |
| 2 (issue #493) | Supertonic provider work | Implement `TtsStreamProvider::synthesise_stream` natively, emitting PCM frames as the model produces them. |
| 3 (post-#493) | Playback service | Switch playback to consume the streaming receiver, with a bounded `mpsc::channel(N)` per utterance and an `AbortHandle` for cancellation. Buffer-only providers continue to work via the default impl. |
| 4 (issue #495) | Soak/measurement gate | Add a contract test that measures first-PCM-frame latency for both buffer-only and native-streaming providers. |

## Test cases

- **Default impl emits a single final chunk.** A buffer-only provider
  produces exactly one `TtsAudioChunk` with `is_final = true` matching
  the `TtsResult`. ✓ covered by
  `tts_stream_tests::default_stream_impl_emits_single_final_chunk`.
- **Error propagates once and terminates the stream.** ✓ covered by
  `tts_stream_tests::default_stream_impl_propagates_error_once`.
- **Dropped receiver does not panic** (50 ms cancellation contract). ✓
  covered by
  `tts_stream_tests::default_stream_impl_tolerates_dropped_receiver`.
- **Google adapter still passes the buffer contract.** ✓ existing tests
  in `src/providers/google/tts.rs`.

The remaining acceptance-criterion test cases require a real streaming
provider and live audio:

- 25-word utterance, first-PCM-frame vs full-buffer latency measurement.
- TUI render never awaits synth (architectural property — enforced by
  the runtime wiring in #493 and the soak gate in #495).

These are explicitly out of scope for this contract spike; they belong
to the Supertonic implementation track (#493) and the measurement gate
(#495).

## Consequences

- **No breaking change** to existing callers. Every `TtsProvider` keeps
  working unchanged.
- The codebase now has a clear, documented place to add streaming
  providers without rewriting the trait surface.
- `TtsStreamProvider` is gated behind `#[allow(dead_code)]` until the
  playback service starts using it (step 3 above); the trait wiring is
  exercised by the contract tests in this PR.

## References

- Parent: SUPERTONIC parent issue [#485](https://github.com/magicpro97/tui-translator/issues/485)
- Sibling: backend-selection contract [#491](https://github.com/magicpro97/tui-translator/issues/491) (SUPERTONIC-06)
- Future implementer: Supertonic provider [#493](https://github.com/magicpro97/tui-translator/issues/493)
- Default-readiness gate: [`docs/adr/supertonic-11-default-readiness.md`](./supertonic-11-default-readiness.md)
