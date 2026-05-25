# Supertonic — Release Checklist (DRAFT)

> **Status: DRAFT.** This checklist is the substrate for the release
> packaging step that will ship Supertonic to end users. It is **not
> yet runnable** because Supertonic provider code (#490, #491, #493),
> model cache (#492), voice catalog (#494), and soak gate (#495) have
> not been merged. No release that includes Supertonic may go out
> without ticking every required item below.
>
> Issue: [#497](https://github.com/magicpro97/tui-translator/issues/497).

Run this checklist **after** the normal release checklist in
`docs/07-packaging-verification.md` and **before** tagging a release
that exposes Supertonic to end users (even as an opt-in provider).

---

## 1. Pre-flight gates (must all be ✅)

- [ ] SUPERTONIC-01 feasibility spike has its `❌ Deferred` rows
      replaced with measured values
      (`verification-evidence/supertonic/SUPERTONIC-01-spike.md` §0, §8).
- [ ] SUPERTONIC-02 deferred blockers B-1 … B-5 are closed with
      committed evidence
      (`verification-evidence/supertonic/SUPERTONIC-02-license-privacy.md`
      §8).
- [ ] SUPERTONIC-11 default-readiness ADR
      (`docs/adr/supertonic-11-default-readiness.md`) status reflects
      the actual decision: still `DEFERRED` if any gate is failing.
- [ ] SUPERTONIC-12 user docs
      (`docs/supertonic-user-guide.md`) revision banner accurately
      reflects whether Supertonic is opt-in or default for this build
      flavour.
- [ ] Soak gate (#495) artifact for this release commit is present in
      `verification-evidence/supertonic/`.

If any pre-flight gate fails, **do not ship this release with
Supertonic enabled**. Either disable the `local-tts` build flavour for
this release or block the release.

## 2. Licence & NOTICE artifacts

- [ ] `LICENSE` (MIT, this project) unchanged or amended only with
      reviewer approval.
- [ ] `NOTICE` includes Supertonic source attribution (MIT, upstream).
- [ ] `NOTICE-OpenRAIL-M.txt` is present at the repo root **and** in
      the release artifact (ZIP / installer). Verify by extracting the
      artifact and listing files.
- [ ] OpenRAIL-M restriction summary text in the consent dialog
      matches the version of `NOTICE-OpenRAIL-M.txt` shipped in this
      release (no drift).
- [ ] `PRIVACY.md` Supertonic section reflects the actual runtime
      behaviour of this build.

## 3. Build & packaging dry-run

> **Planned feature flag.** `local-tts` is the proposed name for the
> Supertonic build flavour, mirroring the existing `local-stt` and
> `local-mt` cargo features in `Cargo.toml`. As of this DRAFT, the
> `local-tts` cargo feature does **not** yet exist. The implementation
> PR (#493) is expected to add it; the final name may differ. Replace
> `local-tts` below with the actual feature name introduced by that
> PR before running this checklist.

- [ ] `cargo build --release --features local-tts` succeeds on Windows
      (using whichever cargo feature name the implementation PR landed).
- [ ] The resulting `.exe` ships **zero model weights** (verify by
      inspecting binary size and any embedded resources).
- [ ] Release archive (ZIP) opens cleanly and contains, at minimum:
      `tui-translator.exe`, `LICENSE`, `NOTICE`,
      `NOTICE-OpenRAIL-M.txt`, `PRIVACY.md`, `README.md`,
      `config.example.json`, and any required runtime DLLs (e.g.
      `onnxruntime.dll` per existing local-MT pattern).
- [ ] `config.example.json` does **not** quietly set
      `tts_provider = "supertonic"` unless the default-readiness ADR
      is `ACCEPTED`.
- [ ] `config.example.json` does **not** quietly set
      `tts_cloud_fallback`. Field is omitted (null).

## 4. Privacy / network behaviour

- [ ] Fresh-install run on a machine without the model triggers the
      consent dialog on first selection. Verified manually.
- [ ] Declining the consent dialog leaves the previous TTS provider
      unchanged. No file is written under
      `%LOCALAPPDATA%\tui-translator\models\tts\`.
- [ ] Network-off run after model install produces **zero egress**
      from the `tui-translator.exe` process during synthesis. Capture
      with Windows Firewall log or equivalent and store under
      `verification-evidence/supertonic/`.
- [ ] Attempting to point a `supertonic`-related URL field at a
      non-loopback address is rejected at config load. Verified by a
      committed unit test reference.
- [ ] Voice-clone code path is absent or feature-gated off in this
      build (v1 ships only the official voice presets).

## 5. Fallback & error behaviour

- [ ] Deleting the model file mid-session surfaces a clear
      `ModelNotFound` error in the TUI and **does not** silently
      contact Google.
- [ ] Corrupting the model file surfaces `ChecksumMismatch` and does
      **not** silently re-download without re-prompting consent.
- [ ] Setting `tts_cloud_fallback = "google"` and then breaking the
      model file produces a visible "falling back to Google TTS"
      message, with no API call when `google_api_key` is absent.

## 6. Documentation links

- [ ] `docs/supertonic-user-guide.md` link-checks cleanly (no broken
      intra-doc links). Run `npx --yes markdown-link-check` or the
      project's existing doc-link gate.
- [ ] `docs/supertonic-release-checklist.md` (this file) link-checks
      cleanly.
- [ ] `docs/adr/supertonic-11-default-readiness.md` link-checks
      cleanly.
- [ ] `verification-evidence/supertonic/SUPERTONIC-01-spike.md` and
      `SUPERTONIC-02-license-privacy.md` link-check cleanly.
- [ ] `README.md` mentions Supertonic only with accurate status (no
      claim of "default" while the ADR is `DEFERRED`).

## 7. Evidence package

- [ ] `verification-evidence/supertonic/` contains, for this release:
      - the SUPERTONIC-01 measured numbers,
      - the SUPERTONIC-02 packaging dry-run output,
      - the SUPERTONIC-10 (#495) soak run output,
      - any bench JSON used by the SUPERTONIC-11 verdict,
      - a `release-<version>.md` index that references each of the
        above with file paths and SHAs.

## 8. Default-flip authorisation (only when flipping)

Skip this section unless this release also flips the default
`tts_provider` for the `local-tts` build flavour.

- [ ] The flip is the **only** behavioural change in the PR that flips
      it, per
      [`docs/adr/supertonic-11-default-readiness.md`](./adr/supertonic-11-default-readiness.md)
      §3.3.
- [ ] Existing user configs with explicit `tts_provider` values are
      preserved (verified by an automated test that loads a fixture
      config and asserts the value is untouched).
- [ ] Human / product approver name and date are recorded in the ADR's
      amended status block.
- [ ] Release notes call the default flip out by name in their first
      bullet.

## 9. Sign-off

| Reviewer role | Name | Date | Verdict |
|---------------|------|------|---------|
| Release engineer |  |  |  |
| Privacy / security reviewer (`tui-security-auditor` for the impl PR) |  |  |  |
| Docs reviewer |  |  |  |
| Product approver (only required when §8 applies) |  |  |  |

Do not tag the release until every applicable row is filled and every
non-skipped checkbox in §1–§7 is ticked.
