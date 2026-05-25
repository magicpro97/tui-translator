# MODEL-01 — Local/remote backend selection contract for STT, MT, TTS

* **Status:** Accepted
* **Date:** 2026-05-26
* **Issues:** [#457](https://github.com/magicpro97/tui-translator/issues/457), [#420](https://github.com/magicpro97/tui-translator/issues/420)
* **Builds on:** #214/#371 (STT local default + `stt_fallback_policy = "google-when-keyed"`), #372 (LF-04 MT routing table), #490/#491/#531 (TTS streaming + cloud-fallback contract)

## Context

The pipeline now supports three stages (STT, MT, TTS) each of which can be
served by a local on-device backend or a cloud (Google) backend.  Earlier
work introduced these primitives piecemeal:

* `AppConfig::stt_provider`, `mt_provider`, `tts_provider` (strings) plus
  per-stage validation in `AppConfig::validate`.
* `stt_fallback_policy = "google-when-keyed"` for STT.
* `mt_cloud_fallback` + LF-04 routing table for MT.
* `tts_cloud_fallback` introduced in #531.

The risk these primitives address is a **silent cloud fallback**: a local
stage that, on local failure or for an unsupported language pair, sends
meeting text or audio to Google purely because `google_api_key` happens to
be configured.  MODEL-01 makes the no-silent-cloud invariant a documented,
testable contract across all three stages.

## Decision

### Configuration matrix

| Stage | Backend field | Default | Other accepted values | Cloud-fallback consent field |
|-------|---------------|---------|------------------------|------------------------------|
| STT   | `stt_provider` | `"local"` | `"google"` | `stt_fallback_policy` (`"google-when-keyed"` requires `stt_provider = "local"` + key) |
| MT    | `mt_provider`  | `"google"` | `"local"` | `mt_cloud_fallback` (only `"google"` accepted; requires key) |
| TTS   | `tts_provider` | `"google"` | (local backends pending #493) | `tts_cloud_fallback` (only `"google"` accepted; requires key; forbidden when `tts_provider = "google"`) |

### Typed projection

`src/providers/backend_selection.rs` exposes a small typed surface so the
contract can be reasoned about and tested in one place:

* `BackendKind` (`Google`, `Local`, `Unknown`).
* `CloudFallbackConsent` (`None`, `ExplicitWithKey`, `ExplicitWithoutKey`).
* `StageSelection { backend, cloud_fallback }` with
  `cloud_call_reachable()`.
* `BackendSelection { stt, mt, tts }` projected from `AppConfig`.

The module **does not duplicate validation**; it depends on
`AppConfig::validate` having been called.  It exists so the no-silent-cloud
invariant can be asserted once across stages by reading config rather than
inspecting runtime call sites.

### Runtime MT routing (JV-12, #420)

When `mt_provider = "local"`, the runtime MT provider is built as an
`MtRouter<LocalOpusMtProvider, GoogleMtProvider>`.  On every translation
call the router consults the LF-04 routing table:

* `LocalDirect` → call the local provider.
* `CloudFallback` → only reachable when the router was constructed with
  `Some(cloud)`.  The router is given `Some(cloud)` **only** when
  `mt_cloud_fallback = "google"` AND `google_api_key` is non-empty.  Key
  presence alone leaves the cloud slot empty.  Each fallback call emits a
  `tracing::warn!` so the privacy boundary crossing is auditable.
* `Unsupported` → returns `ProviderError::InvalidInput` with a message
  pointing the operator at `mt_cloud_fallback`.  No cloud call is made.
* `LocalPivotPlanned` → returns `ProviderError::Unimplemented`.  No cloud
  call is made.

### No-silent-cloud invariant

> For every stage S ∈ {STT, MT, TTS}, when the operator selects a local
> backend without explicitly opting into cloud fallback for S, no cloud
> call is reachable for that stage even if `google_api_key` is set.

This is asserted by:

* Config-level tests in `src/config/mod.rs` (existing) and
  `src/providers/backend_selection.rs` (new) covering all
  `(backend, consent, key)` combinations.
* Runtime-level tests in `src/providers/mt/router.rs` exercising mock
  local/cloud providers and asserting Google call count = 0 in the
  no-consent case and call count = 1 only when consent + key are present.
* Existing STT fallback tests in `src/pipeline/fallback.rs`.

### Out of scope

* Implementing additional local backends (#419, #493).
* Model manifest expansion (#417).
* Default flip of `mt_provider` to `"local"` (#421).
* TUI route status surfacing (#422).
* Privacy/security audit (#423).

## Consequences

* The router wrapper adds one branch per MT call.  Cost is one
  language-tag normalisation plus a table lookup; negligible compared with
  the translation request itself.
* `RuntimeMtProvider` gains a third variant (`LocalRouted`).  Existing
  `Local` and `Google` variants are unchanged so callers that bypass the
  builder (tests, benches) keep working.
* `build_slot_mt_provider` gains a `mt_cloud_fallback: Option<&str>`
  parameter.  Both call sites (slot A, slot B) were updated.
* No behaviour change for the default `mt_provider = "google"`
  configuration.
* Operators that previously ran `mt_provider = "local"` without
  `mt_cloud_fallback` continue to see the same `InvalidInput` error for
  unsupported pairs — now with a message explicitly directing them at the
  consent knob instead of a model-specific tokenisation error.
