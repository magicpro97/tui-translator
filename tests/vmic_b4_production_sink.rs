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

#[test]
fn vmic_b4_production_sink_contract_is_present() {
    let sink = read_file("src/pipeline/audio_sink.rs");

    for needle in [
        "pub struct OemCableSink",
        "impl AudioSink for OemCableSink",
        "pub fn try_play_bytes",
        "pub enum ProductionSinkError",
        "pub fn run_memory_production_sink_roundtrip",
        "audio_sink_contract_oem_cable_sink_writes_pcm",
        "production_sink_roundtrip_memory_passes_latency_rms_gate",
        "DEFAULT_PRODUCTION_SINK_P95_GATE_MS",
    ] {
        assert_contains("src/pipeline/audio_sink.rs", &sink, needle);
    }
}

#[test]
fn vmic_b4_evidence_records_roundtrip_and_failure_contract() {
    let evidence = read_file("verification-evidence/vmic/VMIC-B4-production-sink-roundtrip.json");

    for needle in [
        "\"issue\": \"#324\"",
        "\"status\": \"pass\"",
        "\"selected_path\": \"oem_commercial_virtual_cable\"",
        "\"sink\": \"OemCableSink\"",
        "\"tier\": \"memory_pcm_roundtrip\"",
        "\"p95_gate_ms\": 10.0",
        "\"dropped_frames\": 0",
        "\"human_acceptance_required\": false",
        "\"zoom_or_teams_required\": false",
        "audio_sink_contract_oem_cable_sink_writes_pcm",
        "production_sink_roundtrip_memory_passes_latency_rms_gate",
        "endpoint write failure is logged by AudioSink and returned by try_play_bytes",
    ] {
        assert_contains("VMIC-B4 evidence", &evidence, needle);
    }
}

#[test]
fn vmic_b4_ci_gate_is_registered() {
    let workflow = read_file(".github/workflows/ci.yml");

    for needle in [
        "VMIC-B4 production sink round-trip",
        "cargo test --features production-audio production_sink_roundtrip -- --nocapture",
        "VMIC-B4-production-sink-roundtrip.json",
    ] {
        assert_contains(".github/workflows/ci.yml", &workflow, needle);
    }
}
