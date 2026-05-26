# Supply-chain gates — rationale and matrix

> Issue: [#462 — SEC-01](https://github.com/magicpro97/tui-translator/issues/462)
> Workflow: [`.github/workflows/security.yml`](../../.github/workflows/security.yml)
> Top-level policy: [`SECURITY.md`](../../SECURITY.md)

This document explains **why** each supply-chain gate is shaped the way it is
in `security.yml`, and the criteria under which an advisory gate is promoted
to a required gate. It is the companion to `SECURITY.md`, which is the
user-facing entry point.

## 1. Gate matrix

| Gate | Tool | CI status | Local equivalent | Promotion criteria |
|---|---|---|---|---|
| Dependency advisories + licenses + bans + sources | `cargo-deny check all` | **Required** | `cargo deny check all` | Always required — `deny.toml` is maintained and stable. |
| Known CVEs in dependencies | `cargo-audit --deny warnings` | Advisory (`continue-on-error: true`) | `cargo audit` | Promote to required once one full release cycle passes with zero noise from the RustSec DB after the cargo-deny advisories check. |
| Secret scanning (CI diff) | `gitleaks-action@v2` | Advisory (`continue-on-error: true`) | [`.github/hooks/secret-detector.py`](../../.github/hooks/secret-detector.py) | Promote to required once `.gitleaks.toml` allowlist exists for: `config.example.json`, `verification-evidence/**/*.json`, snapshot fixtures, and any redacted-log fixtures. |
| SBOM (CycloneDX JSON) | `cargo-cyclonedx --format json --spec-version 1.5 --all` | **Required artifact** | `cargo cyclonedx --format json` | The job already fails if the SBOM is empty or malformed; the next promotion step is to attach the SBOM to release assets (REL-01). |

## 2. Why these tools

- **`cargo-deny`** is the only tool that covers all four supply-chain
  surfaces (advisories, licenses, dependency bans, source registry allowlist)
  with one config file (`deny.toml`). It is already used as a manual gate
  inside the project, so promoting it to a CI gate has zero new policy cost.
- **`cargo-audit`** is kept alongside `cargo-deny` because it consumes the
  RustSec database directly and surfaces advisories that `cargo-deny` may
  classify under a different severity. The two tools are complementary, not
  redundant. It is advisory in CI because the RustSec DB can publish a new
  high-severity entry at any time; a flaky red build is worse than an
  advisory comment when nobody is on call.
- **`gitleaks`** is the standard secret scanner for GitHub Actions. The local
  pre-commit hook (`.github/hooks/secret-detector.py`) is the **primary**
  defence; gitleaks in CI is defence-in-depth that runs against the merge
  commit, catching cases where the local hook is bypassed.
- **`cargo-cyclonedx`** produces a CycloneDX 1.5 SBOM, which is the
  spec version that current SLSA tooling consumes. SPDX is **not** generated
  here because no current consumer requires it; if a downstream consumer
  ever does, add a second job rather than replacing CycloneDX.

## 3. Test-case mapping (acceptance criteria → checks)

| Acceptance test case (issue #462) | Enforced by |
|---|---|
| Fake secret fixture blocks PR without exposing real credentials. | Local hook `secret-detector.py` (required) + advisory `secret-scan` job (CI defence-in-depth). A redacted fake-secret fixture lives under `verification-evidence/security/fixtures/` and is **not** committed with a real key. |
| `cargo deny` failure blocks PR. | `cargo-deny` job in `security.yml` (required, no `continue-on-error`). |
| SBOM validates and is attached to release artifacts. | `sbom` job validates `bomFormat == CycloneDX` and `components.length > 0`, uploads `sbom-cyclonedx-json`. Release-time attachment is tracked in `signing-policy.md` §3 (REL-01 follow-up). |
| `cosign verify` succeeds for untampered artifacts and fails after byte tamper. | Tracked in `signing-policy.md` §2. Not implemented in this PR because cosign requires an OIDC identity / Fulcio root and a release tag; doing so here would commit the project to a signing identity without a maintainer decision. |
| Logs/telemetry do not expose Google API keys, BlackHole device names when sensitive, or user home paths. | Existing PRIVACY.md §3 log-redaction rules + the `secret-detector.py` hook scanning new code; verified by `tui-security-auditor` review on PRs that change logging surfaces. |

## 4. What this PR explicitly does NOT do

The following are explicitly **out of scope** for SEC-01 in this PR because
they require credentials, identities, or maintainer decisions that cannot be
made inside a code PR:

1. **Code-signing certificates.** The Windows installer is NOT Authenticode
   signed in this PR. Procurement of an EV or OV code-signing cert is
   tracked by REL-01.
2. **macOS notarization.** No `notarytool` credentials exist; a macOS build
   is not yet shipped as a stable artefact.
3. **SLSA provenance generation at release time.** The reusable
   `slsa-framework/slsa-github-generator` workflow is documented in
   `signing-policy.md` as the chosen target but is not wired into
   `release.yml` until REL-01 promotes the release pipeline.
4. **cosign signing.** Same reason as SLSA — requires a Fulcio identity
   commitment.

These items have a documented target state in
[`signing-policy.md`](signing-policy.md) so the next implementer does not
have to re-derive the design.

## 5. Evidence

- `verification-evidence/security/SEC-01-policy.md` — checklist mapping issue
  #462 acceptance criteria to the files and jobs in this PR.
