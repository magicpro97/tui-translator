# REL-01 — Cross-Platform Release Packaging, Notarization, and GA Promotion Plan

> **Issue**: #463  
> **Status**: In progress — macOS packaging workflow and scripts implemented;
> notarization requires Apple Developer Program membership and secrets.

---

## Release channels

| Channel  | Tag pattern | Artifacts required | Human gate |
|----------|-------------|-------------------|-----------|
| Nightly  | `v*-nightly.*` | CI pass only | None |
| Beta     | `v*-beta.*` | CI + L4 soak pass | Opt-in testers |
| RC       | `v*-rc.*` | All gates below | Named reviewers |
| Stable   | `v*` (no suffix) | Full checklist | All L5 reviewers |

---

## Platform artifacts

| Platform | Format | Script | CI workflow |
|----------|--------|--------|-------------|
| Windows | `.exe` portable zip + `.msi` installer | `packaging/windows/` | `release.yml` (existing) |
| macOS | `.app` bundle inside `.dmg` | `scripts/package-macos.sh` | `release-macos.yml` |
| Linux | `tar.gz` / `.deb` / `.rpm` / AppImage | `scripts/package-linux.sh` | `release-linux.yml` |

---

## GA promotion checklist

Use this checklist against the **exact release commit SHA**, not a branch tip.

### Layer 1 — CI gates (automated)
- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test --all` passes on Windows, macOS, and Linux
- [ ] `cargo audit` reports no RUSTSEC advisories
- [ ] `cargo deny check` passes
- [ ] Conventional Commits + DCO check passes
- [ ] Opus review evidence present (`verification-evidence/proc-opus-gate/`)

### Layer 2 — Contract and integration tests (automated)
- [ ] Contract tests (mock-only) pass on all platforms
- [ ] PTY harness tests pass
- [ ] Feature matrix (audio-integration, production-audio) CI passes

### Layer 3 — Build and packaging (automated)
- [ ] Windows portable `.zip` and `.msi` build successfully
- [ ] macOS universal `.dmg` builds successfully on `macos-14`
- [ ] Linux `tar.gz`, `.deb`, `.rpm`, AppImage build successfully (REL-02)
- [ ] `SHA256SUMS` present in each platform artifact set
- [ ] `lipo -info` confirms universal binary architecture on macOS
- [ ] No VC++ runtime DLLs in Windows artifact (verified by `dumpbin`)

### Layer 4 — Soak and performance gates (requires hardware)
- [ ] 30-minute Windows soak: 0 panics, RSS ≤ 20 MB growth, 0 capture drops
- [ ] JV-16 local MT soak artifact present (if local MT is default)
- [ ] QA8-05 8-hour soak artifact present and reviewed by `tui-soak-monitor`

### Layer 4.5 — Security gates
- [ ] SEC-01 and SEC-02 artifacts present for release commit
- [ ] JV-15 MT logs and fallback consent audit clean
- [ ] No API keys in packaged artifacts (`grep -r "AIza" dist/`)

### Layer 4.5 — Signing and notarization (macOS, requires Apple Developer ID)
- [ ] `codesign --verify --verbose=4` passes on `.app` and `.dmg`
- [ ] `xcrun notarytool submit --wait` returns `Accepted`
- [ ] `xcrun stapler staple` succeeds
- [ ] `spctl --assess --type install` passes (Gatekeeper)
- [ ] Verification logs committed to `verification-evidence/rel-01/`

### Layer 5 — Human acceptance (requires named reviewers)
- [ ] L5-1 subtitle accuracy review signed off (`#116`)
- [ ] L5-2 bilingual translation review signed off (`#117`)
- [ ] L5-3 translated audio toggle validated (`#118`)
- [ ] L5-4 terminal compatibility matrix verified (`#119`)
- [ ] L5-5 non-developer onboarding review complete (`#120`)
- [ ] L5-6 live meeting readability review complete (`#121`)
- [ ] Acceptance log signed and dated (`#122`)

### Rollback plan
1. Re-publish the previous GitHub Release as `latest`.
2. Update the channel manifest `channel-manifest.json` to point `stable` back.
3. Post a release announcement noting the rollback reason and expected fix ETA.
4. Tag the broken release as `v*-yanked` to preserve audit trail.
5. Delete the broken stable tag from the release page (do not delete the underlying commit).

### Post-release monitoring (first 72 hours)
- Monitor GitHub Discussions / Issues for crash reports.
- Check Sentry / crash dump if configured.
- Track download counts vs. previous release to detect update adoption.

---

## macOS packaging steps (manual, for releases without CI secrets)

```bash
# Prerequisites:
#   - Xcode Command Line Tools installed
#   - Rust targets: rustup target add aarch64-apple-darwin x86_64-apple-darwin
#   - Optional: Apple Developer ID certificate imported into Keychain

# Build unsigned DMG
./scripts/package-macos.sh --arch universal

# Build signed DMG (requires APPLE_DEVELOPER_ID_APP env var)
export APPLE_DEVELOPER_ID_APP="Developer ID Application: Your Name (TEAM_ID)"
export APPLE_BUNDLE_ID="com.yourname.tui-translator"
./scripts/package-macos.sh --arch universal --sign

# Build signed + notarized DMG (requires stored credentials)
xcrun notarytool store-credentials "tui-translator-notarize" \
  --apple-id "you@example.com" \
  --team-id "TEAM_ID"
export APPLE_KEYCHAIN_PROFILE="tui-translator-notarize"
./scripts/package-macos.sh --arch universal --notarize
```

---

## Linux packaging steps (manual)

```bash
# Prerequisites: cargo-deb, cargo-generate-rpm, appimagetool
cargo install cargo-deb cargo-generate-rpm
./scripts/package-linux.sh  # or ./scripts/package-linux.sh --skip-appimage
```

---

## Channel manifest

A `channel-manifest.json` file should be published alongside each release to
support the auto-update checker (JV-20):

```json
{
  "schema_version": "channel-manifest-v1",
  "channels": {
    "stable":  { "version": "0.0.0", "tag": "v0.0.0", "published_at": "2026-01-01T00:00:00Z" },
    "beta":    { "version": "0.0.0", "tag": "v0.0.0-beta.1", "published_at": "2026-01-01T00:00:00Z" },
    "nightly": { "version": "0.0.0", "tag": "v0.0.0-nightly.1", "published_at": "2026-01-01T00:00:00Z" }
  },
  "signatures": {
    "stable": "<sha256-of-stable-manifest-section>"
  }
}
```

The manifest is signed with the same Developer ID and published as a GitHub Release asset.
Linux artifact signing uses GPG detached signatures alongside each artifact.

---

## Evidence paths

| Gate | Evidence path |
|------|--------------|
| macOS codesign | `verification-evidence/rel-01/codesign.log` |
| macOS notarization | `verification-evidence/rel-01/notarize.log` |
| macOS stapler | `verification-evidence/rel-01/stapler.log` |
| macOS spctl | `verification-evidence/rel-01/spctl.log` |
| Linux rollback drill | `verification-evidence/rel-03/rollback-drill.md` |
| Windows artifact SBOM | `verification-evidence/rel-01/sbom-windows.json` |

---

## Blocked gates

| Gate | Blocker | Resolution |
|------|---------|-----------|
| macOS codesign / notarize | Apple Developer Program membership required | Out of scope for CI; run locally when credentials available |
| Windows code signing | EV certificate required | Out of scope; run locally when certificate available |
| L5 human acceptance | Named human reviewers required | Track in issues #115–#122, #366 |
| 8-hour soak | Windows hardware with 8h runtime | Track in QA8-05 #503 |

> **Note**: Packaging scripts and CI workflows are fully implemented. The manual
> signing / notarization gates are documented above with exact commands.
> Run `./scripts/package-macos.sh --notarize` locally when Apple credentials
> are available and commit the evidence files to `verification-evidence/rel-01/`.
