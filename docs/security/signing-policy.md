# Release signing & provenance policy (target state)

> Issue: [#462 — SEC-01](https://github.com/magicpro97/tui-translator/issues/462)
> Status: **policy declared; enforcement deferred to REL-01.**
> Top-level policy: [`SECURITY.md`](../../SECURITY.md)

This document defines the target state for signing, provenance, and
attestation of `tui-translator` release artifacts. It exists so that REL-01
can implement the policy without re-deciding the shape, and so that external
consumers can audit what we will eventually attest.

It is **not yet enforced** in CI. Every item in this document is currently
gated by the existence of an identity (a code-signing certificate, a Fulcio
OIDC identity commitment, or a notarization Apple ID). The policy is written
so that each item can be turned on independently when its identity becomes
available.

## 1. Artifacts covered

The release workflow ([`release.yml`](../../.github/workflows/release.yml))
produces two artefacts per tag:

1. `tui-translator-<tag>-x86_64-pc-windows-msvc.zip` — portable build.
2. `tui-translator-<tag>-setup.exe` — Inno Setup per-user installer.

The SBOM job in [`security.yml`](../../.github/workflows/security.yml)
produces `bom.json` per Cargo workspace member; the release-time variant
will include the full transitive dependency graph for the
`x86_64-pc-windows-msvc` target build.

## 2. Signing policy

### 2.1 Sigstore / cosign (preferred — keyless, OIDC)

- **Identity provider:** GitHub Actions OIDC (`https://token.actions.githubusercontent.com`).
- **Subject claim:** the release workflow path
  `https://github.com/magicpro97/tui-translator/.github/workflows/release.yml@refs/tags/v*`.
- **Tool:** [`sigstore/cosign-installer`](https://github.com/sigstore/cosign-installer) +
  `cosign sign-blob --yes <artifact>` producing a `.sig` and a `.pem`
  certificate referencing the Fulcio root.
- **Verification command** (published in release notes):

  ```sh
  cosign verify-blob \
    --certificate <artifact>.pem \
    --signature   <artifact>.sig \
    --certificate-identity-regexp '^https://github.com/magicpro97/tui-translator/\.github/workflows/release\.yml@refs/tags/v.*$' \
    --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
    <artifact>
  ```

- **Tamper test (acceptance #462):** flipping one byte of `<artifact>` must
  cause `cosign verify-blob` to exit non-zero. This is a release-gate test
  recorded by REL-01.

### 2.2 Windows Authenticode (deferred — requires certificate)

- **Tool:** `signtool sign /fd SHA256 /tr <RFC3161-tsa> /td SHA256`.
- **Certificate source:** to be procured by the maintainer; tracked by
  REL-01. Until then, the installer is published unsigned and marked as
  pre-release.
- **Target:** sign both `tui-translator.exe` *before* it is packaged into the
  zip, and the Inno Setup `.exe` installer *after* it is built.

### 2.3 macOS notarization (deferred — requires Apple ID)

- **Tool:** `xcrun notarytool submit ... --wait`, then `stapler staple`.
- Tracked separately under the macOS-readiness epic; not part of the
  Windows-first v1 release surface.

## 3. SLSA provenance

- **Generator:** [`slsa-framework/slsa-github-generator`](https://github.com/slsa-framework/slsa-github-generator)
  reusable workflow (`generic_v1.10.0` or newer at the time of REL-01).
- **Provenance level target:** SLSA v1.0 **Build L3** (hermetic, isolated,
  parameterless). The current release workflow already meets L1 (versioned
  build script) and L2 (hosted-runner build with provenance); L3 requires
  invoking the reusable workflow so the GitHub-hosted attestation signer is
  in the trust chain.
- **Provenance artefact:** `<release>.intoto.jsonl` published next to the
  zip and the installer.
- **Verification command:**

  ```sh
  slsa-verifier verify-artifact \
    --provenance-path <release>.intoto.jsonl \
    --source-uri github.com/magicpro97/tui-translator \
    --source-tag <tag> \
    <artifact>
  ```

## 4. SBOM attachment at release time

- The release job will run the same `cargo-cyclonedx` step as the PR-time
  SBOM job (see [`security.yml`](../../.github/workflows/security.yml)) but
  targeted at the MSVC release build, then attach `bom.json` to the GitHub
  Release alongside the zip and the installer.
- The SBOM file is **not** signed independently; the SLSA provenance
  attestation covers it transitively because it is listed in the release
  artefact set.

## 5. Implementation roadmap (for REL-01)

1. Wire `slsa-framework/slsa-github-generator` into `release.yml` as a
   reusable workflow call (no credentials required — uses the workflow's
   OIDC identity). Acceptance: `slsa-verifier verify-artifact` passes for a
   freshly cut tag.
2. Wire `sigstore/cosign-installer` + `cosign sign-blob` into `release.yml`
   (no credentials required). Acceptance: `cosign verify-blob` passes for a
   freshly cut tag; passes negatively after byte tamper.
3. Attach the release-time CycloneDX SBOM to the GitHub Release.
4. Procure a Windows code-signing certificate (maintainer task) and add
   `signtool` to the packaging step. Acceptance: Windows SmartScreen no
   longer warns on unsigned-publisher.
5. (Future, macOS) Procure an Apple Developer ID and add `notarytool` +
   `stapler`.

Each step is independently shippable; do not block step 1 on step 4.

## 6. Threat model coverage

| Threat | Mitigation in this policy |
|---|---|
| Tampered binary served from a mirror or compromised release page. | §2.1 cosign signature + §3 SLSA provenance. |
| Tampered binary on an end-user's machine post-download. | §2.1 cosign + §2.2 Authenticode (when shipped). |
| Compromised maintainer account publishing a malicious tag. | §3 SLSA L3 (hermetic build from public source) gives downstream a way to detect that the tag did not come from the canonical reusable workflow. |
| Dependency confusion / yanked crate. | `deny.toml` `sources.unknown-registry = "deny"` + `yanked = "deny"`, enforced by `cargo-deny` in [`security.yml`](../../.github/workflows/security.yml). |
| Secret leak in build logs. | `gitleaks` (advisory) + local `secret-detector.py` hook + `tui-security-auditor` PR review. |
