# JV-15 — Security & Privacy Audit: Local MT, Benchmarks, Fallback Consent

> **Scope.** This audit covers the privacy and security surfaces introduced
> by the JV (Japanese↔Vietnamese local-MT) workstream: local MT logs,
> benchmark artifact retention (`src/bin/mt_bench.rs`, `docs/evidence/`),
> cloud-fallback consent gating, and the model bundle install path added in
> JV-09. It is the documented counterpart to the issue #423 acceptance
> criteria and is intended to be re-run on every release-candidate.
>
> **Status:** **CLEAN** as of branch `feat/jv-manifest-and-audit`. Re-run
> the checklist before promoting any default-flip change (#421) or shipping
> real model bytes (#419).

---

## 1. Threat model

| # | Asset | Threat | Mitigation |
|---|-------|--------|------------|
| T1 | Meeting source text (Japanese audio transcript) | Leak via persisted logs, crash dumps, or benchmark artifacts | `tracing` macros never receive raw source/translated text. Benchmark artifacts use fixture-only corpora (see §3). |
| T2 | Translated text (Vietnamese rendering) | Same as T1 | Same as T1. The TUI keeps live text in RAM only; no debug log of rendered lines. |
| T3 | Google API key (cloud STT / cloud MT fallback) | Leak via logs, redaction failure, or accidental commit | Key is loaded from `config.json` (gitignored) or env. `mt_bench.rs` rejects placeholder keys and never prints the key value (only a `warning: --google-api-key looks like a placeholder` notice). No `tracing::*!` site logs the key. |
| T4 | Local model bytes | Tampered/poisoned weights at rest or in transit | Every bundle file is SHA-256-verified; mismatches are quarantined to `<file>.corrupt` and surfaced as `ChecksumMismatch` with the expected/actual digests. |
| T5 | Filesystem outside model cache | Path traversal via crafted manifest | `safe_relative_path` rejects `..`, drive prefixes (`C:\...`), absolute paths, and any non-Normal component. Tests: `manifest_rejects_parent_directory_paths`, `manifest_rejects_drive_prefixed_paths`. |
| T6 | Cloud fallback (silent egress) | Cloud provider invoked without explicit operator consent | `MtRouter::new` only stores the cloud leg when `mt_cloud_fallback = "google"` is explicitly set **and** the API key is present. Key presence alone does not enable fallback. Every fallback hop emits a single `tracing::warn!` privacy-boundary marker. |
| T7 | Offline guarantee | Inadvertent network request in offline mode | `TUI_TRANSLATOR_OFFLINE=1` makes `offline_guard()` return `BootstrapError::Offline` before any socket is opened; covered by `offline_guard_fails_when_var_set`. |
| T8 | Consent record forgery | Stale consent silently honoured after licence change | `model_consent_status` compares both `version` and `license_url`; a difference returns `ConsentStatus::Stale { reason }` and forces re-consent. Covered in `bootstrap.rs` unit tests. |

---

## 2. Code surfaces audited

| Surface | File | Notes |
|---------|------|-------|
| Local model bundle install | `src/providers/local/model_download.rs` | Path safety, SHA-256, resume-and-finalize, disk space pre-flight, content-range/length validation. |
| Bootstrap consent + offline guard | `src/providers/local/bootstrap.rs` | Consent dir `%LOCALAPPDATA%\tui-translator\consent`, atomic write, sanitised filenames, validate license text (rejects control chars + DEL). |
| MT runtime router | `src/providers/mt/router.rs`, `src/providers/mt/routing.rs` | No-silent-cloud invariant; cloud leg only present when explicit `mt_cloud_fallback` set. |
| MT benchmark binary | `src/bin/mt_bench.rs` | No raw text in `println!`/`eprintln!`. Validates `license_source_url` is `https://`. Cost preflight and dry-run paths print only counts/USD. |
| TUI rendering | `src/tui/*.rs` | Live text stays in RAM; no log statements include subtitle bodies. |
| Config | `src/config/*.rs`, `config.example.json` | `mt_cloud_fallback` is `Option<String>`; absent by default. Key is `Option<String>`. |

---

## 3. Benchmark artifact retention

* **Corpora.** Benchmarks consume only fixtures committed under `tests/fixtures/` (e.g. `tests/fixtures/jv02/`) and reference assets listed in `docs/evidence/ja-vi-benchmark-corpus-plan.md`. Real meeting recordings are out of scope.
* **Outputs.** `mt_bench` writes:
  * `<output>.json` — schema-validated artifact (model metadata, scores, license URLs, latency stats).
  * `<output>.md` — human-readable summary.
  * `<output>.ndjson` — per-language-pair rows.
  None of the writers serialise raw source/translated text — only score aggregates, identifiers, and licence URLs.
* **Retention.** Evidence artifacts in `docs/evidence/` are versioned and reviewed in PRs. Operator-private benchmark runs MUST be written under `verification-evidence/` or `target/` (gitignored). The pre-commit artifact guard (`.github/hooks/artifact-guard.py`) rejects accidental commits of unredacted artifacts.
* **Recommendation.** Any future change that adds `--keep-raw-text` to `mt_bench` MUST update this document and require an explicit consent flag.

---

## 4. Fallback-consent verification

* **Configuration contract.** `config.example.json` documents
  `mt_cloud_fallback` as the *only* knob that promotes a cloud provider into
  the MT runtime. The default config does not contain the field, so the
  cloud leg of `MtRouter` is `None`.
* **Code invariant.** `MtRouter::new(local, cloud_fallback)` accepts an
  `Option<C>`. The construction site in `src/main.rs` only passes
  `Some(provider)` when `cfg.mt_cloud_fallback == Some("google")` *and* a
  non-empty `google_api_key` is configured. Either of these missing causes
  every unsupported pair to fall through to `ResolvedRoute::Unsupported` →
  user-visible provider error, no silent cloud call.
* **Logging.** When a fallback hop is taken, the router emits a single
  `tracing::warn!` privacy-boundary marker carrying only the language pair
  (no text). This is by design and is part of the auditable cross-boundary
  trail.
* **Test coverage.** `tests/mt_routing.rs` and the unit tests in
  `src/providers/mt/router.rs` exercise both "no cloud configured" and
  "cloud configured but pair routable locally" paths.

---

## 5. Manifest / download / consent verification (JV-09 additions)

* **JSON schema** committed at
  `docs/specs/jv-09/model-bundle-manifest.schema.json`. Operators or CI can
  validate any third-party manifest against the schema before publishing.
* **Example fixture manifest** at
  `docs/specs/jv-09/example-mt-bundle-manifest.json` uses dummy
  `example.invalid` URLs and tiny payload sizes (`hello world`, `hello`) so
  the schema and parser can be smoke-tested without real model bytes.
* **Consent persistence.** `cargo run -- --install-local-mt-model <path> --yes`
  now writes a consent record via `write_model_consent_record` *before*
  any byte is downloaded. The record contains the model id, version,
  licence URL (taken from `source_url`), and Unix timestamp. Re-running with
  the same manifest is idempotent (Fresh status). Bumping `version` or
  changing the `source_url` produces `ConsentStatus::Stale { reason }`,
  forcing the operator to re-confirm.
* **License text display.** The install path prints the full `license` text
  to stdout before the `--yes` gate. The text is validated to reject ASCII
  control characters (other than `\t \n \r`) and the DEL character by
  `validate_license_text`.
* **Path safety.** `ModelBundleManifest::validate` rejects empty paths,
  duplicate paths, `..`, drive prefixes, root prefixes, and any non-Normal
  component. Covered by `manifest_rejects_parent_directory_paths` and
  `manifest_rejects_drive_prefixed_paths`.

---

## 6. Secret-detector sweep

The following grep patterns were run across `src/`, `tests/`,
`docs/evidence/`, and `verification-evidence/`:

```
tracing::(info|warn|debug|error|trace)!.*(source_text|translated|input_text|output_text|api[_-]?key|secret)
println!.*\$\{?(api_key|google_api_key|secret)
AIza[0-9A-Za-z_-]{35}   # Google API key prefix
ya29\.[0-9A-Za-z_-]+    # OAuth bearer
```

**Result:** No matches found in production code paths or committed
artifacts. The only mention of `api_key` in `mt_bench.rs` is a *placeholder
detector* (`is_placeholder_key`) that never prints the key value.

---

## 7. Checklist (re-run before each release / default-flip)

- [x] No `tracing::*!` macro receives raw source/translated text.
- [x] No `println!`/`eprintln!`/`writeln!` site prints `google_api_key` or `mt_cloud_fallback` secret values.
- [x] Benchmark artifact writers serialise only metadata + scores + licence URLs.
- [x] `mt_cloud_fallback` consent gate present in `MtRouter::new` construction site.
- [x] Offline guard rejects network when `TUI_TRANSLATOR_OFFLINE=1`.
- [x] Model bundle paths reject `..`, drive prefixes, root prefixes (`safe_relative_path`).
- [x] SHA-256 mismatch quarantines the file and returns `ChecksumMismatch`.
- [x] Consent record persisted with model id, version, licence URL, and Unix timestamp.
- [x] Consent re-prompt fires on version OR licence URL change.
- [x] Disk-space pre-flight returns `InsufficientDiskSpace` before any HTTP request.
- [x] Secret-detector sweep over `src/`, `tests/`, `docs/evidence/`, `verification-evidence/`.

---

## 8. Verdict

**CLEAN** for the JV-09 + JV-15 scope on `feat/jv-manifest-and-audit`.
This audit does NOT clear:

* Real-weight publication (#419) — re-run §3 and §5 once real model URLs
  are added to a manifest.
* Default flip to local MT (#421) — re-run §4 and confirm the default
  config does not enable cloud fallback implicitly.
* End-to-end benchmark execution (#413–#415) — re-run §3 once artifacts
  with real meeting fixtures are produced.

Open the audit as a tracked issue if any new MT log / artifact / consent
surface is introduced between this audit and the next release.
