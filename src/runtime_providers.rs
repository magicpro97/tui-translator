use std::sync::{atomic::AtomicBool, Arc, Mutex, OnceLock, RwLock};

use crate::{config, metrics::SttSource, pipeline, providers};

/// Runtime STT provider selected from app configuration.
pub(crate) enum RuntimeSttProvider {
    Google(providers::google::stt::GoogleSttProvider),
    #[cfg(feature = "local-stt")]
    Local(providers::local::LocalWhisperSttProvider),
    GoogleWithLocalFallback(
        pipeline::fallback::FallbackSttProvider<
            providers::google::stt::GoogleSttProvider,
            providers::local::LocalWhisperSttProvider,
        >,
    ),
    #[cfg(feature = "local-stt")]
    LocalWithGoogleFallback(
        pipeline::fallback::FallbackSttProvider<
            providers::local::LocalWhisperSttProvider,
            providers::google::stt::GoogleSttProvider,
        >,
    ),
    #[cfg(feature = "local-stt")]
    LocalFailedWithGoogleFallback(
        pipeline::fallback::FallbackSttProvider<
            pipeline::fallback::FailedLocalSttProvider,
            providers::google::stt::GoogleSttProvider,
        >,
    ),
    /// Offline no-op STT used only by hermetic tests (PTY / smoke runs in
    /// keyless CI).  It performs no network I/O and never returns an
    /// `AuthError`, so the steady-state UI renders promptly instead of
    /// stalling on a 30 s Google STT request that 400s for lack of a key and
    /// then raises the persistent "API calls halted" banner.  Selected via the
    /// `TUI_TRANSLATOR_STT_OFFLINE` env var; never reachable from a real
    /// configuration.
    Offline(OfflineSttProvider),
}

/// No-op STT provider for hermetic tests: every chunk yields an empty,
/// non-final result and no network call is made.  See
/// [`RuntimeSttProvider::Offline`] and `stt_offline_mode_requested`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OfflineSttProvider;

impl providers::SttProvider for OfflineSttProvider {
    async fn transcribe(
        &self,
        _chunk: &providers::PcmChunk,
        _language_code: &str,
    ) -> std::result::Result<providers::SttResult, providers::ProviderError> {
        Ok(providers::SttResult {
            text: String::new(),
            confidence: None,
            is_final: false,
        })
    }
}

/// Whether the process was asked to run STT in offline (no-op) mode.
///
/// Test-only escape hatch driven by the `TUI_TRANSLATOR_STT_OFFLINE`
/// environment variable.  The PTY harness sets it so steady-state UI tests
/// in keyless CI do not issue a live Google STT request (which blocks the
/// pipeline for up to 30 s and then raises the persistent auth-error banner,
/// masking the UI the tests assert on).
pub(crate) fn stt_offline_mode_requested() -> bool {
    std::env::var_os("TUI_TRANSLATOR_STT_OFFLINE")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
}

/// Runtime MT provider selected from app configuration.
pub(crate) enum RuntimeMtProvider {
    Google(providers::google::mt::GoogleMtProvider),
    LocalRouted(
        providers::mt::router::MtRouter<
            providers::local::LocalOpusMtProvider,
            providers::google::mt::GoogleMtProvider,
        >,
    ),
    Llm(providers::llm::LlmMtProvider),
}

// ── CTRL-02: TTS voice catalog and hot-swap (issue #455) ─────────────────────

/// Runtime TTS provider selected from app configuration.
pub(crate) enum RuntimeTtsProvider {
    Google(providers::google::tts::GoogleTtsProvider),
    Disabled(DisabledTtsProvider),
    #[cfg(feature = "local-tts")]
    Supertonic(providers::local::supertonic_provider::SupertonicTtsProvider),
}

/// Disabled TTS provider used when translated audio is unavailable.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DisabledTtsProvider;

/// Process-wide TTS voice runtime used by TUI and hot-reload voice changes.
struct TtsVoiceRuntime {
    active_voice: Arc<RwLock<Option<providers::VoiceSelection>>>,
    catalog: Arc<RwLock<Vec<providers::VoiceSelection>>>,
}

static TTS_VOICE_RUNTIME: OnceLock<Arc<TtsVoiceRuntime>> = OnceLock::new();

impl providers::MtProvider for RuntimeMtProvider {
    #[tracing::instrument(skip_all, level = "trace", fields(provider = "runtime-mt"))]
    async fn translate(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
    ) -> std::result::Result<providers::MtResult, providers::ProviderError> {
        match self {
            Self::Google(provider) => {
                providers::MtProvider::translate(provider, text, source_language, target_language)
                    .await
            }
            Self::LocalRouted(provider) => {
                providers::MtProvider::translate(provider, text, source_language, target_language)
                    .await
            }
            Self::Llm(provider) => {
                providers::MtProvider::translate(provider, text, source_language, target_language)
                    .await
            }
        }
    }
}

impl providers::TtsProvider for RuntimeTtsProvider {
    #[tracing::instrument(skip_all, level = "trace", fields(provider = "runtime-tts"))]
    async fn synthesise(
        &self,
        text: &str,
        language_code: &str,
    ) -> std::result::Result<providers::TtsResult, providers::ProviderError> {
        match self {
            Self::Google(provider) => {
                providers::TtsProvider::synthesise(provider, text, language_code).await
            }
            Self::Disabled(provider) => {
                providers::TtsProvider::synthesise(provider, text, language_code).await
            }
            #[cfg(feature = "local-tts")]
            Self::Supertonic(provider) => {
                providers::TtsProvider::synthesise(provider, text, language_code).await
            }
        }
    }

    async fn list_voices(
        &self,
    ) -> std::result::Result<Vec<providers::VoiceSelection>, providers::ProviderError> {
        match self {
            Self::Google(provider) => providers::TtsProvider::list_voices(provider).await,
            Self::Disabled(_) => Ok(Vec::new()),
            #[cfg(feature = "local-tts")]
            Self::Supertonic(provider) => providers::TtsProvider::list_voices(provider).await,
        }
    }

    fn set_active_voice(
        &self,
        voice: Option<providers::VoiceSelection>,
    ) -> std::result::Result<(), providers::ProviderError> {
        match self {
            Self::Google(provider) => providers::TtsProvider::set_active_voice(provider, voice),
            Self::Disabled(_) => {
                if voice.is_some() {
                    Err(providers::ProviderError::InvalidInput(
                        "cannot select a TTS voice while TTS is disabled \
                         (configure google_api_key and tts_enabled first)"
                            .to_string(),
                    ))
                } else {
                    Ok(())
                }
            }
            #[cfg(feature = "local-tts")]
            Self::Supertonic(provider) => providers::TtsProvider::set_active_voice(provider, voice),
        }
    }

    fn active_voice(&self) -> Option<providers::VoiceSelection> {
        match self {
            Self::Google(provider) => providers::TtsProvider::active_voice(provider),
            Self::Disabled(_) => None,
            #[cfg(feature = "local-tts")]
            Self::Supertonic(provider) => providers::TtsProvider::active_voice(provider),
        }
    }
}

impl providers::TtsProvider for DisabledTtsProvider {
    #[tracing::instrument(skip_all, level = "trace", fields(provider = "disabled-tts"))]
    async fn synthesise(
        &self,
        _text: &str,
        _language_code: &str,
    ) -> std::result::Result<providers::TtsResult, providers::ProviderError> {
        Err(providers::ProviderError::InvalidInput(
            "translated audio is disabled because no Google API key is configured for TTS"
                .to_string(),
        ))
    }
}

impl providers::SttProvider for RuntimeSttProvider {
    async fn transcribe(
        &self,
        chunk: &providers::PcmChunk,
        language_code: &str,
    ) -> std::result::Result<providers::SttResult, providers::ProviderError> {
        match self {
            Self::Google(provider) => {
                providers::SttProvider::transcribe(provider, chunk, language_code).await
            }
            #[cfg(feature = "local-stt")]
            Self::Local(provider) => {
                providers::SttProvider::transcribe(provider, chunk, language_code).await
            }
            Self::GoogleWithLocalFallback(provider) => {
                providers::SttProvider::transcribe(provider, chunk, language_code).await
            }
            #[cfg(feature = "local-stt")]
            Self::LocalWithGoogleFallback(provider) => {
                providers::SttProvider::transcribe(provider, chunk, language_code).await
            }
            #[cfg(feature = "local-stt")]
            Self::LocalFailedWithGoogleFallback(provider) => {
                providers::SttProvider::transcribe(provider, chunk, language_code).await
            }
            Self::Offline(provider) => {
                providers::SttProvider::transcribe(provider, chunk, language_code).await
            }
        }
    }
}

/// Apply a `tts_voice` config value to the live provider.
pub(crate) fn apply_tts_voice_from_config(
    voice_name: Option<&str>,
) -> std::result::Result<(), providers::ProviderError> {
    let Some(rt) = TTS_VOICE_RUNTIME.get() else {
        return Ok(());
    };
    let selection = resolve_voice_by_name(voice_name)?;
    providers::google::tts::apply_voice_selection(&rt.active_voice, &rt.catalog, selection)
}

/// Cycle to the next configured TTS voice for the target language.
pub(crate) fn cycle_tts_voice_for_language(
    target_language: &str,
) -> std::result::Result<Option<providers::VoiceSelection>, providers::ProviderError> {
    let Some(rt) = TTS_VOICE_RUNTIME.get() else {
        return Err(providers::ProviderError::Unimplemented(
            "TTS voice is unavailable (TTS provider is not active)".to_string(),
        ));
    };
    let catalog_snapshot = rt.catalog.read().map_err(|_| {
        providers::ProviderError::Unknown("TTS voice catalog is unavailable".to_string())
    })?;
    if catalog_snapshot.is_empty() {
        return Err(providers::ProviderError::InvalidInput(
            "TTS voice catalog is empty".to_string(),
        ));
    }

    let filtered = filtered_voice_catalog(&catalog_snapshot, target_language);
    drop(catalog_snapshot);
    let current_voice_name = rt
        .active_voice
        .read()
        .map_err(|_| {
            providers::ProviderError::Unknown("TTS active voice is unavailable".to_string())
        })?
        .as_ref()
        .map(|v| v.name.clone());
    let next_voice = next_voice_selection(&filtered, current_voice_name.as_deref());
    providers::google::tts::apply_voice_selection(
        &rt.active_voice,
        &rt.catalog,
        next_voice.clone(),
    )?;
    Ok(next_voice)
}

impl RuntimeSttProvider {
    /// Initial source label used by metrics before any fallback transition.
    pub(crate) fn initial_stt_source(&self) -> SttSource {
        match self {
            Self::Google(_) | Self::GoogleWithLocalFallback(_) => SttSource::GoogleConfigured,
            #[cfg(feature = "local-stt")]
            Self::Local(_) | Self::LocalWithGoogleFallback(_) => SttSource::Local,
            #[cfg(feature = "local-stt")]
            Self::LocalFailedWithGoogleFallback(_) => SttSource::GoogleFallback,
            // Offline test stub: report as Google-configured so the title
            // indicator matches the fixture's `stt_provider = "google"`.
            Self::Offline(_) => SttSource::GoogleConfigured,
        }
    }

    /// Whether the runtime starts with local STT as the active primary.
    pub(crate) fn initial_provider_is_local(&self) -> bool {
        match self {
            #[cfg(feature = "local-stt")]
            Self::Local(_) => true,
            #[cfg(feature = "local-stt")]
            Self::LocalWithGoogleFallback(_) => true,
            _ => false,
        }
    }
}

fn resolve_voice_by_name(
    name: Option<&str>,
) -> std::result::Result<Option<providers::VoiceSelection>, providers::ProviderError> {
    let Some(name) = name else { return Ok(None) };
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let Some(rt) = TTS_VOICE_RUNTIME.get() else {
        return Err(providers::ProviderError::Unimplemented(
            "TTS voice runtime is not initialised (TTS provider is not active)".to_string(),
        ));
    };
    let catalog = rt.catalog.read().map_err(|_| {
        providers::ProviderError::Unknown("TTS voice catalog lock was poisoned".to_string())
    })?;
    catalog
        .iter()
        .find(|v| v.name == trimmed)
        .cloned()
        .map(Some)
        .ok_or_else(|| {
            providers::ProviderError::InvalidInput(format!(
                "voice {trimmed:?} is not in the TTS voice catalog"
            ))
        })
}

pub(crate) fn filtered_voice_catalog(
    catalog: &[providers::VoiceSelection],
    target_language: &str,
) -> Vec<providers::VoiceSelection> {
    let target_prefix = target_language
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if target_prefix.is_empty() {
        return catalog.to_vec();
    }

    let filtered: Vec<_> = catalog
        .iter()
        .filter(|voice| {
            voice
                .language
                .split(['-', '_'])
                .next()
                .map(|prefix| prefix.eq_ignore_ascii_case(&target_prefix))
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    if filtered.is_empty() {
        catalog.to_vec()
    } else {
        filtered
    }
}

pub(crate) fn next_voice_selection(
    voices: &[providers::VoiceSelection],
    current_voice_name: Option<&str>,
) -> Option<providers::VoiceSelection> {
    match current_voice_name {
        None => voices.first().cloned(),
        Some(name) => {
            let pos = voices.iter().position(|voice| voice.name == name);
            match pos {
                Some(index) if index + 1 < voices.len() => Some(voices[index + 1].clone()),
                Some(_) => None,
                None => voices.first().cloned(),
            }
        }
    }
}

fn has_google_api_key(google_api_key: Option<&str>) -> bool {
    google_api_key
        .map(str::trim)
        .is_some_and(|key| !key.is_empty())
}

/// Returns `true` when a permanent local-unavailable error must halt the
/// pipeline for the given `stt_provider` string from a slot config.
pub(crate) fn stt_local_unavailable_is_fatal_for_slot(
    stt_provider: &str,
    cfg: &config::AppConfig,
) -> bool {
    match (stt_provider, cfg.stt_fallback_policy.as_str()) {
        ("google", "local") => true,
        ("local", "none") => true,
        ("local", "google-when-keyed") => !has_google_api_key(cfg.google_api_key.as_deref()),
        _ => false,
    }
}

/// Build the runtime TTS provider for the main slot.
pub(crate) fn build_runtime_tts_provider(
    cfg: &config::AppConfig,
    google_api_key: Option<&str>,
    cost_reporter: Arc<dyn providers::CostReporter>,
) -> std::result::Result<RuntimeTtsProvider, providers::ProviderError> {
    // Local Supertonic TTS path: no API key required.
    #[cfg(feature = "local-tts")]
    if cfg.tts_provider == "local" {
        return providers::local::supertonic_provider::SupertonicTtsProvider::new()
            .map(RuntimeTtsProvider::Supertonic);
    }

    if !cfg.tts_enabled && google_api_key.is_none() {
        return Ok(RuntimeTtsProvider::Disabled(DisabledTtsProvider));
    }

    let key = google_api_key.ok_or_else(|| {
        providers::ProviderError::InvalidInput(
            "Google Text-to-Speech requires google_api_key when tts_enabled=true".to_string(),
        )
    })?;

    providers::google::tts::GoogleTtsProvider::new(key)
        .map(|provider| provider.with_cost_reporter(cost_reporter))
        .inspect(|provider| {
            let _ = TTS_VOICE_RUNTIME.set(Arc::new(TtsVoiceRuntime {
                active_voice: provider.active_voice_handle(),
                catalog: provider.voice_catalog_handle(),
            }));
            if let Err(err) = apply_tts_voice_from_config(cfg.tts_voice.as_deref()) {
                tracing::warn!(
                    error = %err,
                    "configured tts_voice could not be applied; using provider default"
                );
            }
        })
        .map(RuntimeTtsProvider::Google)
}

/// Build the runtime STT provider for a single slot.
pub(crate) fn build_slot_stt_provider(
    stt_provider: &str,
    cfg: &config::AppConfig,
    google_api_key: Option<&str>,
    status_msg: Arc<Mutex<Option<String>>>,
    local_provider_active: Arc<AtomicBool>,
    stt_source: Arc<Mutex<SttSource>>,
) -> std::result::Result<RuntimeSttProvider, providers::ProviderError> {
    // Test-only escape hatch: in offline mode (set by the PTY/smoke harness in
    // keyless CI) short-circuit to a no-op provider so no live STT request is
    // issued, the pipeline never stalls on the 30 s timeout, and the
    // persistent auth-error banner never masks the steady-state UI.
    if stt_offline_mode_requested() {
        return Ok(RuntimeSttProvider::Offline(OfflineSttProvider));
    }
    match stt_provider {
        "google" => {
            let key = google_api_key.ok_or_else(|| {
                providers::ProviderError::InvalidInput(
                    "Google STT requires google_api_key".to_string(),
                )
            })?;
            let google = providers::google::stt::GoogleSttProvider::new(key)?
                .with_phrase_hints(cfg.stt_phrase_hints.clone());

            let policy =
                pipeline::fallback::SttFallbackPolicy::from_config(&cfg.stt_fallback_policy)
                    .unwrap_or(pipeline::fallback::SttFallbackPolicy::None);

            if policy == pipeline::fallback::SttFallbackPolicy::Local {
                let (fallback, fallback_err) = match providers::local::LocalWhisperSttProvider::new(
                    providers::local::ModelId::Tiny,
                ) {
                    Ok(p) => {
                        tracing::info!("local Whisper (tiny) ready for STT fallback (issue #214)");
                        (Some(p), None)
                    }
                    Err(e) => {
                        tracing::warn!("local STT not available for fallback (issue #214): {e}");
                        (None, Some(e.to_string()))
                    }
                };
                Ok(RuntimeSttProvider::GoogleWithLocalFallback(
                    pipeline::fallback::FallbackSttProvider::new(
                        google,
                        fallback,
                        fallback_err,
                        policy,
                        status_msg,
                        local_provider_active,
                        stt_source,
                    ),
                ))
            } else {
                Ok(RuntimeSttProvider::Google(google))
            }
        }
        #[cfg(feature = "local-stt")]
        "local" => {
            let policy =
                pipeline::fallback::SttFallbackPolicy::from_config(&cfg.stt_fallback_policy)
                    .unwrap_or(pipeline::fallback::SttFallbackPolicy::None);

            if policy == pipeline::fallback::SttFallbackPolicy::GoogleWhenKeyed {
                let Some(key) = google_api_key else {
                    tracing::info!(
                        "stt_fallback_policy is google-when-keyed but no google_api_key is \
                         configured; running local STT without cloud fallback (issue #371)"
                    );
                    return providers::local::LocalWhisperSttProvider::new(
                        providers::local::ModelId::Tiny,
                    )
                    .map(|p| {
                        local_provider_active.store(true, std::sync::atomic::Ordering::Relaxed);
                        RuntimeSttProvider::Local(p)
                    });
                };

                let google = providers::google::stt::GoogleSttProvider::new(key)?
                    .with_phrase_hints(cfg.stt_phrase_hints.clone());

                match providers::local::LocalWhisperSttProvider::new(
                    providers::local::ModelId::Tiny,
                ) {
                    Ok(local) => {
                        tracing::info!(
                            "local Whisper (tiny) ready as primary STT; Google as fallback \
                             (google-when-keyed, issue #371)"
                        );
                        local_provider_active.store(true, std::sync::atomic::Ordering::Relaxed);
                        Ok(RuntimeSttProvider::LocalWithGoogleFallback(
                            pipeline::fallback::FallbackSttProvider::new(
                                local,
                                Some(google),
                                None,
                                policy,
                                status_msg,
                                local_provider_active,
                                stt_source,
                            ),
                        ))
                    }
                    Err(e) => {
                        tracing::warn!(
                            "local STT unavailable at startup; will use Google fallback \
                             immediately (google-when-keyed, issue #371): {e}"
                        );
                        Ok(RuntimeSttProvider::LocalFailedWithGoogleFallback(
                            pipeline::fallback::FallbackSttProvider::new(
                                pipeline::fallback::FailedLocalSttProvider::new(e.to_string()),
                                Some(google),
                                None,
                                policy,
                                status_msg,
                                local_provider_active,
                                stt_source,
                            ),
                        ))
                    }
                }
            } else {
                providers::local::LocalWhisperSttProvider::new(providers::local::ModelId::Tiny).map(
                    |p| {
                        local_provider_active.store(true, std::sync::atomic::Ordering::Relaxed);
                        RuntimeSttProvider::Local(p)
                    },
                )
            }
        }
        #[cfg(not(feature = "local-stt"))]
        "local" => Err(providers::ProviderError::Unimplemented(
            "local Whisper STT requires a build compiled with `--features local-stt`".to_string(),
        )),
        other => Err(providers::ProviderError::InvalidInput(format!(
            "unsupported STT provider {other:?}"
        ))),
    }
}

/// Build the runtime MT provider for a single slot.
///
/// **Note:** `"llm"` is not handled here because it requires async work (model
/// loading + optional auto-download).  Use `build_llm_mt_provider` (in
/// `crate::llm_startup`) for that case and call it via `rt.block_on(...)`
/// before delegating to this function.
pub(crate) fn build_slot_mt_provider(
    mt_provider: &str,
    google_api_key: Option<&str>,
    mt_cloud_fallback: Option<&str>,
    cost_reporter: Arc<dyn providers::CostReporter>,
) -> std::result::Result<RuntimeMtProvider, providers::ProviderError> {
    match mt_provider {
        "google" => {
            let key = google_api_key.ok_or_else(|| {
                providers::ProviderError::InvalidInput(
                    "Google Translation requires google_api_key".to_string(),
                )
            })?;
            providers::google::mt::GoogleMtProvider::new(key)
                .map(|p| p.with_cost_reporter(cost_reporter))
                .map(RuntimeMtProvider::Google)
        }
        "local" => {
            let local = providers::local::LocalOpusMtProvider::new_japanese_to_vietnamese()?;
            let cloud_fallback = match (mt_cloud_fallback, google_api_key) {
                (Some("google"), Some(key)) if !key.trim().is_empty() => Some(
                    providers::google::mt::GoogleMtProvider::new(key)?
                        .with_cost_reporter(Arc::clone(&cost_reporter)),
                ),
                _ => None,
            };
            Ok(RuntimeMtProvider::LocalRouted(
                providers::mt::router::MtRouter::new(local, cloud_fallback),
            ))
        }
        other => Err(providers::ProviderError::InvalidInput(format!(
            "unsupported MT provider {other:?}; call build_llm_mt_provider for \"llm\""
        ))),
    }
}

// LLM startup helpers (resolve_llm_model_dir, build_llm_mt_provider) live in
// src/llm_startup.rs to keep this file under the 600-line LOC gate.
