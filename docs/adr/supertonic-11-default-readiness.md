# ADR SUPERTONIC-11 — Default-readiness gate and conditional default flip (DRAFT)

> **Issue:** [#496](https://github.com/magicpro97/tui-translator/issues/496)
> **Parent:** [#485](https://github.com/magicpro97/tui-translator/issues/485)
> **Status:** **DRAFT — default flip DEFERRED.** No code change in this ADR.
> **Date:** 2026-05-25
> **Routing decision confidence:** 1.0 — the routing decision (do **not**
> flip the default) is committed. The eventual default-flip is gated on
> evidence that does not yet exist; that evidence is enumerated in §6.

This ADR mirrors `docs/adr/jv-08-default-eligibility-decision.md` in
shape and tone but covers TTS rather than MT.

---

## Decision

Do **not** flip the default TTS provider to Supertonic.

The default remains:

```json
{
  "tts_enabled": false
}
```

When `tts_enabled` is set to `true` by the user, the default provider
remains **Google Cloud Text-to-Speech**. Supertonic is an **opt-in**
implementation track and is **not default-eligible** until every gate
in §3 passes with committed evidence in `verification-evidence/`.

The default for `tts_cloud_fallback` remains **null** (not set). This
ADR does not loosen that default and explicitly forbids any
implementation PR from doing so silently.

---

## Why this decision is confidence 1.0

The empirical evidence required to select Supertonic as a default is
missing (see §6 deferred blockers), but the evidence is sufficient to
route the release plan:

1. Keep the user-visible default unchanged (Google or off).
2. Land Supertonic as an opt-in provider once #486, #487, #490–#494
   ship.
3. Re-open this ADR for amendment only after the gates in §3 pass on
   the hardware classes in §4.
4. The default flip, if it ever happens, is a one-line config-default
   change shipped behind an explicit human/product approval (§5).

This separates **routing confidence** (clear: do not flip) from
**product-quality confidence for a Supertonic default** (not yet
established).

---

## 1. What "default-ready" means here

A provider is default-ready when **all** of the following hold,
simultaneously, on **two reference hardware classes** (§4) across
**three repeated runs**:

1. Functional gate — synthesises `en`, `ja`, `vi` end-to-end through
   the real `TtsProvider` impl.
2. Performance gate — meets RTF, TTFA, cold-start, and RSS budgets to
   be set by SUPERTONIC-03 (the bench spike) and SUPERTONIC-04.
3. Privacy gate — SUPERTONIC-02 (#487) consent flow, NOTICE, and
   no-silent-network test all pass.
4. Stability gate — SUPERTONIC-10 soak completes with no `_exit`-style
   shutdown workaround and no crash.
5. Fallback gate — when the Supertonic path returns
   `ModelNotFound` / `ChecksumMismatch`, the UX surfaces a clear error
   and does **not** silently fall back to cloud unless
   `tts_cloud_fallback` is explicitly set.
6. Documentation gate — SUPERTONIC-12 (#497) user docs, troubleshooting,
   and release checklist are merged and link-clean.

Any single failing gate keeps the default OFF / Google.

## 2. Default-eligibility rules (machine-readable)

A future tooling pass can read these rules verbatim. They are also the
exit criteria for this ADR's "DEFERRED" status.

```yaml
default_ready:
  build_flavor:
    local-tts:
      condition: ALL_GATES_PASS AND human_approval_recorded
      then: default_tts_provider = "supertonic"
      else: default_tts_provider = "google"  # or off
    non-local-tts:
      always: default_tts_provider = "google"  # or off
gates_required:
  - functional: [#486]
  - performance: [SUPERTONIC-03, SUPERTONIC-04]
  - privacy:    [#487]
  - stability:  [SUPERTONIC-10]
  - fallback:   [#487, #496]
  - docs:       [#497]
hardware_classes_required: 2
repeated_runs_required: 3
existing_explicit_config_overrides: preserved   # see §3.4 ("No migration code")
```

The current verdict is `default_ready = false`.

---

## 3. Conditional default-flip implementation plan

When (and only when) all gates pass with committed evidence:

1. **Land the provider behind feature gating only.** The implementation
   PR (#490 / #491 / #493) ships Supertonic as an opt-in `tts_provider`
   value. No default change in that PR.
2. **Land the gate evidence.** A separate evidence PR appends signed
   `verification-evidence/supertonic/SUPERTONIC-*-bench.json` and the
   matching markdown summaries.
3. **Land the default flip as a single, isolated PR.** Diff scope:
   - `config.example.json` default value change (commented).
   - `src/config/defaults.rs` (or equivalent) one-line constant change,
     conditional on the `local-tts` build flavor.
   - `docs/supertonic-user-guide.md` revision banner moves from
     `DRAFT — Google remains the default` to
     `Default in local-tts builds since vX.Y`.
   - This ADR amended from `DEFERRED` to `ACCEPTED — date, evidence
     links, human approver name`.
4. **No migration code.** Existing user configs with explicit
   `tts_provider` values **must remain unchanged**. The flip changes
   only the implicit default for newly created configs and for configs
   that have never set `tts_provider`.

## 4. Reference hardware classes

The gate verdict must replicate on at least two of the following:

| Class | Why it matters | Acceptable proxy |
|-------|----------------|------------------|
| A — Modern Windows 11 laptop, 8c/16t Intel/AMD, 16+GB RAM | Target user baseline | Any contributor laptop meeting spec |
| B — Older Windows 10 device, 4c/8t, 8GB RAM | Lower-bound user device | CI runner of equivalent class is acceptable only if its CPU model is recorded |
| C — Windows 11 with iGPU only, no dGPU | Confirms CPU-only inference is the supported path | Optional third class |

Two of {A, B, C} required; A+B preferred.

## 5. Rollback path (one config flag)

If the default flip is shipped and any user regression surfaces within
the next release cycle:

1. Revert the single-line default change.
2. Land a follow-up release with `default_tts_provider = "google"`
   restored.
3. No data migration is required — explicit user configs were never
   touched (§3.4).
4. Existing on-disk model weights remain on disk; the user can re-enable
   Supertonic explicitly without re-downloading.

A user can pre-emptively pin Google by writing `"tts_provider":
"google"` in their `config.json` today. This is the same rollback path
they would use, by hand, after a default flip.

### 5.1 Test cases for the migration / non-migration behaviour

These tests are required for the default-flip PR (not for this ADR):

- Fresh `local-tts` build, no prior config: defaults to Supertonic
  **only** when gate verdict is PASS; otherwise defaults to off /
  Google.
- Non-`local-tts` build: defaults to Google (or off) regardless of
  Supertonic gate state.
- Existing config with `tts_provider = "google"`: remains Google.
- Existing config with no `tts_provider` key: takes the new default for
  its build flavor.
- `tts_cloud_fallback` remains `null` unless the user opted in; the
  default flip PR must not touch this field's default.

## 6. Deferred blockers (mirror the SUPERTONIC-02 §8 pattern)

Until each row is closed by a separate PR with committed evidence, the
verdict in §1 stays `default_ready = false`.

| ID | Blocker | Owning issue |
|----|---------|--------------|
| G-1 | No measured bench numbers on the hardware classes in §4 | SUPERTONIC-03 / SUPERTONIC-04 |
| G-2 | Privacy gate (§3 row 3) has no runtime evidence yet | #487 §8 B-1..B-5 |
| G-3 | Soak gate (§3 row 4) has no committed soak artifact | SUPERTONIC-10 (#495) |
| G-4 | Fallback gate has no automated test for the `ModelNotFound` / `ChecksumMismatch` paths | implementation PR for #490 / #491 |
| G-5 | User docs (§3 row 6) marked DRAFT until evidence lands | #497 (this PR) |
| G-6 | No recorded human/product approval for a default flip | governance step, post-PR-review |

## 7. Acceptance-criterion mapping (issue #496)

| Criterion | Where addressed |
|---|---|
| Default-readiness ADR | this file |
| Machine-readable verdict | §2 YAML block |
| Conditional default-flip implementation plan | §3 |
| Rollback path | §5 |
| Config migration tests | §5.1 (specified; tests live in the eventual default-flip PR) |
| User-facing onboarding/consent text | `docs/supertonic-user-guide.md` (#497, this PR, DRAFT) |
| All upstream gates pass on ≥2 hardware classes ×3 runs | §1, §4 — gating policy; evidence deferred |
| No silent migration | §3.4 |
| Explicit human/product approval recorded | §3.3, §6 G-6 |
| Opus review CLEAN | this PR's review thread |

## 8. Cross-links

- ADR JV-08 (`docs/adr/jv-08-default-eligibility-decision.md`) — same
  shape, MT side. Read before amending this file.
- SUPERTONIC-01 spike (`verification-evidence/supertonic/SUPERTONIC-01-spike.md`)
  — recommended provider shape (Option A, native ORT).
- SUPERTONIC-02 memo
  (`verification-evidence/supertonic/SUPERTONIC-02-license-privacy.md`)
  — binding privacy / NOTICE inputs.
- SUPERTONIC-12 user docs (`docs/supertonic-user-guide.md`) — user-facing
  surface; must move from `DRAFT` to `ACCEPTED` in lockstep with this
  ADR.
