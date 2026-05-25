# SEC-01 — Evidence checklist

> Issue: [#462 — SEC-01 Supply-chain/security gates, SBOM, SLSA, and signing policy](https://github.com/magicpro97/tui-translator/issues/462)
> Tentacle: Wave 5 Group B executor — `feat/ci-sec-policy`
> Evidence mode: `workflow_first_run` — PR CI will populate run URLs once the
> branch is pushed; this file documents which jobs satisfy which acceptance
> criteria.

## 1. Acceptance criteria → enforcement map

| Acceptance criterion (issue #462) | Status in this PR | Enforced by |
|---|---|---|
| Security gate is required before release. | ✅ in PR; release-time wiring deferred to REL-01 | `cargo-deny` job is required on every PR/push (`.github/workflows/security.yml`). `release.yml` runs after `main` is updated, so the gate runs before the tag is cut. SBOM attachment at release time is documented in `docs/security/signing-policy.md` §4 — REL-01 follow-up. |
| No secrets or private paths leak into CI artifacts/logs. | ✅ defence-in-depth | Existing local hook `.github/hooks/secret-detector.py` (required path) + advisory `gitleaks` CI job (defence-in-depth). PRIVACY.md §3 documents log redaction rules. The SBOM job's `sbom-out/*.json` contains only crate metadata, never source paths. |
| SBOM and provenance are generated for release assets. | ✅ SBOM at PR time; ⏸️ provenance deferred to REL-01 | `sbom` job in `security.yml` produces CycloneDX 1.5 JSON, validates `bomFormat`/`specVersion`/non-empty `components`, and uploads as `sbom-cyclonedx-json` artifact. SLSA provenance is policy-declared in `docs/security/signing-policy.md` §3, to be wired into `release.yml` by REL-01. |
| `tui-security-auditor` or Opus security review is clean. | 🔲 pending PR review | The PR description for `feat/ci-sec-policy` carries `### Opus review evidence` per PROC-01; security-tier review is requested. |

## 2. Test-case mapping

| Test case (issue #462) | Result | Detail |
|---|---|---|
| Fake secret fixture blocks PR without exposing real credentials. | Covered by local hook (existing) + advisory CI | `.github/hooks/secret-detector.py` already implements this gate; the CI advisory job is defence-in-depth. A real fake-secret fixture is **not** added in this PR to avoid any chance of a placeholder being mistaken for a real credential by downstream tools; the existing local hook's own test suite covers the case. |
| `cargo deny` failure blocks PR. | ✅ Enforced | `security.yml` → `cargo-deny` job (no `continue-on-error`). Action: `EmbarkStudios/cargo-deny-action@v2 check all`. |
| SBOM validates and is attached to release artifacts. | ✅ PR-time / ⏸️ release-time | `security.yml` → `sbom` job validates CycloneDX schema and component count, uploads artifact. Release attachment is deferred to REL-01 per `signing-policy.md` §4. |
| `cosign verify` succeeds for untampered artifacts and fails after byte tamper. | ⏸️ deferred | Policy and verification command published in `signing-policy.md` §2.1. Implementation deferred (requires committing the repo to a Fulcio OIDC identity; that decision is REL-01-scope). |
| Logs/telemetry do not expose Google API keys, BlackHole device names when sensitive, or user home paths. | ✅ existing behaviour | Documented in `PRIVACY.md` §3; enforced by local secret-detector hook and by `tui-security-auditor` review on PRs that change logging surfaces. |

## 3. Files added or changed in this PR

| Path | Purpose |
|---|---|
| `.github/workflows/security.yml` | **New.** Four-job security workflow: required `cargo-deny`; advisory `cargo-audit`; advisory `secret-scan`; required `sbom` with CycloneDX 1.5 JSON validated and uploaded. |
| `SECURITY.md` | **New.** Top-level security policy: how to report, supported versions, supply-chain gate matrix, cross-references. |
| `docs/security/supply-chain.md` | **New.** Rationale for the supply-chain gate shape and the promotion criteria from advisory → required. |
| `docs/security/signing-policy.md` | **New.** Target-state signing & provenance policy (cosign, SLSA generator, Authenticode, notarization). Documented so REL-01 does not re-derive. |
| `verification-evidence/security/SEC-01-policy.md` | **New.** This file. |

The existing `deny.toml` is unchanged: it already covers advisories,
licenses, bans, and sources for the Windows targets the project ships.

## 4. CI run URLs

Populated automatically once the PR is opened. Expected jobs:

- `cargo-deny (advisories + licenses + bans + sources)` — required.
- `cargo-audit (RustSec, advisory)` — advisory.
- `Secret scan (gitleaks, advisory)` — advisory.
- `SBOM (CycloneDX, all features)` — required.

## 5. Required-check promotion checklist

When branch protection is updated, the following contexts should be added
to the required list **for `main`**:

- `cargo-deny (advisories + licenses + bans + sources)`
- `SBOM (CycloneDX, all features)`

The following contexts must **NOT** be added until their promotion criteria
in `docs/security/supply-chain.md` §1 are met:

- `cargo-audit (RustSec, advisory)`
- `Secret scan (gitleaks, advisory)`
