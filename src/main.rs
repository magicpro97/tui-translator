//! Entry point for TUI Translator.
//!
//! Phase 0 goal: open a terminal window, show the placeholder loading screen
//! (issue #26), and exit cleanly when the user presses `q` or `Ctrl+C`.
//! A live audio-level bar (issue #31) is fed by the audio capture foundation
//! and grows/shrinks with captured audio energy in real time.
//!
//! The bilingual subtitle pane (issues #54–57) is wired in here: Up/Down
//! arrows scroll the pane; End jumps back to the auto-follow bottom.
//!
//! Issues #41, #51, #58–#66 (Wave 5): richer status/metrics strip, STT state
//! indicator, TTS toggle (T), metrics expand/collapse (M), help overlay (?),
//! and quit/session-summary behaviour.
//!
//! Architecture (Wave 5 additions):
//! - Issue #63: a dedicated `tokio::task::spawn_blocking` keyboard task
//!   converts crossterm key events into [`UserAction`] values and sends them
//!   via an `std::sync::mpsc` channel so the event loop is decoupled from raw
//!   key scanning.
//! - Issue #61: a background `tokio::spawn` task publishes updated
//!   [`SessionMetrics`] to a `tokio::sync::watch` channel every second.
//! - Issue #64: all required commands are implemented (Space, L, R, Esc, Q).
//! - Issue #65: the control hints bar is always rendered, one row high.
//!
//! Phase 2 additions (issues #84–#89):
//! - Issue #84: `run_orchestrator` drives the STT → MT → TTS pipeline.
//! - Issue #85: exhausted retries produce visible status messages.
//! - Issue #86: `AuthError` halts the pipeline until the application restarts.
//! - Issue #87: graceful shutdown — waits up to 2 s for in-progress calls.
//! - Issue #88: Windows console control handler catches forced terminal close.

use anyhow::{bail, Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write as IoWrite},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, SystemTime},
};
use tui_input::InputRequest;

mod audio;
mod config;
mod metrics;
mod pipeline;
mod providers;
mod session;
mod tui;

use audio::DEFAULT_SILENCE_THRESHOLD;
use metrics::{
    spawn_process_metrics_task, LatencyHistogram, LossMetrics, MemoryGuard, MetricsSnapshot,
    NetworkMetrics, ProcessSnapshot,
};
use tui::{
    draw_session_summary, draw_ui, help_overlay_max_scroll, subtitle_inner_area, AppState,
    ConfigEditorMode, UserAction, AUDIO_LEVEL_SCALE,
};

type SharedPlaybackService = Arc<Mutex<Option<pipeline::playback::PlaybackService>>>;

struct FinishMainArgs<'a> {
    state: &'a AppState,
    restart_required: &'a Arc<AtomicBool>,
    cfg_path: &'a Path,
    current_config: &'a Arc<Mutex<config::AppConfig>>,
    playback_service: &'a SharedPlaybackService,
    orchestrator_join: Option<tokio::task::JoinHandle<()>>,
    orchestrator_shutdown: Arc<AtomicBool>,
    // ── Observability (issues #79–#83) ─────────────────────────────────────
    /// Watch receiver for per-second process snapshots (issue #79).
    process_rx: tokio::sync::watch::Receiver<ProcessSnapshot>,
    /// Shared E2E latency histogram populated by the orchestrator (issue #83).
    e2e_latency: Arc<LatencyHistogram>,
    /// Shared network byte counters populated by the orchestrator (issue #80).
    network_metrics: Arc<NetworkMetrics>,
    /// Shared audio-chunk loss counters populated by the orchestrator (issue #81).
    loss_metrics: Arc<LossMetrics>,
    // ── CPU throttle (issue #230) ──────────────────────────────────────────
    /// CPU gate shared with the orchestrator; the metrics publisher updates
    /// its CPU reading and reads the skip counter for the snapshot.
    cpu_gate: Arc<pipeline::cpu_gate::CpuGate>,
    /// RAM budget guard shared with the metrics publisher.
    memory_guard: Arc<MemoryGuard>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionExportFormat {
    Srt,
    Txt,
}

impl SessionExportFormat {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "srt" => Ok(Self::Srt),
            "txt" => Ok(Self::Txt),
            other => bail!("--export-format must be \"srt\" or \"txt\", got {other:?}"),
        }
    }

    fn render(self, segments: &[session::TranscriptSegment]) -> String {
        match self {
            Self::Srt => session::export_srt(segments),
            Self::Txt => session::export_txt(segments),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionExportArgs {
    input: PathBuf,
    output: PathBuf,
    format: SessionExportFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalMtModelInstallArgs {
    manifest: PathBuf,
    model_dir: Option<PathBuf>,
    yes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalSttModelPrefetchArgs {
    source: LocalSttModelPrefetchSource,
    model_cache_dir: Option<PathBuf>,
    yes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalSttModelPrefetchSource {
    BuiltinModel(providers::local::ModelId),
    Manifest(PathBuf),
}

/// Arguments for `--replay-session`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayArgs {
    /// Path to the session JSONL file to replay.
    pub path: PathBuf,
}

// ── Issue #88 — Windows console-control handler ───────────────────────────────
//
// Handles CTRL_C_EVENT, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT (window X button),
// CTRL_LOGOFF_EVENT, and CTRL_SHUTDOWN_EVENT by setting a global flag that the
// event loop checks at every frame.
//
// On non-Windows builds this is a no-op; Ctrl+C is still handled by the
// crossterm keyboard task.

/// Set to `true` by the Windows console handler when the terminal is closed
/// or the process is signalled to exit.
pub(crate) static FORCED_SHUTDOWN: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
mod windows_signal {
    use super::FORCED_SHUTDOWN;
    use std::sync::atomic::Ordering;

    const CTRL_C_EVENT: u32 = 0;
    const CTRL_BREAK_EVENT: u32 = 1;
    const CTRL_CLOSE_EVENT: u32 = 2;
    const CTRL_LOGOFF_EVENT: u32 = 5;
    const CTRL_SHUTDOWN_EVENT: u32 = 6;

    /// Windows console control handler.
    ///
    /// # Safety
    /// Called by the OS on a dedicated thread; only touches an `AtomicBool`.
    unsafe extern "system" fn ctrl_handler(ctrl_type: u32) -> i32 {
        match ctrl_type {
            CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT
            | CTRL_SHUTDOWN_EVENT => {
                FORCED_SHUTDOWN.store(true, Ordering::Relaxed);
                1 // TRUE — we handled it; do not call the next handler
            }
            _ => 0, // FALSE — pass to next handler
        }
    }

    extern "system" {
        #[link_name = "SetConsoleCtrlHandler"]
        fn set_console_ctrl_handler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }

    /// Register the console-control handler.
    ///
    /// Must be called once before the TUI starts so window-close events are
    /// caught even if the user never presses Ctrl+C.
    pub fn install() {
        // SAFETY: `ctrl_handler` only writes to an `AtomicBool`.
        let ok = unsafe { set_console_ctrl_handler(Some(ctrl_handler), 1) };
        if ok != 0 {
            tracing::info!("Windows console control handler installed (issue #88)");
        } else {
            tracing::warn!(
                "SetConsoleCtrlHandler failed; forced-close events may not be caught (issue #88)"
            );
        }
    }
}

#[cfg(not(windows))]
mod windows_signal {
    /// No-op on non-Windows platforms.
    pub fn install() {}
}

// ── CostReporter impl ─────────────────────────────────────────────────────────
// Bridge between the `providers::CostReporter` hook (defined in the providers
// module so it is accessible from the contract test binary) and the concrete
// `metrics::CostCounter` that lives here in the binary crate root.
impl providers::CostReporter for metrics::CostCounter {
    fn record_translated_characters(&self, count: usize) {
        // Use UFCS to unambiguously call the inherent method, not this trait impl.
        metrics::CostCounter::record_translated_characters(self, count);
    }

    fn record_synthesized_characters(&self, count: usize) {
        // Use UFCS to unambiguously call the inherent method, not this trait impl.
        metrics::CostCounter::record_synthesized_characters(self, count);
    }
}

enum RuntimeSttProvider {
    Google(providers::google::stt::GoogleSttProvider),
    #[cfg(feature = "local-stt")]
    Local(providers::local::LocalWhisperSttProvider),
    /// Google STT with a local Whisper fallback (issue #214).
    ///
    /// Active when `stt_provider = "google"` and `stt_fallback_policy =
    /// "local"`.  On the first `AuthError` from Google the provider switches
    /// permanently to local Whisper and writes a visible status message.
    /// This variant is compiled in stub builds too so users receive the same
    /// actionable local-STT setup error path as `stt_provider = "local"`.
    GoogleWithLocalFallback(
        pipeline::fallback::FallbackSttProvider<
            providers::google::stt::GoogleSttProvider,
            providers::local::LocalWhisperSttProvider,
        >,
    ),
}

enum RuntimeMtProvider {
    Google(providers::google::mt::GoogleMtProvider),
    Local(providers::local::LocalOpusMtProvider),
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
            Self::Local(provider) => {
                providers::MtProvider::translate(provider, text, source_language, target_language)
                    .await
            }
        }
    }
}

enum RuntimeTtsProvider {
    Google(providers::google::tts::GoogleTtsProvider),
    Disabled(DisabledTtsProvider),
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
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DisabledTtsProvider;

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
        }
    }
}

fn build_runtime_stt_provider(
    cfg: &config::AppConfig,
    google_api_key: Option<&str>,
    status_msg: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    local_provider_active: Arc<AtomicBool>,
) -> std::result::Result<RuntimeSttProvider, providers::ProviderError> {
    match cfg.stt_provider.as_str() {
        "google" => {
            let key = google_api_key.ok_or_else(|| {
                providers::ProviderError::InvalidInput(
                    "Google STT requires google_api_key".to_string(),
                )
            })?;
            let google = providers::google::stt::GoogleSttProvider::new(key)?
                .with_phrase_hints(cfg.stt_phrase_hints.clone());

            // Issue #214: if the fallback policy is "local", wrap Google in a
            // FallbackSttProvider so it automatically switches to local Whisper
            // on the first AuthError.
            let policy =
                pipeline::fallback::SttFallbackPolicy::from_config(&cfg.stt_fallback_policy)
                    .unwrap_or(pipeline::fallback::SttFallbackPolicy::None);

            if policy == pipeline::fallback::SttFallbackPolicy::Local {
                // Attempt to pre-build the local provider.  Failures here are
                // not fatal at startup: the error is stored so it surfaces
                // with an actionable message when the fallback is first needed.
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
                    ),
                ))
            } else {
                Ok(RuntimeSttProvider::Google(google))
            }
        }
        #[cfg(feature = "local-stt")]
        "local" => providers::local::LocalWhisperSttProvider::new(providers::local::ModelId::Tiny)
            .map(RuntimeSttProvider::Local),
        #[cfg(not(feature = "local-stt"))]
        "local" => Err(providers::ProviderError::Unimplemented(
            "local Whisper STT requires a build compiled with `--features local-stt`".to_string(),
        )),
        other => Err(providers::ProviderError::InvalidInput(format!(
            "unsupported STT provider {other:?}"
        ))),
    }
}

fn build_runtime_mt_provider(
    cfg: &config::AppConfig,
    google_api_key: Option<&str>,
    cost_reporter: Arc<dyn providers::CostReporter>,
) -> std::result::Result<RuntimeMtProvider, providers::ProviderError> {
    match cfg.mt_provider.as_str() {
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
        "local" => providers::local::LocalOpusMtProvider::new_japanese_to_vietnamese()
            .map(RuntimeMtProvider::Local),
        other => Err(providers::ProviderError::InvalidInput(format!(
            "unsupported MT provider {other:?}"
        ))),
    }
}

fn build_runtime_tts_provider(
    cfg: &config::AppConfig,
    google_api_key: Option<&str>,
    cost_reporter: Arc<dyn providers::CostReporter>,
) -> std::result::Result<RuntimeTtsProvider, providers::ProviderError> {
    if !cfg.tts_enabled && google_api_key.is_none() {
        return Ok(RuntimeTtsProvider::Disabled(DisabledTtsProvider));
    }

    let key = google_api_key.ok_or_else(|| {
        providers::ProviderError::InvalidInput(
            "Google Text-to-Speech requires google_api_key when tts_enabled=true".to_string(),
        )
    })?;

    providers::google::tts::GoogleTtsProvider::new(key)
        .map(|p| p.with_cost_reporter(cost_reporter))
        .map(RuntimeTtsProvider::Google)
}

fn start_session_recorder(
    rt: &tokio::runtime::Runtime,
    cfg: &config::AppConfig,
    status_slot: &Arc<Mutex<Option<String>>>,
    started_at_unix_ms: u64,
    session_id: &str,
) -> session::SessionRecorder {
    if !cfg.session_store.enabled {
        return session::SessionRecorder::disabled();
    }

    let directory = match cfg
        .session_store
        .directory
        .as_ref()
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(config::default_sessions_dir)
    {
        Ok(path) => path,
        Err(err) => {
            let msg = session_recording_disabled_status(&err);
            tracing::warn!("session recording disabled: {err:#}");
            *status_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
            return session::SessionRecorder::disabled();
        }
    };

    let header = session::SessionHeader {
        schema_version: session::SESSION_LOG_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        started_at_unix_ms,
        source_language: cfg.source_language.clone(),
        target_language: cfg.target_language.clone(),
        stt_provider: cfg.stt_provider.clone(),
        mt_provider: cfg.mt_provider.clone(),
        tts_enabled: cfg.tts_enabled,
        capture_device: cfg.capture_device.clone(),
    };

    match rt.block_on(session::SessionRecorder::start(
        session::SessionRecorderConfig::enabled(directory),
        header,
    )) {
        Ok(recorder) => {
            if let Some(path) = recorder.path() {
                tracing::info!(path = %path.display(), "session transcript recording enabled");
            }
            recorder
        }
        Err(err) => {
            let msg = session_recording_disabled_status(&err);
            tracing::warn!("session recording disabled: {err:#}");
            *status_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
            session::SessionRecorder::disabled()
        }
    }
}

fn session_recording_disabled_status(err: &anyhow::Error) -> String {
    format!("⚠ Session recording disabled: {err}").replace(['\r', '\n'], " ")
}

fn start_audio_archive(
    cfg: &config::AppConfig,
    session_id: &str,
    status_slot: &Arc<Mutex<Option<String>>>,
) -> audio::AudioArchiveWriter {
    if !cfg.audio_archive.store_audio || !cfg.audio_archive.consent_given {
        return audio::AudioArchiveWriter::disabled();
    }

    let directory = match cfg
        .audio_archive
        .directory
        .as_ref()
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(config::default_audio_archive_dir)
    {
        Ok(path) => path,
        Err(err) => {
            let msg = audio_archive_disabled_status(&err);
            tracing::warn!("audio archive disabled: {err:#}");
            *status_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
            return audio::AudioArchiveWriter::disabled();
        }
    };

    let archive_config = audio::AudioArchiveWriterConfig {
        enabled: true,
        directory,
        max_size_bytes: cfg.audio_archive.max_size_mb.saturating_mul(1024 * 1024),
    };

    match audio::AudioArchiveWriter::start(&archive_config, session_id) {
        Ok(writer) => {
            if let Some(path) = writer.path() {
                tracing::info!(path = %path.display(), "raw audio archive enabled");
                *status_slot.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some(format!("⚠ Audio archiving enabled: {}", path.display()));
            }
            writer
        }
        Err(err) => {
            let msg = audio_archive_disabled_status(&err);
            tracing::warn!("audio archive disabled: {err:#}");
            *status_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
            audio::AudioArchiveWriter::disabled()
        }
    }
}

fn audio_archive_disabled_status(err: &anyhow::Error) -> String {
    format!("⚠ Audio archive disabled: {err}").replace(['\r', '\n'], " ")
}

fn attach_audio_archive(
    rt: &tokio::runtime::Runtime,
    stream: audio::CaptureStream,
    mut writer: audio::AudioArchiveWriter,
    status_slot: Arc<Mutex<Option<String>>>,
) -> audio::CaptureStream {
    if writer.is_disabled() {
        return stream;
    }

    let info = stream.info;
    let mut input_rx = stream.receiver;
    let (output_tx, output_rx) = tokio::sync::mpsc::channel(64);

    rt.spawn(async move {
        while let Some(chunk) = input_rx.recv().await {
            if !writer.is_disabled() {
                if let Err(err) = writer.append_chunk(&chunk) {
                    let msg = audio_archive_disabled_status(&err);
                    tracing::warn!("audio archive disabled after write error: {err:#}");
                    *status_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
                    writer.disable();
                }
            }

            if output_tx.send(chunk).await.is_err() {
                break;
            }
        }
    });

    audio::CaptureStream {
        info,
        receiver: output_rx,
    }
}

/// Initialise the tracing subscriber, routing output to a log file so that
/// diagnostic lines never reach the ConPTY terminal stream (issue #183).
///
/// The log file is `tui-translator.log` in the OS temp directory. Appending is
/// used so successive runs accumulate in one file without truncating earlier
/// output. Falls back to stderr if the log file cannot be opened.
fn init_tracing() {
    let log_dir = std::env::temp_dir();

    match tracing_appender::rolling::RollingFileAppender::builder()
        .filename_prefix("tui-translator")
        .filename_suffix("log")
        .build(&log_dir)
    {
        Ok(file_appender) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "tui_translator=info".into()),
                )
                .with_writer(file_appender)
                .init();
        }
        Err(error) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "tui_translator=info".into()),
                )
                .init();
            tracing::warn!(
                log_dir = %log_dir.display(),
                error = %error,
                "failed to initialize temp log file; falling back to stderr"
            );
        }
    }
}

fn should_list_audio_devices() -> bool {
    std::env::args().skip(1).any(|arg| {
        arg == "--list-audio-devices"
            || arg == "--list-capture-devices"
            || arg == "list-audio-devices"
    })
}

fn print_audio_devices_to_stdout() -> Result<()> {
    let devices = audio::list_capture_devices().context("failed to list audio capture devices")?;
    let mut stdout = io::stdout();
    write_audio_devices(&mut stdout, &devices).context("failed to write audio device list")?;
    Ok(())
}

fn write_audio_devices(
    writer: &mut impl IoWrite,
    devices: &[audio::CaptureDeviceInfo],
) -> io::Result<()> {
    writeln!(
        writer,
        "Audio capture devices for WASAPI loopback (Windows playback endpoints):"
    )?;
    writeln!(
        writer,
        "  [default] Windows default playback device (leave capture_device blank)"
    )?;
    if devices.is_empty() {
        writeln!(
            writer,
            "  No active playback devices were reported by Windows."
        )?;
    } else {
        for device in devices {
            let marker = if device.is_default {
                " (current Windows default)"
            } else {
                ""
            };
            writeln!(writer, "  - {}{}", device.name, marker)?;
            writeln!(writer, "      endpoint_id: {}", device.id)?;
        }
    }
    Ok(())
}

fn parse_local_mt_model_install_args_from<I>(args: I) -> Result<Option<LocalMtModelInstallArgs>>
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

fn parse_local_stt_model_prefetch_args_from<I>(args: I) -> Result<Option<LocalSttModelPrefetchArgs>>
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

fn validate_local_stt_bundle_manifest(
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

fn run_local_mt_model_install(args: &LocalMtModelInstallArgs) -> Result<()> {
    let manifest = read_model_bundle_manifest(&args.manifest)?;
    let model_dir = match &args.model_dir {
        Some(path) => path.clone(),
        None => providers::local::model_cache_dir()
            .context("failed to resolve local model cache directory")?
            .join("mt")
            .join(&manifest.id),
    };

    let mut stdout = io::stdout();
    writeln!(stdout, "{}", manifest.preview_text()).context("failed to write model preview")?;
    writeln!(stdout, "Destination: {}", model_dir.display())
        .context("failed to write model destination")?;

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

fn run_local_stt_model_prefetch(args: &LocalSttModelPrefetchArgs) -> Result<()> {
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

fn parse_session_export_args_from<I>(args: I) -> Result<Option<SessionExportArgs>>
where
    I: IntoIterator<Item = OsString>,
{
    let mut saw_export_arg = false;
    let mut input = None;
    let mut output = None;
    let mut format = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if arg == OsStr::new("--export-session") {
            saw_export_arg = true;
            input = Some(PathBuf::from(next_cli_arg(&mut iter, "--export-session")?));
        } else if arg == OsStr::new("--export-output") {
            saw_export_arg = true;
            output = Some(PathBuf::from(next_cli_arg(&mut iter, "--export-output")?));
        } else if arg == OsStr::new("--export-format") {
            saw_export_arg = true;
            let value = next_cli_arg(&mut iter, "--export-format")?;
            let value = value
                .into_string()
                .map_err(|_| anyhow::anyhow!("--export-format must be valid UTF-8"))?;
            format = Some(SessionExportFormat::parse(&value)?);
        } else if saw_export_arg {
            bail!("unknown session export argument {:?}", arg);
        }
    }

    if !saw_export_arg {
        return Ok(None);
    }

    Ok(Some(SessionExportArgs {
        input: input.context("missing --export-session <session.jsonl>")?,
        output: output.context("missing --export-output <path>")?,
        format: format.context("missing --export-format <srt|txt>")?,
    }))
}

fn run_session_export(args: &SessionExportArgs) -> Result<()> {
    let contents = fs::read_to_string(&args.input)
        .with_context(|| format!("failed to read session log {}", args.input.display()))?;
    let segments = session::transcript_segments_from_jsonl(&contents)
        .with_context(|| format!("failed to parse session log {}", args.input.display()))?;
    let rendered = args.format.render(&segments);

    if let Some(parent) = args
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create export directory {}", parent.display()))?;
    }
    fs::write(&args.output, rendered)
        .with_context(|| format!("failed to write export {}", args.output.display()))?;

    let mut stdout = io::stdout();
    writeln!(
        stdout,
        "Exported {} transcript segment(s) to {}",
        segments.len(),
        args.output.display()
    )
    .context("failed to write export summary")?;
    Ok(())
}

// ── Session replay (issue #226 / EP-F.3) ─────────────────────────────────────

/// Parse `--replay-session <path>` from an argument iterator.
///
/// Returns `Ok(None)` when no replay flag is present.  Returns an error when
/// `--replay-session` appears without a following path value.
fn parse_replay_args_from<I>(args: I) -> Result<Option<ReplayArgs>>
where
    I: IntoIterator<Item = OsString>,
{
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == OsStr::new("--replay-session") {
            let value = iter
                .next()
                .with_context(|| "missing value after --replay-session")?;
            if value.to_string_lossy().starts_with("--") {
                bail!("missing value after --replay-session");
            }
            return Ok(Some(ReplayArgs {
                path: PathBuf::from(value),
            }));
        }
    }
    Ok(None)
}

/// Run the TUI in session-replay mode.
///
/// Reads `args.path`, loads all [`TranscriptSegment`]s via
/// [`session::SessionReplayer`] (malformed lines are skipped with a warning),
/// populates the subtitle pane one segment at a time, and shows the normal TUI.
/// Audio capture, STT, MT, and TTS providers are **not started**.
///
/// The existing `Space` pause / resume key works: when the user pauses, the
/// replay feeder stops advancing until they resume.
fn run_session_replay(args: &ReplayArgs) -> Result<()> {
    let contents = fs::read_to_string(&args.path)
        .with_context(|| format!("failed to read session log {}", args.path.display()))?;

    let replayer = session::SessionReplayer::load(&contents)
        .with_context(|| format!("failed to load replay session {}", args.path.display()))?;
    let segment_count = replayer.segment_count();
    let skipped_count = replayer.skipped_count();

    if skipped_count > 0 {
        tracing::warn!(
            skipped = skipped_count,
            path = %args.path.display(),
            "skipped malformed lines during replay load"
        );
    }
    tracing::info!(
        segments = segment_count,
        skipped = skipped_count,
        path = %args.path.display(),
        "starting session replay"
    );

    let state = AppState::new();
    // Display a clear indicator in the device-name slot so the operator knows
    // they are watching a replay, not a live session.
    overwrite_device_name(
        &state.device_name,
        &format!("REPLAY: {}", args.path.display()),
    );
    overwrite_capture_device_label(
        &state.capture_device_label,
        &Some("Replay session".to_string()),
    );
    *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
        metrics::SttState::Error("replay mode — no live audio".to_string());

    // Wrap the replayer in Arc<Mutex<>> so the feeder task and the TUI can both
    // access it.  The TUI's `paused` AtomicBool drives pause/resume for replay.
    let replayer = Arc::new(Mutex::new(replayer));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build Tokio runtime for replay")?;

    // Spawn the replay feeder task: push one SubtitlePair per 600 ms, honoring
    // the `state.paused` flag (Space key).
    {
        let subtitle_pane = Arc::clone(&state.subtitle_pane);
        let paused = Arc::clone(&state.paused);
        let replayer = Arc::clone(&replayer);
        rt.spawn(async move {
            loop {
                // When paused, poll at 50 ms so the resume is snappy.
                if paused.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }

                let maybe_seg = {
                    let mut r = replayer.lock().unwrap_or_else(|p| p.into_inner());
                    r.next_segment()
                };

                match maybe_seg {
                    None => {
                        // Replayer is done (or paused, handled above).
                        // If done, wait a bit and re-check; exit when is_done.
                        let done = replayer.lock().unwrap_or_else(|p| p.into_inner()).is_done();
                        if done {
                            tracing::info!("session replay finished; all segments displayed");
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    Some(seg) => {
                        let pair = tui::SubtitlePair::new(seg.source_text, seg.target_text);
                        subtitle_pane
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .push(pair);
                        // 600 ms between segments keeps replay readable.
                        tokio::time::sleep(Duration::from_millis(600)).await;
                    }
                }
            }
        });
    }

    // Dummy config objects — replay mode does not read or write config.
    let restart_required = Arc::new(AtomicBool::new(false));
    let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
    let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

    let (key_tx, key_rx) = mpsc::channel::<UserAction>();
    let keyboard_shutdown = Arc::new(AtomicBool::new(false));
    {
        let lang_flag = Arc::clone(&state.lang_prompt_active);
        let config_editor_flag = Arc::clone(&state.config_editor_active);
        let keyboard_shutdown = Arc::clone(&keyboard_shutdown);
        rt.spawn(async move {
            tokio::task::spawn_blocking(move || {
                keyboard_task(key_tx, lang_flag, config_editor_flag, keyboard_shutdown)
            })
            .await
            .ok();
        });
    }

    // Use a dummy cfg_path; replay mode never writes config.
    let cfg_path = PathBuf::from("replay.json");

    windows_signal::install();

    let tui_context = TuiRuntimeContext {
        restart_required: &restart_required,
        cfg_path: &cfg_path,
        current_config: &current_config,
        playback_service: &playback_service,
        interaction_mode: TuiInteractionMode::ReplayReadOnly,
    };
    let result = run_tui(&state, &tui_context, &keyboard_shutdown, key_rx);

    rt.shutdown_background();

    if result.is_ok() {
        print_session_summary_to_stdout(&state);
    }

    result
}

fn main() -> Result<()> {
    init_tracing();

    tracing::info!("tui-translator starting");

    if should_list_audio_devices() {
        print_audio_devices_to_stdout()?;
        return Ok(());
    }

    if let Some(install_args) = parse_local_mt_model_install_args_from(std::env::args_os().skip(1))?
    {
        run_local_mt_model_install(&install_args)?;
        return Ok(());
    }

    if let Some(prefetch_args) =
        parse_local_stt_model_prefetch_args_from(std::env::args_os().skip(1))?
    {
        run_local_stt_model_prefetch(&prefetch_args)?;
        return Ok(());
    }

    // Replay mode — bypasses all audio/STT/MT/TTS provider construction.
    if let Some(replay_args) = parse_replay_args_from(std::env::args_os().skip(1))? {
        run_session_replay(&replay_args)?;
        return Ok(());
    }

    if let Some(export_args) = parse_session_export_args_from(std::env::args_os().skip(1))? {
        run_session_export(&export_args)?;
        return Ok(());
    }

    // Issue #88 — install the Windows console control handler before the TUI
    // enters alternate-screen mode so a forced close triggers our cleanup.
    windows_signal::install();

    // Copy legacy executable-side config into the per-user location before path
    // selection. The original file stays in place so portable installs keep
    // working; the copy gives installed users a safe per-user migration path.
    let legacy_migration_notice = match migrate_legacy_config_to_per_user_if_needed() {
        Ok(notice) => notice,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "legacy config migration skipped; continuing with startup config lookup"
            );
            None
        }
    };

    // Load configuration from the selected startup path unless a test or caller
    // explicitly overrides it through TUI_TRANSLATOR_CONFIG.
    let cfg_path = config_json_path();
    prepare_per_user_config_dir_for_startup(&cfg_path)?;
    bootstrap_legacy_config_if_needed(&cfg_path)?;
    let (cfg, load_state, load_error) = config::load_for_startup(&cfg_path)?;
    let startup_config_mode = startup_config_mode(
        load_state,
        has_explicit_config_override(),
        skip_onboarding(),
    );
    let onboarding_required = startup_config_mode == StartupConfigMode::OnboardingRequired;
    let config_recovery_required = startup_config_mode == StartupConfigMode::ConfigRecoveryRequired;
    let current_config = Arc::new(Mutex::new(cfg.clone()));
    let restart_required = Arc::new(AtomicBool::new(false));

    // Start the hot-reload watcher; keep the receiver alive for the process lifetime.
    let config_rx = match config::start_watcher(&cfg_path, cfg.clone(), restart_required.clone()) {
        Ok(rx) => Some(rx),
        Err(err) => {
            tracing::warn!("config hot-reload unavailable: {err:#}");
            None
        }
    };

    let state = AppState::new();
    if let Some(notice) = legacy_migration_notice {
        *state
            .startup_notice_msg
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(notice.user_message());
    }
    let loaded_config = current_config
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    state.set_target_language(loaded_config.target_language.clone());
    // Initialise source_language from the loaded config so AppState and the
    // orchestrator start with the same value.
    overwrite_source_language(&state.source_language, &loaded_config.source_language);
    // Initialise the operator-facing capture device label from config (issue #197).
    overwrite_capture_device_label(&state.capture_device_label, &loaded_config.capture_device);
    state.set_tts_enabled(loaded_config.tts_enabled);
    let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
    if !onboarding_required && !config_recovery_required {
        let current_cfg = current_config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        sync_playback_service_state(&playback_service, &current_cfg, current_cfg.tts_enabled);
    }
    if onboarding_required {
        state.open_config_editor(ConfigEditorMode::Onboarding, &cfg, &cfg_path);
        populate_capture_device_options(&state);
        let _ = state.with_config_editor_mut(|editor| {
            editor.set_status_message(" Fill required fields, then press Enter to save.");
        });
        overwrite_device_name(&state.device_name, "first-run setup required");
        tracing::info!(
            path = %cfg_path.display(),
            "per-user config missing; opening first-run setup"
        );
    }
    if config_recovery_required {
        state.open_config_editor(ConfigEditorMode::Settings, &cfg, &cfg_path);
        populate_capture_device_options(&state);
        let _ = state.with_config_editor_mut(|editor| {
            let detail = if load_error
                .as_deref()
                .is_some_and(|message| message.contains("source_language"))
            {
                "source_language is invalid"
            } else if load_error
                .as_deref()
                .is_some_and(|message| message.contains("target_language"))
            {
                "target_language is invalid"
            } else {
                "config is invalid"
            };
            editor.set_status_message(format!(
                " Config needs repair: {detail}. Fix fields and press Enter."
            ));
        });
        overwrite_device_name(&state.device_name, "config needs repair");
        tracing::warn!(
            path = %cfg_path.display(),
            "config invalid; opening settings repair mode"
        );
    }

    // Build a multi-threaded Tokio runtime for background tasks.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    if let Some(mut config_rx) = config_rx {
        let current_config = Arc::clone(&current_config);
        let target_language = Arc::clone(&state.target_language);
        let source_language = Arc::clone(&state.source_language);
        let capture_device_label = Arc::clone(&state.capture_device_label);
        let tts_enabled = Arc::clone(&state.tts_enabled);
        let restart_required = Arc::clone(&restart_required);
        let playback_service = Arc::clone(&playback_service);
        rt.spawn(async move {
            while config_rx.changed().await.is_ok() {
                let next_cfg = config_rx.borrow().clone();
                // PlaybackService::new does blocking I/O (startup_rx.recv + device enum);
                // offload to a dedicated thread so we don't block a Tokio worker.
                let cc = Arc::clone(&current_config);
                let tl = Arc::clone(&target_language);
                let sl = Arc::clone(&source_language);
                let cdl = Arc::clone(&capture_device_label);
                let te = Arc::clone(&tts_enabled);
                let rr = Arc::clone(&restart_required);
                let ps = Arc::clone(&playback_service);
                tokio::task::spawn_blocking(move || {
                    apply_runtime_config(&cc, &tl, &sl, &cdl, &te, &rr, &ps, next_cfg);
                })
                .await
                .ok();
            }
        });
    }

    // ── Issue #84 — orchestrator shutdown signal ──────────────────────────────
    // Shared with `run_tui` so it can signal the orchestrator when Q is pressed.
    let orchestrator_shutdown = Arc::new(AtomicBool::new(false));
    // Handle returned by the orchestrator task; `None` when no API key is set.
    let orchestrator_join: Option<tokio::task::JoinHandle<()>>;

    // ── Issues #79–#83 — shared observability objects ─────────────────────────
    // Created here so both the orchestrator and the metrics-publisher can share
    // the same Arcs.
    let e2e_latency = Arc::new(LatencyHistogram::new());
    let network_metrics = Arc::new(NetworkMetrics::new());
    let loss_metrics = Arc::new(LossMetrics::new());

    // Issue #230: create the CPU gate from the loaded config.
    // `cpu_budget_pct = 0.0` (default) disables throttling, preserving
    // existing behaviour for all users who have not set the field.
    let cpu_gate = {
        let cfg = current_config.lock().unwrap_or_else(|p| p.into_inner());
        Arc::new(pipeline::cpu_gate::CpuGate::new(cfg.cpu_budget_pct))
    };
    let memory_guard = {
        let cfg = current_config.lock().unwrap_or_else(|p| p.into_inner());
        Arc::new(MemoryGuard::new(
            cfg.ram_budget_mb.saturating_mul(1024 * 1024),
        ))
    };

    // Issue #79: start the process-metrics polling task and get its receiver.
    // Pass the runtime handle explicitly so spawn_blocking works before the
    // first block_on call (tokio::task::spawn_blocking requires a current
    // runtime context which is not yet established at this point).
    let (process_tx, process_rx) = tokio::sync::watch::channel(ProcessSnapshot::default());
    spawn_process_metrics_task(process_tx, rt.handle());

    if onboarding_required || config_recovery_required {
        let orchestrator_join = None;
        return finish_main(
            rt,
            FinishMainArgs {
                state: &state,
                restart_required: &restart_required,
                cfg_path: &cfg_path,
                current_config: &current_config,
                playback_service: &playback_service,
                orchestrator_join,
                orchestrator_shutdown,
                process_rx,
                e2e_latency,
                network_metrics,
                loss_metrics,
                cpu_gate,
                memory_guard,
            },
        );
    }

    // ── Audio source selection (issue #110) ──────────────────────────────────
    // Read the audio_source / audio_file_path / capture_device from the loaded
    // config and start the appropriate capture backend. WASAPI is the
    // production path; "file" is used for soak tests and local replay runs.
    let (audio_source_kind, audio_file_path, capture_device) = {
        let cfg = current_config.lock().unwrap_or_else(|p| p.into_inner());
        (
            cfg.audio_source.clone(),
            cfg.audio_file_path.clone(),
            cfg.capture_device.clone(),
        )
    };
    let capture_result = if audio_source_kind == "file" {
        let path = audio_file_path.as_deref().unwrap_or("");
        rt.block_on(audio::start_file_capture(path, DEFAULT_SILENCE_THRESHOLD))
    } else {
        rt.block_on(audio::start_capture_with_device(
            capture_device.as_deref(),
            DEFAULT_SILENCE_THRESHOLD,
        ))
    };

    match capture_result {
        Ok(stream) => {
            overwrite_device_name(&state.device_name, &stream.info.device_name);
            *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
                metrics::SttState::Listening;

            // ── Issue #84 — wire orchestrator or fall back to metrics-only ────
            let cfg_snapshot = current_config
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            let started_at_unix_ms = session::system_time_unix_ms(SystemTime::now());
            let session_id = session::generate_session_id(started_at_unix_ms);
            let stream = attach_audio_archive(
                &rt,
                stream,
                start_audio_archive(&cfg_snapshot, &session_id, &state.pipeline_error_msg),
                Arc::clone(&state.pipeline_error_msg),
            );

            if let Some(provider_msg) = runtime_provider_error(&cfg_snapshot) {
                tracing::warn!("{provider_msg}");
                *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
                    metrics::SttState::Error(provider_msg.clone());
                *state
                    .pipeline_error_msg
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) = Some(provider_msg);
                spawn_metrics_only_audio_task(&rt, stream, &state);
                orchestrator_join = None;
            } else if let Some(provider_msg) = missing_google_api_key_error(&cfg_snapshot) {
                tracing::warn!("{provider_msg}");
                *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
                    metrics::SttState::Error(provider_msg.clone());
                *state
                    .pipeline_error_msg
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) = Some(provider_msg);
                spawn_metrics_only_audio_task(&rt, stream, &state);
                orchestrator_join = None;
            } else if cfg_snapshot.google_api_key.is_none()
                && cfg_snapshot.stt_provider == "google"
                && cfg_snapshot.mt_provider == "google"
            {
                // Preserve the no-key startup path: users can still verify
                // audio capture and settings before adding a Google key.
                tracing::info!(
                    "no google_api_key configured; running without STT/MT/TTS (issue #84)"
                );
                spawn_metrics_only_audio_task(&rt, stream, &state);
                orchestrator_join = None;
            } else {
                // Build the selected STT provider plus Google MT/TTS, then
                // start the orchestrator.
                // Reuse the Arc already held in AppState so hot-reload writes
                // are visible to the running orchestrator.
                let google_api_key = cfg_snapshot.google_api_key.as_deref();
                let source_language = Arc::clone(&state.source_language);

                // Issue #230 + #214: shared STT-local flag read by the CPU gate.
                // It starts true only for configured local STT, then flips true
                // if Google falls back to local Whisper at runtime. Local MT does
                // not drop audio before Google STT has a chance to transcribe it.
                let provider_is_local =
                    Arc::new(AtomicBool::new(cfg_snapshot.stt_provider == "local"));
                let local_unavailable_is_fatal = cfg_snapshot.stt_provider == "google"
                    && cfg_snapshot.stt_fallback_policy == "local";

                let stt_provider = match build_runtime_stt_provider(
                    &cfg_snapshot,
                    google_api_key,
                    Arc::clone(&state.pipeline_error_msg),
                    Arc::clone(&provider_is_local),
                ) {
                    Ok(p) => p,
                    Err(err) => {
                        tracing::error!("failed to create STT provider: {err}");
                        // Fall back to metrics-only audio task.
                        spawn_metrics_only_audio_task(&rt, stream, &state);
                        orchestrator_join = None;
                        return finish_main(
                            rt,
                            FinishMainArgs {
                                state: &state,
                                restart_required: &restart_required,
                                cfg_path: &cfg_path,
                                current_config: &current_config,
                                playback_service: &playback_service,
                                orchestrator_join,
                                orchestrator_shutdown,
                                process_rx: process_rx.clone(),
                                e2e_latency: Arc::clone(&e2e_latency),
                                network_metrics: Arc::clone(&network_metrics),
                                loss_metrics: Arc::clone(&loss_metrics),
                                cpu_gate: Arc::clone(&cpu_gate),
                                memory_guard: Arc::clone(&memory_guard),
                            },
                        );
                    }
                };
                // Issue #71–#76 and #217: wire the configured MT provider.
                // Google reports billable character usage; local OPUS-MT ignores
                // the reporter but shares the same runtime trait.
                let mt_provider = match build_runtime_mt_provider(
                    &cfg_snapshot,
                    google_api_key,
                    Arc::clone(&state.cost_counter) as Arc<dyn providers::CostReporter>,
                ) {
                    Ok(p) => p,
                    Err(err) => {
                        tracing::error!("failed to create MT provider: {err}");
                        let provider_msg = format!("Machine translation unavailable: {err}");
                        *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
                            metrics::SttState::Error(provider_msg.clone());
                        *state
                            .pipeline_error_msg
                            .lock()
                            .unwrap_or_else(|p| p.into_inner()) = Some(provider_msg);
                        spawn_metrics_only_audio_task(&rt, stream, &state);
                        orchestrator_join = None;
                        return finish_main(
                            rt,
                            FinishMainArgs {
                                state: &state,
                                restart_required: &restart_required,
                                cfg_path: &cfg_path,
                                current_config: &current_config,
                                playback_service: &playback_service,
                                orchestrator_join,
                                orchestrator_shutdown,
                                process_rx: process_rx.clone(),
                                e2e_latency: Arc::clone(&e2e_latency),
                                network_metrics: Arc::clone(&network_metrics),
                                loss_metrics: Arc::clone(&loss_metrics),
                                cpu_gate: Arc::clone(&cpu_gate),
                                memory_guard: Arc::clone(&memory_guard),
                            },
                        );
                    }
                };
                // Issue #71–#76: wire cost reporter so TTS API usage is billed
                // against the shared CostCounter. When TTS is disabled and no
                // key is configured, use a disabled provider that is never
                // called unless the operator toggles T at runtime.
                let tts_provider = match build_runtime_tts_provider(
                    &cfg_snapshot,
                    google_api_key,
                    Arc::clone(&state.cost_counter) as Arc<dyn providers::CostReporter>,
                ) {
                    Ok(p) => p,
                    Err(err) => {
                        tracing::error!("failed to create TTS provider: {err}");
                        spawn_metrics_only_audio_task(&rt, stream, &state);
                        orchestrator_join = None;
                        return finish_main(
                            rt,
                            FinishMainArgs {
                                state: &state,
                                restart_required: &restart_required,
                                cfg_path: &cfg_path,
                                current_config: &current_config,
                                playback_service: &playback_service,
                                orchestrator_join,
                                orchestrator_shutdown,
                                process_rx: process_rx.clone(),
                                e2e_latency: Arc::clone(&e2e_latency),
                                network_metrics: Arc::clone(&network_metrics),
                                loss_metrics: Arc::clone(&loss_metrics),
                                cpu_gate: Arc::clone(&cpu_gate),
                                memory_guard: Arc::clone(&memory_guard),
                            },
                        );
                    }
                };
                let session_recorder = start_session_recorder(
                    &rt,
                    &cfg_snapshot,
                    &state.pipeline_error_msg,
                    started_at_unix_ms,
                    &session_id,
                );

                let ctx = pipeline::OrchestratorContext {
                    audio_level: Arc::clone(&state.audio_level),
                    stt_state: Arc::clone(&state.stt_state),
                    subtitle_pane: Arc::clone(&state.subtitle_pane),
                    session_metrics: Arc::clone(&state.session_metrics),
                    cost_counter: Arc::clone(&state.cost_counter),
                    pipeline_error_msg: Arc::clone(&state.pipeline_error_msg),
                    auth_error_banner: Arc::clone(&state.auth_error_banner),
                    pipeline_halted: Arc::clone(&state.pipeline_halted),
                    paused: Arc::clone(&state.paused),
                    tts_enabled: Arc::clone(&state.tts_enabled),
                    source_language,
                    target_language: Arc::clone(&state.target_language),
                    stt_provider_name: cfg_snapshot.stt_provider.clone(),
                    mt_provider_name: cfg_snapshot.mt_provider.clone(),
                    playback: Arc::clone(&playback_service),
                    shutdown: Arc::clone(&orchestrator_shutdown),
                    e2e_latency: Arc::clone(&e2e_latency),
                    network_metrics: Arc::clone(&network_metrics),
                    loss_metrics: Arc::clone(&loss_metrics),
                    cpu_gate: Arc::clone(&cpu_gate),
                    provider_is_local: Arc::clone(&provider_is_local),
                    local_unavailable_is_fatal,
                    // Pass VAD config when enabled; None preserves existing behaviour.
                    vad_config: if cfg_snapshot.vad.enabled {
                        Some(audio::VadConfig {
                            threshold: cfg_snapshot.vad.threshold,
                            min_speech_ms: cfg_snapshot.vad.min_speech_ms,
                            speech_pad_ms: cfg_snapshot.vad.speech_pad_ms,
                            min_silence_ms: cfg_snapshot.vad.min_silence_ms,
                            pre_roll_ms: cfg_snapshot.vad.pre_roll_ms,
                        })
                    } else {
                        None
                    },
                    // Pipeline windowing/aggregation knobs (issue #270 / EP-I.7).
                    pipeline_max_window_ms: cfg_snapshot.pipeline.max_window_ms,
                    pipeline_early_flush_on_vad_end: cfg_snapshot.pipeline.early_flush_on_vad_end,
                    pipeline_idle_flush_ms: cfg_snapshot.pipeline.idle_flush_ms,
                    pipeline_idle_min_ms: cfg_snapshot.pipeline.idle_min_ms,
                    stabilizer: Arc::new(std::sync::Mutex::new(
                        pipeline::segmentation::SegmentStabilizer::new(),
                    )),
                    sentence_aggregator: Arc::new(std::sync::Mutex::new(
                        pipeline::sentence_aggregator::SentenceAggregator::with_max_age(
                            std::time::Duration::from_millis(
                                cfg_snapshot.pipeline.sentence_max_age_ms,
                            ),
                        ),
                    )),
                    session_recorder,
                };

                orchestrator_join = Some(rt.spawn(pipeline::run_orchestrator(
                    stream.receiver,
                    stt_provider,
                    mt_provider,
                    tts_provider,
                    ctx,
                )));
            }
        }
        Err(err) => {
            // Issue #196: surface the failure clearly in the TUI instead of
            // leaving the operator with only a silent missing audio gauge.
            let err_msg = err.to_string();
            *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
                metrics::SttState::Error(err_msg);
            *state
                .capture_error_msg
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(
                "Press [S] to open Settings, or run --list-capture-devices and set capture_device."
                    .to_string(),
            );
            overwrite_device_name(&state.device_name, "audio capture unavailable");
            tracing::error!("audio capture failed to start: {err:#}");
            orchestrator_join = None;
        }
    }

    finish_main(
        rt,
        FinishMainArgs {
            state: &state,
            restart_required: &restart_required,
            cfg_path: &cfg_path,
            current_config: &current_config,
            playback_service: &playback_service,
            orchestrator_join,
            orchestrator_shutdown,
            process_rx,
            e2e_latency,
            network_metrics,
            loss_metrics,
            cpu_gate,
            memory_guard,
        },
    )
}

/// Spawn the legacy metrics-only audio task (used when no API key is set).
fn spawn_metrics_only_audio_task(
    rt: &tokio::runtime::Runtime,
    mut stream: audio::CaptureStream,
    state: &AppState,
) {
    let level_tx = Arc::clone(&state.audio_level);
    let paused = Arc::clone(&state.paused);
    let session_metrics = Arc::clone(&state.session_metrics);
    let stt_state = Arc::clone(&state.stt_state);
    let metrics_only_cost_counter = Arc::new(metrics::CostCounter::new());
    rt.spawn(async move {
        loop {
            let Some(chunk) = stream.receiver.recv().await else {
                mark_audio_capture_stopped(&level_tx, &stt_state);
                break;
            };
            handle_audio_chunk(
                chunk,
                &paused,
                &level_tx,
                &session_metrics,
                &metrics_only_cost_counter,
            );
        }
    });
}

/// Run the TUI event loop, then orchestrate graceful shutdown and print the
/// session summary.
///
/// Extracted from `main` so both the happy path and provider-init error paths
/// share identical shutdown logic.
fn finish_main(rt: tokio::runtime::Runtime, args: FinishMainArgs<'_>) -> Result<()> {
    let FinishMainArgs {
        state,
        restart_required,
        cfg_path,
        current_config,
        playback_service,
        orchestrator_join,
        orchestrator_shutdown,
        process_rx,
        e2e_latency,
        network_metrics,
        loss_metrics,
        cpu_gate,
        memory_guard,
    } = args;

    // ── Issues #61, #82: metrics observability background task ───────────────
    // Publish a fresh `MetricsSnapshot` to the watch channel every second so the
    // UI can read it lock-free via `state.metrics_snapshot()` (issue #82).
    {
        let metrics_src = Arc::clone(&state.session_metrics);
        let metrics_tx = Arc::clone(&state.metrics_tx);
        let subtitle_pane = Arc::clone(&state.subtitle_pane);
        let cost_counter = Arc::clone(&state.cost_counter);
        let cpu_gate = Arc::clone(&cpu_gate);
        let memory_guard = Arc::clone(&memory_guard);
        let current_config = Arc::clone(current_config);
        let mut process_rx = process_rx;
        rt.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            let mut last_ram_budget_bytes = u64::MAX;
            loop {
                interval.tick().await;
                let session = metrics_src
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                let line_pairs_shown = subtitle_pane
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .pair_count() as u64;
                let net_snap = network_metrics.drain_window(1.0);
                let proc_snap = process_rx.borrow_and_update().clone();

                // Issue #230: keep the CPU gate's reading in sync with the
                // process snapshot so throttle decisions are up to date.
                cpu_gate.update_cpu_pct(proc_snap.cpu_pct);

                let mut snapshot = MetricsSnapshot {
                    audio_seconds_sent: session.audio_seconds_sent,
                    chars_translated: session.chars_translated,
                    // Issue #71–#76: derive running cost from shared CostCounter so
                    // STT, MT, and TTS costs are all included.
                    estimated_cost_usd: cost_counter.current_estimate_usd(),
                    line_pairs_shown,
                    session_start: session.session_start,
                    // Issue #269: quality / diagnostic counters.
                    truncation_rate: if session.total_windows > 0 {
                        session.truncated_windows as f64 / session.total_windows as f64
                    } else {
                        0.0
                    },
                    flicker_count: session.flicker_count,
                    mt_call_count: session.mt_call_count,
                    ..MetricsSnapshot::default()
                };

                // Issue #79: apply CPU/RAM from the process-metrics task.
                snapshot.apply_process(&proc_snap);
                let ram_budget_bytes = current_config
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .ram_budget_mb
                    .saturating_mul(1024 * 1024);
                if ram_budget_bytes != last_ram_budget_bytes {
                    memory_guard.update_budget_bytes(ram_budget_bytes);
                    last_ram_budget_bytes = ram_budget_bytes;
                }
                memory_guard.update_ram_bytes(snapshot.ram_bytes);
                snapshot.apply_memory_guard(&memory_guard);
                // Issue #80: apply window network kbps.
                snapshot.apply_network(&net_snap);
                // Issue #83: apply E2E latency percentiles.
                snapshot.e2e_latency_ms = e2e_latency.current_ms();
                snapshot.e2e_latency_mean_ms = e2e_latency.mean_ms();
                snapshot.e2e_latency_p95_ms = e2e_latency.percentile_ms(95.0);
                // Issue #81: apply loss counters.
                snapshot.loss_pct = loss_metrics.loss_pct();
                snapshot.total_chunks = loss_metrics.total_chunks();
                snapshot.dropped_chunks = loss_metrics.dropped_chunks();
                // Issue #230: publish how many local-inference chunks were
                // intentionally skipped due to CPU pressure.
                snapshot.local_inferences_skipped = cpu_gate.skipped_count();

                let _ = metrics_tx.send(snapshot);
            }
        });
    }

    // ── Issue #63: dedicated keyboard task ───────────────────────────────────
    // A `tokio::task::spawn_blocking` task blocks on crossterm key reads so the
    // event loop never needs to poll for keys directly.
    let (key_tx, key_rx) = mpsc::channel::<UserAction>();
    let keyboard_shutdown = Arc::new(AtomicBool::new(false));
    {
        let lang_flag = Arc::clone(&state.lang_prompt_active);
        let config_editor_flag = Arc::clone(&state.config_editor_active);
        let keyboard_shutdown = Arc::clone(&keyboard_shutdown);
        rt.spawn(async move {
            tokio::task::spawn_blocking(move || {
                keyboard_task(key_tx, lang_flag, config_editor_flag, keyboard_shutdown)
            })
            .await
            .ok();
        });
    }

    let tui_context = TuiRuntimeContext {
        restart_required,
        cfg_path,
        current_config,
        playback_service,
        interaction_mode: TuiInteractionMode::Live,
    };
    let result = run_tui(state, &tui_context, &keyboard_shutdown, key_rx);

    // ── Issue #87 — graceful orchestrator shutdown ────────────────────────────
    // Signal the orchestrator to stop processing new chunks.
    orchestrator_shutdown.store(true, Ordering::Relaxed);
    // Wait up to 2 seconds for any in-progress STT/MT/TTS call to finish.
    if let Some(join_handle) = orchestrator_join {
        rt.block_on(async {
            let _ = tokio::time::timeout(Duration::from_secs(2), join_handle).await;
        });
    }

    // Cancel remaining background tasks without waiting for them to finish.
    rt.shutdown_background();

    // ── Issue #64: print session summary to stdout after terminal is restored ─
    // The alternate screen was left when `run_tui` returned (TerminalGuard::drop).
    // Only print when the user quit intentionally (not on error paths).
    if result.is_ok() {
        print_session_summary_to_stdout(state);
    }

    result
}

/// Returns the path to `config.json` to load at startup.
///
/// Resolution order:
/// 1. `TUI_TRANSLATOR_CONFIG` environment variable — allows the soak runner
///    (and other callers) to inject a temporary configuration without touching
///    the user's profile.
/// 2. An existing `config.json` next to the executable, preserving portable ZIP
///    usage.
/// 3. The OS-specific per-user config directory from `directories::BaseDirs`
///    (or `TUI_TRANSLATOR_CONFIG_DIR`) joined with `config.json`.
/// 4. The directory that contains the running executable joined with
///    `config.json` as a legacy fallback when the OS config directory cannot
///    be resolved.
/// 5. The literal string `"config.json"` in the current working directory as a
///    last resort.
fn config_json_path() -> PathBuf {
    let per_user_config = config::default_config_path();
    if let Err(error) = &per_user_config {
        tracing::warn!(
            error = %error,
            "failed to resolve per-user config path; falling back to portable config path"
        );
    }
    select_config_json_path(
        explicit_config_json_path(),
        existing_legacy_config_json_path(),
        per_user_config,
        legacy_config_json_path(),
    )
}

fn explicit_config_json_path() -> Option<PathBuf> {
    std::env::var_os("TUI_TRANSLATOR_CONFIG")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn legacy_config_json_path() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("config.json")))
}

fn existing_legacy_config_json_path() -> Option<PathBuf> {
    let path = legacy_config_json_path()?;
    match path.try_exists() {
        Ok(true) => Some(path),
        Ok(false) => None,
        Err(err) => {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "failed to check portable config path"
            );
            None
        }
    }
}

fn select_config_json_path(
    explicit_override: Option<PathBuf>,
    portable_config: Option<PathBuf>,
    per_user_config: Result<PathBuf>,
    fallback_config: Option<PathBuf>,
) -> PathBuf {
    if let Some(path) = explicit_override {
        return path;
    }
    if let Some(path) = portable_config {
        return path;
    }
    per_user_config
        .unwrap_or_else(|_| fallback_config.unwrap_or_else(|| PathBuf::from("config.json")))
}

fn has_explicit_config_override() -> bool {
    explicit_config_json_path().is_some()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegacyConfigMigrationNotice {
    from: PathBuf,
    to: PathBuf,
}

impl LegacyConfigMigrationNotice {
    fn user_message(&self) -> String {
        "Config copied to per-user folder; old executable-side config left unchanged.".to_string()
    }
}

fn migrate_legacy_config_to_per_user_if_needed() -> Result<Option<LegacyConfigMigrationNotice>> {
    if has_explicit_config_override() {
        return Ok(None);
    }

    let Some(legacy_path) = legacy_config_json_path() else {
        return Ok(None);
    };
    let per_user_path = match config::default_config_path() {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to resolve per-user config path; skipping legacy config migration"
            );
            return Ok(None);
        }
    };

    copy_legacy_config_if_needed(&legacy_path, &per_user_path)
}

fn copy_legacy_config_if_needed(
    legacy_path: &Path,
    target_path: &Path,
) -> Result<Option<LegacyConfigMigrationNotice>> {
    if legacy_path == target_path {
        return Ok(None);
    }
    if !legacy_path
        .try_exists()
        .with_context(|| format!("failed to access {}", legacy_path.display()))?
    {
        return Ok(None);
    }
    if target_path
        .try_exists()
        .with_context(|| format!("failed to access {}", target_path.display()))?
    {
        return Ok(None);
    }

    let parent = target_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .context("per-user config path must have a parent directory")?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create per-user config directory {}",
            parent.display()
        )
    })?;
    fs::copy(legacy_path, target_path).with_context(|| {
        format!(
            "failed to copy legacy config from {} to {}",
            legacy_path.display(),
            target_path.display()
        )
    })?;

    let notice = LegacyConfigMigrationNotice {
        from: legacy_path.to_path_buf(),
        to: target_path.to_path_buf(),
    };
    tracing::warn!(
        from = %notice.from.display(),
        to = %notice.to.display(),
        notice = %notice.user_message(),
        "legacy executable-side config copied to per-user config location"
    );
    Ok(Some(notice))
}

fn prepare_per_user_config_dir_for_startup(config_path: &Path) -> Result<()> {
    if !should_prepare_per_user_config_dir(
        has_explicit_config_override(),
        config_path,
        config::default_config_path(),
    ) {
        return Ok(());
    }

    create_config_parent_dir(config_path)
}

fn should_prepare_per_user_config_dir(
    explicit_override: bool,
    selected_path: &Path,
    per_user_config: Result<PathBuf>,
) -> bool {
    if explicit_override {
        return false;
    }
    per_user_config
        .map(|path| path == selected_path)
        .unwrap_or(false)
}

fn create_config_parent_dir(config_path: &Path) -> Result<()> {
    let parent = config_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .context("per-user config path must have a parent directory")?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create per-user config directory {}",
            parent.display()
        )
    })
}

fn skip_onboarding() -> bool {
    matches!(
        std::env::var("TUI_TRANSLATOR_SKIP_ONBOARDING"),
        Ok(value) if value == "1" || value.eq_ignore_ascii_case("true")
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupConfigMode {
    Normal,
    OnboardingRequired,
    ConfigRecoveryRequired,
}

fn startup_config_mode(
    load_state: config::LoadState,
    explicit_config_override: bool,
    skip_onboarding: bool,
) -> StartupConfigMode {
    match load_state {
        config::LoadState::Missing if !explicit_config_override && !skip_onboarding => {
            StartupConfigMode::OnboardingRequired
        }
        config::LoadState::Invalid if !skip_onboarding => StartupConfigMode::ConfigRecoveryRequired,
        _ => StartupConfigMode::Normal,
    }
}

fn bootstrap_legacy_config_if_needed(target_path: &Path) -> Result<()> {
    if has_explicit_config_override() {
        return Ok(());
    }
    if target_path
        .try_exists()
        .with_context(|| format!("failed to access {}", target_path.display()))?
    {
        return Ok(());
    }

    let Some(legacy_path) = legacy_config_json_path() else {
        return Ok(());
    };
    if legacy_path == target_path
        || !legacy_path
            .try_exists()
            .with_context(|| format!("failed to access {}", legacy_path.display()))?
    {
        return Ok(());
    }

    copy_legacy_config_if_needed(&legacy_path, target_path).map(|_| ())
}

fn overwrite_device_name(slot: &Arc<std::sync::Mutex<String>>, next_name: &str) {
    match slot.lock() {
        Ok(mut guard) => {
            *guard = next_name.to_string();
        }
        Err(poisoned) => {
            tracing::warn!("device_name mutex was poisoned; recovering last known state");
            let mut guard = poisoned.into_inner();
            *guard = next_name.to_string();
        }
    }
}

/// Derive the operator-facing capture device label from the config field.
///
/// Returns `"Default device"` when no explicit non-whitespace device is
/// configured, or the trimmed device name when one has been selected. This
/// label is shown in the audio gauge title (issue #197).
fn capture_device_label_from_config(capture_device: &Option<String>) -> String {
    match capture_device.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => "Default device".to_string(),
    }
}

fn overwrite_capture_device_label(
    slot: &Arc<std::sync::Mutex<String>>,
    capture_device: &Option<String>,
) {
    let label = capture_device_label_from_config(capture_device);
    match slot.lock() {
        Ok(mut guard) => {
            *guard = label;
        }
        Err(poisoned) => {
            tracing::warn!("capture_device_label mutex was poisoned; recovering last known state");
            let mut guard = poisoned.into_inner();
            *guard = label;
        }
    }
}

fn populate_capture_device_options(state: &AppState) {
    match audio::list_capture_devices() {
        Ok(devices) => {
            let names = devices.into_iter().map(|device| device.name).collect();
            let _ = state.with_config_editor_mut(|editor| {
                editor.set_capture_device_options(names);
            });
        }
        Err(err) => {
            tracing::warn!("failed to list WASAPI capture devices for settings picker: {err:#}");
        }
    }
}

fn overwrite_target_language(slot: &Arc<std::sync::Mutex<String>>, next_language: &str) {
    match slot.lock() {
        Ok(mut guard) => {
            *guard = next_language.to_string();
        }
        Err(poisoned) => {
            tracing::warn!("target_language mutex was poisoned; recovering last known state");
            let mut guard = poisoned.into_inner();
            *guard = next_language.to_string();
        }
    }
}

fn overwrite_source_language(slot: &Arc<std::sync::Mutex<String>>, next_language: &str) {
    match slot.lock() {
        Ok(mut guard) => {
            *guard = next_language.to_string();
        }
        Err(poisoned) => {
            tracing::warn!("source_language mutex was poisoned; recovering last known state");
            let mut guard = poisoned.into_inner();
            *guard = next_language.to_string();
        }
    }
}

/// Update `current_config`, language strings, capture-device label, TTS state, and the
/// restart-required flag from `next_cfg`.
///
/// Called on live config hot-reload (R key), after saving the settings overlay,
/// and from the file-watcher task.
#[allow(clippy::too_many_arguments)]
fn apply_runtime_config(
    current_config: &Arc<Mutex<config::AppConfig>>,
    target_language: &Arc<std::sync::Mutex<String>>,
    source_language: &Arc<std::sync::Mutex<String>>,
    capture_device_label: &Arc<std::sync::Mutex<String>>,
    tts_enabled: &Arc<AtomicBool>,
    restart_required: &Arc<AtomicBool>,
    playback_service: &SharedPlaybackService,
    next_cfg: config::AppConfig,
) {
    let requires_restart = {
        let mut current = current_config.lock().unwrap_or_else(|p| p.into_inner());
        let requires_restart = current.requires_restart(&next_cfg);
        *current = next_cfg.clone();
        requires_restart
    };

    if requires_restart {
        restart_required.store(true, Ordering::Relaxed);
    }

    overwrite_target_language(target_language, &next_cfg.target_language);
    overwrite_source_language(source_language, &next_cfg.source_language);
    overwrite_capture_device_label(capture_device_label, &next_cfg.capture_device);
    // Sync the backend first; only set the UI flag to match what actually succeeded.
    let service_ok = sync_playback_service_state(playback_service, &next_cfg, next_cfg.tts_enabled);
    tts_enabled.store(next_cfg.tts_enabled && service_ok, Ordering::Relaxed);
}

fn normalize_optional_field(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn onboarding_status_message() -> &'static str {
    "Press Ctrl+C to quit and edit config manually."
}

fn validate_onboarding_editor(editor: &tui::ConfigEditorState) -> Result<()> {
    if editor.mode != ConfigEditorMode::Onboarding {
        return Ok(());
    }
    if editor.source_language.trim().is_empty() {
        bail!(
            "onboarding requires a source language. {}",
            onboarding_status_message()
        );
    }
    if editor.target_language.trim().is_empty() {
        bail!(
            "onboarding requires a target language. {}",
            onboarding_status_message()
        );
    }
    if editor.google_api_key.trim().is_empty() {
        bail!(
            "onboarding requires a Google API key. {}",
            onboarding_status_message()
        );
    }
    Ok(())
}

fn parse_editor_bool(field_name: &str, value: &str) -> Result<bool> {
    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => {
            bail!("validation failed: `{field_name}` must be \"true\" or \"false\", got {other:?}")
        }
    }
}

fn parse_editor_u32(field_name: &str, value: &str) -> Result<u32> {
    let trimmed = value.trim();
    trimmed.parse::<u32>().map_err(|_| {
        anyhow::anyhow!(
            "validation failed: `{field_name}` must be a positive integer (e.g. 1500), \
             got {trimmed:?}"
        )
    })
}

fn parse_editor_u64(field_name: &str, value: &str) -> Result<u64> {
    let trimmed = value.trim();
    trimmed.parse::<u64>().map_err(|_| {
        anyhow::anyhow!(
            "validation failed: `{field_name}` must be a positive integer (e.g. 600), \
             got {trimmed:?}"
        )
    })
}

fn build_config_from_editor(
    editor: &tui::ConfigEditorState,
    current_config: &config::AppConfig,
) -> Result<config::AppConfig> {
    let mut next_cfg = current_config.clone();
    next_cfg.source_language = editor.source_language.trim().to_string();
    next_cfg.target_language = editor.target_language.trim().to_string();
    next_cfg.google_api_key = normalize_optional_field(&editor.google_api_key);
    next_cfg.audio_source = editor.audio_source.trim().to_string();
    next_cfg.capture_device = normalize_optional_field(&editor.capture_device);
    next_cfg.audio_file_path = normalize_optional_field(&editor.audio_file_path);
    next_cfg.stt_provider = editor.stt_provider.trim().to_string();
    next_cfg.mt_provider = editor.mt_provider.trim().to_string();
    next_cfg.tts_enabled = parse_editor_bool("tts_enabled", &editor.tts_enabled)?;
    next_cfg.stt_fallback_policy = editor.stt_fallback_policy.trim().to_string();
    // Pipeline knobs (issue #270).
    next_cfg.vad.pre_roll_ms = parse_editor_u32("vad.pre_roll_ms", &editor.vad_pre_roll_ms)?;
    next_cfg.pipeline.max_window_ms =
        parse_editor_u32("pipeline.max_window_ms", &editor.pipeline_max_window_ms)?;
    next_cfg.pipeline.early_flush_on_vad_end = parse_editor_bool(
        "pipeline.early_flush_on_vad_end",
        &editor.pipeline_early_flush_on_vad_end,
    )?;
    next_cfg.pipeline.idle_flush_ms =
        parse_editor_u64("pipeline.idle_flush_ms", &editor.pipeline_idle_flush_ms)?;
    next_cfg.pipeline.idle_min_ms =
        parse_editor_u32("pipeline.idle_min_ms", &editor.pipeline_idle_min_ms)?;
    next_cfg.pipeline.sentence_max_age_ms = parse_editor_u64(
        "pipeline.sentence_max_age_ms",
        &editor.pipeline_sentence_max_age_ms,
    )?;
    Ok(next_cfg)
}

fn runtime_provider_error(cfg: &config::AppConfig) -> Option<String> {
    let mut unsupported = Vec::new();
    match cfg.stt_provider.as_str() {
        "google" => {}
        #[cfg(feature = "local-stt")]
        "local" => {}
        #[cfg(not(feature = "local-stt"))]
        "local" => {
            unsupported.push("stt_provider=\"local\" (requires a local-stt build)".to_string())
        }
        _ => unsupported.push(format!("stt_provider={:?}", cfg.stt_provider)),
    }
    match cfg.mt_provider.as_str() {
        "google" => {}
        #[cfg(feature = "local-mt")]
        "local" => {}
        #[cfg(not(feature = "local-mt"))]
        "local" => {
            unsupported.push("mt_provider=\"local\" (requires a local-mt build)".to_string())
        }
        _ => unsupported.push(format!("mt_provider={:?}", cfg.mt_provider)),
    }

    if unsupported.is_empty() {
        None
    } else {
        Some(format!(
            "Some saved provider settings are not available in this build ({}). Set unsupported providers to \"google\", save, and restart.",
            unsupported.join(", ")
        ))
    }
}

fn missing_google_api_key_error(cfg: &config::AppConfig) -> Option<String> {
    if cfg.google_api_key.is_some() {
        return None;
    }
    if cfg.stt_provider == "google" && cfg.mt_provider == "google" && !cfg.tts_enabled {
        return None;
    }

    let mut requires_key = Vec::new();
    if cfg.stt_provider == "google" {
        requires_key.push("Google STT");
    }
    if cfg.mt_provider == "google" {
        requires_key.push("Google Translation");
    }
    if cfg.tts_enabled {
        requires_key.push("Google TTS");
    }

    (!requires_key.is_empty()).then(|| {
        let verb = if requires_key.len() == 1 {
            "requires"
        } else {
            "require"
        };
        let mut actions = vec!["add google_api_key".to_string()];
        let mut local_switches = Vec::new();
        if cfg.stt_provider == "google" {
            local_switches.push("stt_provider");
        }
        if cfg.mt_provider == "google" {
            local_switches.push("mt_provider");
        }
        if !local_switches.is_empty() {
            actions.push(format!(
                "switch {} to \"local\"",
                local_switches.join(" and ")
            ));
        }
        if cfg.tts_enabled {
            actions.push("disable translated audio".to_string());
        }

        format!(
            "{} {verb} google_api_key. {}, then save and restart.",
            requires_key.join(" and "),
            actions.join(", or ")
        )
    })
}

fn save_config_editor(
    state: &AppState,
    cfg_path: &Path,
    current_config: &Arc<Mutex<config::AppConfig>>,
    restart_required: &Arc<AtomicBool>,
    playback_service: &SharedPlaybackService,
) -> Result<()> {
    let editor = state
        .config_editor_snapshot()
        .context("config editor save requested with no active editor")?;
    validate_onboarding_editor(&editor)?;
    let next_cfg = {
        let current = current_config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let mut next = build_config_from_editor(&editor, &current)?;
        config::apply_editor_defaults(cfg_path, &mut next)?;
        next
    };

    config::write_config(cfg_path, &next_cfg)?;
    apply_runtime_config(
        current_config,
        &state.target_language,
        &state.source_language,
        &state.capture_device_label,
        &state.tts_enabled,
        restart_required,
        playback_service,
        next_cfg,
    );
    state.close_config_editor();
    tracing::info!(path = %cfg_path.display(), mode = ?editor.mode, "config saved from UI");
    Ok(())
}

fn sync_playback_service_state(
    playback_service: &SharedPlaybackService,
    config: &config::AppConfig,
    enabled: bool,
) -> bool {
    let mut service_slot = playback_service.lock().unwrap_or_else(|p| p.into_inner());

    if let Some(service) = service_slot.as_ref() {
        service.set_enabled(enabled);
        return true;
    }

    if !enabled {
        return true;
    }

    match pipeline::playback::PlaybackService::new(enabled, config.tts_output_device.as_deref()) {
        Ok(service) => {
            *service_slot = Some(service);
            true
        }
        Err(err) => {
            tracing::warn!("TTS playback unavailable: {err}");
            false
        }
    }
}

fn handle_audio_chunk(
    chunk: audio::AudioChunk,
    paused: &Arc<AtomicBool>,
    level_tx: &Arc<AtomicU32>,
    session_metrics: &Arc<Mutex<metrics::SessionMetrics>>,
    cost_counter: &Arc<metrics::CostCounter>,
) {
    if paused.load(Ordering::Relaxed) {
        level_tx.store(0, Ordering::Relaxed);
        return;
    }

    let encoded = (chunk.rms_energy().clamp(0.0, 1.0) * AUDIO_LEVEL_SCALE as f32) as u32;
    level_tx.store(encoded, Ordering::Relaxed);

    let audio_secs = f64::from(chunk.duration_ms) / 1000.0;
    session_metrics
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .record_audio_seconds_sent(audio_secs);
    // Record STT usage in the shared CostCounter (issues #71–#76).
    // MT and TTS providers will call their respective record_* methods when wired.
    cost_counter.record_audio_seconds(audio_secs);
}

fn mark_audio_capture_stopped(
    level_tx: &Arc<AtomicU32>,
    stt_state: &Arc<Mutex<metrics::SttState>>,
) {
    level_tx.store(0, Ordering::Relaxed);
    *stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
        metrics::SttState::Error("audio capture stopped".to_string());
}

fn metrics_warning_row_active(
    expanded: bool,
    cost_warning_usd: f64,
    metrics: &MetricsSnapshot,
) -> bool {
    expanded
        && ((cost_warning_usd > 0.0 && metrics.estimated_cost_usd > cost_warning_usd)
            || metrics.ram_warning)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiInteractionMode {
    Live,
    ReplayReadOnly,
}

impl TuiInteractionMode {
    fn blocks_action(self, action: &UserAction) -> bool {
        self == Self::ReplayReadOnly
            && matches!(
                action,
                UserAction::OpenSettings
                    | UserAction::ConfigChar(_)
                    | UserAction::ConfigBackspace
                    | UserAction::ConfigInput(_)
                    | UserAction::ConfigNextField
                    | UserAction::ConfigPrevField
                    | UserAction::ConfigSave
                    | UserAction::ConfigCycleCaptureDevice
                    | UserAction::ReloadConfig
                    | UserAction::ToggleTts
            )
    }
}

fn ignore_replay_side_effect_action(state: &AppState, action: &UserAction) {
    tracing::info!(?action, "ignored side-effecting action in replay mode");
    *state
        .pipeline_error_msg
        .lock()
        .unwrap_or_else(|p| p.into_inner()) =
        Some("settings and TTS are read-only in replay mode".to_string());
    state.close_config_editor();
}

struct TuiRuntimeContext<'a> {
    restart_required: &'a Arc<AtomicBool>,
    cfg_path: &'a Path,
    current_config: &'a Arc<Mutex<config::AppConfig>>,
    playback_service: &'a SharedPlaybackService,
    interaction_mode: TuiInteractionMode,
}

/// Run the terminal interface.  Enters the alternate screen, runs the event
/// loop, then returns.  The [`TerminalGuard`] restores the terminal on drop.
fn run_tui(
    state: &AppState,
    context: &TuiRuntimeContext<'_>,
    keyboard_shutdown: &Arc<AtomicBool>,
    key_rx: mpsc::Receiver<UserAction>,
) -> Result<()> {
    let mut terminal_guard = TerminalGuard::enter()?;
    let result = event_loop(terminal_guard.terminal_mut(), state, context, key_rx);
    keyboard_shutdown.store(true, Ordering::Relaxed);
    result
}

/// Main event loop: draw the UI, then process key actions from the keyboard
/// task channel.
///
/// The loop runs at approximately 20 fps (50 ms sleep between draws).
/// Key actions arrive on `key_rx` from the dedicated keyboard task (issue #63).
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &AppState,
    context: &TuiRuntimeContext<'_>,
    key_rx: mpsc::Receiver<UserAction>,
) -> Result<()> {
    loop {
        // ── Issue #88 — check for Windows forced-close signal ─────────────────
        if FORCED_SHUTDOWN.load(Ordering::Relaxed) {
            tracing::info!("forced shutdown signal received; exiting event loop (issue #88)");
            break;
        }

        let expanded = state.metrics_expanded.load(Ordering::Relaxed);
        let show_restart = context.restart_required.load(Ordering::Relaxed);
        let cost_warning_usd = context
            .current_config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cost_warning_usd;
        let metrics_snap = state.metrics_snapshot();
        let over_threshold = metrics_warning_row_active(expanded, cost_warning_usd, &metrics_snap);
        let pane_area = subtitle_inner_area(terminal.size()?, expanded, over_threshold);

        {
            let mut pane = state
                .subtitle_pane
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            pane.clamp_scroll(pane_area.width, pane_area.height);
        }

        let level = state.level_ratio();
        terminal.draw(|frame| {
            draw_ui(frame, state, level, show_restart, cost_warning_usd);
        })?;

        // Drain all pending key actions without blocking.
        let mut should_quit = false;
        loop {
            match key_rx.try_recv() {
                Ok(UserAction::Quit) => {
                    should_quit = true;
                }
                Ok(UserAction::AnyKey) => {}
                Ok(action) => {
                    if context.interaction_mode.blocks_action(&action) {
                        ignore_replay_side_effect_action(state, &action);
                        continue;
                    }
                    let terminal_area = terminal.size()?;
                    handle_action(
                        &action,
                        state,
                        terminal_area,
                        context.restart_required,
                        context.cfg_path,
                        context.current_config,
                        context.playback_service,
                    );
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Keyboard task exited; treat as quit.
                    should_quit = true;
                    break;
                }
            }
        }

        if should_quit {
            tracing::info!("user requested shutdown");
            // Draw the in-TUI session summary overlay, then wait for any key.
            terminal.draw(|frame| {
                draw_session_summary(frame, state, show_restart);
            })?;
            loop {
                match key_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            break;
        }

        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

// ── Issue #63: keyboard task ──────────────────────────────────────────────────

/// Translate a raw crossterm [`KeyEvent`] into a [`UserAction`].
///
/// `in_lang_prompt` and `in_config_editor` route character input to the active
/// overlay instead of the normal command set.
fn key_to_action(
    key: &KeyEvent,
    in_lang_prompt: bool,
    in_config_editor: bool,
) -> Option<UserAction> {
    if in_config_editor {
        return match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::Quit)
            }
            KeyCode::Enter => Some(UserAction::ConfigSave),
            KeyCode::Esc => Some(UserAction::DismissOverlay),
            KeyCode::Backspace => Some(UserAction::ConfigBackspace),
            KeyCode::Delete => Some(UserAction::ConfigInput(InputRequest::DeleteNextChar)),
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::ConfigInput(InputRequest::GoToPrevWord))
            }
            KeyCode::Left => Some(UserAction::ConfigInput(InputRequest::GoToPrevChar)),
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::ConfigInput(InputRequest::GoToNextWord))
            }
            KeyCode::Right => Some(UserAction::ConfigInput(InputRequest::GoToNextChar)),
            KeyCode::Home => Some(UserAction::ConfigInput(InputRequest::GoToStart)),
            KeyCode::End => Some(UserAction::ConfigInput(InputRequest::GoToEnd)),
            KeyCode::Tab | KeyCode::Down => Some(UserAction::ConfigNextField),
            KeyCode::BackTab | KeyCode::Up => Some(UserAction::ConfigPrevField),
            KeyCode::F(2) => Some(UserAction::ConfigCycleCaptureDevice),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::ConfigCycleCaptureDevice)
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::ConfigSave)
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::ConfigInput(InputRequest::GoToStart))
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::ConfigInput(InputRequest::GoToEnd))
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::ConfigInput(InputRequest::GoToPrevChar))
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::ConfigInput(InputRequest::GoToNextChar))
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::ConfigInput(InputRequest::DeleteLine))
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::ConfigInput(InputRequest::DeleteTillEnd))
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::ConfigInput(InputRequest::DeletePrevWord))
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::ConfigChar(c))
            }
            _ => Some(UserAction::AnyKey),
        };
    }
    if in_lang_prompt {
        return match key.code {
            KeyCode::Enter => Some(UserAction::LangApply),
            KeyCode::Esc => Some(UserAction::LangCancel),
            KeyCode::Backspace => Some(UserAction::LangBackspace),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::LangChar(c))
            }
            _ => Some(UserAction::AnyKey),
        };
    }
    match key.code {
        // Quit (issue #64)
        KeyCode::Char('q') | KeyCode::Char('Q') => Some(UserAction::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UserAction::Quit)
        }
        // Overlay dismissal — Escape (issue #64)
        KeyCode::Esc => Some(UserAction::DismissOverlay),
        // Commands (issue #64)
        KeyCode::Char(' ') => Some(UserAction::TogglePause),
        KeyCode::Char('t') | KeyCode::Char('T') => Some(UserAction::ToggleTts),
        KeyCode::Char('m') | KeyCode::Char('M') => Some(UserAction::ToggleMetrics),
        KeyCode::Char('l') | KeyCode::Char('L') => Some(UserAction::PromptLanguage),
        KeyCode::Char('s') | KeyCode::Char('S') => Some(UserAction::OpenSettings),
        KeyCode::Char('r') | KeyCode::Char('R') => Some(UserAction::ReloadConfig),
        KeyCode::Char('?') => Some(UserAction::ToggleHelp),
        // Scrolling
        KeyCode::Up => Some(UserAction::ScrollUp),
        KeyCode::Down => Some(UserAction::ScrollDown),
        KeyCode::Home => Some(UserAction::ScrollTop),
        KeyCode::End => Some(UserAction::ScrollBottom),
        _ => Some(UserAction::AnyKey),
    }
}

/// Blocking keyboard reader managed by Tokio (issue #63).
///
/// Runs forever on a dedicated OS thread (via `tokio::task::spawn_blocking`).
/// Converts every crossterm key event to a [`UserAction`] and sends it to the
/// event loop via `key_tx`. Uses `event::poll()` so shutdown is observed even
/// while no actionable key is pressed.
fn keyboard_task(
    key_tx: mpsc::Sender<UserAction>,
    lang_prompt_active: Arc<AtomicBool>,
    config_editor_active: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Relaxed) {
        match event::poll(Duration::from_millis(100)) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(_) => break,
        }

        match event::read() {
            Ok(Event::Key(key)) => {
                if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    continue;
                }
                let in_lang_prompt = lang_prompt_active.load(Ordering::Relaxed);
                let in_config_editor = config_editor_active.load(Ordering::Relaxed);
                if let Some(action) = key_to_action(&key, in_lang_prompt, in_config_editor) {
                    if key_tx.send(action).is_err() {
                        // Receiver dropped; app is shutting down.
                        break;
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

// ── Issue #64: action handler ─────────────────────────────────────────────────

/// Execute a [`UserAction`] against the shared application state.
fn handle_action(
    action: &UserAction,
    state: &AppState,
    terminal_area: ratatui::layout::Rect,
    restart_required: &Arc<AtomicBool>,
    cfg_path: &Path,
    current_config: &Arc<Mutex<config::AppConfig>>,
    playback_service: &SharedPlaybackService,
) {
    match action {
        // Scrolling — when the help overlay is open, ↑/↓/Home/End scroll the
        // help panel instead of the subtitle pane (issue #191).
        UserAction::ScrollUp => {
            if state.show_help.load(Ordering::Relaxed) {
                state.scroll_help_up(u32::from(help_overlay_max_scroll(terminal_area)));
            } else {
                let expanded = state.metrics_expanded.load(Ordering::Relaxed);
                let cost_warning_usd = current_config
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .cost_warning_usd;
                let metrics_snap = state.metrics_snapshot();
                let over_threshold =
                    metrics_warning_row_active(expanded, cost_warning_usd, &metrics_snap);
                let pane_area = subtitle_inner_area(terminal_area, expanded, over_threshold);
                state
                    .subtitle_pane
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .scroll_up(pane_area.width, pane_area.height);
            }
        }
        UserAction::ScrollDown => {
            if state.show_help.load(Ordering::Relaxed) {
                state.scroll_help_down(u32::from(help_overlay_max_scroll(terminal_area)));
            } else {
                let expanded = state.metrics_expanded.load(Ordering::Relaxed);
                let cost_warning_usd = current_config
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .cost_warning_usd;
                let metrics_snap = state.metrics_snapshot();
                let over_threshold =
                    metrics_warning_row_active(expanded, cost_warning_usd, &metrics_snap);
                let pane_area = subtitle_inner_area(terminal_area, expanded, over_threshold);
                state
                    .subtitle_pane
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .scroll_down(pane_area.width, pane_area.height);
            }
        }
        UserAction::ScrollTop => {
            if state.show_help.load(Ordering::Relaxed) {
                state.scroll_help_to_top();
            } else {
                let expanded = state.metrics_expanded.load(Ordering::Relaxed);
                let cost_warning_usd = current_config
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .cost_warning_usd;
                let metrics_snap = state.metrics_snapshot();
                let over_threshold =
                    metrics_warning_row_active(expanded, cost_warning_usd, &metrics_snap);
                let pane_area = subtitle_inner_area(terminal_area, expanded, over_threshold);
                state
                    .subtitle_pane
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .scroll_to_top(pane_area.width, pane_area.height);
            }
        }
        UserAction::ScrollBottom => {
            if state.show_help.load(Ordering::Relaxed) {
                state.scroll_help_to_bottom(u32::from(help_overlay_max_scroll(terminal_area)));
            } else {
                state
                    .subtitle_pane
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .scroll_to_bottom();
            }
        }

        // Space — pause / resume (issue #64)
        UserAction::TogglePause => {
            let v = state.paused.load(Ordering::Relaxed);
            state.paused.store(!v, Ordering::Relaxed);
            tracing::info!("translation {}", if !v { "paused" } else { "resumed" });
        }

        // T — toggle TTS (issue #58)
        UserAction::ToggleTts => {
            let was_enabled = state.tts_enabled.load(Ordering::Relaxed);
            let want_enabled = !was_enabled;
            let current = current_config
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            // Sync the backend first; only update the UI flag to reflect what succeeded.
            let service_ok = sync_playback_service_state(playback_service, &current, want_enabled);
            let actual = want_enabled && service_ok;
            state.tts_enabled.store(actual, Ordering::Relaxed);
            tracing::info!("TTS toggled: {}", if actual { "on" } else { "off" });
        }

        // M — expand / collapse metrics (issue #41)
        UserAction::ToggleMetrics => {
            state.toggle_metrics();
        }

        // ? — show / hide help (issue #66, #191)
        UserAction::ToggleHelp => {
            let show_help = !state.show_help.load(Ordering::Relaxed);
            state.show_help.store(show_help, Ordering::Relaxed);
            if show_help {
                // Reset scroll so the overlay always opens at the top.
                state.reset_help_scroll();
                state.lang_prompt_active.store(false, Ordering::Relaxed);
                *state.lang_input.lock().unwrap_or_else(|p| p.into_inner()) = String::new();
                state.close_config_editor();
            }
        }

        // Escape — dismiss open overlay (issue #64)
        UserAction::DismissOverlay => {
            if state.config_editor_active.load(Ordering::Relaxed) {
                let onboarding = state
                    .with_config_editor_mut(|editor| {
                        if editor.mode == ConfigEditorMode::Onboarding {
                            editor.set_status_message(
                                " Setup is still required. Fill the fields and press Enter to save.",
                            );
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if !onboarding {
                    state.close_config_editor();
                }
            } else if state.lang_prompt_active.load(Ordering::Relaxed) {
                state.lang_prompt_active.store(false, Ordering::Relaxed);
                *state.lang_input.lock().unwrap_or_else(|p| p.into_inner()) = String::new();
            } else if state.show_help.load(Ordering::Relaxed) {
                state.show_help.store(false, Ordering::Relaxed);
                state.reset_help_scroll();
            }
        }

        // L — open language prompt (issue #64)
        UserAction::PromptLanguage => {
            if !state.lang_prompt_active.load(Ordering::Relaxed)
                && !state.config_editor_active.load(Ordering::Relaxed)
            {
                state.show_help.store(false, Ordering::Relaxed);
                *state.lang_input.lock().unwrap_or_else(|p| p.into_inner()) = String::new();
                state.lang_prompt_active.store(true, Ordering::Relaxed);
            }
        }

        UserAction::OpenSettings => {
            let current = current_config
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            state.open_config_editor(ConfigEditorMode::Settings, &current, cfg_path);
            populate_capture_device_options(state);
        }

        // Language prompt input (issue #64)
        UserAction::LangChar(c) => {
            state
                .lang_input
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(*c);
        }
        UserAction::LangBackspace => {
            state
                .lang_input
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .pop();
        }
        UserAction::LangApply => {
            let input = state
                .lang_input
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            let next_language = input.trim();
            if !next_language.is_empty() {
                match config::validate_language_code(next_language) {
                    Ok(()) => {
                        state.set_target_language(next_language.to_string());
                        {
                            let mut current =
                                current_config.lock().unwrap_or_else(|p| p.into_inner());
                            current.target_language = next_language.to_string();
                        }
                        tracing::info!("target language changed to {next_language}");
                        state.lang_prompt_active.store(false, Ordering::Relaxed);
                        *state.lang_input.lock().unwrap_or_else(|p| p.into_inner()) = String::new();
                    }
                    Err(err) => {
                        tracing::warn!("invalid target language entered from prompt: {err}");
                    }
                }
            }
        }
        UserAction::LangCancel => {
            state.lang_prompt_active.store(false, Ordering::Relaxed);
            *state.lang_input.lock().unwrap_or_else(|p| p.into_inner()) = String::new();
        }

        UserAction::ConfigChar(c) => {
            let _ = state.with_config_editor_mut(|editor| editor.push_char(*c));
        }
        UserAction::ConfigBackspace => {
            let _ = state.with_config_editor_mut(|editor| editor.backspace());
        }
        UserAction::ConfigInput(request) => {
            let _ = state.with_config_editor_mut(|editor| editor.handle_input_request(*request));
        }
        UserAction::ConfigNextField => {
            let _ = state.with_config_editor_mut(|editor| editor.next_field());
        }
        UserAction::ConfigPrevField => {
            let _ = state.with_config_editor_mut(|editor| editor.prev_field());
        }
        UserAction::ConfigCycleCaptureDevice => {
            let _ = state.with_config_editor_mut(|editor| editor.cycle_active_field());
        }
        UserAction::ConfigSave => {
            if let Err(err) = save_config_editor(
                state,
                cfg_path,
                current_config,
                restart_required,
                playback_service,
            ) {
                tracing::warn!("config save requested from UI failed: {err:#}");
                let _ = state.with_config_editor_mut(|editor| {
                    editor.set_status_message(config_save_error_status(&err))
                });
            }
        }

        UserAction::AnyKey => {}

        // R — signal config reload (issue #64)
        UserAction::ReloadConfig => match config::load(cfg_path) {
            Ok(next_cfg) => {
                apply_runtime_config(
                    current_config,
                    &state.target_language,
                    &state.source_language,
                    &state.capture_device_label,
                    &state.tts_enabled,
                    restart_required,
                    playback_service,
                    next_cfg,
                );
                // Issue #86 — auth-error recovery requires a full restart.
                // Providers still hold the old (possibly invalid) credential
                // in-process; no un-halt is possible here regardless of
                // whether the key changed.  The banner and pipeline_halted
                // flag stay set until the application is restarted.
                tracing::info!("config reloaded from {}", cfg_path.display());
            }
            Err(err) => {
                tracing::warn!("config reload requested with R key failed: {err:#}");
            }
        },

        // Quit is handled in the outer loop, not here.
        UserAction::Quit => {}
    }
}

fn config_save_error_status(err: &anyhow::Error) -> String {
    format!(" Save failed: {}", single_line_error_chain(err))
}

fn single_line_error_chain(err: &anyhow::Error) -> String {
    let parts: Vec<String> = err
        .chain()
        .map(|cause| cause.to_string())
        .map(|part| part.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        "unknown error".to_string()
    } else {
        parts.join(": ")
    }
}

// ── Issue #64: stdout session summary ────────────────────────────────────────

/// Print the session summary to stdout after the alternate screen is left.
///
/// This satisfies the requirement that Q/Ctrl+C "restores the terminal and
/// prints a session summary to stdout before exiting with code 0".
fn print_session_summary_to_stdout(state: &AppState) {
    let metrics = state.metrics_snapshot();
    let pair_count = state
        .subtitle_pane
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .pair_count();
    let tts_on = state.tts_enabled.load(Ordering::Relaxed);

    println!();
    println!(
        "\u{2500}\u{2500}\u{2500} TUI Translator \u{2014} Session Summary \u{2500}\u{2500}\u{2500}"
    );
    println!("  Duration:        {}", metrics.format_elapsed());
    println!("  Subtitle pairs:  {pair_count}");
    println!("  Audio processed: {:.0}s", metrics.audio_seconds_sent);
    println!("  MT input chars:  {}", metrics.chars_translated);
    println!(
        "  Estimated cost:  {}",
        metrics::cost::format_cost_or_zero_state(metrics.estimated_cost_usd)
    );
    println!("  TTS output:      {}", if tts_on { "on" } else { "off" });
    println!("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
    println!();
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        // Set up the terminal in raw mode so we can read individual key presses.
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }

        let backend = CrosstermBackend::new(stdout);
        let terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(error) => {
                let _ = disable_raw_mode();
                let mut cleanup_stdout = io::stdout();
                let _ = execute!(cleanup_stdout, LeaveAlternateScreen);
                return Err(error.into());
            }
        };
        Ok(Self { terminal })
    }

    fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<io::Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let raw_mode_result = disable_raw_mode();
        let leave_screen_result = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let cursor_result = self.terminal.show_cursor();

        if let Err(error) = raw_mode_result {
            eprintln!("tui-translator cleanup warning: failed to disable raw mode: {error}");
        }
        if let Err(error) = leave_screen_result {
            eprintln!("tui-translator cleanup warning: failed to leave alternate screen: {error}");
        }
        if let Err(error) = cursor_result {
            eprintln!("tui-translator cleanup warning: failed to show cursor: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioChunk;
    use crate::config::test_env::{EnvVarGuard, ENV_LOCK};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn metrics_warning_row_active_for_ram_warning() {
        let metrics = MetricsSnapshot {
            ram_warning: true,
            ..MetricsSnapshot::default()
        };
        assert!(
            metrics_warning_row_active(true, 0.0, &metrics),
            "expanded layout must reserve the warning row for RAM pressure"
        );
        assert!(
            !metrics_warning_row_active(false, 0.0, &metrics),
            "compact layout height must stay fixed even when RAM warning is active"
        );
    }

    #[test]
    fn startup_config_mode_missing_home_config_requires_onboarding() {
        assert_eq!(
            startup_config_mode(config::LoadState::Missing, false, false),
            StartupConfigMode::OnboardingRequired
        );
    }

    #[test]
    fn startup_config_mode_explicit_config_override_bypasses_onboarding() {
        assert_eq!(
            startup_config_mode(config::LoadState::Missing, true, false),
            StartupConfigMode::Normal
        );
    }

    #[test]
    fn startup_config_mode_invalid_config_requires_repair() {
        assert_eq!(
            startup_config_mode(config::LoadState::Invalid, false, false),
            StartupConfigMode::ConfigRecoveryRequired
        );
    }

    #[test]
    fn startup_config_mode_skip_flag_bypasses_interactive_startup() {
        assert_eq!(
            startup_config_mode(config::LoadState::Missing, false, true),
            StartupConfigMode::Normal
        );
        assert_eq!(
            startup_config_mode(config::LoadState::Invalid, false, true),
            StartupConfigMode::Normal
        );
    }

    #[test]
    fn replay_mode_blocks_settings_reload_and_tts_actions() {
        let blocked = [
            UserAction::OpenSettings,
            UserAction::ConfigChar('x'),
            UserAction::ConfigBackspace,
            UserAction::ConfigNextField,
            UserAction::ConfigPrevField,
            UserAction::ConfigSave,
            UserAction::ConfigCycleCaptureDevice,
            UserAction::ReloadConfig,
            UserAction::ToggleTts,
        ];

        for action in blocked {
            assert!(
                TuiInteractionMode::ReplayReadOnly.blocks_action(&action),
                "replay mode must block side-effecting action {action:?}"
            );
            assert!(
                !TuiInteractionMode::Live.blocks_action(&action),
                "live mode must keep existing action behavior for {action:?}"
            );
        }

        assert!(
            !TuiInteractionMode::ReplayReadOnly.blocks_action(&UserAction::PromptLanguage),
            "replay mode can still allow in-memory language prompt changes"
        );
    }

    #[test]
    fn write_audio_devices_shows_default_and_detected_devices() {
        let devices = vec![
            audio::CaptureDeviceInfo {
                id: "{0.0.0.00000000}.{speakers}".to_string(),
                name: "Speakers (Realtek Audio)".to_string(),
                is_default: true,
            },
            audio::CaptureDeviceInfo {
                id: "{0.0.0.00000000}.{headphones}".to_string(),
                name: "Headphones (USB Audio)".to_string(),
                is_default: false,
            },
        ];
        let mut output = Vec::new();

        write_audio_devices(&mut output, &devices).unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(rendered.contains("leave capture_device blank"));
        assert!(rendered.contains("Speakers (Realtek Audio) (current Windows default)"));
        assert!(rendered.contains("endpoint_id: {0.0.0.00000000}.{speakers}"));
        assert!(rendered.contains("Headphones (USB Audio)"));
        assert!(rendered.contains("endpoint_id: {0.0.0.00000000}.{headphones}"));
    }

    #[test]
    fn parse_local_mt_model_install_args_accepts_manifest_directory_and_yes() {
        let parsed = parse_local_mt_model_install_args_from(vec![
            OsString::from("--install-local-mt-model"),
            OsString::from(r"C:\models\manifest.json"),
            OsString::from("--local-mt-model-dir"),
            OsString::from(r"C:\models\opus-mt-ja-vi"),
            OsString::from("--yes"),
        ])
        .unwrap()
        .expect("install args should be detected");

        assert_eq!(parsed.manifest, PathBuf::from(r"C:\models\manifest.json"));
        assert_eq!(
            parsed.model_dir,
            Some(PathBuf::from(r"C:\models\opus-mt-ja-vi"))
        );
        assert!(parsed.yes);
    }

    #[test]
    fn parse_local_mt_model_install_args_requires_manifest_path() {
        let error = parse_local_mt_model_install_args_from(vec![
            OsString::from("--install-local-mt-model"),
            OsString::from("--yes"),
        ])
        .expect_err("missing manifest path should be rejected");

        assert!(error.to_string().contains("--install-local-mt-model"));
    }

    #[test]
    fn parse_local_mt_model_install_args_ignores_auxiliary_flag_without_install_command() {
        let parsed = parse_local_mt_model_install_args_from(vec![
            OsString::from("--local-mt-model-dir"),
            OsString::from(r"C:\models\opus-mt-ja-vi"),
            OsString::from("--replay-session"),
            OsString::from("meeting.jsonl"),
        ])
        .unwrap();

        assert!(parsed.is_none());
    }

    #[test]
    fn parse_local_stt_model_prefetch_args_accepts_model_cache_and_yes() {
        let parsed = parse_local_stt_model_prefetch_args_from(vec![
            OsString::from("--prefetch-local-stt-model"),
            OsString::from("tiny"),
            OsString::from("--model-cache-dir"),
            OsString::from(r"C:\models\whisper"),
            OsString::from("-y"),
        ])
        .unwrap()
        .expect("prefetch args should be detected");

        assert_eq!(
            parsed.source,
            LocalSttModelPrefetchSource::BuiltinModel(providers::local::ModelId::Tiny)
        );
        assert_eq!(
            parsed.model_cache_dir,
            Some(PathBuf::from(r"C:\models\whisper"))
        );
        assert!(parsed.yes);
    }

    #[test]
    fn parse_local_stt_model_prefetch_args_rejects_unknown_model() {
        let error = parse_local_stt_model_prefetch_args_from(vec![
            OsString::from("--prefetch-local-stt-model"),
            OsString::from("large"),
        ])
        .expect_err("unknown local STT model should be rejected");

        assert!(error.to_string().contains("supported values"));
    }

    #[test]
    fn parse_local_stt_model_prefetch_args_accepts_manifest_source() {
        let parsed = parse_local_stt_model_prefetch_args_from(vec![
            OsString::from("--prefetch-local-stt-manifest"),
            OsString::from(r"C:\models\whisper-manifest.json"),
            OsString::from("--yes"),
        ])
        .unwrap()
        .expect("prefetch args should be detected");

        assert_eq!(
            parsed.source,
            LocalSttModelPrefetchSource::Manifest(PathBuf::from(
                r"C:\models\whisper-manifest.json"
            ))
        );
        assert!(parsed.yes);
    }

    #[test]
    fn parse_local_stt_model_prefetch_args_rejects_duplicate_sources() {
        let error = parse_local_stt_model_prefetch_args_from(vec![
            OsString::from("--prefetch-local-stt-model"),
            OsString::from("tiny"),
            OsString::from("--prefetch-local-stt-manifest"),
            OsString::from(r"C:\models\whisper-manifest.json"),
        ])
        .expect_err("duplicate local STT source should be rejected");

        assert!(error.to_string().contains("use only one"));
    }

    #[test]
    fn parse_local_stt_model_prefetch_args_rejects_unknown_flag_before_source() {
        let error = parse_local_stt_model_prefetch_args_from(vec![
            OsString::from("--model-cache-dri"),
            OsString::from(r"C:\models\wrong"),
            OsString::from("--prefetch-local-stt-model"),
            OsString::from("tiny"),
        ])
        .expect_err("unknown prefetch flag before source should be rejected");

        assert!(error
            .to_string()
            .contains("unknown local STT model prefetch argument"));
    }

    #[test]
    fn parse_local_stt_model_prefetch_args_ignores_auxiliary_flag_without_command() {
        let parsed = parse_local_stt_model_prefetch_args_from(vec![
            OsString::from("--model-cache-dir"),
            OsString::from(r"C:\models\whisper"),
            OsString::from("--replay-session"),
            OsString::from("meeting.jsonl"),
        ])
        .unwrap();

        assert!(parsed.is_none());
    }

    #[test]
    fn validate_local_stt_bundle_manifest_accepts_builtin_tiny_manifest() {
        let spec = providers::local::ModelManifest::builtin()
            .find(providers::local::ModelId::Tiny)
            .unwrap();
        let manifest = providers::local::stt_model_bundle_manifest(spec);

        validate_local_stt_bundle_manifest(&manifest).unwrap();
    }

    #[test]
    fn validate_local_stt_bundle_manifest_rejects_checksum_mismatch() {
        let spec = providers::local::ModelManifest::builtin()
            .find(providers::local::ModelId::Tiny)
            .unwrap();
        let mut manifest = providers::local::stt_model_bundle_manifest(spec);
        manifest.files[0].sha256 =
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();

        let error = validate_local_stt_bundle_manifest(&manifest)
            .expect_err("mismatched STT manifest checksum should be rejected");

        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn parse_session_export_args_accepts_required_flags() {
        let parsed = parse_session_export_args_from(vec![
            OsString::from("--export-session"),
            OsString::from(r"C:\sessions\meeting.jsonl"),
            OsString::from("--export-format"),
            OsString::from("srt"),
            OsString::from("--export-output"),
            OsString::from(r"C:\exports\meeting.srt"),
        ])
        .unwrap()
        .expect("export args should be detected");

        assert_eq!(parsed.input, PathBuf::from(r"C:\sessions\meeting.jsonl"));
        assert_eq!(parsed.output, PathBuf::from(r"C:\exports\meeting.srt"));
        assert_eq!(parsed.format, SessionExportFormat::Srt);
    }

    #[test]
    fn parse_session_export_args_requires_output_path() {
        let error = parse_session_export_args_from(vec![
            OsString::from("--export-session"),
            OsString::from("meeting.jsonl"),
            OsString::from("--export-format"),
            OsString::from("txt"),
        ])
        .expect_err("missing output path should be rejected");

        assert!(error.to_string().contains("--export-output"));
    }

    #[test]
    fn run_session_export_writes_srt_from_jsonl() {
        let temp = TempDir::new().unwrap();
        let input = temp.path().join("meeting.jsonl");
        let output = temp.path().join("meeting.srt");
        fs::write(
            &input,
            include_str!("../tests/fixtures/session_log_v1.jsonl"),
        )
        .unwrap();

        run_session_export(&SessionExportArgs {
            input,
            output: output.clone(),
            format: SessionExportFormat::Srt,
        })
        .unwrap();

        let exported = fs::read_to_string(output).unwrap();
        assert!(exported.starts_with("1\n"));
        assert!(exported.contains("00:00:00,500 --> 00:00:02,000"));
        assert!(exported.contains("おはようございます"));
        assert!(exported.contains("Xin chào buổi sáng"));
    }

    #[test]
    fn f2_cycles_capture_device_in_config_editor() {
        let key = KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE);

        assert_eq!(
            key_to_action(&key, false, true),
            Some(UserAction::ConfigCycleCaptureDevice)
        );
    }

    #[test]
    fn lang_apply_updates_runtime_target_language() {
        let state = AppState::new();
        state.set_target_language("vi");
        state.lang_prompt_active.store(true, Ordering::Relaxed);
        *state.lang_input.lock().unwrap() = "en-US".to_string();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        handle_action(
            &UserAction::LangApply,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            Path::new("config.json"),
            &current_config,
            &playback_service,
        );

        assert_eq!(state.target_language(), "en-US");
        assert_eq!(current_config.lock().unwrap().target_language, "en-US");
        assert!(!state.lang_prompt_active.load(Ordering::Relaxed));
        assert!(state.lang_input.lock().unwrap().is_empty());
    }

    #[test]
    fn lang_apply_rejects_invalid_target_language() {
        let state = AppState::new();
        state.set_target_language("vi");
        state.lang_prompt_active.store(true, Ordering::Relaxed);
        *state.lang_input.lock().unwrap() = "ja-JPdas".to_string();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        handle_action(
            &UserAction::LangApply,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            Path::new("config.json"),
            &current_config,
            &playback_service,
        );

        assert_eq!(state.target_language(), "vi");
        assert_eq!(current_config.lock().unwrap().target_language, "vi");
        assert!(state.lang_prompt_active.load(Ordering::Relaxed));
        assert_eq!(state.lang_input.lock().unwrap().as_str(), "ja-JPdas");
    }

    #[test]
    fn reload_config_applies_tts_and_target_language() {
        let mut config_file = NamedTempFile::new().unwrap();
        write!(
            config_file,
            r#"{{"source_language":"ja-JP","target_language":"en","tts_enabled":true}}"#
        )
        .unwrap();

        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        handle_action(
            &UserAction::ReloadConfig,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            config_file.path(),
            &current_config,
            &playback_service,
        );

        assert_eq!(state.target_language(), "en");
        assert_eq!(current_config.lock().unwrap().target_language, "en");

        // The UI flag mirrors whether the PlaybackService actually started.
        // On CI runners without an audio output device the service startup
        // fails and the flag stays false; on hardware where it succeeds the
        // flag becomes true.  Either way the flag must be consistent with the
        // service slot rather than blindly reflecting config intent.
        let service_running = playback_service.lock().unwrap().is_some();
        assert_eq!(
            state.tts_enabled.load(Ordering::Relaxed),
            service_running,
            "tts_enabled UI flag must match whether PlaybackService actually started"
        );
    }

    #[test]
    fn paused_audio_chunk_is_dropped_without_updating_metrics() {
        let paused = Arc::new(AtomicBool::new(true));
        let level_tx = Arc::new(AtomicU32::new(42));
        let session_metrics = Arc::new(Mutex::new(metrics::SessionMetrics::default()));

        handle_audio_chunk(
            AudioChunk::new(vec![i16::MAX; 160]),
            &paused,
            &level_tx,
            &session_metrics,
            &Arc::new(metrics::CostCounter::new()),
        );

        assert_eq!(level_tx.load(Ordering::Relaxed), 0);
        assert_eq!(
            session_metrics
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .audio_seconds_sent,
            0.0
        );
    }

    #[test]
    fn active_audio_chunk_updates_level_and_metrics() {
        let paused = Arc::new(AtomicBool::new(false));
        let level_tx = Arc::new(AtomicU32::new(0));
        let session_metrics = Arc::new(Mutex::new(metrics::SessionMetrics::default()));

        handle_audio_chunk(
            AudioChunk::new(vec![i16::MAX; 160]),
            &paused,
            &level_tx,
            &session_metrics,
            &Arc::new(metrics::CostCounter::new()),
        );

        assert!(level_tx.load(Ordering::Relaxed) > 0);
        assert!(
            session_metrics
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .audio_seconds_sent
                > 0.0
        );
    }

    #[test]
    fn audio_archive_disabled_status_is_single_line() {
        let err = anyhow::anyhow!("first line\r\nsecond line");

        assert_eq!(
            audio_archive_disabled_status(&err),
            "⚠ Audio archive disabled: first line  second line"
        );
    }

    #[test]
    fn attach_audio_archive_writes_and_forwards_chunks() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = TempDir::new().unwrap();
        let archive_config = audio::AudioArchiveWriterConfig {
            enabled: true,
            directory: dir.path().to_path_buf(),
            max_size_bytes: 0,
        };
        let writer = audio::AudioArchiveWriter::start(&archive_config, "archive-forward").unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let stream = audio::CaptureStream {
            info: audio::CaptureInfo {
                device_name: "test source".to_string(),
                native_sample_rate: 16_000,
            },
            receiver: rx,
        };
        let mut archived = attach_audio_archive(&rt, stream, writer, Arc::new(Mutex::new(None)));
        let chunk = AudioChunk::new(vec![123i16; 16_000]);

        rt.block_on(async {
            tx.send(chunk.clone()).await.unwrap();
            drop(tx);

            let forwarded = archived.receiver.recv().await.unwrap();
            assert_eq!(forwarded.samples, chunk.samples);
            assert!(archived.receiver.recv().await.is_none());
        });

        let wav_path = dir.path().join("archive-forward.wav");
        let source = audio::WavFileSource::open(&wav_path).unwrap();
        assert_eq!(source.total_samples(), chunk.samples.len());
    }

    #[test]
    fn audio_capture_stop_sets_error_state_and_clears_level() {
        let level_tx = Arc::new(AtomicU32::new(99));
        let stt_state = Arc::new(Mutex::new(metrics::SttState::Listening));

        mark_audio_capture_stopped(&level_tx, &stt_state);

        assert_eq!(level_tx.load(Ordering::Relaxed), 0);
        assert!(matches!(
            &*stt_state.lock().unwrap_or_else(|p| p.into_inner()),
            metrics::SttState::Error(message) if message == "audio capture stopped"
        ));
    }

    #[test]
    fn unmapped_key_wakes_any_key_waits() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);

        assert_eq!(key_to_action(&key, false, false), Some(UserAction::AnyKey));
    }

    #[test]
    fn settings_shortcut_opens_settings_outside_text_overlays() {
        for key_code in [KeyCode::Char('s'), KeyCode::Char('S')] {
            let key = KeyEvent::new(key_code, KeyModifiers::NONE);

            assert_eq!(
                key_to_action(&key, false, false),
                Some(UserAction::OpenSettings)
            );
        }
    }

    #[test]
    fn prompt_language_hides_help_overlay() {
        let state = AppState::new();
        state.show_help.store(true, Ordering::Relaxed);
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        handle_action(
            &UserAction::PromptLanguage,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            Path::new("config.json"),
            &current_config,
            &playback_service,
        );

        assert!(state.lang_prompt_active.load(Ordering::Relaxed));
        assert!(!state.show_help.load(Ordering::Relaxed));
    }

    #[test]
    fn opening_help_closes_language_prompt() {
        let state = AppState::new();
        state.lang_prompt_active.store(true, Ordering::Relaxed);
        *state.lang_input.lock().unwrap() = "vi".to_string();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        handle_action(
            &UserAction::ToggleHelp,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            Path::new("config.json"),
            &current_config,
            &playback_service,
        );

        assert!(state.show_help.load(Ordering::Relaxed));
        assert!(!state.lang_prompt_active.load(Ordering::Relaxed));
        assert!(state.lang_input.lock().unwrap().is_empty());
    }

    #[test]
    fn config_editor_keys_route_when_settings_are_open() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(
            key_to_action(&key, false, true),
            Some(UserAction::ConfigChar('x'))
        );
        assert_eq!(
            key_to_action(
                &KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                false,
                true
            ),
            Some(UserAction::Quit)
        );
        assert_eq!(
            key_to_action(
                &KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                false,
                true
            ),
            Some(UserAction::ConfigNextField)
        );
        assert_eq!(
            key_to_action(
                &KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
                false,
                true
            ),
            Some(UserAction::ConfigInput(InputRequest::GoToPrevChar))
        );
        assert_eq!(
            key_to_action(
                &KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
                false,
                true
            ),
            Some(UserAction::ConfigInput(InputRequest::DeleteNextChar))
        );
    }

    #[test]
    fn open_settings_activates_config_editor() {
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        handle_action(
            &UserAction::OpenSettings,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            Path::new("config.json"),
            &current_config,
            &playback_service,
        );

        assert!(state.config_editor_active.load(Ordering::Relaxed));
        let editor = state.config_editor_snapshot().expect("editor snapshot");
        assert_eq!(editor.mode, ConfigEditorMode::Settings);
    }

    #[test]
    fn editor_state_loads_provider_fields_from_config() {
        let mut cfg = config::AppConfig::default();
        cfg.stt_provider = "local".to_string();
        cfg.mt_provider = "local".to_string();
        let cfg_path = Path::new(r"C:\Users\demo\.tui-translator\config.json");
        let editor =
            tui::ConfigEditorState::from_config(&cfg, cfg_path, ConfigEditorMode::Settings);

        assert_eq!(editor.stt_provider, "local");
        assert_eq!(editor.mt_provider, "local");
    }

    #[test]
    fn build_config_from_editor_persists_provider_fields() {
        let current = config::AppConfig::default();
        let cfg_path = Path::new(r"C:\Users\demo\.tui-translator\config.json");
        let mut editor =
            tui::ConfigEditorState::from_config(&current, cfg_path, ConfigEditorMode::Settings);
        editor.stt_provider = "local".to_string();
        editor.mt_provider = "local".to_string();

        let next = build_config_from_editor(&editor, &current).unwrap();

        assert_eq!(next.stt_provider, "local");
        assert_eq!(next.mt_provider, "local");
    }

    #[test]
    fn runtime_provider_error_allows_default_google_mode() {
        let cfg = config::AppConfig::default();
        assert!(runtime_provider_error(&cfg).is_none());
    }

    #[test]
    fn missing_google_api_key_error_preserves_default_metrics_only_without_key() {
        let cfg = config::AppConfig::default();

        assert!(missing_google_api_key_error(&cfg).is_none());
    }

    #[test]
    fn missing_google_api_key_error_explains_local_stt_still_needs_google_mt() {
        let mut cfg = config::AppConfig::default();
        cfg.stt_provider = "local".to_string();

        let msg = missing_google_api_key_error(&cfg)
            .expect("local STT with Google MT/TTS should require google_api_key");

        assert!(msg.contains("Google Translation"));
        assert!(msg.contains("Google Translation requires google_api_key"));
        assert!(msg.contains("switch mt_provider to \"local\""));
        assert!(msg.contains("google_api_key"));
    }

    #[test]
    fn missing_google_api_key_error_gives_tts_specific_action() {
        let mut cfg = config::AppConfig::default();
        cfg.tts_enabled = true;

        let msg = missing_google_api_key_error(&cfg)
            .expect("Google providers with TTS should require google_api_key");

        assert!(msg.contains("Google STT and Google Translation and Google TTS require"));
        assert!(msg.contains("switch stt_provider and mt_provider to \"local\""));
        assert!(msg.contains("disable translated audio"));
    }

    #[test]
    fn missing_google_api_key_error_allows_fully_local_text_pipeline_without_tts() {
        let mut cfg = config::AppConfig::default();
        cfg.stt_provider = "local".to_string();
        cfg.mt_provider = "local".to_string();
        cfg.tts_enabled = false;

        assert!(missing_google_api_key_error(&cfg).is_none());
    }

    #[test]
    fn missing_google_api_key_error_allows_local_stt_when_key_is_present() {
        let mut cfg = config::AppConfig::default();
        cfg.stt_provider = "local".to_string();
        cfg.google_api_key = Some("demo-key".to_string());

        assert!(missing_google_api_key_error(&cfg).is_none());
    }

    #[cfg(not(feature = "local-stt"))]
    #[test]
    fn runtime_provider_error_rejects_local_stt_without_feature() {
        let mut cfg = config::AppConfig::default();
        cfg.stt_provider = "local".to_string();
        let msg =
            runtime_provider_error(&cfg).expect("local STT should require the local-stt feature");

        assert!(msg.contains("stt_provider=\"local\""));
        assert!(msg.contains("local-stt"));
        assert!(msg.contains("google"));
    }

    #[cfg(feature = "local-stt")]
    #[test]
    fn runtime_provider_error_allows_local_stt_with_feature() {
        let mut cfg = config::AppConfig::default();
        cfg.stt_provider = "local".to_string();

        assert!(runtime_provider_error(&cfg).is_none());
    }

    #[cfg(not(feature = "local-mt"))]
    #[test]
    fn runtime_provider_error_rejects_local_mt_mode() {
        let mut cfg = config::AppConfig::default();
        cfg.mt_provider = "local".to_string();
        let msg = runtime_provider_error(&cfg)
            .expect("local MT should require the local-mt feature in default builds");

        assert!(msg.contains("mt_provider=\"local\""));
        assert!(msg.contains("local-mt"));
        assert!(msg.contains("google"));
    }

    #[cfg(feature = "local-mt")]
    #[test]
    fn runtime_provider_error_allows_local_mt_with_feature() {
        let mut cfg = config::AppConfig::default();
        cfg.mt_provider = "local".to_string();

        assert!(runtime_provider_error(&cfg).is_none());
    }

    #[test]
    fn config_save_persists_provider_fields() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        state.open_config_editor(
            ConfigEditorMode::Settings,
            &config::AppConfig::default(),
            &cfg_path,
        );
        let _ = state.with_config_editor_mut(|editor| {
            editor.stt_provider = "local".to_string();
            editor.mt_provider = "local".to_string();
        });

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        let persisted = config::load(&cfg_path).unwrap();
        assert_eq!(persisted.stt_provider, "local");
        assert_eq!(persisted.mt_provider, "local");
    }

    #[test]
    fn config_save_sets_restart_required_when_provider_changes() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        state.open_config_editor(
            ConfigEditorMode::Settings,
            &config::AppConfig::default(),
            &cfg_path,
        );
        let _ = state.with_config_editor_mut(|editor| {
            editor.stt_provider = "local".to_string();
        });

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        assert!(
            restart_required.load(Ordering::Relaxed),
            "changing stt_provider must set restart_required"
        );
    }

    #[test]
    fn config_save_applies_hot_reloadable_language_changes_without_restart() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let mut starting_config = config::AppConfig::default();
        starting_config.source_language = "ja-JP".to_string();
        starting_config.target_language = "vi".to_string();
        let current_config = Arc::new(Mutex::new(starting_config.clone()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        state.open_config_editor(ConfigEditorMode::Settings, &starting_config, &cfg_path);
        let _ = state.with_config_editor_mut(|editor| {
            editor.source_language = "en-US".to_string();
            editor.target_language = "fr".to_string();
        });

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        let persisted = config::load(&cfg_path).unwrap();
        assert_eq!(persisted.source_language, "en-US");
        assert_eq!(persisted.target_language, "fr");
        assert_eq!(current_config.lock().unwrap().source_language, "en-US");
        assert_eq!(current_config.lock().unwrap().target_language, "fr");
        assert_eq!(state.source_language(), "en-US");
        assert_eq!(state.target_language(), "fr");
        assert!(
            !restart_required.load(Ordering::Relaxed),
            "source/target language changes are hot-reloadable and must not signal restart"
        );
        assert!(!state.config_editor_active.load(Ordering::Relaxed));
    }

    #[test]
    fn watcher_replay_after_hot_reloadable_config_save_is_noop_for_restart_flag() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        state.open_config_editor(
            ConfigEditorMode::Settings,
            &config::AppConfig::default(),
            &cfg_path,
        );
        let _ = state.with_config_editor_mut(|editor| {
            editor.target_language = "en".to_string();
        });

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        let saved = config::load(&cfg_path).unwrap();
        apply_runtime_config(
            &current_config,
            &state.target_language,
            &state.source_language,
            &state.capture_device_label,
            &state.tts_enabled,
            &restart_required,
            &playback_service,
            saved,
        );

        assert_eq!(state.target_language(), "en");
        assert_eq!(current_config.lock().unwrap().target_language, "en");
        assert!(
            !restart_required.load(Ordering::Relaxed),
            "watcher replay of the just-saved hot-reloadable config must not create a restart loop"
        );
    }

    #[test]
    fn config_save_persists_home_style_config() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        state.open_config_editor(
            ConfigEditorMode::Onboarding,
            &config::AppConfig::default(),
            &cfg_path,
        );
        let _ = state.with_config_editor_mut(|editor| {
            editor.source_language = "en-US".to_string();
            editor.target_language = "fr".to_string();
            editor.google_api_key = "saved-key".to_string();
        });

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        let persisted = config::load(&cfg_path).unwrap();
        assert_eq!(persisted.google_api_key.as_deref(), Some("saved-key"));
        assert_eq!(persisted.source_language, "en-US");
        assert_eq!(persisted.target_language, "fr");
        let runtime = current_config.lock().unwrap().clone();
        assert_eq!(runtime.google_api_key.as_deref(), Some("saved-key"));
        assert_eq!(runtime.source_language, "en-US");
        assert_eq!(runtime.target_language, "fr");
        assert_eq!(state.source_language(), "en-US");
        assert_eq!(state.target_language(), "fr");
        assert!(
            restart_required.load(Ordering::Relaxed),
            "onboarding save should signal restart for provider credential changes"
        );
        assert!(!state.config_editor_active.load(Ordering::Relaxed));
    }

    #[test]
    fn onboarding_save_rejects_empty_google_api_key_and_keeps_editor_open() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        state.open_config_editor(
            ConfigEditorMode::Onboarding,
            &config::AppConfig::default(),
            &cfg_path,
        );

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        let editor = state.config_editor_snapshot().expect("editor remains open");
        assert!(
            editor
                .status_message
                .as_deref()
                .is_some_and(|message| message.contains("Google API key")
                    && message.contains("edit config manually")),
            "save feedback should explain missing Google API key and manual fallback: {:?}",
            editor.status_message
        );
        assert!(
            !cfg_path.exists(),
            "invalid onboarding input must not write config"
        );
    }

    #[test]
    fn onboarding_save_rejects_empty_source_language_and_keeps_editor_open() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        state.open_config_editor(
            ConfigEditorMode::Onboarding,
            &config::AppConfig::default(),
            &cfg_path,
        );
        let _ = state.with_config_editor_mut(|editor| {
            editor.source_language.clear();
            editor.google_api_key = "saved-key".to_string();
        });

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        let editor = state.config_editor_snapshot().expect("editor remains open");
        assert!(
            editor
                .status_message
                .as_deref()
                .is_some_and(|message| message.contains("source language")
                    && message.contains("edit config manually")),
            "save feedback should explain missing source language and manual fallback: {:?}",
            editor.status_message
        );
        assert!(
            !cfg_path.exists(),
            "invalid onboarding input must not write config"
        );
    }

    #[test]
    fn onboarding_save_rejects_empty_target_language_and_keeps_editor_open() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        state.open_config_editor(
            ConfigEditorMode::Onboarding,
            &config::AppConfig::default(),
            &cfg_path,
        );
        let _ = state.with_config_editor_mut(|editor| {
            editor.target_language.clear();
            editor.google_api_key = "saved-key".to_string();
        });

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        let editor = state.config_editor_snapshot().expect("editor remains open");
        assert!(
            editor
                .status_message
                .as_deref()
                .is_some_and(|message| message.contains("target language")
                    && message.contains("edit config manually")),
            "save feedback should explain missing target language and manual fallback: {:?}",
            editor.status_message
        );
        assert!(
            !cfg_path.exists(),
            "invalid onboarding input must not write config"
        );
    }

    #[test]
    fn config_save_defaults_blank_file_audio_path_to_config_dir() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let mut starting_config = config::AppConfig::default();
        starting_config.audio_source = "file".to_string();
        starting_config.audio_file_path = Some(r"C:\old\fixture.wav".to_string());
        let current_config = Arc::new(Mutex::new(starting_config));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        state.open_config_editor(
            ConfigEditorMode::Settings,
            &current_config.lock().unwrap().clone(),
            &cfg_path,
        );
        let _ = state.with_config_editor_mut(|editor| {
            editor.audio_source = "file".to_string();
            editor.audio_file_path.clear();
        });

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        let persisted = config::load(&cfg_path).unwrap();
        let expected_path = cfg_path
            .parent()
            .unwrap()
            .join("audio-input.wav")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            persisted.audio_file_path.as_deref(),
            Some(expected_path.as_str())
        );
        assert!(restart_required.load(Ordering::Relaxed));
        assert!(!state.config_editor_active.load(Ordering::Relaxed));
    }

    #[test]
    fn config_save_persists_capture_device_selection() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        state.open_config_editor(
            ConfigEditorMode::Settings,
            &config::AppConfig::default(),
            &cfg_path,
        );
        let _ = state.with_config_editor_mut(|editor| {
            editor.capture_device = "Headphones (USB Audio)".to_string();
        });

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        let persisted = config::load(&cfg_path).unwrap();
        assert_eq!(
            persisted.capture_device.as_deref(),
            Some("Headphones (USB Audio)")
        );
        assert!(restart_required.load(Ordering::Relaxed));
        assert_eq!(state.capture_device_label(), "Headphones (USB Audio)");
        let reopened =
            tui::ConfigEditorState::from_config(&persisted, &cfg_path, ConfigEditorMode::Settings);
        assert_eq!(reopened.capture_device, "Headphones (USB Audio)");
        assert!(!state.config_editor_active.load(Ordering::Relaxed));
    }

    #[test]
    fn capture_device_label_defaults_for_blank_and_trims_configured_name() {
        assert_eq!(capture_device_label_from_config(&None), "Default device");
        assert_eq!(
            capture_device_label_from_config(&Some("   ".to_string())),
            "Default device"
        );
        assert_eq!(
            capture_device_label_from_config(&Some("  Headphones (USB Audio)  ".to_string())),
            "Headphones (USB Audio)"
        );
    }

    #[test]
    fn config_save_rejects_invalid_language_and_keeps_editor_open() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        state.open_config_editor(
            ConfigEditorMode::Settings,
            &config::AppConfig::default(),
            &cfg_path,
        );
        let _ = state.with_config_editor_mut(|editor| {
            editor.source_language = "ja-JPdas".to_string();
        });

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        assert!(state.config_editor_active.load(Ordering::Relaxed));
        let editor = state.config_editor_snapshot().expect("editor snapshot");
        let status = editor.status_message.expect("save failure message");
        assert!(
            status.contains("Save failed") && status.contains("validation"),
            "status should mention validation failure: {status}"
        );
        assert!(!cfg_path.exists(), "invalid config must not be written");
    }

    #[test]
    fn config_save_rejects_invalid_tts_enabled_and_keeps_editor_open() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        state.open_config_editor(
            ConfigEditorMode::Settings,
            &config::AppConfig::default(),
            &cfg_path,
        );
        let _ = state.with_config_editor_mut(|editor| {
            editor.tts_enabled = "True".to_string();
        });

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        assert!(state.config_editor_active.load(Ordering::Relaxed));
        let editor = state.config_editor_snapshot().expect("editor snapshot");
        let status = editor.status_message.expect("save failure message");
        assert!(
            status.contains("Save failed") && status.contains("tts_enabled"),
            "status should mention the invalid TTS field: {status}"
        );
        assert!(!cfg_path.exists(), "invalid config must not be written");
    }

    #[test]
    fn config_save_error_status_flattens_error_chain_for_tui_line() {
        let err = anyhow::anyhow!("leaf\nfield detail").context("top\ncontext");
        let status = config_save_error_status(&err);

        assert!(!status.contains('\n'));
        assert!(status.contains("top context"));
        assert!(status.contains("leaf field detail"));
    }

    #[test]
    fn config_save_rejects_invalid_settings_without_overwriting_existing_file() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let mut existing = config::AppConfig::default();
        existing.google_api_key = Some("old-key".to_string());
        existing.target_language = "vi".to_string();
        config::write_config(&cfg_path, &existing).unwrap();

        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(existing.clone()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        state.open_config_editor(ConfigEditorMode::Settings, &existing, &cfg_path);
        let _ = state.with_config_editor_mut(|editor| {
            editor.audio_source = "bogus".to_string();
            editor.target_language = "en".to_string();
        });

        handle_action(
            &UserAction::ConfigSave,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            &cfg_path,
            &current_config,
            &playback_service,
        );

        assert!(state.config_editor_active.load(Ordering::Relaxed));
        let editor = state.config_editor_snapshot().expect("editor snapshot");
        let status = editor.status_message.expect("save failure message");
        assert!(
            status.contains("Save failed") && status.contains("audio_source"),
            "status should name the invalid field: {status}"
        );
        assert!(
            !status.contains('\n'),
            "status line must not contain embedded newlines: {status:?}"
        );
        let persisted = config::load(&cfg_path).unwrap();
        assert_eq!(
            persisted.google_api_key.as_deref(),
            Some("old-key"),
            "invalid settings save must preserve existing config file"
        );
        assert_eq!(persisted.target_language, "vi");
        assert_eq!(*current_config.lock().unwrap(), existing);
        assert!(!restart_required.load(Ordering::Relaxed));
    }

    #[test]
    fn startup_invalid_config_opens_repair_editor_state() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let mut cfg = config::AppConfig::default();
        cfg.source_language = "ja-JPdas".to_string();
        let state = AppState::new();

        state.open_config_editor(ConfigEditorMode::Settings, &cfg, &cfg_path);
        let _ = state.with_config_editor_mut(|editor| {
            editor.set_status_message(
                " Config needs repair: `source_language` must look like a simple BCP-47 tag",
            );
        });

        assert!(state.config_editor_active.load(Ordering::Relaxed));
        let editor = state.config_editor_snapshot().expect("editor snapshot");
        assert_eq!(editor.source_language, "ja-JPdas");
        assert!(editor
            .status_message
            .expect("repair message")
            .contains("Config needs repair"));
    }

    #[test]
    fn disabled_tts_does_not_create_playback_service() {
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        let config = config::AppConfig::default();

        sync_playback_service_state(&playback_service, &config, false);

        assert!(playback_service
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_none());
    }

    // ── Issue #86 — auth-recovery ReloadConfig tests ──────────────────────────

    /// Pressing R when the pipeline is already halted and the API key is
    /// unchanged must NOT clear the banner or un-halt the pipeline.
    ///
    /// The same invalid key is still in use; clearing the banner would cause
    /// an immediate re-halt on the next chunk.
    #[test]
    fn reload_config_does_not_clear_banner_when_key_unchanged_and_halted() {
        let mut config_file = NamedTempFile::new().unwrap();
        // Write a config with no API key (same as default → restart_required stays false).
        write!(
            config_file,
            r#"{{"source_language":"en-US","target_language":"vi","tts_enabled":false}}"#
        )
        .unwrap();

        let state = AppState::new();
        // Simulate an active auth-error halt.
        *state.auth_error_banner.lock().unwrap() =
            Some("authentication error: invalid API key".to_string());
        state.pipeline_halted.store(true, Ordering::Relaxed);

        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        handle_action(
            &UserAction::ReloadConfig,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            config_file.path(),
            &current_config,
            &playback_service,
        );

        assert!(
            state.pipeline_halted.load(Ordering::Relaxed),
            "pipeline must remain halted when the key is unchanged and still invalid"
        );
        assert!(
            state.auth_error_banner.lock().unwrap().is_some(),
            "auth_error_banner must remain visible when the key is unchanged and still invalid"
        );
    }

    /// Pressing R when there is no active auth error (pipeline running normally)
    /// must leave the pipeline unhalted and the banner as `None`.
    ///
    /// This is the happy-path reload: no auth condition needs fixing so the
    /// pipeline state is untouched.
    #[test]
    fn reload_config_leaves_pipeline_running_when_no_auth_error() {
        let mut config_file = NamedTempFile::new().unwrap();
        write!(
            config_file,
            r#"{{"source_language":"en-US","target_language":"fr","tts_enabled":false}}"#
        )
        .unwrap();

        let state = AppState::new();
        // No active auth error — pipeline is running normally.
        *state.auth_error_banner.lock().unwrap() = None;
        state.pipeline_halted.store(false, Ordering::Relaxed);

        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        handle_action(
            &UserAction::ReloadConfig,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            config_file.path(),
            &current_config,
            &playback_service,
        );

        assert!(
            !state.pipeline_halted.load(Ordering::Relaxed),
            "pipeline must remain unhalted when there was no auth error"
        );
        assert!(
            state.auth_error_banner.lock().unwrap().is_none(),
            "auth_error_banner must remain None when there was no auth error"
        );
    }

    /// Pressing R after saving a new API key (restart_required = true) while
    /// the pipeline is halted must NOT clear the banner or un-halt the pipeline.
    ///
    /// Providers still carry the old (invalid) credential in-process; only a
    /// full application restart picks up the new key.
    #[test]
    fn reload_config_does_not_clear_banner_when_key_changed_and_halted() {
        let mut config_file = NamedTempFile::new().unwrap();
        // Key differs from default → apply_runtime_config will set restart_required true.
        write!(
            config_file,
            r#"{{"source_language":"en-US","target_language":"vi","tts_enabled":false,"google_api_key":"new-key-value"}}"#
        )
        .unwrap();

        let state = AppState::new();
        *state.auth_error_banner.lock().unwrap() =
            Some("authentication error: invalid API key".to_string());
        state.pipeline_halted.store(true, Ordering::Relaxed);

        // Simulate the flag already set from a previous watcher-detected change.
        let restart_required = Arc::new(AtomicBool::new(true));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));

        handle_action(
            &UserAction::ReloadConfig,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            config_file.path(),
            &current_config,
            &playback_service,
        );

        assert!(
            state.pipeline_halted.load(Ordering::Relaxed),
            "pipeline must remain halted even after a key change (restart needed)"
        );
        assert!(
            state.auth_error_banner.lock().unwrap().is_some(),
            "auth_error_banner must remain visible even after a key change (restart needed)"
        );
    }

    #[test]
    fn help_scroll_bottom_then_up_moves_immediately() {
        let state = AppState::new();
        state.show_help.store(true, Ordering::Relaxed);
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        let terminal_area = Rect::new(0, 0, 80, 8);

        handle_action(
            &UserAction::ScrollBottom,
            &state,
            terminal_area,
            &restart_required,
            Path::new("config.json"),
            &current_config,
            &playback_service,
        );
        let max_scroll = u32::from(help_overlay_max_scroll(terminal_area));
        assert_eq!(state.help_scroll.load(Ordering::Relaxed), max_scroll);

        handle_action(
            &UserAction::ScrollUp,
            &state,
            terminal_area,
            &restart_required,
            Path::new("config.json"),
            &current_config,
            &playback_service,
        );

        assert_eq!(
            state.help_scroll.load(Ordering::Relaxed),
            max_scroll.saturating_sub(1),
            "ScrollUp should move immediately after jumping to the bottom"
        );
    }

    #[test]
    fn help_scroll_down_clamps_so_up_recovers_immediately() {
        let state = AppState::new();
        state.show_help.store(true, Ordering::Relaxed);
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        let terminal_area = Rect::new(0, 0, 80, 8);
        let max_scroll = u32::from(help_overlay_max_scroll(terminal_area));

        for _ in 0..(max_scroll + 5) {
            handle_action(
                &UserAction::ScrollDown,
                &state,
                terminal_area,
                &restart_required,
                Path::new("config.json"),
                &current_config,
                &playback_service,
            );
        }
        assert_eq!(
            state.help_scroll.load(Ordering::Relaxed),
            max_scroll,
            "ScrollDown should clamp at the real help overlay bottom"
        );

        handle_action(
            &UserAction::ScrollUp,
            &state,
            terminal_area,
            &restart_required,
            Path::new("config.json"),
            &current_config,
            &playback_service,
        );

        assert_eq!(
            state.help_scroll.load(Ordering::Relaxed),
            max_scroll.saturating_sub(1),
            "ScrollUp should move immediately after repeated ScrollDown presses"
        );
    }

    #[test]
    fn help_scroll_up_clamps_stale_offset_after_resize() {
        let state = AppState::new();
        state.show_help.store(true, Ordering::Relaxed);
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        let short_terminal = Rect::new(0, 0, 80, 8);
        let tall_terminal = Rect::new(0, 0, 80, 24);
        let max_scroll = u32::from(help_overlay_max_scroll(short_terminal));

        handle_action(
            &UserAction::ScrollBottom,
            &state,
            short_terminal,
            &restart_required,
            Path::new("config.json"),
            &current_config,
            &playback_service,
        );
        assert_eq!(state.help_scroll.load(Ordering::Relaxed), max_scroll);

        handle_action(
            &UserAction::ScrollUp,
            &state,
            tall_terminal,
            &restart_required,
            Path::new("config.json"),
            &current_config,
            &playback_service,
        );

        assert_eq!(
            state.help_scroll.load(Ordering::Relaxed),
            0,
            "ScrollUp should clamp stale help_scroll to the resized max before decrementing"
        );
    }

    // ── config_json_path — TUI_TRANSLATOR_CONFIG env-var override (issue #110) ──

    /// When `TUI_TRANSLATOR_CONFIG` is set, `config_json_path` must return that
    /// path verbatim so the soak runner's generated config is actually loaded.
    #[test]
    fn config_json_path_uses_env_override() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let var = "TUI_TRANSLATOR_CONFIG";
        let expected = r"C:\tmp\soak-config.json";
        let _config = EnvVarGuard::set(var, expected);
        let config_dir = TempDir::new().unwrap();
        let _config_dir = EnvVarGuard::set(config::CONFIG_DIR_OVERRIDE_ENV, config_dir.path());
        let path = config_json_path();
        assert_eq!(
            path,
            std::path::PathBuf::from(expected),
            "config_json_path must honour TUI_TRANSLATOR_CONFIG"
        );
    }

    #[test]
    fn empty_config_env_var_is_not_an_explicit_override() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _config = EnvVarGuard::set("TUI_TRANSLATOR_CONFIG", "");

        assert!(
            !has_explicit_config_override(),
            "empty TUI_TRANSLATOR_CONFIG must behave like no override"
        );
        assert!(
            explicit_config_json_path().is_none(),
            "empty TUI_TRANSLATOR_CONFIG must not produce a path"
        );
    }

    #[test]
    fn should_prepare_per_user_config_dir_only_for_selected_per_user_path() {
        let per_user = PathBuf::from(r"C:\Users\demo\AppData\Roaming\tui-translator\config.json");
        let portable = PathBuf::from(r"C:\tools\tui-translator\config.json");

        assert!(should_prepare_per_user_config_dir(
            false,
            &per_user,
            Ok(per_user.clone())
        ));
        assert!(!should_prepare_per_user_config_dir(
            true,
            &per_user,
            Ok(per_user.clone())
        ));
        assert!(!should_prepare_per_user_config_dir(
            false,
            &portable,
            Ok(per_user)
        ));
    }

    #[test]
    fn create_config_parent_dir_creates_missing_parent_directory() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp
            .path()
            .join("missing")
            .join("config-root")
            .join("config.json");

        create_config_parent_dir(&cfg_path).unwrap();

        assert!(cfg_path.parent().unwrap().is_dir());
    }

    #[test]
    fn create_config_parent_dir_surfaces_create_errors() {
        let parent_file = NamedTempFile::new().unwrap();
        let cfg_path = parent_file.path().join("config.json");

        let error = create_config_parent_dir(&cfg_path).unwrap_err();
        let message = format!("{error:#}");

        assert!(
            message.contains("failed to create per-user config directory"),
            "error should explain which startup directory creation failed; got: {message}"
        );
    }

    #[test]
    fn copy_legacy_config_if_needed_copies_raw_config_and_keeps_legacy_file() {
        let temp = TempDir::new().unwrap();
        let legacy_path = temp.path().join("portable").join("config.json");
        let target_path = temp.path().join("per-user").join("config.json");
        let raw_config = br#"{"source_language":"ja-JP","target_language":"vi","unknown":"kept"}
"#;
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        fs::write(&legacy_path, raw_config).unwrap();

        let notice = copy_legacy_config_if_needed(&legacy_path, &target_path)
            .unwrap()
            .expect("migration should copy existing legacy config");

        assert_eq!(notice.from, legacy_path);
        assert_eq!(notice.to, target_path);
        assert_eq!(
            fs::read(&notice.from).unwrap(),
            raw_config,
            "migration must leave the executable-side config unchanged"
        );
        assert_eq!(
            fs::read(&notice.to).unwrap(),
            raw_config,
            "migration must copy raw JSON without reserializing or dropping fields"
        );
        assert!(
            notice.user_message().contains("left unchanged"),
            "notice should make the non-destructive migration explicit"
        );
    }

    #[test]
    fn copy_legacy_config_if_needed_does_not_overwrite_existing_target() {
        let temp = TempDir::new().unwrap();
        let legacy_path = temp.path().join("portable").join("config.json");
        let target_path = temp.path().join("per-user").join("config.json");
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        fs::create_dir_all(target_path.parent().unwrap()).unwrap();
        fs::write(&legacy_path, br#"{"target_language":"legacy"}"#).unwrap();
        fs::write(&target_path, br#"{"target_language":"existing"}"#).unwrap();

        let notice = copy_legacy_config_if_needed(&legacy_path, &target_path).unwrap();

        assert!(
            notice.is_none(),
            "migration should skip when the per-user config already exists"
        );
        assert_eq!(
            fs::read(&target_path).unwrap(),
            br#"{"target_language":"existing"}"#,
            "migration must not overwrite an existing per-user config"
        );
    }

    /// Without `TUI_TRANSLATOR_CONFIG`, `config_json_path` uses the default
    /// per-user config directory.
    #[test]
    fn config_json_path_fallback_uses_default_config_directory() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let config_dir = TempDir::new().unwrap();
        let _config = EnvVarGuard::remove("TUI_TRANSLATOR_CONFIG");
        let _config_dir = EnvVarGuard::set(config::CONFIG_DIR_OVERRIDE_ENV, config_dir.path());
        let path = config_json_path();
        assert_eq!(
            path,
            config_dir.path().join("config.json"),
            "fallback path must use the default config location; got {path:?}"
        );
    }

    #[test]
    fn select_config_json_path_explicit_override_wins() {
        let selected = select_config_json_path(
            Some(PathBuf::from("override.json")),
            Some(PathBuf::from("portable.json")),
            Ok(PathBuf::from("per-user.json")),
            Some(PathBuf::from("fallback.json")),
        );

        assert_eq!(selected, PathBuf::from("override.json"));
    }

    #[test]
    fn select_config_json_path_prefers_portable_config_over_per_user() {
        let selected = select_config_json_path(
            None,
            Some(PathBuf::from("portable.json")),
            Ok(PathBuf::from("per-user.json")),
            Some(PathBuf::from("fallback.json")),
        );

        assert_eq!(selected, PathBuf::from("portable.json"));
    }

    #[test]
    fn select_config_json_path_uses_per_user_when_no_portable_config_exists() {
        let selected = select_config_json_path(
            None,
            None,
            Ok(PathBuf::from("per-user.json")),
            Some(PathBuf::from("fallback.json")),
        );

        assert_eq!(selected, PathBuf::from("per-user.json"));
    }

    #[test]
    fn select_config_json_path_falls_back_when_per_user_path_is_unavailable() {
        let selected = select_config_json_path(
            None,
            None,
            Err(anyhow::anyhow!("no OS config directory")),
            Some(PathBuf::from("fallback.json")),
        );

        assert_eq!(selected, PathBuf::from("fallback.json"));
    }

    // ── build_config_from_editor — new fields ─────────────────────────────────

    #[test]
    fn build_config_from_editor_persists_tts_enabled_true() {
        let cfg_path = Path::new(r"C:\Users\demo\.tui-translator\config.json");
        let current = config::AppConfig::default();
        let mut editor =
            tui::ConfigEditorState::from_config(&current, cfg_path, ConfigEditorMode::Settings);
        editor.tts_enabled = "true".to_string();

        let result = build_config_from_editor(&editor, &current).unwrap();

        assert!(result.tts_enabled, "tts_enabled must be persisted as true");
    }

    #[test]
    fn build_config_from_editor_persists_tts_enabled_false() {
        let cfg_path = Path::new(r"C:\Users\demo\.tui-translator\config.json");
        let mut current = config::AppConfig::default();
        current.tts_enabled = true;
        let mut editor =
            tui::ConfigEditorState::from_config(&current, cfg_path, ConfigEditorMode::Settings);
        editor.tts_enabled = "false".to_string();

        let result = build_config_from_editor(&editor, &current).unwrap();

        assert!(
            !result.tts_enabled,
            "tts_enabled must be persisted as false"
        );
    }

    #[test]
    fn build_config_from_editor_persists_stt_fallback_policy() {
        let cfg_path = Path::new(r"C:\Users\demo\.tui-translator\config.json");
        let current = config::AppConfig::default();
        let mut editor =
            tui::ConfigEditorState::from_config(&current, cfg_path, ConfigEditorMode::Settings);
        editor.stt_fallback_policy = "local".to_string();

        let result = build_config_from_editor(&editor, &current).unwrap();

        assert_eq!(
            result.stt_fallback_policy, "local",
            "stt_fallback_policy must be persisted from editor"
        );
    }

    #[test]
    fn build_config_from_editor_keeps_real_api_key_not_masked() {
        let cfg_path = Path::new(r"C:\Users\demo\.tui-translator\config.json");
        let current = config::AppConfig::default();
        let mut editor =
            tui::ConfigEditorState::from_config(&current, cfg_path, ConfigEditorMode::Settings);
        // Set the real key directly (as the editor holds the unmasked value).
        editor.google_api_key = "AIzaSyRealKey123456789".to_string();

        let result = build_config_from_editor(&editor, &current).unwrap();

        assert_eq!(
            result.google_api_key.as_deref(),
            Some("AIzaSyRealKey123456789"),
            "build_config_from_editor must preserve the real (unmasked) API key"
        );
    }

    #[test]
    fn build_config_from_editor_round_trips_tts_and_fallback() {
        // Verify that a full round-trip preserves the exact fields.
        let cfg_path = Path::new(r"C:\Users\demo\.tui-translator\config.json");
        let mut cfg = config::AppConfig::default();
        cfg.tts_enabled = true;
        cfg.stt_fallback_policy = "local".to_string();

        let editor =
            tui::ConfigEditorState::from_config(&cfg, cfg_path, ConfigEditorMode::Settings);
        let result = build_config_from_editor(&editor, &cfg).unwrap();

        assert!(result.tts_enabled);
        assert_eq!(result.stt_fallback_policy, "local");
    }

    // ── Replay CLI parsing tests (issue #226) ─────────────────────────────────

    #[test]
    fn parse_replay_args_accepts_flag_and_path() {
        let parsed = parse_replay_args_from(vec![
            OsString::from("--replay-session"),
            OsString::from(r"C:\sessions\meeting.jsonl"),
        ])
        .unwrap()
        .expect("replay args should be detected");

        assert_eq!(parsed.path, PathBuf::from(r"C:\sessions\meeting.jsonl"));
    }

    #[test]
    fn parse_replay_args_returns_none_when_flag_absent() {
        let result = parse_replay_args_from(vec![
            OsString::from("--export-session"),
            OsString::from("meeting.jsonl"),
        ])
        .unwrap();

        assert!(result.is_none(), "no replay flag → must return None");
    }

    #[test]
    fn parse_replay_args_requires_path_value() {
        // Flag without a following path value must return an error.
        let err = parse_replay_args_from(vec![OsString::from("--replay-session")])
            .expect_err("missing path should be rejected");
        assert!(
            err.to_string().contains("--replay-session"),
            "error message must name the flag"
        );
    }

    #[test]
    fn parse_replay_args_rejects_another_flag_as_value() {
        let err = parse_replay_args_from(vec![
            OsString::from("--replay-session"),
            OsString::from("--other-flag"),
        ])
        .expect_err("another flag as value must be rejected");
        assert!(err.to_string().contains("--replay-session"));
    }

    /// Verify that `parse_replay_args_from` returns `None` for an empty
    /// argument list, proving the bypass path is not accidentally triggered.
    #[test]
    fn replay_bypass_not_triggered_with_no_args() {
        let result = parse_replay_args_from(std::iter::empty::<OsString>()).unwrap();
        assert!(
            result.is_none(),
            "no args must not trigger replay mode; audio/provider startup proceeds normally"
        );
    }

    // ── config_json_path / bootstrap — path lookup and migration rules (issue #182) ──

    /// When all config overrides are absent, `config_json_path` must still
    /// resolve to a `config.json` file. On hosts where the OS config directory
    /// is unavailable it may fall back to the portable executable-adjacent path
    /// or the bare CWD `"config.json"`.
    #[test]
    fn config_json_path_without_overrides_resolves_config_json_file() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _config = EnvVarGuard::remove("TUI_TRANSLATOR_CONFIG");
        let _config_dir = EnvVarGuard::remove(config::CONFIG_DIR_OVERRIDE_ENV);
        let _userprofile = EnvVarGuard::remove("USERPROFILE");
        let _home = EnvVarGuard::remove("HOME");

        let path = config_json_path();

        assert_eq!(
            path.file_name(),
            Some(std::ffi::OsStr::new("config.json")),
            "config_json_path must resolve to a config.json file without overrides; \
             got {path:?}"
        );
    }

    /// When `TUI_TRANSLATOR_CONFIG` is set, `bootstrap_legacy_config_if_needed`
    /// must return `Ok(())` immediately without creating or touching any file.
    #[test]
    fn bootstrap_skips_migration_when_override_active() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _config = EnvVarGuard::set("TUI_TRANSLATOR_CONFIG", "irrelevant.json");

        let dir = TempDir::new().unwrap();
        let target = dir.path().join("should-not-be-created.json");

        let result = bootstrap_legacy_config_if_needed(&target);

        assert!(
            result.is_ok(),
            "bootstrap must succeed when TUI_TRANSLATOR_CONFIG override is active"
        );
        assert!(
            !target.exists(),
            "bootstrap must not create the target when TUI_TRANSLATOR_CONFIG is set"
        );
    }

    /// When the target config file already exists, `bootstrap_legacy_config_if_needed`
    /// must be a no-op — it must not overwrite an existing user config.
    #[test]
    fn bootstrap_skips_migration_when_target_already_exists() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _config = EnvVarGuard::remove("TUI_TRANSLATOR_CONFIG");

        let mut target = NamedTempFile::new().unwrap();
        // Write a sentinel value that must survive the bootstrap call unchanged.
        let sentinel = br#"{"source_language":"ja-JP","target_language":"sentinel"}"#;
        target.write_all(sentinel).unwrap();
        target.flush().unwrap();

        bootstrap_legacy_config_if_needed(target.path()).unwrap();

        let content = std::fs::read(target.path()).unwrap();
        assert_eq!(
            content, sentinel,
            "bootstrap must not modify the target when it already exists"
        );
    }

    /// When there is no legacy config next to the executable (the common case
    /// in per-user installs), `bootstrap_legacy_config_if_needed` must return
    /// `Ok(())` without creating the target.
    #[test]
    fn bootstrap_skips_migration_when_no_legacy_present() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _config = EnvVarGuard::remove("TUI_TRANSLATOR_CONFIG");

        let dir = TempDir::new().unwrap();
        // Use a path inside a fresh temp dir so it definitely does not exist.
        let target = dir.path().join("new-home-config.json");

        // The test binary typically has no config.json next to it, so the
        // legacy path does not exist — bootstrap should skip migration silently.
        let result = bootstrap_legacy_config_if_needed(&target);

        assert!(
            result.is_ok(),
            "bootstrap must succeed when there is nothing to migrate"
        );
        assert!(
            !target.exists(),
            "bootstrap must not create the target when there is no legacy config"
        );
    }
}
