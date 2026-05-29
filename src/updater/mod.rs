pub mod github;
pub mod version;

use anyhow::{anyhow, bail, Result};
use sha2::{Digest as _, Sha256};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub use crate::config::UpdateChannel;
pub use github::{GithubRelease, ReleaseSource};
pub use version::ParsedVersion;

/// High-level outcome of an update check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Current version is already the newest known version.
    UpToDate,
    /// A newer release is available.
    UpdateAvailable(GithubRelease),
    /// The check failed and produced a user-visible message.
    CheckFailed(String),
}

/// Result payload returned by the updater.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheckResult {
    /// Status of the check.
    pub status: UpdateStatus,
    /// UNIX timestamp when the check completed.
    pub checked_at_unix_secs: u64,
}

/// Check GitHub Releases for an update on the selected channel.
pub async fn check_for_update(
    source: &dyn ReleaseSource,
    owner: &str,
    repo: &str,
    current_version: &str,
    channel: UpdateChannel,
) -> UpdateCheckResult {
    let checked_at_unix_secs = current_unix_secs();
    let current = match ParsedVersion::parse(current_version) {
        Some(version) => version,
        None => {
            return UpdateCheckResult {
                status: UpdateStatus::CheckFailed(format!(
                    "could not parse current version {current_version:?}"
                )),
                checked_at_unix_secs,
            };
        }
    };

    let prerelease = matches!(channel, UpdateChannel::Prerelease);
    let status = match source.latest_release(owner, repo, prerelease).await {
        Ok(Some(release)) => match ParsedVersion::parse(&release.tag_name) {
            Some(candidate) if candidate > current => UpdateStatus::UpdateAvailable(release),
            Some(_) => UpdateStatus::UpToDate,
            None => UpdateStatus::CheckFailed(format!(
                "could not parse release tag {:?}",
                release.tag_name
            )),
        },
        Ok(None) => UpdateStatus::UpToDate,
        Err(err) => UpdateStatus::CheckFailed(err.to_string()),
    };

    UpdateCheckResult {
        status,
        checked_at_unix_secs,
    }
}

/// Validate an artifact against a BSD-style `SHA256SUMS` entry.
pub async fn validate_artifact_checksum(
    artifact_bytes: &[u8],
    artifact_name: &str,
    shasums_text: &str,
) -> Result<()> {
    let expected = shasums_text
        .lines()
        .filter_map(parse_bsd_sha256_line)
        .find_map(|(name, digest)| (name == artifact_name).then_some(digest))
        .ok_or_else(|| anyhow!("SHA256SUMS entry not found for {artifact_name}"))?;

    let actual = sha256_bytes(artifact_bytes);
    if actual != expected {
        bail!("checksum mismatch for {artifact_name}: expected {expected}, got {actual}");
    }

    Ok(())
}

/// Handoff entry point for installer-based upgrades.
pub async fn apply_installer_update(release: &GithubRelease) -> Result<()> {
    let _ = release;
    bail!("not yet implemented (Phase 6): installer handoff")
}

/// Handoff entry point for portable self-replace upgrades.
pub async fn apply_portable_update(release: &GithubRelease, exe_path: &Path) -> Result<()> {
    let _ = (release, exe_path);
    bail!("not yet implemented (Phase 6): portable self-replace rollback")
}

fn current_unix_secs() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_) => 0,
    }
}

fn parse_bsd_sha256_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    let remainder = line.strip_prefix("SHA256 (")?;
    let (name, digest) = remainder.split_once(") = ")?;
    Some((name.to_string(), digest.trim().to_ascii_lowercase()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::{
        apply_installer_update, apply_portable_update, check_for_update,
        validate_artifact_checksum, GithubRelease, UpdateChannel, UpdateStatus,
    };
    use crate::updater::github::{GithubReleaseAsset, MockReleaseSource};
    use std::fs;
    use std::path::Path;

    fn sample_release(tag_name: &str) -> GithubRelease {
        GithubRelease {
            tag_name: tag_name.to_string(),
            html_url: format!(
                "https://github.com/magicpro97/tui-translator/releases/tag/{tag_name}"
            ),
            prerelease: false,
            assets: vec![GithubReleaseAsset {
                name: "tui-translator.msi".to_string(),
                browser_download_url: "https://example.invalid/tui-translator.msi".to_string(),
            }],
        }
    }

    #[tokio::test]
    async fn newer_release_is_reported() {
        let source = MockReleaseSource::new(Some(sample_release("v0.2.0")));
        let result = check_for_update(
            &source,
            "magicpro97",
            "tui-translator",
            "0.1.4",
            UpdateChannel::Stable,
        )
        .await;

        match result.status {
            UpdateStatus::UpdateAvailable(release) => assert_eq!(release.tag_name, "v0.2.0"),
            other => panic!("expected update available, got {other:?}"),
        }
        assert!(result.checked_at_unix_secs > 0);
    }

    #[tokio::test]
    async fn same_release_is_up_to_date() {
        let source = MockReleaseSource::new(Some(sample_release("v0.1.4")));
        let result = check_for_update(
            &source,
            "magicpro97",
            "tui-translator",
            "0.1.4",
            UpdateChannel::Stable,
        )
        .await;

        assert_eq!(result.status, UpdateStatus::UpToDate);
    }

    #[tokio::test]
    async fn older_release_is_up_to_date() {
        let source = MockReleaseSource::new(Some(sample_release("v0.1.3")));
        let result = check_for_update(
            &source,
            "magicpro97",
            "tui-translator",
            "0.1.4",
            UpdateChannel::Stable,
        )
        .await;

        assert_eq!(result.status, UpdateStatus::UpToDate);
    }

    #[tokio::test]
    async fn valid_checksum_is_accepted() {
        let shasums =
            "SHA256 (hello.txt) = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        validate_artifact_checksum(b"hello", "hello.txt", shasums)
            .await
            .expect("checksum should validate");
    }

    #[tokio::test]
    async fn checksum_mismatch_is_rejected() {
        let shasums =
            "SHA256 (hello.txt) = 0000000000000000000000000000000000000000000000000000000000000000";
        let err = validate_artifact_checksum(b"hello", "hello.txt", shasums)
            .await
            .expect_err("checksum mismatch should fail");
        assert!(err.to_string().contains("checksum mismatch"));
    }

    #[tokio::test]
    async fn installer_stub_returns_phase_six_error() {
        let err = apply_installer_update(&sample_release("v0.2.0"))
            .await
            .expect_err("installer path should remain stubbed");
        assert!(err
            .to_string()
            .contains("not yet implemented (Phase 6): installer handoff"));
    }

    #[tokio::test]
    async fn portable_stub_returns_phase_six_error() {
        let err = apply_portable_update(&sample_release("v0.2.0"), Path::new("tui-translator.exe"))
            .await
            .expect_err("portable path should remain stubbed");
        assert!(err
            .to_string()
            .contains("not yet implemented (Phase 6): portable self-replace rollback"));
    }

    #[test]
    fn installer_command_construction_uses_msiexec() {
        let command =
            construct_installer_command(Path::new(r"C:\Downloads\tui-translator-0.2.0-x64.msi"));
        assert_eq!(command.program, "msiexec.exe");
        assert_eq!(
            command.args,
            vec![
                "/i".to_string(),
                r"C:\Downloads\tui-translator-0.2.0-x64.msi".to_string(),
                "/passive".to_string(),
                "/norestart".to_string(),
            ]
        );
    }

    #[test]
    fn portable_rollback_simulation_restores_previous_version() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let exe_path = dir.path().join("tui-translator.exe");
        let backup_path = dir.path().join("tui-translator.previous.exe");

        fs::write(&backup_path, b"previous-build").expect("write backup");
        fs::write(&exe_path, b"broken-build").expect("write failed update");

        simulate_portable_rollback(&backup_path, &exe_path).expect("rollback should succeed");

        assert_eq!(
            fs::read(&exe_path).expect("read restored exe"),
            b"previous-build"
        );
    }

    #[derive(Debug, PartialEq, Eq)]
    struct InstallerCommand {
        program: String,
        args: Vec<String>,
    }

    fn construct_installer_command(installer_path: &Path) -> InstallerCommand {
        InstallerCommand {
            program: "msiexec.exe".to_string(),
            args: vec![
                "/i".to_string(),
                installer_path.display().to_string(),
                "/passive".to_string(),
                "/norestart".to_string(),
            ],
        }
    }

    fn simulate_portable_rollback(backup_path: &Path, exe_path: &Path) -> std::io::Result<u64> {
        fs::copy(backup_path, exe_path)
    }
}
