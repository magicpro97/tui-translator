use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_decision() -> String {
    fs::read_to_string(
        repo_root().join("verification-evidence/vmic/VMIC-B3-production-path-decision.md"),
    )
    .expect("VMIC-B3 decision artifact must be readable")
}

fn assert_contains(contents: &str, needle: &str) {
    assert!(
        contents.contains(needle),
        "decision must contain {needle:?}"
    );
}

#[test]
fn vmic_b3_decision_artifact_exists_with_required_sections() {
    let decision = read_decision();

    for heading in [
        "# VMIC-B3 production virtual mic path decision",
        "## Decision",
        "## Evidence and citations",
        "## Rejected unsupported path",
        "## Risk matrix",
        "## Prototype plan for selected path",
        "## Follow-up issue split",
        "## Go / no-go",
    ] {
        assert_contains(&decision, heading);
    }
}

#[test]
fn vmic_b3_decision_rejects_unsupported_user_mode_claims() {
    let decision = read_decision();

    for needle in [
        "No pure user-mode microphone endpoint path is accepted",
        "unsupported until backed by official Microsoft documentation",
        "endpoint-enumeration proof",
        "https://learn.microsoft.com/en-us/windows/win32/coreaudio/audio-endpoint-devices",
    ] {
        assert_contains(&decision, needle);
    }
}

#[test]
fn vmic_b3_decision_covers_signing_installer_ci_and_rollback() {
    let decision = read_decision();

    for needle in [
        "WDK",
        "HLK",
        "Partner Center",
        "signing",
        "installer",
        "rollback",
        "CI automation risk",
        "support ownership",
        "https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/driver-signing-offerings",
    ] {
        assert_contains(&decision, needle);
    }
}

#[test]
fn vmic_b3_selects_oem_path_and_lists_follow_up_issues() {
    let decision = read_decision();

    for needle in [
        "Choose the **OEM/commercial virtual cable path**",
        "**GO** for OEM/commercial cable production implementation in VMIC-B4",
        "Implement `OemCableSink` behind `AudioSink`",
        "skip-safe real-cable CI probe",
        "Create a WDK SysVAD/WaveRT proof-of-concept issue",
    ] {
        assert_contains(&decision, needle);
    }
}
