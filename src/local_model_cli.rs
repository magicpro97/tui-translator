use anyhow::{bail, Context, Result};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write as IoWrite},
    path::{Path, PathBuf},
    time::Duration,
};

use crate::providers;

/// Parsed arguments for `--install-local-mt-model`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalMtModelInstallArgs {
    pub(crate) manifest: PathBuf,
    pub(crate) model_dir: Option<PathBuf>,
    pub(crate) yes: bool,
}

/// Parsed arguments for local STT model prefetch commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalSttModelPrefetchArgs {
    pub(crate) source: LocalSttModelPrefetchSource,
    pub(crate) model_cache_dir: Option<PathBuf>,
    pub(crate) yes: bool,
}

/// Source selector for local STT model prefetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LocalSttModelPrefetchSource {
    BuiltinModel(providers::local::ModelId),
    Manifest(PathBuf),
}

/// Parsed arguments for `--model-verify`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelVerifyArgs {
    model_id: providers::local::ModelId,
    model_cache_dir: Option<PathBuf>,
}

/// Parse local MT model install flags.
pub(crate) fn parse_local_mt_model_install_args_from<I>(
    args: I,
) -> Result<Option<LocalMtModelInstallArgs>>
where
    I: IntoIterator<Item = OsString>,
{
    let mut saw_install_arg = false;
    let mut manifest = None;
    let mut model_dir = None;
    let mut yes = false;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if arg == OsStr::new("--install-local-mt-model") {
            saw_install_arg = true;
            manifest = Some(PathBuf::from(next_cli_arg(
                &mut iter,
                "--install-local-mt-model",
            )?));
        } else if arg == OsStr::new("--local-mt-model-dir") {
            model_dir = Some(PathBuf::from(next_cli_arg(
                &mut iter,
                "--local-mt-model-dir",
            )?));
        } else if arg == OsStr::new("--yes") || arg == OsStr::new("-y") {
            yes = true;
        } else if saw_install_arg {
            bail!("unknown local MT model install argument {:?}", arg);
        }
    }

    if !saw_install_arg {
        return Ok(None);
    }

    Ok(Some(LocalMtModelInstallArgs {
        manifest: manifest.context("missing --install-local-mt-model <manifest.json>")?,
        model_dir,
        yes,
    }))
}

/// Parse local STT model prefetch flags.
pub(crate) fn parse_local_stt_model_prefetch_args_from<I>(
    args: I,
) -> Result<Option<LocalSttModelPrefetchArgs>>
where
    I: IntoIterator<Item = OsString>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    let is_prefetch_command = args.iter().any(|arg| {
        arg == OsStr::new("--prefetch-local-stt-model")
            || arg == OsStr::new("--prefetch-local-stt-manifest")
    });
    if !is_prefetch_command {
        return Ok(None);
    }

    let mut source = None;
    let mut model_cache_dir = None;
    let mut yes = false;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if arg == OsStr::new("--prefetch-local-stt-model") {
            let raw = next_cli_arg(&mut iter, "--prefetch-local-stt-model")?;
            let raw = raw.to_string_lossy();
            set_local_stt_prefetch_source(
                &mut source,
                LocalSttModelPrefetchSource::BuiltinModel(parse_local_stt_model_id(&raw)?),
            )?;
        } else if arg == OsStr::new("--prefetch-local-stt-manifest") {
            set_local_stt_prefetch_source(
                &mut source,
                LocalSttModelPrefetchSource::Manifest(PathBuf::from(next_cli_arg(
                    &mut iter,
                    "--prefetch-local-stt-manifest",
                )?)),
            )?;
        } else if arg == OsStr::new("--model-cache-dir") {
            model_cache_dir = Some(PathBuf::from(next_cli_arg(&mut iter, "--model-cache-dir")?));
        } else if arg == OsStr::new("--yes") || arg == OsStr::new("-y") {
            yes = true;
        } else {
            bail!("unknown local STT model prefetch argument {:?}", arg);
        }
    }

    Ok(Some(LocalSttModelPrefetchArgs {
        source: source
            .context("missing --prefetch-local-stt-model <model-id> or --prefetch-local-stt-manifest <manifest.json>")?,
        model_cache_dir,
        yes,
    }))
}

fn set_local_stt_prefetch_source(
    current: &mut Option<LocalSttModelPrefetchSource>,
    next: LocalSttModelPrefetchSource,
) -> Result<()> {
    if current.replace(next).is_some() {
        bail!(
            "use only one local STT prefetch source: --prefetch-local-stt-model or --prefetch-local-stt-manifest"
        );
    }
    Ok(())
}

fn parse_local_stt_model_id(raw: &str) -> Result<providers::local::ModelId> {
    providers::local::ModelId::parse(raw).with_context(|| {
        format!(
            "unknown local STT model {raw:?}; supported values: {}",
            supported_local_stt_model_ids()
        )
    })
}

fn supported_local_stt_model_ids() -> String {
    providers::local::ModelId::ALL
        .iter()
        .map(|id| id.display_name())
        .collect::<Vec<_>>()
        .join(", ")
}

fn next_cli_arg(iter: &mut impl Iterator<Item = OsString>, flag: &'static str) -> Result<OsString> {
    let value = iter
        .next()
        .with_context(|| format!("missing value after {flag}"))?;
    if value.to_string_lossy().starts_with("--") {
        bail!("missing value after {flag}");
    }
    Ok(value)
}

fn model_download_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60 * 60))
        .build()
        .context("failed to create HTTP client for model download")
}

fn read_model_bundle_manifest(path: &Path) -> Result<providers::local::ModelBundleManifest> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read model manifest {}", path.display()))?;
    providers::local::ModelBundleManifest::from_json(&raw)
        .with_context(|| format!("failed to parse model manifest {}", path.display()))
}

/// Validate that a local STT manifest matches a built-in Whisper model.
pub(crate) fn validate_local_stt_bundle_manifest(
    manifest: &providers::local::ModelBundleManifest,
) -> Result<()> {
    let [file] = manifest.files.as_slice() else {
        bail!("local STT manifest must contain exactly one Whisper model file");
    };
    let Some(spec) = providers::local::ModelManifest::builtin()
        .iter()
        .find(|spec| spec.file_name == file.relative_path)
    else {
        bail!(
            "local STT manifest file {:?} is not one of the built-in Whisper model files",
            file.relative_path
        );
    };
    if spec.sha256 != file.sha256 || spec.size_bytes != file.size_bytes {
        bail!(
            "local STT manifest metadata for {} does not match the built-in Whisper checksum and size",
            file.relative_path
        );
    }
    Ok(())
}

/// Install a local MT model bundle from a manifest.
pub(crate) fn run_local_mt_model_install(args: &LocalMtModelInstallArgs) -> Result<()> {
    let manifest = read_model_bundle_manifest(&args.manifest)?;
    let consent_manifest = manifest
        .consent_manifest()
        .context("local MT bundle manifest is missing consent metadata")?;
    let model_dir = match &args.model_dir {
        Some(path) => path.clone(),
        None => providers::local::model_cache_dir()
            .context("failed to resolve local model cache directory")?
            .join("mt")
            .join(&manifest.id),
    };

    let mut stdout = io::stdout();
    writeln!(stdout, "{}", manifest.preview_text()).context("failed to write model preview")?;
    writeln!(stdout, "License text:\n{}\n", consent_manifest.license_text)
        .context("failed to write model license text")?;
    writeln!(stdout, "Destination: {}", model_dir.display())
        .context("failed to write model destination")?;

    let prior_consent = providers::local::model_consent_status(&consent_manifest)
        .context("failed to read existing local MT consent status")?;
    match &prior_consent {
        providers::local::ConsentStatus::Fresh => {
            writeln!(
                stdout,
                "Existing consent record found for {} {} — re-using.",
                consent_manifest.name, consent_manifest.version,
            )
            .context("failed to write consent reuse note")?;
        }
        providers::local::ConsentStatus::Missing => {
            writeln!(
                stdout,
                "No prior consent record for {} {} — a new record will be written on --yes.",
                consent_manifest.name, consent_manifest.version,
            )
            .context("failed to write consent missing note")?;
        }
        providers::local::ConsentStatus::Stale { reason } => {
            writeln!(
                stdout,
                "Existing consent record is stale and must be reconfirmed ({reason}).",
            )
            .context("failed to write consent stale note")?;
        }
    }

    if !args.yes {
        writeln!(
            stdout,
            "No files were downloaded. Re-run with --yes after reviewing the model license and size."
        )
        .context("failed to write model confirmation hint")?;
        return Ok(());
    }

    providers::local::write_model_consent_record(&consent_manifest).with_context(|| {
        format!(
            "failed to persist consent record for local MT model {} {}",
            consent_manifest.name, consent_manifest.version,
        )
    })?;
    writeln!(
        stdout,
        "Consent recorded for {} {} (license: {}).",
        consent_manifest.name, consent_manifest.version, consent_manifest.license_url,
    )
    .context("failed to write consent persistence summary")?;

    let client = model_download_client()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create model download runtime")?;
    let report = rt
        .block_on(providers::local::install_model_bundle(
            &client, &manifest, &model_dir,
        ))
        .context("failed to install local MT model")?;

    writeln!(
        stdout,
        "Installed model bundle to {} (downloaded {}, reused {}).",
        report.model_dir.display(),
        report.downloaded_files,
        report.reused_files
    )
    .context("failed to write model install summary")?;
    Ok(())
}

/// Prefetch a local STT model bundle.
pub(crate) fn run_local_stt_model_prefetch(args: &LocalSttModelPrefetchArgs) -> Result<()> {
    let bundle = match &args.source {
        LocalSttModelPrefetchSource::BuiltinModel(model_id) => {
            let manifest = providers::local::ModelManifest::builtin();
            let spec = manifest
                .find(*model_id)
                .with_context(|| format!("local STT model {model_id} is not available"))?;
            providers::local::stt_model_bundle_manifest(spec)
        }
        LocalSttModelPrefetchSource::Manifest(path) => {
            let manifest = read_model_bundle_manifest(path)?;
            validate_local_stt_bundle_manifest(&manifest)?;
            manifest
        }
    };
    let model_cache_dir = match &args.model_cache_dir {
        Some(path) => path.clone(),
        None => {
            providers::local::model_cache_dir().context("failed to resolve local model cache")?
        }
    };

    let mut stdout = io::stdout();
    writeln!(stdout, "{}", bundle.preview_text()).context("failed to write model preview")?;
    writeln!(stdout, "Model cache: {}", model_cache_dir.display())
        .context("failed to write model cache path")?;
    writeln!(
        stdout,
        "Verified marker: {}",
        model_cache_dir
            .join(providers::local::INSTALLED_MANIFEST_FILE)
            .display()
    )
    .context("failed to write verified marker path")?;

    if !args.yes {
        writeln!(
            stdout,
            "No files were downloaded. Re-run with --yes after reviewing the model license and size."
        )
        .context("failed to write model confirmation hint")?;
        return Ok(());
    }

    let client = model_download_client()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create model download runtime")?;
    let report = rt
        .block_on(providers::local::install_model_bundle(
            &client,
            &bundle,
            &model_cache_dir,
        ))
        .context("failed to prefetch local STT model")?;

    writeln!(
        stdout,
        "Prefetched local STT model bundle {} into {} (downloaded {}, reused {}).",
        bundle.display_name,
        report.model_dir.display(),
        report.downloaded_files,
        report.reused_files
    )
    .context("failed to write model prefetch summary")?;
    Ok(())
}

/// Return `true` when the user passed `--model-list`.
pub(crate) fn should_list_local_models<I>(args: I) -> bool
where
    I: IntoIterator<Item = OsString>,
{
    args.into_iter().any(|a| a == OsStr::new("--model-list"))
}

/// Print a table of all built-in Whisper models and their cache status.
pub(crate) fn run_model_list() -> Result<()> {
    let migration_err = providers::local::try_migrate_legacy_cache()
        .map(|_| ())
        .err();
    if let Some(ref err) = migration_err {
        tracing::warn!(%err, "LF-01 legacy cache migration failed");
    }

    let cache_dir = providers::local::model_cache_dir()
        .context("failed to resolve local model cache directory")?;

    let mut stdout = io::stdout();
    writeln!(stdout, "Model cache: {}", cache_dir.display())
        .context("failed to write model list header")?;

    if let Some(ref err) = migration_err {
        if let Ok(legacy) = providers::local::bootstrap::legacy_model_cache_dir() {
            if legacy.exists() {
                writeln!(
                    stdout,
                    "Legacy cache: {} (migration failed: {err}; \
                     models may need to be moved manually)",
                    legacy.display()
                )
                .context("failed to write legacy cache warning")?;
            }
        }
    }

    writeln!(stdout, "{:<14} {:>12}  Status", "Name", "Size")
        .context("failed to write model list column header")?;
    writeln!(stdout, "{}", "-".repeat(44)).context("failed to write separator")?;

    let manifest = providers::local::ModelManifest::builtin();
    for spec in manifest.iter() {
        let path = cache_dir.join(spec.file_name);
        let status = if path.exists() {
            "cached"
        } else {
            "not cached"
        };
        let size_mb = spec.size_bytes / 1_048_576;
        writeln!(
            stdout,
            "{:<14} {:>9} MB  {}",
            spec.id.display_name(),
            size_mb,
            status
        )
        .context("failed to write model list row")?;
    }
    Ok(())
}

/// Parse `--model-verify <model-id>` and optional cache directory flags.
pub(crate) fn parse_model_verify_args_from<I>(args: I) -> Result<Option<ModelVerifyArgs>>
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    let has_flag = args.iter().any(|a| a == OsStr::new("--model-verify"));
    if !has_flag {
        return Ok(None);
    }

    let mut model_id = None;
    let mut model_cache_dir = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if arg == OsStr::new("--model-verify") {
            let raw = next_cli_arg(&mut iter, "--model-verify")?;
            model_id = Some(parse_local_stt_model_id(&raw.to_string_lossy())?);
        } else if arg == OsStr::new("--model-cache-dir") {
            model_cache_dir = Some(PathBuf::from(next_cli_arg(&mut iter, "--model-cache-dir")?));
        }
    }

    Ok(Some(ModelVerifyArgs {
        model_id: model_id.context("missing model-id after --model-verify")?,
        model_cache_dir,
    }))
}

/// Verify a Whisper model in the cache directory and report the result.
pub(crate) fn run_model_verify(args: &ModelVerifyArgs) -> Result<()> {
    if let Err(err) = providers::local::try_migrate_legacy_cache() {
        tracing::warn!(
            %err,
            "LF-01 legacy cache migration failed; \
             verification may report model-not-found if the model is still in the legacy location"
        );
    }

    let cache_dir = match &args.model_cache_dir {
        Some(p) => p.clone(),
        None => providers::local::model_cache_dir()
            .context("failed to resolve local model cache directory")?,
    };

    let manifest = providers::local::ModelManifest::builtin();
    let spec = manifest
        .find(args.model_id)
        .with_context(|| format!("model '{}' not in built-in manifest", args.model_id))?;

    let path = cache_dir.join(spec.file_name);

    let mut stdout = io::stdout();
    writeln!(
        stdout,
        "Verifying {} at {}",
        spec.id.display_name(),
        path.display()
    )
    .context("failed to write verify header")?;

    match providers::local::verify_model_checksum(spec, &path) {
        Ok(()) => {
            writeln!(stdout, "OK — checksum matches manifest.")
                .context("failed to write result")?;
        }
        Err(err) => {
            writeln!(stdout, "FAIL — {err}").context("failed to write failure")?;
            anyhow::bail!("model verification failed");
        }
    }
    Ok(())
}
