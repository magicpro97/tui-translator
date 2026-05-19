# VMIC-B5 production readiness report

Issue: #325  
Decision: **GO for production release checkpoint** on the OEM/commercial virtual cable path.  
Baseline production commit: `2161ac64a30eaa86afe10bbe45071a2d197b08e0` (`main`, after VMIC-B4).  
Automation rule: no Zoom, Teams, live meetings, manual audio checks, or human acceptance is required for this checkpoint.

## Summary

All VMIC-B production child issues are closed, all required VMIC-B evidence artifacts are present, and the selected production path is ready for release-gate validation through automated build, round-trip, packaging, smoke, and soak jobs.

No manual Zoom/Teams/human acceptance remains in the required path. Meeting-app setup remains documented operator guidance only; required readiness is proven by deterministic evidence artifacts, test doubles, Windows CI jobs, release executable smoke commands, and explicit unsupported-runner skip reasons where real cables are unavailable.

## Production child issue closure and evidence

| Work item | GitHub issue | Issue state | Evidence | Evidence status |
|-----------|--------------|-------------|----------|-----------------|
| VMIC-B1 PCM negotiation | #321 | closed | `verification-evidence/vmic/VMIC-B1-format-negotiation.json` | pass |
| VMIC-B2 OEM/commercial registry | #322 | closed | `verification-evidence/vmic/VMIC-B2-oem-registry.json` | pass |
| VMIC-B3 production path decision | #323 | closed | `verification-evidence/vmic/VMIC-B3-production-path-decision.md` | pass |
| VMIC-B4 production sink round-trip | #324 | closed | `verification-evidence/vmic/VMIC-B4-production-sink-roundtrip.json` | pass |

All VMIC-B production child issues are closed or represented by this final checkpoint; no child issue is intentionally deferred as a blocking release item.

## Supported production path and limitations

The supported production path is **OEM/commercial virtual cable**:

1. TUI Translator writes translated TTS audio through `OemCableSink` into a Windows render endpoint.
2. The endpoint is discovered or overridden through the VMIC-B2 `virtual_device_patterns` registry.
3. The paired recording endpoint is selected by the meeting app outside the automated gate.
4. The app executable is an unsigned application executable unless a release pipeline signs it; no kernel-mode audio driver is bundled.
5. Core code does not bundle, install, license, or redistribute any VB-CABLE, VAC, Voicemeeter, OEM, or custom vendor driver binary.

Explicit limitations:

- TUI Translator does not create a Windows microphone endpoint by itself.
- A pure app/user-mode microphone endpoint remains unsupported without official Microsoft documentation and automated endpoint-enumeration proof.
- A project-owned SysVAD/WaveRT driver remains **NO-GO** for this app repository until a separate WDK, signing, installer, rollback, HLK, and support plan exists.
- Zoom and Teams behavior is not a required acceptance step; setup guidance is documented, but no manual Zoom/Teams test is required to pass this checkpoint.

## Automated checkpoint commands

| Gate | Command or evidence source | Result |
|------|----------------------------|--------|
| Production evidence aggregation | `pwsh -NoProfile -File scripts/check-vmic-production-evidence.ps1 -Json` | pass |
| Rustfmt | `cargo +stable-x86_64-pc-windows-gnu fmt --check` | pass |
| B5 readiness test | `cargo +stable-x86_64-pc-windows-gnu test --test vmic_b5_production_readiness --quiet` | pass |
| B4 production round-trip | `cargo +stable-x86_64-pc-windows-gnu test --features production-audio production_sink_roundtrip --quiet` | pass |
| Full default test suite | `cargo +stable-x86_64-pc-windows-gnu test --all-targets --quiet` | pass |
| Clippy default local gate | `cargo +stable-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings` | pass |
| Clippy all-features CI gate | GitHub Actions `Lint (clippy)` uses `cargo clippy --all-targets --all-features -- -D warnings` | required green before merge |
| All-features test suite | GitHub Actions `Lint (clippy)` also runs `cargo test --all-features -- --nocapture --skip real_api` | required green before merge |
| Windows release build | `cargo +stable-x86_64-pc-windows-gnu build --release --bin tui-translator --quiet` | pass |
| Production release artifact gate | GitHub Actions `VMIC-B5 production readiness` writes `verification-evidence\vmic\VMIC-B5-release-sha256.txt` | required green before merge |
| Release smoke log | GitHub Actions `VMIC-B5 production readiness` writes `verification-evidence\vmic\VMIC-B5-smoke-log.txt` from `--list-audio-devices` and `--list-capture-devices` | required green before merge |
| Packaging CI | GitHub Actions `Packaging verification (MSVC static exe)` | required green before merge |
| Soak CI | GitHub Actions `Soak runner dry-run` and `Soak fixture validation` | required green before merge |
| VMIC-B4 CI | GitHub Actions `VMIC-B4 production sink round-trip` | required green before merge |

## Release artifact

| Artifact | Value |
|----------|-------|
| Local path | `target\release\tui-translator.exe` |
| Build command | `cargo +stable-x86_64-pc-windows-gnu build --release --bin tui-translator --quiet` |
| Artifact signing label | unsigned application executable |
| Driver bundling label | no virtual-cable or kernel driver binary bundled |
| CI hash artifact | `verification-evidence\vmic\VMIC-B5-release-sha256.txt` |
| CI smoke log | `verification-evidence\vmic\VMIC-B5-smoke-log.txt` |
| Smoke commands | `--list-audio-devices` and `--list-capture-devices` |

The CI-built release executable is produced from the PR merge commit on `main`. The hash artifact records `sha256=...`, `bytes=...`, `unsigned=true`, and `driver_bundled=false` so downstream packaging cannot confuse app release validation with driver signing or redistribution.

## Board-ready go/no-go summary

**GO for production release checkpoint:** the virtual microphone production path is limited to an OEM/commercial virtual cable, all production child evidence is present, automated round-trip and soak gates are required in CI, and release smoke commands are automated.

**NO-GO for bundled/custom driver release:** no project-owned driver, vendor driver, installer, or driver signing workflow is included in this app release path.
