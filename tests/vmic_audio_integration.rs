//! VMIC-A6 feature-gated virtual-cable integration tests.
//!
//! These tests intentionally pass on hosted runners without VB-CABLE/VAC by
//! requiring explicit skip reasons in the JSON evidence.  If a virtual endpoint
//! is detected, the probe attempts the real render-to-loopback tier and fails on
//! bad RMS/latency evidence.

#[cfg(feature = "audio-integration")]
#[path = "../src/audio/mod.rs"]
mod audio;

#[cfg(feature = "audio-integration")]
mod audio_integration {
    use std::path::PathBuf;
    use std::process::Command;

    use super::audio;

    fn probe_bin() -> PathBuf {
        if let Ok(path) = std::env::var("CARGO_BIN_EXE_vbcable_ci_probe") {
            return PathBuf::from(path);
        }
        #[cfg(windows)]
        let suffix = ".exe";
        #[cfg(not(windows))]
        let suffix = "";
        PathBuf::from(format!("target/debug/vbcable_ci_probe{suffix}"))
    }

    fn test_report_path(file_name: &str) -> String {
        let mut dir =
            PathBuf::from(std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into()));
        dir.push("vmic-a6-test");
        std::fs::create_dir_all(&dir).expect("failed to create VMIC-A6 test output dir");
        dir.push(file_name);
        dir.to_string_lossy().into_owned()
    }

    fn run_probe(output_path: &str, extra_args: &[&str]) -> serde_json::Value {
        let _ = std::fs::remove_file(output_path);
        let mut command = Command::new(probe_bin());
        command.args(["--out", output_path, "--duration-ms", "250"]);
        command.args(extra_args);
        let status = command.status().expect("failed to spawn vbcable_ci_probe");
        assert!(
            status.success(),
            "vbcable_ci_probe must exit 0 on pass or skip-safe unsupported runner; got {status}"
        );
        let raw = std::fs::read_to_string(output_path).expect("VMIC-A6 report not written");
        serde_json::from_str(&raw).expect("VMIC-A6 report is not valid JSON")
    }

    #[test]
    fn virtual_cable_presence_probe() {
        match audio::probe_virtual_audio_devices() {
            Ok(devices) if devices.is_empty() => {
                println!("VMIC-A6 skip-safe: no VB-CABLE/VAC/Voicemeeter endpoint detected");
            }
            Ok(devices) => {
                for device in devices {
                    assert!(
                        !device.name.is_empty(),
                        "virtual device name must not be empty"
                    );
                    assert!(!device.id.is_empty(), "virtual device id must not be empty");
                }
            }
            Err(err) => {
                let message = format!("{err:#}");
                println!("VMIC-A6 skip-safe: virtual-cable probe unavailable: {message}");
                assert!(
                    !message.trim().is_empty(),
                    "probe errors must provide a deterministic skip reason"
                );
            }
        }
    }

    #[test]
    fn virtual_cable_write_tone() {
        let output_path = test_report_path("VMIC-A6-write-tone-test.json");
        let report = run_probe(&output_path, &["--memory-only"]);

        assert_eq!(report["schema_version"], 1);
        assert_eq!(report["issue"], "#318");
        assert_eq!(report["status"], "pass");
        assert!(
            report["generated_audio"]["rms"].as_f64().unwrap_or(0.0) > 0.30,
            "generated tone RMS must be non-silent"
        );
        let tiers = report["tiers"].as_array().expect("tiers must be an array");
        let memory = tiers
            .iter()
            .find(|tier| tier["name"] == "memory_pcm")
            .expect("memory_pcm tier missing");
        assert_eq!(memory["status"], "pass");
        assert!(
            memory["write"]["sample_count"].as_u64().unwrap_or(0) > 0,
            "memory tier must write PCM samples"
        );
    }

    #[test]
    fn virtual_cable_roundtrip_rms() {
        let output_path = test_report_path("VMIC-A6-vbcable-ci-test.json");
        let report = run_probe(&output_path, &[]);

        assert_eq!(report["status"], "pass");
        let tiers = report["tiers"].as_array().expect("tiers must be an array");
        let real = tiers
            .iter()
            .find(|tier| tier["name"] == "real_virtual_cable")
            .expect("real_virtual_cable tier missing");
        match real["status"].as_str().unwrap_or("") {
            "pass" => {
                assert!(
                    real["capture"]["rms"].as_f64().unwrap_or(0.0)
                        >= report["min_expected_rms"].as_f64().unwrap_or(1.0),
                    "real virtual-cable capture RMS must meet threshold"
                );
                assert!(
                    real["latency"]["p95_ms"].is_number(),
                    "real tier must include p95 latency"
                );
            }
            "skipped" => {
                assert!(
                    real["skip_reason"]
                        .as_str()
                        .unwrap_or("")
                        .contains("VB-CABLE")
                        || real["skip_reason"].as_str().unwrap_or("").contains("probe")
                        || real["skip_reason"]
                            .as_str()
                            .unwrap_or("")
                            .contains("Windows"),
                    "skip must explain why the real cable tier was unavailable"
                );
            }
            other => panic!("real virtual-cable tier must pass or skip, got {other}"),
        }
    }
}

#[cfg(not(feature = "audio-integration"))]
#[test]
fn audio_integration_feature_disabled() {
    println!("VMIC-A6 tests are gated behind --features audio-integration");
}
