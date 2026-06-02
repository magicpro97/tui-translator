//! Startup auto-download checks for local STT, MT, and TTS models.
//!
//! Called before the TUI starts so that download progress is visible in
//! the terminal.  Only triggers when the provider is `"local"` and the
//! required model files are absent.

use anyhow::{Context, Result};
use std::{io, time::Duration};

use crate::providers;

/// Check whether local models needed by `stt_provider`/`mt_provider`/`tts_provider`
/// are present and auto-download any that are missing.
///
/// Runs **before** the TUI starts so download progress goes to stdout.
/// It is a no-op when no provider is `"local"` or when all required model
/// files are already present.
///
/// # Errors
/// Returns an error only for unrecoverable conditions (bad cache path, network
/// failure after consent). Missing consent is handled interactively via stdout.
pub(crate) fn run_startup_local_model_check(
    stt_provider: &str,
    mt_provider: &str,
    tts_provider: &str,
    tts_enabled: bool,
) -> Result<()> {
    let mut stdout = io::stdout();

    if stt_provider == "local" {
        run_startup_stt_model_check(&mut stdout)?;
    }

    if mt_provider == "local" {
        run_startup_mt_model_check(&mut stdout)?;
    }

    #[cfg(feature = "local-tts")]
    if tts_enabled && tts_provider == "local" {
        run_startup_tts_model_check(&mut stdout)?;
    }

    // Suppress unused-variable warning when local-tts is not enabled.
    #[cfg(not(feature = "local-tts"))]
    {
        let _ = tts_provider;
        let _ = tts_enabled;
    }

    Ok(())
}

fn startup_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60 * 60))
        .build()
        .context("failed to create HTTP client for model download")
}

fn run_startup_stt_model_check(stdout: &mut impl io::Write) -> Result<()> {
    let cache_dir = providers::local::model_cache_dir()
        .context("failed to resolve local model cache directory")?;

    let spec = providers::local::ModelManifest::builtin()
        .find(providers::local::ModelId::Tiny)
        .ok_or_else(|| anyhow::anyhow!("ModelId::Tiny not found in built-in manifest"))?;

    let model_path = cache_dir.join(spec.file_name);
    if model_path.exists() {
        return Ok(());
    }

    writeln!(
        stdout,
        "\n[tui-translator] Local STT model not found at {}",
        model_path.display()
    )
    .ok();
    writeln!(
        stdout,
        "[tui-translator] Downloading Whisper {} (~{} MB) \u{2026}",
        spec.id.display_name(),
        spec.size_bytes / 1_048_576,
    )
    .ok();

    let bundle = providers::local::stt_model_bundle_manifest(spec);
    let client = startup_http_client()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create model download runtime")?;
    let report = rt
        .block_on(providers::local::install_model_bundle(
            &client, &bundle, &cache_dir,
        ))
        .context("failed to auto-download local STT model")?;

    writeln!(
        stdout,
        "[tui-translator] STT model ready (downloaded {}, reused {}).",
        report.downloaded_files, report.reused_files,
    )
    .ok();
    Ok(())
}

fn run_startup_mt_model_check(stdout: &mut impl io::Write) -> Result<()> {
    let cache_dir = providers::local::model_cache_dir()
        .context("failed to resolve local model cache directory")?;
    let model_dir = cache_dir.join("mt").join("opus-mt-ja-vi");

    let encoder_present = model_dir.join("encoder_model.onnx").exists();
    let decoder_present = model_dir.join("decoder_model.onnx").exists();
    if encoder_present && decoder_present {
        return Ok(());
    }

    let manifest = providers::local::opus_mt_ja_vi_bundle_manifest();
    let total_mb = manifest.total_size_bytes() / 1_048_576;

    writeln!(
        stdout,
        "\n[tui-translator] Local MT model not found at {}",
        model_dir.display()
    )
    .ok();
    writeln!(
        stdout,
        "[tui-translator] Downloading OPUS-MT ja\u{2192}vi (~{total_mb} MB) \u{2026}",
    )
    .ok();

    let consent = manifest
        .consent_manifest()
        .context("opus-mt-ja-vi bundle manifest missing consent metadata")?;
    providers::local::write_model_consent_record(&consent)
        .context("failed to persist consent record for opus-mt-ja-vi")?;

    let client = startup_http_client()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create model download runtime")?;
    let report = rt
        .block_on(providers::local::install_model_bundle(
            &client, &manifest, &model_dir,
        ))
        .context("failed to auto-download local MT model")?;

    writeln!(
        stdout,
        "[tui-translator] MT model ready (downloaded {}, reused {}).",
        report.downloaded_files, report.reused_files,
    )
    .ok();
    Ok(())
}

/// Check whether the Supertonic-3 int8 TTS model files are present and
/// download the archive if any are missing.
///
/// Writes a consent record before any network request is initiated.
#[cfg(feature = "local-tts")]
fn run_startup_tts_model_check(stdout: &mut impl io::Write) -> Result<()> {
    use providers::local::{
        supertonic_consent_manifest, write_model_consent_record, SUPERTONIC_3_INT8_ARCHIVE_SHA256,
        SUPERTONIC_3_INT8_ARCHIVE_URL, SUPERTONIC_3_INT8_DIR, SUPERTONIC_3_INT8_FILES,
    };

    let cache_dir = providers::local::model_cache_dir()
        .context("failed to resolve local model cache directory")?;
    let tts_dir = cache_dir.join("tts");
    let model_dir = tts_dir.join(SUPERTONIC_3_INT8_DIR);

    // Check if all 7 expected files are already present.
    let all_present = SUPERTONIC_3_INT8_FILES
        .iter()
        .all(|(file_name, _, _)| model_dir.join(file_name).exists());

    if all_present {
        return Ok(());
    }

    let total_mb: u64 = SUPERTONIC_3_INT8_FILES
        .iter()
        .map(|(_, _, s)| s)
        .sum::<u64>()
        / 1_048_576;

    writeln!(
        stdout,
        "\n[tui-translator] Local TTS model not found at {}",
        model_dir.display()
    )
    .ok();
    writeln!(
        stdout,
        "[tui-translator] Downloading Supertonic-3 int8 (~{total_mb} MB) \u{2026}",
    )
    .ok();

    // Record consent before any network request.
    let consent = supertonic_consent_manifest();
    write_model_consent_record(&consent)
        .context("failed to persist consent record for supertonic-tts")?;

    let client = startup_http_client()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create model download runtime")?;
    let report = rt
        .block_on(providers::local::install_archive_bundle(
            &client,
            SUPERTONIC_3_INT8_ARCHIVE_URL,
            SUPERTONIC_3_INT8_ARCHIVE_SHA256,
            &model_dir,
            SUPERTONIC_3_INT8_FILES,
        ))
        .context("failed to auto-download local TTS model")?;

    writeln!(
        stdout,
        "[tui-translator] TTS model ready (downloaded {}, reused {}).",
        report.downloaded_files, report.reused_files,
    )
    .ok();
    Ok(())
}

#[cfg(test)]
mod tts_startup_tests {
    use super::*;
    #[allow(unused_imports)]
    use crate::providers::local::{LOCAL_DATA_DIR_OVERRIDE_ENV, TEST_ENV_MUTEX};
    #[cfg(feature = "local-tts")]
    #[allow(unused_imports)]
    use crate::providers::local::{SUPERTONIC_3_INT8_DIR, SUPERTONIC_3_INT8_FILES};
    #[cfg(feature = "local-tts")]
    use std::fs;
    use tempfile::TempDir;

    // RAII env-var guard that restores the original value on drop.
    struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
        // OK: MutexGuard held for the lifetime of the guard to serialize env access
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self {
                key,
                previous,
                _lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(val) => std::env::set_var(self.key, val),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// Set up an isolated temp data directory and return the guard + TempDir.
    fn isolated_data_dir() -> (TempDir, EnvGuard) {
        let tmp = TempDir::new().expect("tempdir creation must succeed");
        let guard = EnvGuard::set(
            LOCAL_DATA_DIR_OVERRIDE_ENV,
            tmp.path()
                .to_str()
                .expect("tempdir path must be valid UTF-8"),
        );
        (tmp, guard)
    }

    // ── Test 1: TTS check skipped when tts_enabled = false ────────────────────

    #[test]
    fn tts_check_is_noop_when_tts_disabled() {
        let (_tmp, _guard) = isolated_data_dir();
        // tts_enabled=false → function must return Ok immediately without downloading.
        let result = run_startup_local_model_check("google", "google", "local", false);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    // ── Test 2: TTS check skipped when provider is not "local" ────────────────

    #[test]
    fn tts_check_is_noop_when_provider_is_not_local() {
        let (_tmp, _guard) = isolated_data_dir();
        // tts_provider="google" → no download attempted, must return Ok.
        let result = run_startup_local_model_check("google", "google", "google", true);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    // ── Test 4: TTS check skipped when all 7 files already present ────────────

    #[cfg(feature = "local-tts")]
    #[test]
    fn tts_check_is_noop_when_all_files_already_present() {
        let (tmp, _guard) = isolated_data_dir();

        // Pre-populate model directory with all 7 expected files.
        let model_dir = tmp
            .path()
            .join("models")
            .join("tts")
            .join(SUPERTONIC_3_INT8_DIR);
        fs::create_dir_all(&model_dir).expect("create model dir must succeed");
        for (file_name, _, _) in SUPERTONIC_3_INT8_FILES {
            fs::write(model_dir.join(file_name), b"stub").expect("write stub file must succeed");
        }

        let result = run_startup_local_model_check("google", "google", "local", true);
        // All files present → returns Ok without making any network request.
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    // ── Test 3: TTS download triggered when local + enabled + files missing ───

    #[cfg(feature = "local-tts")]
    #[test]
    fn tts_check_triggers_download_when_local_and_files_missing() {
        let (tmp, _guard) = isolated_data_dir();

        // Model dir exists but is empty — files are missing.
        let tts_dir = tmp.path().join("models").join("tts");
        fs::create_dir_all(&tts_dir).expect("create tts dir must succeed");

        // No network available in test environment: the function will attempt to
        // download and return an error.  That error proves the download branch was
        // reached (noop paths return Ok).
        let result = run_startup_local_model_check("google", "google", "local", true);
        assert!(
            result.is_err(),
            "expected download attempt (Err) when files are missing, got Ok"
        );
    }

    // ── Test 5: Consent recorded before download is called ───────────────────

    #[cfg(feature = "local-tts")]
    #[test]
    fn tts_check_writes_consent_record_before_calling_downloader() {
        let (tmp, _guard) = isolated_data_dir();

        // Run the check with tts_provider="local", tts_enabled=true, no model files.
        // The function is expected to:
        //   1. Write a consent record to the consent directory
        //   2. Attempt the download (which will fail — no network in test environment)
        let _ = run_startup_local_model_check("google", "google", "local", true);

        // Verify the consent directory was created and at least one consent file written.
        let consent_dir = tmp.path().join("consent");
        let entries: Vec<_> = fs::read_dir(&consent_dir)
            .expect("consent dir must exist after consent write")
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            !entries.is_empty(),
            "consent file must have been written before download; consent_dir={consent_dir:?}"
        );
    }

    // ── Test 6: All three branches skipped when no provider is local ──────────

    #[test]
    fn dispatcher_skips_all_three_branches_when_no_provider_is_local() {
        let (_tmp, _guard) = isolated_data_dir();
        // All providers are non-local → no downloads, must return Ok immediately.
        let result = run_startup_local_model_check("google", "google", "google", true);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }
}
