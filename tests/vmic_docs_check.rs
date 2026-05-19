use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_doc(relative_path: &str) -> String {
    fs::read_to_string(repo_root().join(relative_path))
        .unwrap_or_else(|err| panic!("failed to read {relative_path}: {err}"))
}

fn assert_contains(doc_name: &str, contents: &str, needle: &str) {
    assert!(
        contents.contains(needle),
        "{doc_name} must contain {needle:?}"
    );
}

fn local_markdown_links(contents: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = contents[cursor..].find("](") {
        let target_start = cursor + offset + 2;
        let Some(close_offset) = contents[target_start..].find(')') else {
            break;
        };
        let raw = contents[target_start..target_start + close_offset].trim();
        let target = raw
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches('<')
            .trim_matches('>');
        if !target.is_empty()
            && !target.starts_with("http://")
            && !target.starts_with("https://")
            && !target.starts_with("mailto:")
            && !target.starts_with('#')
        {
            links.push(target.to_string());
        }
        cursor = target_start + close_offset + 1;
    }
    links
}

fn assert_local_links_exist(source_relative_path: &str, contents: &str) {
    let source = repo_root().join(source_relative_path);
    let base = source.parent().unwrap_or_else(|| Path::new(""));
    for link in local_markdown_links(contents) {
        let path_part = link.split('#').next().unwrap_or("");
        if path_part.is_empty() {
            continue;
        }
        let resolved = base.join(path_part);
        assert!(
            resolved.exists(),
            "local link {link:?} in {source_relative_path} must resolve to {}",
            resolved.display()
        );
    }
}

#[test]
fn vmic_a7_documentation_contract_is_present() {
    let guide = read_doc("docs/12-virtual-mic-setup.md");
    let usage = read_doc("USAGE.md");
    let privacy = read_doc("PRIVACY.md");
    let readme = read_doc("README.md");
    let evidence = read_doc("verification-evidence/vmic/VMIC-A7-docs-check.json");

    for needle in [
        "Speakers",
        "VirtualMic",
        "Both",
        "tts_routing: \"speakers\"",
        "tts_routing: \"virtual_mic\"",
        "tts_routing: \"both\"",
    ] {
        assert_contains("docs/12-virtual-mic-setup.md", &guide, needle);
    }

    for needle in [
        "VB-CABLE/VAC MVP vs production driver distinction",
        "project-owned signed virtual microphone driver",
        "Zoom Original Sound",
        "Teams Noise Suppression",
        "AI-generated translated voice",
        "consent",
        "inaccurate or delayed",
        "verification-evidence/vmic/VMIC-A6-vbcable-ci-report.json",
        "verification-evidence/vmic/VMIC-A7-docs-check.json",
        "tests/vmic_docs_check.rs",
        "does not claim that Zoom or Teams were manually tested",
    ] {
        assert_contains("docs/12-virtual-mic-setup.md", &guide, needle);
    }

    for link in [
        "[USAGE.md](../USAGE.md)",
        "[PRIVACY.md](../PRIVACY.md)",
        "[config.example.json](../config.example.json)",
    ] {
        assert_contains("docs/12-virtual-mic-setup.md", &guide, link);
    }

    assert_contains(
        "USAGE.md",
        &usage,
        "route translated speech into Zoom or Teams",
    );
    assert_contains("USAGE.md", &usage, "AI-generated translated voice");
    assert_contains("USAGE.md", &usage, "docs/12-virtual-mic-setup.md");
    assert_contains("PRIVACY.md", &privacy, "AI-generated translated voice");
    assert_contains("PRIVACY.md", &privacy, "tts_routing");
    assert_contains("README.md", &readme, "docs/12-virtual-mic-setup.md");

    assert_contains("VMIC-A7 evidence", &evidence, "\"issue\": \"#319\"");
    assert_contains("VMIC-A7 evidence", &evidence, "\"status\": \"pass\"");
    assert_contains("VMIC-A7 evidence", &evidence, "\"T1\"");
    assert_contains("VMIC-A7 evidence", &evidence, "\"T2\"");
    assert_contains("VMIC-A7 evidence", &evidence, "\"T3\"");
    assert_contains(
        "VMIC-A7 evidence",
        &evidence,
        "\"project-owned signed virtual microphone driver\"",
    );
    assert_contains(
        "VMIC-A7 evidence",
        &evidence,
        "\"tests/vmic_docs_check.rs\"",
    );
    assert_contains("VMIC-A7 evidence", &evidence, "\"../USAGE.md\"");
    assert_contains("VMIC-A7 evidence", &evidence, "\"../PRIVACY.md\"");
    assert_contains("VMIC-A7 evidence", &evidence, "\"../config.example.json\"");
}

#[test]
fn vmic_a7_new_documentation_links_resolve() {
    let guide = read_doc("docs/12-virtual-mic-setup.md");
    assert_local_links_exist("docs/12-virtual-mic-setup.md", &guide);
}
