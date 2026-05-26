# Security Policy

> Issue: [#462 — SEC-01 Supply-chain/security gates, SBOM, SLSA, and signing policy](https://github.com/magicpro97/tui-translator/issues/462)

This document is the single entry point for security-relevant information about
`tui-translator`. It covers how to report vulnerabilities, which versions are
supported, and which CI gates protect the supply chain.

## Reporting a vulnerability

Please report suspected vulnerabilities **privately**. Do **not** open a public
GitHub issue.

- Preferred: open a private security advisory via GitHub
  ([Security → Advisories → Report a vulnerability](https://github.com/magicpro97/tui-translator/security/advisories/new)).
- Alternative: email the maintainer listed in
  [`.github/CODEOWNERS`](.github/CODEOWNERS) (if present) or the repository
  owner's GitHub profile.

Please include:

1. A description of the issue and the impact you observed.
2. Steps to reproduce, ideally with a minimal config and a captured log
   excerpt that does **not** contain a real API key (redact with `****`).
3. The version (git tag or commit SHA) you tested.

Acknowledgement target: **within 7 days** of receipt. The maintainer will then
either request more information, propose a fix, or explain why the report is
not actionable.

## Supported versions

Until v1.0.0 ships, only the latest pre-release tag on `main` receives
security fixes. Older pre-release tags are archived and are not patched.

| Version | Supported |
|---|---|
| `main` (HEAD) | ✅ |
| Latest pre-release tag | ✅ |
| Older pre-release tags | ❌ |

## Supply-chain gates

The repository enforces the following supply-chain gates on every pull
request and push to `main` via
[`.github/workflows/security.yml`](.github/workflows/security.yml):

| Gate | Tool | Status | Notes |
|---|---|---|---|
| Dependency advisories + licenses + bans + sources | `cargo-deny` | **Required** | Policy in [`deny.toml`](deny.toml). |
| Known CVEs in dependencies | `cargo-audit` | Advisory | Promoted to required after noise baseline (tracked in `docs/security/supply-chain.md`). |
| Secret scanning | `gitleaks` | Advisory | Local hook in [`.github/hooks/secret-detector.py`](.github/hooks/secret-detector.py) remains the primary gate; advisory CI is a defence-in-depth check. |
| SBOM generation | `cargo-cyclonedx` | **Required artifact** | Produces CycloneDX 1.5 JSON, uploaded as `sbom-cyclonedx-json`. |

Release-time signing, SLSA provenance, and binary attestation policy are
documented in [`docs/security/signing-policy.md`](docs/security/signing-policy.md).
That policy is **not yet enforced** because it requires identities and
credentials that the repository does not yet hold; the policy doc defines the
target state so the work can be picked up by REL-01 without re-deciding the
shape.

## What this program does and does not capture

User-facing privacy behaviour (audio capture surface, network destinations,
session archives, log redaction rules) is documented in
[`PRIVACY.md`](PRIVACY.md). The security policy here is concerned with the
build supply chain and release artifacts; the privacy statement is concerned
with what the running program does with user data.

## Cross-references

- Supply-chain rationale and gate matrix:
  [`docs/security/supply-chain.md`](docs/security/supply-chain.md)
- Signing & provenance policy (target state):
  [`docs/security/signing-policy.md`](docs/security/signing-policy.md)
- Evidence package: [`verification-evidence/security/`](verification-evidence/security/)
- Privacy statement: [`PRIVACY.md`](PRIVACY.md)
- `cargo-deny` policy: [`deny.toml`](deny.toml)
