use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_file(relative_path: &str) -> String {
    fs::read_to_string(repo_root().join(relative_path))
        .unwrap_or_else(|err| panic!("failed to read {relative_path}: {err}"))
}

fn assert_contains(file_name: &str, contents: &str, needle: &str) {
    assert!(
        contents.contains(needle),
        "{file_name} must contain {needle:?}"
    );
}

fn assert_evidence_file(relative_path: &str, required_terms: &[&str]) {
    let contents = read_file(relative_path);
    for term in required_terms {
        assert_contains(relative_path, &contents, term);
    }
}

#[test]
fn vmic_mvp_child_evidence_is_complete() {
    for (path, terms) in [
        (
            "verification-evidence/vmic/VMIC-00-audio-sink-report.json",
            vec!["\"status\": \"pass\"", "\"issue\": 312"],
        ),
        (
            "verification-evidence/vmic/VMIC-A1-device-probe.json",
            vec!["\"status\": \"pass\"", "\"issue\": 313"],
        ),
        (
            "verification-evidence/vmic/VMIC-A2-config-schema.json",
            vec!["\"status\": \"pass\"", "\"issue\": 314"],
        ),
        (
            "verification-evidence/vmic/VMIC-A3-routing-report.json",
            vec!["\"status\": \"pass\"", "\"issue\": 315"],
        ),
        (
            "verification-evidence/vmic/VMIC-A4-settings-pty.json",
            vec!["\"status\": \"pass\"", "\"issue\": 316"],
        ),
        (
            "verification-evidence/vmic/VMIC-A5-status-render.json",
            vec!["\"status\": \"pass\"", "\"issue\": 317"],
        ),
        (
            "verification-evidence/vmic/VMIC-A6-vbcable-ci-report.json",
            vec![
                "\"status\": \"pass\"",
                "\"memory_pcm\"",
                "\"real_virtual_cable\"",
            ],
        ),
        (
            "verification-evidence/vmic/VMIC-A7-docs-check.json",
            vec![
                "\"status\": \"pass\"",
                "\"issue\": \"#319\"",
                "\"T1\"",
                "\"T2\"",
                "\"T3\"",
            ],
        ),
    ] {
        assert_evidence_file(path, &terms);
    }
}

#[test]
fn vmic_mvp_architecture_seams_are_present() {
    let audio_sink = read_file("src/pipeline/audio_sink.rs");
    assert_contains(
        "src/pipeline/audio_sink.rs",
        &audio_sink,
        "pub trait AudioSink",
    );
    assert_contains(
        "src/pipeline/audio_sink.rs",
        &audio_sink,
        "pub struct MockAudioSink",
    );

    let config = read_file("src/config/mod.rs");
    for term in [
        "pub enum TtsRouting",
        "Speakers",
        "VirtualMic",
        "Both",
        "pub virtual_mic_device",
    ] {
        assert_contains("src/config/mod.rs", &config, term);
    }

    let playback = read_file("src/pipeline/playback.rs");
    for term in [
        "pub enum PlaybackSinkTarget",
        "pub struct PlaybackRoutePlan",
        "PlaybackSinkTarget::VirtualMic",
        "fn play_to_audio_sinks",
        "Self::both",
    ] {
        assert_contains("src/pipeline/playback.rs", &playback, term);
    }

    let virtual_device = read_file("src/audio/virtual_device.rs");
    for term in [
        "probe_virtual_audio_devices",
        "VirtualDeviceKind::VbCable",
        "VirtualDeviceKind::Vac",
        "VirtualDeviceKind::Voicemeeter",
    ] {
        assert_contains("src/audio/virtual_device.rs", &virtual_device, term);
    }

    let vbcable_ci = read_file("src/audio/vbcable_ci.rs");
    for term in ["run_memory_pcm_tier", "TierEvidence", "latency"] {
        assert_contains("src/audio/vbcable_ci.rs", &vbcable_ci, term);
    }

    let vbcable_probe = read_file("src/bin/vbcable_ci_probe.rs");
    for term in ["real_virtual_cable", "run_real_virtual_cable_tier"] {
        assert_contains("src/bin/vbcable_ci_probe.rs", &vbcable_probe, term);
    }
}

#[test]
fn vmic_mvp_readiness_report_records_go_decision() {
    let report_path = "verification-evidence/vmic/VMIC-A8-mvp-readiness-report.md";
    let report = read_file(report_path);

    for term in [
        "GO for MVP release",
        "GO for production phase",
        "All MVP child issues are closed",
        "No human acceptance step is required",
        "OEM/DriverSink",
        "without modifying STT/MT/TTS orchestration",
        "scripts/check-vmic-mvp-evidence.ps1",
        "cargo test --all-features",
        "Local verification SHA-256",
        "VMIC-A8-release-sha256.txt",
    ] {
        assert_contains(report_path, &report, term);
    }
}

#[test]
fn vmic_mvp_checker_script_is_present() {
    let script_path = "scripts/check-vmic-mvp-evidence.ps1";
    let script = read_file(script_path);
    for term in [
        "VMIC-00-audio-sink-report.json",
        "VMIC-A7-docs-check.json",
        "VMIC-A8-mvp-readiness-report.md",
        "pub trait AudioSink",
        "pub enum TtsRouting",
    ] {
        assert_contains(script_path, &script, term);
    }
}
