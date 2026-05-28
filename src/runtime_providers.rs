use std::sync::{atomic::AtomicBool, Arc, Mutex};

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
}

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
        }
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
        }
    }
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

/// Build the runtime STT provider for a single slot.
pub(crate) fn build_slot_stt_provider(
    stt_provider: &str,
    cfg: &config::AppConfig,
    google_api_key: Option<&str>,
    status_msg: Arc<Mutex<Option<String>>>,
    local_provider_active: Arc<AtomicBool>,
    stt_source: Arc<Mutex<SttSource>>,
) -> std::result::Result<RuntimeSttProvider, providers::ProviderError> {
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
            "unsupported MT provider {other:?}"
        ))),
    }
}
