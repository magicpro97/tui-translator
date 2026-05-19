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
fn vmic_b_production_child_evidence_is_complete() {
    for (path, terms) in [
        (
            "verification-evidence/vmic/VMIC-B1-format-negotiation.json",
            vec!["\"issue\": \"#321\"", "\"status\": \"pass\""],
        ),
        (
            "verification-evidence/vmic/VMIC-B2-oem-registry.json",
            vec!["\"issue\": 322", "\"status\": \"pass\""],
        ),
        (
            "verification-evidence/vmic/VMIC-B3-production-path-decision.md",
            vec![
                "OEM/commercial cable production implementation in VMIC-B4",
                "NO-GO** for project-owned custom driver implementation",
            ],
        ),
        (
            "verification-evidence/vmic/VMIC-B4-production-sink-roundtrip.json",
            vec![
                "\"issue\": \"#324\"",
                "\"status\": \"pass\"",
                "\"sink\": \"OemCableSink\"",
                "\"human_acceptance_required\": false",
            ],
        ),
    ] {
        assert_evidence_file(path, &terms);
    }
}

#[test]
fn vmic_b5_readiness_report_records_release_gate() {
    let report_path = "verification-evidence/vmic/VMIC-B5-production-readiness-report.md";
    let report = read_file(report_path);

    for term in [
        "GO for production release checkpoint",
        "All VMIC-B production child issues are closed",
        "No manual Zoom/Teams/human acceptance remains in the required path",
        "OEM/commercial virtual cable",
        "unsigned application executable",
        "driver_bundled=false",
        "VMIC-B5-release-sha256.txt",
        "VMIC-B5-smoke-log.txt",
        "scripts/check-vmic-production-evidence.ps1",
        "test --features production-audio production_sink_roundtrip",
        "Soak runner dry-run",
        "VMIC-B4 production sink round-trip",
    ] {
        assert_contains(report_path, &report, term);
    }
}

#[test]
fn vmic_b5_checker_script_and_ci_gate_are_present() {
    let script_path = "scripts/check-vmic-production-evidence.ps1";
    let script = read_file(script_path);
    for term in [
        "VMIC-B1-format-negotiation.json",
        "VMIC-B4-production-sink-roundtrip.json",
        "VMIC-B5-production-readiness-report.md",
        "ReleaseHashPath",
        "SmokeLogPath",
        "unsigned=true",
        "driver_bundled=false",
    ] {
        assert_contains(script_path, &script, term);
    }

    let workflow = read_file(".github/workflows/ci.yml");
    for term in [
        "VMIC-B5 production readiness",
        "scripts\\check-vmic-production-evidence.ps1",
        "VMIC-B5-release-sha256.txt",
        "VMIC-B5-smoke-log.txt",
        "unsigned=true",
        "driver_bundled=false",
    ] {
        assert_contains(".github/workflows/ci.yml", &workflow, term);
    }
}

#[test]
fn vmic_b5_docs_list_supported_path_and_limitations() {
    let guide = read_file("docs/12-virtual-mic-setup.md");

    for term in [
        "Supported production path and limitations",
        "OEM/commercial virtual cable",
        "does not create a Windows microphone endpoint by itself",
        "unsigned application executable",
        "pipeline signs it",
        "No manual Zoom or Teams acceptance is required",
        "VMIC-B5-production-readiness-report.md",
    ] {
        assert_contains("docs/12-virtual-mic-setup.md", &guide, term);
    }
}
