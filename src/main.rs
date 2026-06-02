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
//!   `SessionMetrics` to a `tokio::sync::watch` channel every second.
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
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, SystemTime},
};
use tui_input::InputRequest;

mod audio;
mod audio_device_cli;
#[cfg(test)]
mod audio_device_cli_tests;
mod config;
mod diagnostics;
mod i18n;
mod local_model_cli;
#[cfg(test)]
mod local_model_cli_tests;
mod local_model_startup;
#[cfg(test)]
mod main_metrics_tests;
mod metrics;
mod metrics_export;
mod pipeline;
mod providers;
mod runtime_providers;
#[cfg(test)]
mod runtime_providers_tests;
mod runtime_recording;
#[cfg(test)]
mod runtime_recording_tests;
mod session;
mod session_export_cli;
mod session_replay_cli;
#[cfg(test)]
mod session_replay_cli_tests;
mod storage;
mod tui;
pub mod updater;

use audio::DEFAULT_SILENCE_THRESHOLD;
use audio_device_cli::{print_audio_devices_to_stdout, should_list_audio_devices};
use local_model_cli::{
    parse_local_mt_model_install_args_from, parse_local_stt_model_prefetch_args_from,
    parse_model_verify_args_from, run_local_mt_model_install, run_local_stt_model_prefetch,
    run_model_list, run_model_verify, should_list_local_models,
};
use local_model_startup::run_startup_local_model_check;
use metrics::{
    spawn_process_metrics_task, LatencyHistogram, LossMetrics, MemoryGuard, MetricsSnapshot,
    NetworkMetrics, ProcessSnapshot,
};
use metrics_export::{write_metrics_snapshot_export, StorageMetricsHandles, METRICS_SNAPSHOT_ENV};
use runtime_providers::{
    apply_tts_voice_from_config, build_runtime_tts_provider, build_slot_mt_provider,
    build_slot_stt_provider, cycle_tts_voice_for_language, stt_local_unavailable_is_fatal_for_slot,
    DisabledTtsProvider, RuntimeTtsProvider,
};
use runtime_recording::{log_measurement_mode_status, start_audio_archive, start_session_recorder};
use session_export_cli::{parse_session_export_args_from, run_session_export};
use session_replay_cli::{parse_replay_args_from, ReplayArgs};
use tui::frame_pacer::FramePacer;
use tui::onboarding::{
    LocalModelLicense, OnboardingBranch, OnboardingEvent, OnboardingOutcome, OnboardingWizardState,
};
use tui::{
    draw_session_summary, draw_ui_with_route, help_overlay_max_scroll, subtitle_inner_area,
    AppState, ConfigEditorMode, TtsRouteStatus, UserAction, AUDIO_LEVEL_SCALE,
};

type SharedPlaybackService = Arc<Mutex<Option<pipeline::playback::PlaybackService>>>;

struct CaptureHotSwapRuntime {
    router_handle: Mutex<Option<audio::CaptureRouterHandle>>,
    runtime_handle: tokio::runtime::Handle,
    device_name: Arc<Mutex<String>>,
    pipeline_error_msg: Arc<Mutex<Option<String>>>,
}

static CAPTURE_HOT_SWAP_RUNTIME: OnceLock<Arc<CaptureHotSwapRuntime>> = OnceLock::new();

struct SlotAUiArcs {
    pipeline_halted: Arc<AtomicBool>,
    pipeline_error_msg: Arc<Mutex<Option<String>>>,
    auth_error_banner: Arc<Mutex<Option<String>>>,
}

fn slot_a_ui_arcs_for_mode(slot_mode: config::SlotMode, state: &AppState) -> SlotAUiArcs {
    if slot_mode == config::SlotMode::Dual {
        SlotAUiArcs {
            pipeline_halted: Arc::new(AtomicBool::new(false)),
            pipeline_error_msg: Arc::new(Mutex::new(None)),
            auth_error_banner: Arc::new(Mutex::new(None)),
        }
    } else {
        SlotAUiArcs {
            pipeline_halted: Arc::clone(&state.pipeline_halted),
            pipeline_error_msg: Arc::clone(&state.pipeline_error_msg),
            auth_error_banner: Arc::clone(&state.auth_error_banner),
        }
    }
}

fn format_slot_error_status(auth: Option<String>, pipeline: Option<String>) -> String {
    if let Some(message) = auth {
        format!("auth: {message}")
    } else if let Some(message) = pipeline {
        format!("error: {message}")
    } else {
        String::new()
    }
}

struct FinishMainArgs<'a> {
    state: &'a AppState,
    restart_required: &'a Arc<AtomicBool>,
    cfg_path: &'a Path,
    current_config: &'a Arc<Mutex<config::AppConfig>>,
    playback_service: &'a SharedPlaybackService,
    orchestrator_join: Option<tokio::task::JoinHandle<()>>,
    // DM-03 (issue #379): slot B join handle; `None` in single-slot mode or
    // when slot B failed to initialise.
    orchestrator_join_b: Option<tokio::task::JoinHandle<()>>,
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
    // ── Storage metrics (issue #393) ──────────────────────────────────────
    /// Shared atomic handles from the session recorder and audio archive.
    storage: StorageMetricsHandles,
    // ── Fanout drop counters (DM-02, issue #378) ──────────────────────────
    /// Shared atomic counters from the fanout node inserted after capture.
    /// Both slots are zero when no fanout is active (onboarding / capture
    /// failure paths).
    fanout_counters: Arc<audio::FanoutDropCounters>,
    // ── Capture router counters (HC-03B, issue #436) ──────────────────────
    /// Shared router metrics when live capture has started.
    capture_router_metrics: Option<Arc<audio::RouterMetrics>>,
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

/// Run the TUI in session-replay mode.
///
/// Reads `args.path`, loads all `TranscriptSegment`s via
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
        let picker_field_active = Arc::clone(&state.picker_field_active);
        let wizard_active_kb = Arc::clone(&state.wizard_active);
        let keyboard_shutdown = Arc::clone(&keyboard_shutdown);
        rt.spawn(async move {
            tokio::task::spawn_blocking(move || {
                keyboard_task(
                    key_tx,
                    lang_flag,
                    config_editor_flag,
                    picker_field_active,
                    wizard_active_kb,
                    keyboard_shutdown,
                )
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
    // LF-02 (issue #370): export OMP_NUM_THREADS before any library is loaded
    // so onnxruntime's OpenMP thread pool honours the same cap as the ORT
    // session builder.  Must happen before init_tracing/Tokio/provider init.
    // Preserved here as a binding so we can log the outcome once tracing is up.
    #[cfg(feature = "local-mt")]
    let omp_status = crate::providers::local::runtime_caps::prepare_omp_env();

    init_tracing();

    // QA8-08 (issue #506): install panic-sidecar hook before any task
    // spawns so backgrounded Tokio panics are still captured. The hook
    // is idempotent and chains to the default hook so existing
    // backtraces continue to print to stderr / tracing.
    let dump_dir = diagnostics::resolve_dump_dir();
    diagnostics::install_panic_hook(dump_dir.clone());
    tracing::info!(
        dump_dir = %dump_dir.display(),
        "QA8-08 panic sidecar hook installed (see docs/13-crash-dump-symbolication.md)"
    );

    #[cfg(feature = "local-mt")]
    {
        if omp_status.applied {
            tracing::info!(
                cap = omp_status.cap,
                "exported OMP_NUM_THREADS for onnxruntime OpenMP (LF-02 #370)"
            );
        } else {
            tracing::info!(
                cap = omp_status.cap,
                "OMP_NUM_THREADS already set in environment; skipping override (LF-02 #370)"
            );
        }
    }

    tracing::info!("tui-translator starting");

    // QA8-07 (#505): install the global backpressure telemetry sink
    // before any audio capture, fanout, sink, or provider dispatch
    // path runs. Wiring sites call thin hooks in the audio /
    // providers / pipeline modules; here we install delegates that
    // forward into `metrics::backpressure::emit::*`. With no
    // installation the wiring is a cheap no-op (tests).
    metrics::backpressure::emit::install(std::sync::Arc::new(
        metrics::backpressure::BackpressureTelemetry::new(),
    ));
    audio::backpressure_hook::install_fanout_drop(metrics::backpressure::emit::fanout_drop);
    audio::backpressure_hook::install_audio_chunk_at(metrics::backpressure::emit::audio_chunk_at);
    audio::backpressure_hook::install_audio_stall(metrics::backpressure::emit::audio_capture_stall);
    audio::backpressure_hook::install_monotonic_now_ns(
        metrics::backpressure::emit::monotonic_now_ns,
    );
    providers::backpressure_hook::install_enqueue(metrics::backpressure::emit::provider_enqueue);
    providers::backpressure_hook::install_dequeue_start(
        metrics::backpressure::emit::provider_dequeue_start,
    );
    providers::backpressure_hook::install_complete(metrics::backpressure::emit::provider_complete);
    providers::backpressure_hook::install_recovered_error(
        metrics::backpressure::emit::provider_recovered_error,
    );
    providers::backpressure_hook::install_permanent_error(
        metrics::backpressure::emit::provider_permanent_error,
    );
    pipeline::backpressure_hook::install_sink_write(metrics::backpressure::emit::sink_write);
    pipeline::backpressure_hook::install_sink_underrun(metrics::backpressure::emit::sink_underrun);
    // QA8-07 (#505): cancellation/shutdown latency wiring. The
    // `issue()` callsite lives in `finish_main` (where
    // `orchestrator_shutdown` is set); each orchestrator loop calls
    // `exit()` when it observes the flag and is about to break.
    pipeline::cancellation_hook::install_issue(metrics::backpressure::emit::cancellation_issue);
    pipeline::cancellation_hook::install_exit(metrics::backpressure::emit::cancellation_exit);
    pipeline::cancellation_hook::install_monotonic_now_ns(
        metrics::backpressure::emit::monotonic_now_ns,
    );

    // I18N-01 (issue #481): build the i18n catalog before the first
    // frame so help-overlay strings resolve without lazy init in the
    // render path.  The active locale is set from AppConfig once the
    // configuration has been loaded, below.
    i18n::init();

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

    if should_list_local_models(std::env::args_os().skip(1)) {
        run_model_list()?;
        return Ok(());
    }

    if let Some(verify_args) = parse_model_verify_args_from(std::env::args_os().skip(1))? {
        run_model_verify(&verify_args)?;
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

    // I18N-01 (issue #481): apply the persisted locale before the first
    // frame so migrated TUI surfaces (currently the help overlay) render
    // in the user's preferred catalog from the very first draw.  Operator
    // `tracing` logs stay English per the ADR migration allowlist, so
    // this call only affects UI string lookups.  Subsequent
    // `apply_runtime_config` calls keep this in sync on hot-reload.
    i18n::set_locale(&cfg.locale);

    // LF-06: best-effort, one-shot migration of pre-LF-06 transcript + audio
    // archive directories from %APPDATA% into the canonical %LOCALAPPDATA%
    // layout.  A `.lf06-migrated` marker prevents repeat work on every launch.
    if let (
        Ok(marker),
        Ok(legacy_sessions),
        Ok(canonical_sessions),
        Ok(legacy_audio),
        Ok(canonical_audio),
    ) = (
        config::lf06_migration_marker_path(),
        config::legacy_sessions_dir(),
        config::default_sessions_dir(),
        config::legacy_audio_archive_dir(),
        config::default_audio_archive_dir(),
    ) {
        if let Err(err) = storage::try_migrate_legacy_storage(
            &legacy_sessions,
            &canonical_sessions,
            &legacy_audio,
            &canonical_audio,
            &marker,
        ) {
            tracing::warn!("LF-06 storage migration skipped: {err:#}");
        }
    }

    // LF-06: print the one-shot startup summary that names where transcripts
    // and raw audio go.  Best-effort: a missing %LOCALAPPDATA% root is logged
    // but not fatal.
    if let (Ok(sessions_root), Ok(audio_root)) = (
        config::default_sessions_dir(),
        config::default_audio_archive_dir(),
    ) {
        storage::print_startup_summary(&sessions_root, &audio_root);
    }
    let skip_interactive_startup = skip_onboarding();
    let startup_config_mode = startup_config_mode(
        load_state,
        has_explicit_config_override(),
        skip_interactive_startup,
    );
    let onboarding_required = startup_config_mode == StartupConfigMode::OnboardingRequired;
    let config_recovery_required = startup_config_mode == StartupConfigMode::ConfigRecoveryRequired;

    // Auto-download any missing local models before the TUI starts so the
    // download progress is visible on the terminal.  Only runs when both
    // providers are already configured as "local" (i.e. not during first-run
    // onboarding, where the user hasn't chosen their provider yet).
    if !onboarding_required && !config_recovery_required && !skip_interactive_startup {
        if let Err(err) = run_startup_local_model_check(&cfg.stt_provider, &cfg.mt_provider) {
            tracing::warn!(%err, "startup local model check failed — continuing without models");
        }
    }

    let pending_consent_manifests =
        if !onboarding_required && !config_recovery_required && !skip_interactive_startup {
            collect_pending_consent_manifests(&cfg)
        } else {
            Vec::new()
        };
    let license_review_required = !pending_consent_manifests.is_empty();
    let current_config = Arc::new(Mutex::new(cfg.clone()));
    // CTRL-01: seed runtime gain controllers from persisted config so the
    // first audio chunk processed after startup honours the saved values.
    audio::audio_gain::set_input_gain_db(cfg.input_gain_db);
    audio::audio_gain::set_output_volume_db(cfg.output_volume_db);
    let restart_required = Arc::new(AtomicBool::new(false));
    let state = AppState::new();
    let pending_restart_reason = Arc::new(Mutex::new(None::<String>));

    // Start the hot-reload watcher; keep the receiver alive for the process lifetime.
    let watcher_notifier: Option<config::WatchApplyNotifier> = {
        let cas = Arc::clone(&state.config_apply_status);
        let cac = Arc::clone(&state.config_apply_count);
        let pending_restart_reason = Arc::clone(&pending_restart_reason);
        Some(std::sync::Arc::new(
            move |notif: config::WatchApplyNotification| {
                let status = match notif {
                    config::WatchApplyNotification::Rejected { reason } => {
                        *pending_restart_reason
                            .lock()
                            .unwrap_or_else(|p| p.into_inner()) = None;
                        tui::ConfigApplyStatus::RolledBack { reason }
                    }
                    config::WatchApplyNotification::ParseError { reason } => {
                        *pending_restart_reason
                            .lock()
                            .unwrap_or_else(|p| p.into_inner()) = None;
                        tui::ConfigApplyStatus::RolledBack {
                            reason: format!("parse error: {reason}"),
                        }
                    }
                    config::WatchApplyNotification::NeedsRestart { reason } => {
                        *pending_restart_reason
                            .lock()
                            .unwrap_or_else(|p| p.into_inner()) = Some(reason);
                        return;
                    }
                };
                tui::record_config_apply_to(&cas, &cac, status);
            },
        ))
    };
    let config_rx = match config::start_watcher(
        &cfg_path,
        cfg.clone(),
        restart_required.clone(),
        watcher_notifier,
    ) {
        Ok(rx) => Some(rx),
        Err(err) => {
            tracing::warn!("config hot-reload unavailable: {err:#}");
            None
        }
    };

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
    // DM-04: remember slot-A MT provider so dual-pane titles can show it.
    *state
        .slot_a_provider_name
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = loaded_config.mt_provider.clone();
    // Initialise source_language from the loaded config so AppState and the
    // orchestrator start with the same value.
    overwrite_source_language(&state.source_language, &loaded_config.source_language);
    // Initialise the operator-facing capture device label from config (issue #197).
    overwrite_capture_device_label(&state.capture_device_label, &loaded_config.capture_device);
    state.set_tts_enabled(loaded_config.tts_enabled);
    state.set_audio_consent(loaded_config.audio_archive.consent_given);
    let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
    if !onboarding_required && !config_recovery_required && !license_review_required {
        let current_cfg = current_config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        sync_playback_service_state(&playback_service, &current_cfg, current_cfg.tts_enabled);
    }
    if onboarding_required {
        let local_licenses = build_local_model_licenses();
        let wizard = OnboardingWizardState::new(local_licenses);
        state.open_wizard(wizard);
        overwrite_device_name(&state.device_name, "first-run setup required");
        tracing::info!(
            path = %cfg_path.display(),
            "per-user config missing; opening first-run wizard"
        );
    }
    if config_recovery_required {
        state.open_config_editor(ConfigEditorMode::Settings, &cfg, &cfg_path);
        populate_config_editor_device_options(&state);
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
    if license_review_required {
        let branch = branch_from_config(&loaded_config);
        let licenses = local_model_licenses_from_manifests(&pending_consent_manifests);
        let wizard = OnboardingWizardState::new_consent_review(licenses, branch);
        state.open_wizard(wizard);
        overwrite_device_name(&state.device_name, "model license review required");
        tracing::info!(
            pending = pending_consent_manifests.len(),
            "local model consent missing or stale; opening license review"
        );
    }

    // Build a multi-threaded Tokio runtime for background tasks.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let capture_hot_swap_runtime = Arc::new(CaptureHotSwapRuntime {
        router_handle: Mutex::new(None),
        runtime_handle: rt.handle().clone(),
        device_name: Arc::clone(&state.device_name),
        pipeline_error_msg: Arc::clone(&state.pipeline_error_msg),
    });
    let _ = CAPTURE_HOT_SWAP_RUNTIME.set(Arc::clone(&capture_hot_swap_runtime));

    if let Some(mut config_rx) = config_rx {
        let current_config = Arc::clone(&current_config);
        let target_language = Arc::clone(&state.target_language);
        let source_language = Arc::clone(&state.source_language);
        let capture_device_label = Arc::clone(&state.capture_device_label);
        let slot_a_provider_name = Arc::clone(&state.slot_a_provider_name);
        let tts_enabled = Arc::clone(&state.tts_enabled);
        let audio_consent = Arc::clone(&state.audio_consent);
        let restart_required = Arc::clone(&restart_required);
        let playback_service = Arc::clone(&playback_service);
        let config_apply_status = Arc::clone(&state.config_apply_status);
        let config_apply_count = Arc::clone(&state.config_apply_count);
        let pending_restart_reason = Arc::clone(&pending_restart_reason);
        rt.spawn(async move {
            while config_rx.changed().await.is_ok() {
                let next_cfg = config_rx.borrow().clone();
                // PlaybackService::new does blocking I/O (startup_rx.recv + device enum);
                // offload to a dedicated thread so we don't block a Tokio worker.
                let cc = Arc::clone(&current_config);
                let tl = Arc::clone(&target_language);
                let sl = Arc::clone(&source_language);
                let cdl = Arc::clone(&capture_device_label);
                let sap = Arc::clone(&slot_a_provider_name);
                let te = Arc::clone(&tts_enabled);
                let ac = Arc::clone(&audio_consent);
                let rr = Arc::clone(&restart_required);
                let ps = Arc::clone(&playback_service);
                let cas = Arc::clone(&config_apply_status);
                let cac = Arc::clone(&config_apply_count);
                let prr = Arc::clone(&pending_restart_reason);
                tokio::task::spawn_blocking(move || {
                    let (apply_requires_restart, actually_changed) = apply_runtime_config(
                        &cc, &tl, &sl, &cdl, &sap, &te, &ac, &rr, &ps, next_cfg,
                    );
                    let pending_reason = prr.lock().unwrap_or_else(|p| p.into_inner()).take();
                    if let Some(status) = config_apply_status_for_watcher_change(
                        apply_requires_restart,
                        actually_changed,
                        pending_reason,
                    ) {
                        tui::record_config_apply_to(&cas, &cac, status);
                    }
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
    // DM-03 (issue #379): slot B join handle; `None` in single-slot mode.
    let mut orchestrator_join_b: Option<tokio::task::JoinHandle<()>> = None;
    // Issue #393: storage metrics handles — populated from recorder/archive when
    // the orchestrator starts; default (all-zero) for early-exit paths.
    let mut storage = StorageMetricsHandles::default();

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

    // DM-02 (issue #378): default counters for paths where audio capture is not
    // started (onboarding, capture failure).  Reassigned after fanout wiring.
    let mut fanout_counters: Arc<audio::FanoutDropCounters> =
        Arc::new(audio::FanoutDropCounters::default());
    let mut capture_router_metrics: Option<Arc<audio::RouterMetrics>> = None;

    if onboarding_required || config_recovery_required || license_review_required {
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
                orchestrator_join_b: None,
                orchestrator_shutdown,
                process_rx,
                e2e_latency,
                network_metrics,
                loss_metrics,
                cpu_gate,
                memory_guard,
                storage: StorageMetricsHandles::default(),
                fanout_counters,
                capture_router_metrics: None,
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
            let audio_archive =
                start_audio_archive(&cfg_snapshot, &session_id, &state.pipeline_error_msg);
            let audio_archive_path = audio_archive.path().map(|p| p.to_path_buf());
            // Issue #393: extract shared storage handles before the archive writer
            // is moved into CaptureRouter.
            let audio_archive_bytes_arc = audio_archive.bytes_arc();
            let audio_archive_sealed_arc = audio_archive.sealed_arc();
            storage.archive_bytes = Arc::clone(&audio_archive_bytes_arc);
            storage.archive_sealed = Arc::clone(&audio_archive_sealed_arc);
            storage.archive_path = audio_archive_path.clone();

            // HC-03B + DM-02: insert CaptureRouter before fanout.  The
            // router keeps fanout/orchestrator receivers stable while
            // capture_device/audio_source hot-swap reopens only the upstream.
            let capture_info = stream.info.clone();
            let (router_handle, capture_rx) = {
                // rt.enter() sets the runtime context so tokio::spawn inside
                // start_router resolves to the correct runtime.
                let _guard = rt.enter();
                audio::start_router(stream, DEFAULT_SILENCE_THRESHOLD, Some(audio_archive))
            };
            capture_router_metrics = Some(Arc::clone(router_handle.metrics()));
            *capture_hot_swap_runtime
                .router_handle
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(router_handle);

            // DM-02 (issue #378): insert fanout node after routed capture.
            // Slot A → primary consumer (orchestrator / metrics-only task).
            // DM-03 (issue #379): in dual-slot mode keep slot B for the second
            // orchestrator; drop it immediately in single-slot mode so the
            // fanout task takes the Closed branch and reports no false drops.
            let fanout_handle = {
                // rt.enter() sets the runtime context so tokio::spawn inside
                // start_fanout resolves to the correct runtime.
                let _guard = rt.enter();
                audio::start_fanout(capture_rx)
            };
            fanout_counters = Arc::clone(&fanout_handle.counters);
            let slot_mode = cfg_snapshot.slot_mode();
            let slot_a_cfg = cfg_snapshot.slot_a();
            let slot_b_receiver = if slot_mode == config::SlotMode::Dual {
                Some(fanout_handle.slot_b)
            } else {
                drop(fanout_handle.slot_b);
                None
            };
            let stream = audio::CaptureStream {
                info: capture_info,
                receiver: fanout_handle.slot_a,
            };
            if let Some(provider_msg) = runtime_provider_error(&cfg_snapshot) {
                tracing::warn!("{provider_msg}");
                *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
                    metrics::SttState::Error(provider_msg.clone());
                *state
                    .pipeline_error_msg
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) = Some(provider_msg);
                spawn_metrics_only_audio_task(&rt, stream, &state, &loss_metrics);
                orchestrator_join = None;
            } else if let Some(provider_msg) = missing_google_api_key_error(&cfg_snapshot) {
                tracing::warn!("{provider_msg}");
                *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
                    metrics::SttState::Error(provider_msg.clone());
                *state
                    .pipeline_error_msg
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) = Some(provider_msg);
                spawn_metrics_only_audio_task(&rt, stream, &state, &loss_metrics);
                orchestrator_join = None;
            } else if cfg_snapshot.google_api_key.is_none()
                && slot_a_cfg.stt_provider == "google"
                && slot_a_cfg.mt_provider == "google"
                && !cfg_snapshot.tts_enabled
            {
                // Preserve the no-key startup path: users can still verify
                // audio capture and settings before adding a Google key.
                tracing::info!(
                    "no google_api_key configured; running without STT/MT/TTS (issue #84)"
                );
                spawn_metrics_only_audio_task(&rt, stream, &state, &loss_metrics);
                orchestrator_join = None;
            } else {
                // Build the selected STT provider plus Google MT/TTS, then
                // start the orchestrator.
                // Reuse the Arc already held in AppState so hot-reload writes
                // are visible to the running orchestrator.
                let google_api_key = cfg_snapshot.google_api_key.as_deref();
                let source_language = Arc::clone(&state.source_language);

                // Issue #230 + #214 + #371: shared STT-local flag read by the
                // CPU gate.  Starts true only for configured local STT; may be
                // updated by the fallback provider at runtime.
                let provider_is_local =
                    Arc::new(AtomicBool::new(slot_a_cfg.stt_provider == "local"));

                // local_unavailable_is_fatal: when true, a local-unavailable
                // error should halt the pipeline (no fallback available).
                // With google-when-keyed and a key, the FallbackSttProvider handles the
                // switch. Without a key, no cloud fallback is wired, so a permanent local
                // setup error must halt instead of spinning on every audio window.
                let local_unavailable_is_fatal = stt_local_unavailable_is_fatal_for_slot(
                    &slot_a_cfg.stt_provider,
                    &cfg_snapshot,
                );

                let stt_provider = match build_slot_stt_provider(
                    &slot_a_cfg.stt_provider,
                    &cfg_snapshot,
                    google_api_key,
                    Arc::clone(&state.pipeline_error_msg),
                    Arc::clone(&provider_is_local),
                    Arc::clone(&state.stt_source),
                ) {
                    Ok(p) => p,
                    Err(err) => {
                        tracing::error!("failed to create STT provider: {err}");
                        let provider_msg = format!("Speech-to-text unavailable: {err}");
                        *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
                            metrics::SttState::Error(provider_msg.clone());
                        *state
                            .pipeline_error_msg
                            .lock()
                            .unwrap_or_else(|p| p.into_inner()) = Some(provider_msg);
                        // Fall back to metrics-only audio task.
                        spawn_metrics_only_audio_task(&rt, stream, &state, &loss_metrics);
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
                                orchestrator_join_b: None,
                                orchestrator_shutdown,
                                process_rx: process_rx.clone(),
                                e2e_latency: Arc::clone(&e2e_latency),
                                network_metrics: Arc::clone(&network_metrics),
                                loss_metrics: Arc::clone(&loss_metrics),
                                cpu_gate: Arc::clone(&cpu_gate),
                                memory_guard: Arc::clone(&memory_guard),
                                storage,
                                fanout_counters: Arc::clone(&fanout_counters),
                                capture_router_metrics: capture_router_metrics.clone(),
                            },
                        );
                    }
                };
                *state.stt_source.lock().unwrap_or_else(|p| p.into_inner()) =
                    stt_provider.initial_stt_source();
                provider_is_local
                    .store(stt_provider.initial_provider_is_local(), Ordering::Relaxed);
                // Issue #71–#76 and #217: wire the configured MT provider.
                // Google reports billable character usage; local OPUS-MT ignores
                // the reporter but shares the same runtime trait.
                let mt_provider = match build_slot_mt_provider(
                    &slot_a_cfg.mt_provider,
                    google_api_key,
                    cfg_snapshot.mt_cloud_fallback.as_deref(),
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
                        spawn_metrics_only_audio_task(&rt, stream, &state, &loss_metrics);
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
                                orchestrator_join_b: None,
                                orchestrator_shutdown,
                                process_rx: process_rx.clone(),
                                e2e_latency: Arc::clone(&e2e_latency),
                                network_metrics: Arc::clone(&network_metrics),
                                loss_metrics: Arc::clone(&loss_metrics),
                                cpu_gate: Arc::clone(&cpu_gate),
                                memory_guard: Arc::clone(&memory_guard),
                                storage,
                                fanout_counters: Arc::clone(&fanout_counters),
                                capture_router_metrics: capture_router_metrics.clone(),
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
                        spawn_metrics_only_audio_task(&rt, stream, &state, &loss_metrics);
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
                                orchestrator_join_b: None,
                                orchestrator_shutdown,
                                process_rx: process_rx.clone(),
                                e2e_latency: Arc::clone(&e2e_latency),
                                network_metrics: Arc::clone(&network_metrics),
                                loss_metrics: Arc::clone(&loss_metrics),
                                cpu_gate: Arc::clone(&cpu_gate),
                                memory_guard: Arc::clone(&memory_guard),
                                storage,
                                fanout_counters: Arc::clone(&fanout_counters),
                                capture_router_metrics: capture_router_metrics.clone(),
                            },
                        );
                    }
                };
                let (recorder_slot_suffix, recorder_slot_label) =
                    if slot_mode == config::SlotMode::Dual {
                        (Some("a"), Some("A"))
                    } else {
                        (None, None)
                    };
                let session_recorder = start_session_recorder(
                    &rt,
                    &cfg_snapshot,
                    &state.pipeline_error_msg,
                    started_at_unix_ms,
                    &session_id,
                    recorder_slot_suffix,
                    recorder_slot_label,
                    true,
                );
                log_measurement_mode_status(
                    &session_id,
                    session_recorder.path().as_deref(),
                    audio_archive_path.as_deref(),
                    &state.pipeline_error_msg,
                );

                // Issue #393: extract shared storage handles before the recorder
                // is moved into the orchestrator context.
                storage.recorder_bytes = session_recorder.bytes_written_arc();
                storage.recorder_path = session_recorder.path();

                // DM-06 (issue #382): in dual mode, slot A gets its own per-slot
                // halt Arc so an auth error on A does not globally halt while B is
                // still healthy.  A background task (spawned below, inside the dual
                // block) aggregates both per-slot flags into `state.pipeline_halted`.
                // In single mode we share the global Arc directly, identical to the
                // pre-DM-06 behaviour.
                let slot_a_ui_arcs = slot_a_ui_arcs_for_mode(slot_mode, &state);

                // DM-06 (issue #382): per-slot TTS status Arc shared with a label
                // copier task so the UI can observe it.  Previously this was a fresh
                // orphaned Arc; now any write by the orchestrator is visible outside.
                let slot_a_tts_status_arc = Arc::new(Mutex::new(pipeline::SlotProviderStatus::Ok));

                // ── #675 WTP model pre-flight ─────────────────────────────────────────────
                #[cfg(feature = "semantic-buffering-wtp")]
                let resolved_wtp_model_dir: Option<String> = {
                    let sb = &cfg_snapshot.pipeline.semantic_buffering;
                    if sb.enabled && sb.wtp_judge_enabled {
                        match rt.block_on(
                            pipeline::completeness::wtp_bootstrap::ensure_wtp_model_ready(
                                cfg_snapshot
                                    .pipeline
                                    .semantic_buffering
                                    .wtp_model_dir
                                    .as_deref(),
                                None,
                            ),
                        ) {
                            Ok(dir) => {
                                tracing::info!(dir = %dir.display(), "WTP model ready");
                                Some(dir.to_string_lossy().into_owned())
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "WTP unavailable, falling back to RuleBasedJudge: {e:#}"
                                );
                                None
                            }
                        }
                    } else {
                        cfg_snapshot
                            .pipeline
                            .semantic_buffering
                            .wtp_model_dir
                            .clone()
                    }
                };
                #[cfg(not(feature = "semantic-buffering-wtp"))]
                let resolved_wtp_model_dir = cfg_snapshot
                    .pipeline
                    .semantic_buffering
                    .wtp_model_dir
                    .clone();

                let ctx = pipeline::OrchestratorContext {
                    slot_id: pipeline::SlotId::A,
                    audio_level: Arc::clone(&state.audio_level),
                    stt_state: Arc::clone(&state.stt_state),
                    mt_state: Arc::clone(&state.mt_state),
                    subtitle_pane: Arc::clone(&state.subtitle_pane),
                    session_metrics: Arc::clone(&state.session_metrics),
                    cost_counter: Arc::clone(&state.cost_counter),
                    pipeline_error_msg: Arc::clone(&slot_a_ui_arcs.pipeline_error_msg),
                    auth_error_banner: Arc::clone(&slot_a_ui_arcs.auth_error_banner),
                    pipeline_halted: Arc::clone(&slot_a_ui_arcs.pipeline_halted),
                    provider_circuits: Arc::new(std::sync::Mutex::new(
                        pipeline::ProviderCircuitBreakers::default(),
                    )),
                    paused: Arc::clone(&state.paused),
                    tts_enabled: Arc::clone(&state.tts_enabled),
                    source_language,
                    target_language: if slot_mode == config::SlotMode::Dual {
                        Arc::new(std::sync::Mutex::new(slot_a_cfg.target_language.clone()))
                    } else {
                        Arc::clone(&state.target_language)
                    },
                    stt_provider_name: slot_a_cfg.stt_provider.clone(),
                    mt_provider_name: slot_a_cfg.mt_provider.clone(),
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
                    sentence_aggregator: Arc::new(std::sync::Mutex::new({
                        let mut agg =
                            pipeline::sentence_aggregator::SentenceAggregator::with_max_age(
                                std::time::Duration::from_millis(
                                    cfg_snapshot.pipeline.sentence_max_age_ms,
                                ),
                            );
                        if let Some(judge) = pipeline::completeness::build_judge(
                            cfg_snapshot.pipeline.semantic_buffering.enabled,
                            cfg_snapshot.pipeline.semantic_buffering.wtp_judge_enabled,
                            resolved_wtp_model_dir.as_deref(),
                            cfg_snapshot
                                .pipeline
                                .semantic_buffering
                                .min_confidence_threshold,
                        ) {
                            agg = agg.with_judge(judge);
                        }
                        agg
                    })),
                    session_recorder,
                    // Slot A is `slot_is_a = true`; mode is single unless slot_mode == Dual.
                    tts_active_for_slot: cfg_snapshot
                        .tts_source
                        .is_active_for_slot(true, slot_mode == config::SlotMode::Dual),
                    // DM-06 (issue #382): share the Arc created above so it is
                    // accessible to the halt-aggregation and label-copier tasks.
                    tts_status: Arc::clone(&slot_a_tts_status_arc),
                };

                orchestrator_join = Some(rt.spawn(pipeline::run_orchestrator(
                    stream.receiver,
                    stt_provider,
                    mt_provider,
                    tts_provider,
                    ctx,
                )));

                // DM-03 (issue #379): in dual-slot mode, build an independent
                // provider set and orchestrator for slot B.  Shared fields
                // (audio_level, paused, shutdown, cost_counter, aggregate
                // metrics/gates) come from the same Arcs used by slot A.
                // Per-slot fields (stt_state, subtitle_pane, session_metrics,
                // pipeline_error_msg, auth_error_banner, pipeline_halted,
                // stt_source) get their own fresh Arcs.
                if slot_mode == config::SlotMode::Dual {
                    if let Some(slot_b_rx) = slot_b_receiver {
                        let slot_b_cfg = cfg_snapshot
                            .slot_b()
                            .expect("slot_b() is Some when slot_mode is Dual");
                        let google_api_key_b = cfg_snapshot.google_api_key.as_deref();
                        let provider_is_local_b =
                            Arc::new(AtomicBool::new(slot_b_cfg.stt_provider == "local"));
                        let local_unavailable_is_fatal_b = stt_local_unavailable_is_fatal_for_slot(
                            &slot_b_cfg.stt_provider,
                            &cfg_snapshot,
                        );
                        let slot_b_state =
                            pipeline::SlotOrchestratorState::new(pipeline::SlotId::B);

                        let stt_b = build_slot_stt_provider(
                            &slot_b_cfg.stt_provider,
                            &cfg_snapshot,
                            google_api_key_b,
                            Arc::clone(&slot_b_state.pipeline_error_msg),
                            Arc::clone(&provider_is_local_b),
                            Arc::clone(&slot_b_state.stt_source),
                        );
                        let mt_b = build_slot_mt_provider(
                            &slot_b_cfg.mt_provider,
                            google_api_key_b,
                            cfg_snapshot.mt_cloud_fallback.as_deref(),
                            Arc::clone(&state.cost_counter) as Arc<dyn providers::CostReporter>,
                        );

                        match (stt_b, mt_b) {
                            (Ok(stt_b), Ok(mt_b)) => {
                                *slot_b_state
                                    .stt_source
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner()) =
                                    stt_b.initial_stt_source();
                                provider_is_local_b
                                    .store(stt_b.initial_provider_is_local(), Ordering::Relaxed);
                                *slot_b_state
                                    .stt_state
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner()) =
                                    metrics::SttState::Listening;
                                let ctx_b = pipeline::OrchestratorContext {
                                    slot_id: pipeline::SlotId::B,
                                    audio_level: Arc::clone(&state.audio_level),
                                    stt_state: Arc::clone(&slot_b_state.stt_state),
                                    mt_state: Arc::clone(&slot_b_state.mt_state),
                                    subtitle_pane: Arc::clone(&slot_b_state.subtitle_pane),
                                    session_metrics: Arc::clone(&slot_b_state.session_metrics),
                                    cost_counter: Arc::clone(&state.cost_counter),
                                    pipeline_error_msg: Arc::clone(
                                        &slot_b_state.pipeline_error_msg,
                                    ),
                                    auth_error_banner: Arc::clone(&slot_b_state.auth_error_banner),
                                    pipeline_halted: Arc::clone(&slot_b_state.pipeline_halted),
                                    provider_circuits: Arc::new(std::sync::Mutex::new(
                                        pipeline::ProviderCircuitBreakers::default(),
                                    )),
                                    paused: Arc::clone(&state.paused),
                                    tts_enabled: Arc::clone(&state.tts_enabled),
                                    source_language: Arc::clone(&state.source_language),
                                    target_language: Arc::new(std::sync::Mutex::new(
                                        slot_b_cfg.target_language.clone(),
                                    )),
                                    stt_provider_name: slot_b_cfg.stt_provider.clone(),
                                    mt_provider_name: slot_b_cfg.mt_provider.clone(),
                                    playback: Arc::clone(&playback_service),
                                    shutdown: Arc::clone(&orchestrator_shutdown),
                                    e2e_latency: Arc::clone(&e2e_latency),
                                    network_metrics: Arc::clone(&network_metrics),
                                    loss_metrics: Arc::clone(&loss_metrics),
                                    cpu_gate: Arc::clone(&cpu_gate),
                                    provider_is_local: provider_is_local_b,
                                    local_unavailable_is_fatal: local_unavailable_is_fatal_b,
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
                                    pipeline_max_window_ms: cfg_snapshot.pipeline.max_window_ms,
                                    pipeline_early_flush_on_vad_end: cfg_snapshot
                                        .pipeline
                                        .early_flush_on_vad_end,
                                    pipeline_idle_flush_ms: cfg_snapshot.pipeline.idle_flush_ms,
                                    pipeline_idle_min_ms: cfg_snapshot.pipeline.idle_min_ms,
                                    stabilizer: Arc::new(std::sync::Mutex::new(
                                        pipeline::segmentation::SegmentStabilizer::new(),
                                    )),
                                    sentence_aggregator: Arc::new(std::sync::Mutex::new({
                                        let mut agg =
                                            pipeline::sentence_aggregator::SentenceAggregator::with_max_age(
                                                std::time::Duration::from_millis(
                                                    cfg_snapshot.pipeline.sentence_max_age_ms,
                                                ),
                                            );
                                        if let Some(judge) = pipeline::completeness::build_judge(
                                            cfg_snapshot.pipeline.semantic_buffering.enabled,
                                            cfg_snapshot
                                                .pipeline
                                                .semantic_buffering
                                                .wtp_judge_enabled,
                                            resolved_wtp_model_dir.as_deref(),
                                            cfg_snapshot
                                                .pipeline
                                                .semantic_buffering
                                                .min_confidence_threshold,
                                        ) {
                                            agg = agg.with_judge(judge);
                                        }
                                        agg
                                    })),
                                    // DM-05: start per-slot session recorder for slot B.
                                    // Uses the same session_id and directory as slot A
                                    // (same per-session subdir) but writes to 00001-b.jsonl.
                                    // apply_retention=false because slot A's recorder already
                                    // ran the storage-cap/TTL sweep.
                                    session_recorder: start_session_recorder(
                                        &rt,
                                        &cfg_snapshot,
                                        &slot_b_state.pipeline_error_msg,
                                        started_at_unix_ms,
                                        &session_id,
                                        Some("b"),
                                        Some("B"),
                                        false,
                                    ),
                                    // DM-06 (issue #382): slot B is always dual; slot_is_a = false.
                                    tts_active_for_slot: cfg_snapshot
                                        .tts_source
                                        .is_active_for_slot(false, true),
                                    tts_status: Arc::clone(&slot_b_state.tts_status),
                                };
                                orchestrator_join_b = Some(rt.spawn(pipeline::run_orchestrator(
                                    slot_b_rx,
                                    stt_b,
                                    mt_b,
                                    RuntimeTtsProvider::Disabled(DisabledTtsProvider),
                                    ctx_b,
                                )));
                                tracing::info!("dual-slot mode: slot B orchestrator started");
                                state.wire_slot_b(
                                    Arc::clone(&slot_b_state.subtitle_pane),
                                    slot_b_cfg.target_language.clone(),
                                    slot_b_cfg.mt_provider.clone(),
                                );

                                // DM-06 (issue #382): halt-aggregation background task.
                                // Computes `aggregate_halt_state(a, Some(b))` at 100 ms cadence
                                // and writes the result to `state.pipeline_halted`.  This means
                                // a slot-A auth error halts A locally but leaves the global flag
                                // clear (and B running) until BOTH slots are halted.
                                {
                                    let a_halt = Arc::clone(&slot_a_ui_arcs.pipeline_halted);
                                    let b_halt = Arc::clone(&slot_b_state.pipeline_halted);
                                    let global_halt = Arc::clone(&state.pipeline_halted);
                                    rt.spawn(async move {
                                        let mut interval =
                                            tokio::time::interval(Duration::from_millis(100));
                                        loop {
                                            interval.tick().await;
                                            let a = a_halt.load(Ordering::Relaxed);
                                            let b = b_halt.load(Ordering::Relaxed);
                                            global_halt.store(
                                                pipeline::aggregate_halt_state(a, Some(b)),
                                                Ordering::Relaxed,
                                            );
                                        }
                                    });
                                }
                                {
                                    let auth_arc = Arc::clone(&slot_a_ui_arcs.auth_error_banner);
                                    let err_arc = Arc::clone(&slot_a_ui_arcs.pipeline_error_msg);
                                    let label = Arc::clone(&state.slot_a_error_status_label);
                                    rt.spawn(async move {
                                        let mut interval =
                                            tokio::time::interval(Duration::from_millis(200));
                                        loop {
                                            interval.tick().await;
                                            let auth = auth_arc
                                                .lock()
                                                .unwrap_or_else(|p| p.into_inner())
                                                .clone();
                                            let err = err_arc
                                                .lock()
                                                .unwrap_or_else(|p| p.into_inner())
                                                .clone();
                                            *label.lock().unwrap_or_else(|p| p.into_inner()) =
                                                format_slot_error_status(auth, err);
                                        }
                                    });
                                }
                                {
                                    let auth_arc = Arc::clone(&slot_b_state.auth_error_banner);
                                    let err_arc = Arc::clone(&slot_b_state.pipeline_error_msg);
                                    let label = Arc::clone(&state.slot_b_error_status_label);
                                    rt.spawn(async move {
                                        let mut interval =
                                            tokio::time::interval(Duration::from_millis(200));
                                        loop {
                                            interval.tick().await;
                                            let auth = auth_arc
                                                .lock()
                                                .unwrap_or_else(|p| p.into_inner())
                                                .clone();
                                            let err = err_arc
                                                .lock()
                                                .unwrap_or_else(|p| p.into_inner())
                                                .clone();
                                            *label.lock().unwrap_or_else(|p| p.into_inner()) =
                                                format_slot_error_status(auth, err);
                                        }
                                    });
                                }

                                // DM-06 (issue #382): TTS status label copier tasks.
                                // Poll each slot's SlotProviderStatus at 200 ms and write
                                // the formatted string to the AppState label Arcs so the TUI
                                // can show per-slot TTS health without a pipeline-to-tui import.
                                {
                                    let tts_arc = Arc::clone(&slot_a_tts_status_arc);
                                    let label = Arc::clone(&state.slot_a_tts_status_label);
                                    rt.spawn(async move {
                                        let mut interval =
                                            tokio::time::interval(Duration::from_millis(200));
                                        loop {
                                            interval.tick().await;
                                            let s = tts_arc
                                                .lock()
                                                .unwrap_or_else(|p| p.into_inner())
                                                .to_string();
                                            *label.lock().unwrap_or_else(|p| p.into_inner()) = s;
                                        }
                                    });
                                }
                                {
                                    let tts_arc = Arc::clone(&slot_b_state.tts_status);
                                    let label = Arc::clone(&state.slot_b_tts_status_label);
                                    rt.spawn(async move {
                                        let mut interval =
                                            tokio::time::interval(Duration::from_millis(200));
                                        loop {
                                            interval.tick().await;
                                            let s = tts_arc
                                                .lock()
                                                .unwrap_or_else(|p| p.into_inner())
                                                .to_string();
                                            *label.lock().unwrap_or_else(|p| p.into_inner()) = s;
                                        }
                                    });
                                }
                            }
                            (Err(err), _) => {
                                tracing::error!(
                                    "dual-slot mode: failed to build slot B STT provider, \
                                     slot B will not run: {err}"
                                );
                            }
                            (_, Err(err)) => {
                                tracing::error!(
                                    "dual-slot mode: failed to build slot B MT provider, \
                                     slot B will not run: {err}"
                                );
                            }
                        }
                    }
                }
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
            orchestrator_join_b,
            orchestrator_shutdown,
            process_rx,
            e2e_latency,
            network_metrics,
            loss_metrics,
            cpu_gate,
            memory_guard,
            storage,
            fanout_counters,
            capture_router_metrics,
        },
    )
}

/// Spawn the legacy metrics-only audio task (used when no API key is set).
fn spawn_metrics_only_audio_task(
    rt: &tokio::runtime::Runtime,
    mut stream: audio::CaptureStream,
    state: &AppState,
    loss_metrics: &Arc<LossMetrics>,
) {
    let level_tx = Arc::clone(&state.audio_level);
    let paused = Arc::clone(&state.paused);
    let session_metrics = Arc::clone(&state.session_metrics);
    let stt_state = Arc::clone(&state.stt_state);
    let loss_metrics = Arc::clone(loss_metrics);
    let metrics_only_cost_counter = Arc::new(metrics::CostCounter::new());
    rt.spawn(async move {
        loop {
            let Some(chunk) = stream.receiver.recv().await else {
                mark_audio_capture_stopped(&level_tx, &stt_state);
                break;
            };
            loss_metrics.record_chunk();
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
        orchestrator_join_b,
        orchestrator_shutdown,
        process_rx,
        e2e_latency,
        network_metrics,
        loss_metrics,
        cpu_gate,
        memory_guard,
        storage,
        fanout_counters,
        capture_router_metrics,
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
        let fanout_counters = Arc::clone(&fanout_counters);
        let capture_router_metrics = capture_router_metrics.clone();
        let mut process_rx = process_rx;
        let metrics_snapshot_path = std::env::var_os(METRICS_SNAPSHOT_ENV).map(PathBuf::from);
        // Issue #393: clone the storage handles for the publisher task.
        let storage_recorder_bytes = Arc::clone(&storage.recorder_bytes);
        let storage_recorder_path = storage.recorder_path.clone();
        let storage_archive_bytes = Arc::clone(&storage.archive_bytes);
        let storage_archive_sealed = Arc::clone(&storage.archive_sealed);
        // Path of the configured WAV target. It remains after a runtime
        // write-error disable; that disabled state is surfaced via status text.
        let storage_archive_path = storage.archive_path.clone();
        rt.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            let mut last_ram_budget_bytes = u64::MAX;
            let mut last_cpu_budget_pct_x100 = u32::MAX;
            let mut last_local_inferences_skipped = 0_u64;
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
                let (ram_budget_mb, cpu_budget_pct) = {
                    let cfg = current_config.lock().unwrap_or_else(|p| p.into_inner());
                    (cfg.ram_budget_mb, cfg.cpu_budget_pct)
                };
                let ram_budget_bytes = ram_budget_mb.saturating_mul(1024 * 1024);
                if ram_budget_bytes != last_ram_budget_bytes {
                    memory_guard.update_budget_bytes(ram_budget_bytes);
                    last_ram_budget_bytes = ram_budget_bytes;
                }
                // HC-04 (issue #389): hot-apply cpu_budget_pct changes without
                // restart.  Mirrors the RAM budget pattern above for symmetry.
                let cpu_budget_pct_x100 = (cpu_budget_pct * 100.0) as u32;
                if cpu_budget_pct_x100 != last_cpu_budget_pct_x100 {
                    cpu_gate.update_budget_pct(cpu_budget_pct);
                    last_cpu_budget_pct_x100 = cpu_budget_pct_x100;
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
                let local_inferences_skipped = cpu_gate.skipped_count();
                let local_skip_advanced = local_inferences_skipped > last_local_inferences_skipped;
                last_local_inferences_skipped = local_inferences_skipped;
                snapshot.local_inferences_skipped = local_inferences_skipped;
                // LF-02 (issue #370): publish local-inference runtime caps
                // observability (in-flight operation gauge + local CPU mirror).
                let active_local = providers::local::runtime_caps::active_local_threads();
                let active_local_u32 = u32::try_from(active_local).unwrap_or(u32::MAX);
                snapshot.apply_local_runtime(active_local_u32, local_skip_advanced);
                // Issue #393: apply storage metrics from the session recorder and
                // audio archive.
                snapshot.apply_storage(
                    storage_recorder_bytes.load(Ordering::Relaxed),
                    storage_recorder_path.clone(),
                    storage_archive_bytes.load(Ordering::Relaxed),
                    storage_archive_path.clone(),
                    storage_archive_sealed.load(Ordering::Relaxed),
                );

                // DM-02 (issue #378): publish per-slot fanout drop counters.
                snapshot.apply_fanout_drops(
                    fanout_counters.drops(audio::SLOT_A),
                    fanout_counters.drops(audio::SLOT_B),
                );
                if let Some(router_metrics) = capture_router_metrics.as_ref() {
                    snapshot.apply_capture_router_metrics(
                        router_metrics.swap_count(),
                        router_metrics.dropped_during_swap(),
                    );
                }

                if let Some(path) = metrics_snapshot_path.as_deref() {
                    if let Err(err) = write_metrics_snapshot_export(path, &snapshot) {
                        tracing::warn!(
                            path = %path.display(),
                            "failed to write soak metrics snapshot: {err}"
                        );
                    }
                }
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
        let picker_field_active = Arc::clone(&state.picker_field_active);
        let wizard_active_kb = Arc::clone(&state.wizard_active);
        let keyboard_shutdown = Arc::clone(&keyboard_shutdown);
        rt.spawn(async move {
            tokio::task::spawn_blocking(move || {
                keyboard_task(
                    key_tx,
                    lang_flag,
                    config_editor_flag,
                    picker_field_active,
                    wizard_active_kb,
                    keyboard_shutdown,
                )
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
    // QA8-07 (#505): record the cancellation issuance **before**
    // publishing the shutdown flag. The `Release` store inside
    // `cancellation_hook::issue()` (on `ISSUE_AT_NS`) must
    // happens-before any orchestrator observer that subsequently
    // reads `shutdown=true` and calls `cancellation_hook::exit()`.
    // Reversing this order races: an observer can wake on
    // `shutdown=true`, load `ISSUE_AT_NS=0`, and short-circuit
    // `exit()`, dropping the cancellation-latency sample QA8-07
    // exists to capture.
    pipeline::cancellation_hook::issue();
    // Signal the orchestrator to stop processing new chunks.
    orchestrator_shutdown.store(true, Ordering::Relaxed);
    // Wait up to 2 seconds for any in-progress STT/MT/TTS call to finish.
    // DM-03: both slot A and slot B orchestrators are drained concurrently.
    rt.block_on(async {
        let timeout = Duration::from_secs(2);
        let join_a = async {
            if let Some(join_a) = orchestrator_join {
                let _ = tokio::time::timeout(timeout, join_a).await;
            }
        };
        let join_b = async {
            if let Some(join_b) = orchestrator_join_b {
                let _ = tokio::time::timeout(timeout, join_b).await;
            }
        };
        tokio::join!(join_a, join_b);
    });

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
///
/// Startup calls [`migrate_legacy_config_to_per_user_if_needed`] before this
/// lookup so legacy side-by-side configs are copied to the per-user path once
/// without changing portable-mode precedence.
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
    tracing::info!(
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

/// Build a list of LocalModelLicense for all bundled local models.
fn build_local_model_licenses() -> Vec<LocalModelLicense> {
    all_local_consent_manifests()
        .into_iter()
        .map(|manifest| LocalModelLicense {
            display_name: manifest.name,
            license_text: manifest.license_text,
        })
        .collect()
}

fn whisper_tiny_consent_manifest() -> Option<providers::local::ModelConsentManifest> {
    use providers::local::{bootstrap::ModelBootstrapManifest, ModelId, ModelManifest};
    ModelManifest::builtin()
        .find(ModelId::Tiny)
        .map(|spec| ModelBootstrapManifest::from_spec(spec, "2024-02-01"))
        .map(|manifest| providers::local::ModelConsentManifest::from(&manifest))
}

fn all_local_consent_manifests() -> Vec<providers::local::ModelConsentManifest> {
    let mut manifests = Vec::new();
    if let Some(manifest) = whisper_tiny_consent_manifest() {
        manifests.push(manifest);
    }
    manifests.push(providers::local::opus_mt_ja_vi_consent_manifest());
    manifests
}

fn local_model_licenses_from_manifests(
    manifests: &[providers::local::ModelConsentManifest],
) -> Vec<LocalModelLicense> {
    manifests
        .iter()
        .map(|manifest| LocalModelLicense {
            display_name: manifest.name.clone(),
            license_text: manifest.license_text.clone(),
        })
        .collect()
}

/// Collect local model consent manifests that need consent (missing or stale).
fn collect_pending_consent_manifests(
    cfg: &config::AppConfig,
) -> Vec<providers::local::ModelConsentManifest> {
    use providers::local::{model_consent_status, ConsentStatus};
    let mut pending = Vec::new();

    if cfg.stt_provider == "local" {
        if let Some(manifest) = whisper_tiny_consent_manifest() {
            match model_consent_status(&manifest) {
                Ok(ConsentStatus::Fresh) => {}
                Ok(ConsentStatus::Missing | ConsentStatus::Stale { .. }) | Err(_) => {
                    pending.push(manifest);
                }
            }
        }
    }
    if cfg.mt_provider == "local" {
        let manifest = providers::local::opus_mt_ja_vi_consent_manifest();
        match model_consent_status(&manifest) {
            Ok(ConsentStatus::Fresh) => {}
            Ok(ConsentStatus::Missing | ConsentStatus::Stale { .. }) | Err(_) => {
                pending.push(manifest);
            }
        }
    }
    pending
}

/// Determine the onboarding branch corresponding to an existing config.
#[allow(dead_code)]
fn branch_from_config(cfg: &config::AppConfig) -> OnboardingBranch {
    let local_stt = cfg.stt_provider == "local";
    let local_mt = cfg.mt_provider == "local";
    let has_key = cfg
        .google_api_key
        .as_deref()
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    let has_cloud_fallback = cfg.mt_cloud_fallback.is_some();

    if local_stt && local_mt && (has_key || has_cloud_fallback) {
        OnboardingBranch::LocalGoogleFallback
    } else if !local_stt && !local_mt {
        OnboardingBranch::GoogleCloud
    } else {
        OnboardingBranch::LocalOnly
    }
}

fn write_model_consent_records(
    manifests: &[providers::local::ModelConsentManifest],
) -> anyhow::Result<()> {
    for manifest in manifests {
        providers::local::write_model_consent_record(manifest).with_context(|| {
            format!(
                "failed to write consent record for local model {} {}",
                manifest.name, manifest.version
            )
        })?;
    }
    Ok(())
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

fn populate_virtual_mic_device_options(state: &AppState) {
    match audio::probe_virtual_audio_devices() {
        Ok(devices) => {
            let names = devices.into_iter().map(|device| device.name).collect();
            let _ = state.with_config_editor_mut(|editor| {
                editor.set_virtual_mic_device_options(names);
            });
        }
        Err(err) => {
            tracing::warn!("failed to probe virtual audio devices for settings picker: {err:#}");
        }
    }
}

fn populate_config_editor_device_options(state: &AppState) {
    populate_capture_device_options(state);
    populate_virtual_mic_device_options(state);
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

/// Update `current_config`, language strings, capture-device label, TTS state,
/// audio-consent gate, and the restart-required flag from `next_cfg`.
///
/// Called on live config hot-reload (R key), after saving the settings overlay,
/// and from the file-watcher task.
#[allow(clippy::too_many_arguments)]
fn apply_runtime_config(
    current_config: &Arc<Mutex<config::AppConfig>>,
    target_language: &Arc<std::sync::Mutex<String>>,
    source_language: &Arc<std::sync::Mutex<String>>,
    capture_device_label: &Arc<std::sync::Mutex<String>>,
    slot_a_provider_name: &Arc<Mutex<String>>,
    tts_enabled: &Arc<AtomicBool>,
    audio_consent: &Arc<AtomicBool>,
    restart_required: &Arc<AtomicBool>,
    playback_service: &SharedPlaybackService,
    next_cfg: config::AppConfig,
) -> (
    bool, /* requires_restart */
    bool, /* actually_changed */
) {
    let (
        actually_changed,
        capture_change,
        requires_restart,
        previous_capture_device,
        previous_audio_source,
        previous_audio_file_path,
    ) = {
        let current = current_config.lock().unwrap_or_else(|p| p.into_inner());
        (
            *current != next_cfg,
            config::classify_capture_change(&current, &next_cfg),
            current.requires_restart_ignoring_capture(&next_cfg),
            current.capture_device.clone(),
            current.audio_source.clone(),
            current.audio_file_path.clone(),
        )
    };

    let mut apply_requires_restart = requires_restart;
    let mut capture_apply_failed = false;
    match &capture_change {
        config::CaptureChangeOutcome::Rejected { reason } => {
            set_capture_hot_swap_status(format!("Capture config rejected: {reason}"));
            tracing::warn!(reason = %reason, "capture config change rejected");
            capture_apply_failed = true;
        }
        config::CaptureChangeOutcome::NeedsCaptureHotSwap { reason, .. } if !requires_restart => {
            match apply_capture_hot_swap(&next_cfg, reason) {
                CaptureHotSwapApply::Applied => {}
                CaptureHotSwapApply::RestartRequired => {
                    restart_required.store(true, Ordering::Relaxed);
                    apply_requires_restart = true;
                }
                CaptureHotSwapApply::Failed => {
                    capture_apply_failed = true;
                }
            }
        }
        _ => {}
    }

    let mut effective_cfg = next_cfg.clone();
    if capture_apply_failed {
        effective_cfg.capture_device = previous_capture_device;
        effective_cfg.audio_source = previous_audio_source;
        effective_cfg.audio_file_path = previous_audio_file_path;
    }

    {
        let mut current = current_config.lock().unwrap_or_else(|p| p.into_inner());
        *current = effective_cfg.clone();
    }

    if requires_restart {
        restart_required.store(true, Ordering::Relaxed);
    }
    if capture_apply_failed {
        tracing::warn!("capture change was not applied; non-capture config updates still applied");
    }

    overwrite_target_language(target_language, &next_cfg.target_language);
    overwrite_source_language(source_language, &next_cfg.source_language);
    overwrite_capture_device_label(capture_device_label, &effective_cfg.capture_device);
    *slot_a_provider_name
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = effective_cfg.slot_a().mt_provider.clone();
    // Sync the backend first; only set the UI flag to match what actually succeeded.
    let service_ok =
        sync_playback_service_state(playback_service, &effective_cfg, effective_cfg.tts_enabled);
    tts_enabled.store(effective_cfg.tts_enabled && service_ok, Ordering::Relaxed);
    audio_consent.store(effective_cfg.audio_archive.consent_given, Ordering::Relaxed);
    // CTRL-01: mirror persisted gain/volume into the runtime controllers so
    // a hot-reloaded config.json takes effect without restarting the audio
    // capture or playback threads.
    audio::audio_gain::set_input_gain_db(effective_cfg.input_gain_db);
    audio::audio_gain::set_output_volume_db(effective_cfg.output_volume_db);

    // I18N-01 (issue #481): locale is a hot field.  Update the global
    // catalog so the next frame renders migrated TUI strings (help
    // overlay etc.) in the new locale without a TUI restart.
    i18n::set_locale(&effective_cfg.locale);

    // CTRL-02 (issue #455): tts_voice is a hot field.  Apply it through the
    // shared runtime handle so any later utterance picks up the new voice on
    // its next synthesise call.  Errors are logged here; the V key surfaces
    // user-initiated swap errors visibly via pipeline_error_msg.
    if let Err(err) = apply_tts_voice_from_config(effective_cfg.tts_voice.as_deref()) {
        tracing::warn!(
            error = %err,
            "could not apply tts_voice from reloaded config; keeping previous voice"
        );
    }
    (apply_requires_restart, actually_changed)
}

enum CaptureHotSwapApply {
    Applied,
    RestartRequired,
    Failed,
}

fn apply_capture_hot_swap(next_cfg: &config::AppConfig, reason: &str) -> CaptureHotSwapApply {
    let spec = match capture_source_spec_from_config(next_cfg) {
        Ok(spec) => spec,
        Err(err) => {
            set_capture_hot_swap_status(format!("Capture config rejected: {err}"));
            tracing::warn!("capture config cannot be hot-swapped: {err:#}");
            return CaptureHotSwapApply::Failed;
        }
    };

    let Some(runtime) = CAPTURE_HOT_SWAP_RUNTIME.get().cloned() else {
        tracing::warn!(
            reason = %reason,
            "capture router runtime is unavailable; restart required for capture change"
        );
        return CaptureHotSwapApply::RestartRequired;
    };
    let Some(router_handle) = runtime
        .router_handle
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
    else {
        tracing::warn!(
            reason = %reason,
            "capture router handle is unavailable; restart required for capture change"
        );
        return CaptureHotSwapApply::RestartRequired;
    };

    let spec_label = spec.label();
    tracing::info!(reason = %reason, source = %spec_label, "capture hot-swap requested");
    match runtime
        .runtime_handle
        .block_on(router_handle.hot_swap(spec, DEFAULT_SILENCE_THRESHOLD))
    {
        Ok(info) => {
            overwrite_device_name(&runtime.device_name, &info.device_name);
            clear_capture_hot_swap_status();
            tracing::info!(
                source = %spec_label,
                device = %info.device_name,
                "capture hot-swap applied"
            );
            CaptureHotSwapApply::Applied
        }
        Err(err) => {
            set_capture_hot_swap_status(format!(
                "Capture change failed; still using previous audio stream: {err}"
            ));
            tracing::warn!(
                source = %spec_label,
                error = %err,
                "capture hot-swap failed; preserving previous stream"
            );
            CaptureHotSwapApply::Failed
        }
    }
}

fn capture_source_spec_from_config(cfg: &config::AppConfig) -> Result<audio::CaptureSourceSpec> {
    match cfg.audio_source.as_str() {
        "wasapi" => Ok(audio::CaptureSourceSpec::Wasapi {
            device: cfg.capture_device.clone(),
        }),
        "file" => {
            let path = cfg
                .audio_file_path
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .context("audio_file_path is required when audio_source is \"file\"")?;
            Ok(audio::CaptureSourceSpec::File {
                path: path.to_string(),
            })
        }
        other => bail!("audio_source must be \"wasapi\" or \"file\", got {other:?}"),
    }
}

fn set_capture_hot_swap_status(message: String) {
    if let Some(runtime) = CAPTURE_HOT_SWAP_RUNTIME.get() {
        *runtime
            .pipeline_error_msg
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(message);
    } else {
        tracing::warn!(message = %message, "capture hot-swap status unavailable");
    }
}

fn clear_capture_hot_swap_status() {
    if let Some(runtime) = CAPTURE_HOT_SWAP_RUNTIME.get() {
        let mut slot = runtime
            .pipeline_error_msg
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if slot.as_deref().is_some_and(is_capture_hot_swap_status) {
            *slot = None;
        }
    }
}

fn is_capture_hot_swap_status(message: &str) -> bool {
    message.starts_with("Capture change failed") || message.starts_with("Capture config rejected")
}

fn config_apply_status_for_watcher_change(
    apply_requires_restart: bool,
    actually_changed: bool,
    pending_restart_reason: Option<String>,
) -> Option<tui::ConfigApplyStatus> {
    if !actually_changed {
        return None;
    }
    if apply_requires_restart {
        Some(tui::ConfigApplyStatus::RestartRequired {
            reason: pending_restart_reason
                .unwrap_or_else(|| "restart required for settings to take effect".to_string()),
        })
    } else {
        Some(tui::ConfigApplyStatus::Ok {
            reason: "settings hot-reloaded".to_string(),
        })
    }
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

fn parse_editor_tts_routing(value: &str) -> Result<config::TtsRouting> {
    match value.trim() {
        "speakers" => Ok(config::TtsRouting::Speakers),
        "virtual_mic" => Ok(config::TtsRouting::VirtualMic),
        "both" => Ok(config::TtsRouting::Both),
        other => bail!(
            "validation failed: `tts_routing` must be \"speakers\", \"virtual_mic\", or \"both\", got {other:?}"
        ),
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
    next_cfg.tts_routing = parse_editor_tts_routing(&editor.tts_routing)?;
    next_cfg.virtual_mic_device = normalize_optional_field(&editor.virtual_mic_device);
    next_cfg.stt_fallback_policy = editor.stt_fallback_policy.trim().to_string();
    if next_cfg.stt_provider == "google" && next_cfg.stt_fallback_policy == "google-when-keyed" {
        next_cfg.stt_fallback_policy = "none".to_string();
    }
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
    let slot_a_cfg = cfg.slot_a();
    match slot_a_cfg.stt_provider.as_str() {
        "google" => {}
        #[cfg(feature = "local-stt")]
        "local" => {}
        #[cfg(not(feature = "local-stt"))]
        "local" => {
            unsupported.push("stt_provider=\"local\" (requires a local-stt build)".to_string())
        }
        _ => unsupported.push(format!("stt_provider={:?}", slot_a_cfg.stt_provider)),
    }
    match slot_a_cfg.mt_provider.as_str() {
        "google" => {}
        #[cfg(feature = "local-mt")]
        "local" => {}
        #[cfg(not(feature = "local-mt"))]
        "local" => {
            unsupported.push("mt_provider=\"local\" (requires a local-mt build)".to_string())
        }
        _ => unsupported.push(format!("mt_provider={:?}", slot_a_cfg.mt_provider)),
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
    let slot_a_cfg = cfg.slot_a();
    if slot_a_cfg.stt_provider == "google" && slot_a_cfg.mt_provider == "google" && !cfg.tts_enabled
    {
        return None;
    }

    let mut requires_key = Vec::new();
    if slot_a_cfg.stt_provider == "google" {
        requires_key.push("Google STT");
    }
    if slot_a_cfg.mt_provider == "google" {
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
        if slot_a_cfg.stt_provider == "google" {
            local_switches.push("stt_provider");
        }
        if slot_a_cfg.mt_provider == "google" {
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
    let (requires_restart, _) = apply_runtime_config(
        current_config,
        &state.target_language,
        &state.source_language,
        &state.capture_device_label,
        &state.slot_a_provider_name,
        &state.tts_enabled,
        &state.audio_consent,
        restart_required,
        playback_service,
        next_cfg,
    );
    let apply_status = if requires_restart {
        tui::ConfigApplyStatus::RestartRequired {
            reason: "restart required for settings to take effect".to_string(),
        }
    } else {
        tui::ConfigApplyStatus::Ok {
            reason: "settings saved".to_string(),
        }
    };
    state.record_config_apply(apply_status);
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

    match pipeline::playback::PlaybackService::new_for_config(
        enabled,
        config.tts_routing,
        config.tts_output_device.as_deref(),
        config.virtual_mic_device.as_deref(),
    ) {
        Ok(service) => {
            tracing::info!(route = service.route_label(), "TTS playback service ready");
            *service_slot = Some(service);
            true
        }
        Err(err) => {
            tracing::warn!(
                route = playback_route_label(config.tts_routing),
                "TTS playback unavailable: {err}"
            );
            false
        }
    }
}

fn playback_route_label(route: config::TtsRouting) -> &'static str {
    match route {
        config::TtsRouting::Speakers => "speakers",
        config::TtsRouting::VirtualMic => "virtual_mic",
        config::TtsRouting::Both => "both",
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
                    | UserAction::ConfigPickerNext
                    | UserAction::ConfigPickerPrev
                    | UserAction::ConfigSave
                    | UserAction::ConfigCycleCaptureDevice
                    | UserAction::ReloadConfig
                    | UserAction::ToggleTts
                    | UserAction::CycleTtsVoice
                    | UserAction::AdjustInputGainDb(_)
                    | UserAction::AdjustOutputVolumeDb(_)
                    | UserAction::ResetVolumeAndGain
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
/// The loop runs at approximately 60 fps (adaptive frame pacer; issue #383 /
/// DM-07).  The pacer targets a ~16.6ms per-frame budget and records actual
/// frame intervals in an HDR histogram so p50/p95/p99 evidence can be queried
/// from the returned [`FramePacer`].
///
/// Key actions arrive on `key_rx` from the dedicated keyboard task (issue #63).
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &AppState,
    context: &TuiRuntimeContext<'_>,
    key_rx: mpsc::Receiver<UserAction>,
) -> Result<()> {
    let mut pacer = FramePacer::new();
    loop {
        // ── Issue #88 — check for Windows forced-close signal ─────────────────
        if FORCED_SHUTDOWN.load(Ordering::Relaxed) {
            tracing::info!("forced shutdown signal received; exiting event loop (issue #88)");
            break;
        }

        let show_restart = context.restart_required.load(Ordering::Relaxed);
        let (cost_warning_usd, tts_route) = {
            let cfg = context
                .current_config
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            (cfg.cost_warning_usd, TtsRouteStatus::from_config(&cfg))
        };
        let level = state.level_ratio();
        terminal.draw(|frame| {
            draw_ui_with_route(
                frame,
                state,
                level,
                show_restart,
                cost_warning_usd,
                tts_route.clone(),
            );
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
                    let terminal_area = terminal.size()?.into();
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

        // Adaptive 60fps pacer (issue #383 / DM-07): replaces the fixed 50ms
        // sleep.  Sleeps for the remaining frame budget and records the actual
        // interval in the HDR histogram for p95 evidence.
        pacer.end_frame();
    }
    tracing::debug!(
        p50_ms = pacer.p50_ms(),
        p95_ms = pacer.p95_ms(),
        p99_ms = pacer.p99_ms(),
        total = pacer.total_frames(),
        dropped = pacer.dropped_frames(),
        "event loop frame-pacing summary"
    );
    Ok(())
}

// ── Issue #63: keyboard task ──────────────────────────────────────────────────

/// Translate a raw crossterm [`KeyEvent`] into a [`UserAction`].
///
/// `in_lang_prompt` and `in_config_editor` route character input to the active
/// overlay instead of the normal command set.
pub(crate) fn key_to_action(
    key: &KeyEvent,
    in_lang_prompt: bool,
    in_config_editor: bool,
    in_wizard: bool,
    picker_field_active: bool,
) -> Option<UserAction> {
    if in_wizard {
        return match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::Quit)
            }
            KeyCode::Char('1') => Some(UserAction::WizardKey(OnboardingEvent::SelectBranch1)),
            KeyCode::Char('2') => Some(UserAction::WizardKey(OnboardingEvent::SelectBranch2)),
            KeyCode::Char('3') => Some(UserAction::WizardKey(OnboardingEvent::SelectBranch3)),
            KeyCode::Up => Some(UserAction::WizardKey(OnboardingEvent::ArrowUp)),
            KeyCode::Down => Some(UserAction::WizardKey(OnboardingEvent::ArrowDown)),
            KeyCode::Enter => Some(UserAction::WizardKey(OnboardingEvent::Enter)),
            KeyCode::Esc => Some(UserAction::WizardKey(OnboardingEvent::Escape)),
            KeyCode::Backspace => Some(UserAction::WizardKey(OnboardingEvent::Backspace)),
            KeyCode::Char('l')
            | KeyCode::Char('L')
            | KeyCode::Char('t')
            | KeyCode::Char('T')
            | KeyCode::Char('m')
            | KeyCode::Char('M')
            | KeyCode::Char('s')
            | KeyCode::Char('S')
            | KeyCode::Char('r')
            | KeyCode::Char('R')
            | KeyCode::Char('?')
            | KeyCode::Char('q')
            | KeyCode::Char('Q')
            | KeyCode::Char(' ') => Some(UserAction::WizardKey(OnboardingEvent::Ignored)),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(UserAction::WizardKey(OnboardingEvent::Char(c)))
            }
            _ => Some(UserAction::AnyKey),
        };
    }
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
            KeyCode::Tab | KeyCode::Down => {
                if picker_field_active {
                    Some(UserAction::ConfigPickerNext)
                } else {
                    Some(UserAction::ConfigNextField)
                }
            }
            KeyCode::BackTab | KeyCode::Up => {
                if picker_field_active {
                    Some(UserAction::ConfigPickerPrev)
                } else {
                    Some(UserAction::ConfigPrevField)
                }
            }
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
        // CTRL-02 — cycle active TTS voice (issue #455).
        KeyCode::Char('v') | KeyCode::Char('V') => Some(UserAction::CycleTtsVoice),
        KeyCode::Char('m') | KeyCode::Char('M') => Some(UserAction::ToggleMetrics),
        KeyCode::Char('l') | KeyCode::Char('L') => Some(UserAction::PromptLanguage),
        KeyCode::Char('s') | KeyCode::Char('S') => Some(UserAction::OpenSettings),
        KeyCode::Char('r') | KeyCode::Char('R') => Some(UserAction::ReloadConfig),
        KeyCode::Char('?') => Some(UserAction::ToggleHelp),
        // CTRL-01 — real-time gain/volume controls (issue #454).
        KeyCode::Char('[') => Some(UserAction::AdjustInputGainDb(
            -(crate::audio::audio_gain::DB_STEP as i32 * 100),
        )),
        KeyCode::Char(']') => Some(UserAction::AdjustInputGainDb(
            crate::audio::audio_gain::DB_STEP as i32 * 100,
        )),
        KeyCode::Char('{') => Some(UserAction::AdjustOutputVolumeDb(
            -(crate::audio::audio_gain::DB_STEP as i32 * 100),
        )),
        KeyCode::Char('}') => Some(UserAction::AdjustOutputVolumeDb(
            crate::audio::audio_gain::DB_STEP as i32 * 100,
        )),
        KeyCode::Char('0') => Some(UserAction::ResetVolumeAndGain),
        // Tab — toggle A/B pane focus (DM-04, issue #380).
        // Config editor handles its own Tab (ConfigNextField) in the branch
        // above, so this only fires in normal mode.
        KeyCode::Tab => Some(UserAction::TogglePaneFocus),
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
    picker_field_active: Arc<AtomicBool>,
    wizard_active: Arc<AtomicBool>,
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
                let picker_active = picker_field_active.load(Ordering::Relaxed);
                let in_wizard = wizard_active.load(Ordering::Relaxed);
                if let Some(action) = key_to_action(
                    &key,
                    in_lang_prompt,
                    in_config_editor,
                    in_wizard,
                    picker_active,
                ) {
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

fn handle_wizard_outcome(
    outcome: OnboardingOutcome,
    state: &AppState,
    cfg_path: &Path,
    current_config: &Arc<Mutex<config::AppConfig>>,
    restart_required: &Arc<AtomicBool>,
    playback_service: &SharedPlaybackService,
) {
    match outcome {
        OnboardingOutcome::Cancelled => {
            let config_missing = match cfg_path.try_exists() {
                Ok(exists) => !exists,
                Err(err) => {
                    tracing::warn!(
                        path = %cfg_path.display(),
                        "failed to check config before cancelling wizard: {err:#}"
                    );
                    true
                }
            };
            if config_missing {
                state.open_wizard(OnboardingWizardState::new(build_local_model_licenses()));
                *state
                    .startup_notice_msg
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) =
                    Some("Setup is required before translation can start.".to_string());
                tracing::info!(
                    path = %cfg_path.display(),
                    "first-run setup cancellation ignored because config is missing"
                );
                return;
            }
            state.close_wizard();
            tracing::info!("onboarding wizard cancelled by user");
        }
        OnboardingOutcome::Done(patch) => {
            let consent_only = state.wizard_consent_only.load(Ordering::Relaxed);
            state.close_wizard();

            if !consent_only
                && patch.branch.requires_google_key()
                && patch
                    .google_api_key
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
            {
                *state
                    .startup_notice_msg
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) =
                    Some("Setup failed: Google API key is required for this branch.".to_string());
                return;
            }

            let consent_manifests = if consent_only {
                let cfg = current_config
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                collect_pending_consent_manifests(&cfg)
            } else if patch.branch.uses_local_models() {
                all_local_consent_manifests()
            } else {
                Vec::new()
            };
            if let Err(err) = write_model_consent_records(&consent_manifests) {
                tracing::error!("failed to write local model consent records: {err:#}");
                *state
                    .startup_notice_msg
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) = Some(format!(
                    "Setup failed: could not write model consent: {err:#}"
                ));
                return;
            }

            if consent_only {
                tracing::info!("consent review complete; existing config preserved");
                return;
            }

            if let Err(e) = apply_wizard_patch_to_config(
                patch,
                cfg_path,
                current_config,
                restart_required,
                playback_service,
                state,
            ) {
                tracing::error!("failed to apply wizard config: {e:#}");
                *state
                    .startup_notice_msg
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) = Some(format!("Setup failed: {e:#}"));
            }
        }
    }
}

fn apply_wizard_patch_to_config(
    patch: tui::onboarding::OnboardingConfigPatch,
    cfg_path: &Path,
    current_config: &Arc<Mutex<config::AppConfig>>,
    restart_required: &Arc<AtomicBool>,
    playback_service: &SharedPlaybackService,
    state: &AppState,
) -> anyhow::Result<()> {
    if patch.branch.requires_google_key()
        && patch
            .google_api_key
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
    {
        anyhow::bail!("a Google API key is required for the selected branch but none was provided");
    }

    let mut cfg = config::AppConfig::default();

    match patch.branch {
        OnboardingBranch::LocalOnly => {
            cfg.stt_provider = "local".to_string();
            cfg.mt_provider = "local".to_string();
            cfg.stt_fallback_policy = "none".to_string();
            cfg.mt_cloud_fallback = None;
            cfg.google_api_key = None;
            cfg.tts_enabled = false;
        }
        OnboardingBranch::LocalGoogleFallback => {
            cfg.stt_provider = "local".to_string();
            cfg.mt_provider = "local".to_string();
            cfg.stt_fallback_policy = "google-when-keyed".to_string();
            cfg.mt_cloud_fallback = Some("google".to_string());
            cfg.google_api_key = patch.google_api_key.clone();
            cfg.tts_enabled = false;
        }
        OnboardingBranch::GoogleCloud => {
            cfg.stt_provider = "google".to_string();
            cfg.mt_provider = "google".to_string();
            cfg.stt_fallback_policy = "none".to_string();
            cfg.mt_cloud_fallback = None;
            cfg.google_api_key = patch.google_api_key.clone();
            cfg.tts_enabled = false;
        }
    }

    config::apply_editor_defaults(cfg_path, &mut cfg)?;
    config::write_config(cfg_path, &cfg)?;
    let (requires_restart, _) = apply_runtime_config(
        current_config,
        &state.target_language,
        &state.source_language,
        &state.capture_device_label,
        &state.slot_a_provider_name,
        &state.tts_enabled,
        &state.audio_consent,
        restart_required,
        playback_service,
        cfg,
    );
    let apply_status = if requires_restart {
        tui::ConfigApplyStatus::RestartRequired {
            reason: "restart required for settings to take effect".to_string(),
        }
    } else {
        tui::ConfigApplyStatus::Ok {
            reason: "settings saved".to_string(),
        }
    };
    state.record_config_apply(apply_status);
    tracing::info!(branch = ?patch.branch, "first-run wizard config applied");
    Ok(())
}

/// Persist a CTRL-01 gain/volume change to the in-memory `current_config`
/// snapshot and to disk.  `input_db` and `output_db`, when `Some`, replace
/// the corresponding field; `None` leaves the value untouched so the helper
/// can be reused by all three hotkeys (input / output / reset).
///
/// Write failures are logged but never panic — the runtime gain stays in
/// effect because it lives in [`audio::audio_gain`] and the in-memory
/// `AppConfig` is updated regardless.
fn persist_gain_changes(
    cfg_path: &Path,
    current_config: &Arc<Mutex<config::AppConfig>>,
    input_db: Option<f32>,
    output_db: Option<f32>,
) {
    let next_cfg = {
        let mut current = current_config.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(v) = input_db {
            current.input_gain_db = v;
        }
        if let Some(v) = output_db {
            current.output_volume_db = v;
        }
        current.clone()
    };
    if let Err(err) = config::write_config(cfg_path, &next_cfg) {
        tracing::warn!("CTRL-01: failed to persist gain change to config.json: {err:#}");
    }
}

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
            tracing::info!(
                route = playback_route_label(current.tts_routing),
                "TTS toggled: {}",
                if actual { "on" } else { "off" }
            );
        }

        // M — expand / collapse metrics (issue #41)
        UserAction::ToggleMetrics => {
            state.toggle_metrics();
        }

        // V — cycle TTS voice (CTRL-02, issue #455).
        UserAction::CycleTtsVoice => {
            let target_lang = state
                .target_language
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();
            match cycle_tts_voice_for_language(&target_lang) {
                Ok(next_voice) => {
                    let voice_label = next_voice
                        .as_ref()
                        .map(|v| v.name.clone())
                        .unwrap_or_else(|| "default".to_string());
                    tracing::info!(voice = %voice_label, "TTS voice swapped (CTRL-02)");
                    if let Ok(mut cfg) = current_config.lock() {
                        cfg.tts_voice = next_voice.as_ref().map(|v| v.name.clone());
                    }
                }
                Err(err) => {
                    if let Ok(mut slot) = state.pipeline_error_msg.lock() {
                        *slot = Some(format!("TTS voice swap failed: {err}"));
                    }
                    tracing::warn!(error = %err, "TTS voice swap rejected by provider");
                }
            }
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
            if state.wizard_active.load(Ordering::Relaxed) {
                let outcome = state.with_wizard_mut(|wiz| wiz.handle(OnboardingEvent::Escape));
                if let Some(Some(outcome)) = outcome {
                    handle_wizard_outcome(
                        outcome,
                        state,
                        cfg_path,
                        current_config,
                        restart_required,
                        playback_service,
                    );
                }
            } else if state.config_editor_active.load(Ordering::Relaxed) {
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
            populate_config_editor_device_options(state);
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
            let picker_active = state
                .with_config_editor_mut(|editor| {
                    editor.next_field();
                    editor.is_picker_field_active()
                })
                .unwrap_or(false);
            state
                .picker_field_active
                .store(picker_active, Ordering::Relaxed);
        }
        UserAction::ConfigPrevField => {
            let picker_active = state
                .with_config_editor_mut(|editor| {
                    editor.prev_field();
                    editor.is_picker_field_active()
                })
                .unwrap_or(false);
            state
                .picker_field_active
                .store(picker_active, Ordering::Relaxed);
        }
        UserAction::ConfigPickerNext => {
            let _ = state.with_config_editor_mut(|editor| editor.picker_next());
        }
        UserAction::ConfigPickerPrev => {
            let _ = state.with_config_editor_mut(|editor| editor.picker_prev());
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

        // Tab — DM-04 dual-pane focus toggle.
        UserAction::TogglePaneFocus => {
            state.toggle_pane_focus();
        }

        // CTRL-01 — real-time input gain (issue #454).
        UserAction::AdjustInputGainDb(delta_centi_db) => {
            let delta_db = (*delta_centi_db as f32) / 100.0;
            let current = audio::audio_gain::input_gain_db();
            let new_db = audio::audio_gain::set_input_gain_db(current + delta_db);
            persist_gain_changes(cfg_path, current_config, Some(new_db), None);
            tracing::info!(input_gain_db = new_db, "input gain adjusted");
        }
        // CTRL-01 — real-time TTS playback volume (issue #454).
        UserAction::AdjustOutputVolumeDb(delta_centi_db) => {
            let delta_db = (*delta_centi_db as f32) / 100.0;
            let current = audio::audio_gain::output_volume_db();
            let new_db = audio::audio_gain::set_output_volume_db(current + delta_db);
            persist_gain_changes(cfg_path, current_config, None, Some(new_db));
            tracing::info!(output_volume_db = new_db, "output volume adjusted");
        }
        // CTRL-01 — reset both controllers to unity.
        UserAction::ResetVolumeAndGain => {
            audio::audio_gain::reset_to_unity();
            persist_gain_changes(cfg_path, current_config, Some(0.0), Some(0.0));
            tracing::info!("input gain and output volume reset to 0 dB");
        }

        UserAction::WizardKey(event) => {
            let outcome = state.with_wizard_mut(|wiz| wiz.handle(event.clone()));
            if let Some(Some(outcome)) = outcome {
                handle_wizard_outcome(
                    outcome,
                    state,
                    cfg_path,
                    current_config,
                    restart_required,
                    playback_service,
                );
            }
        }

        // R — signal config reload (issue #64)
        UserAction::ReloadConfig => match config::load(cfg_path) {
            Ok(next_cfg) => {
                let (requires_restart, _) = apply_runtime_config(
                    current_config,
                    &state.target_language,
                    &state.source_language,
                    &state.capture_device_label,
                    &state.slot_a_provider_name,
                    &state.tts_enabled,
                    &state.audio_consent,
                    restart_required,
                    playback_service,
                    next_cfg,
                );
                let apply_status = if requires_restart {
                    tui::ConfigApplyStatus::RestartRequired {
                        reason: "restart required for settings to take effect".to_string(),
                    }
                } else {
                    tui::ConfigApplyStatus::Ok {
                        reason: "settings reloaded".to_string(),
                    }
                };
                state.record_config_apply(apply_status);
                // Issue #86 — auth-error recovery requires a full restart.
                // Providers still hold the old (possibly invalid) credential
                // in-process; no un-halt is possible here regardless of
                // whether the key changed.  The banner and pipeline_halted
                // flag stay set until the application is restarted.
                tracing::info!("config reloaded from {}", cfg_path.display());
            }
            Err(err) => {
                tracing::warn!("config reload requested with R key failed: {err:#}");
                state.record_config_apply(tui::ConfigApplyStatus::RolledBack {
                    reason: format!("reload failed: {err:#}"),
                });
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
    use crate::session_export_cli::{SessionExportArgs, SessionExportFormat};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;
    use std::ffi::OsString;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

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
    fn stt_local_unavailable_is_fatal_when_no_fallback_can_be_wired() {
        let mut cfg = config::AppConfig::default();
        cfg.stt_provider = "local".to_string();
        cfg.stt_fallback_policy = "google-when-keyed".to_string();
        cfg.google_api_key = None;
        assert!(
            stt_local_unavailable_is_fatal_for_slot("local", &cfg),
            "google-when-keyed without a key has no fallback and must halt on permanent local errors"
        );

        cfg.google_api_key = Some("demo-key".to_string());
        assert!(
            !stt_local_unavailable_is_fatal_for_slot("local", &cfg),
            "google-when-keyed with a key is handled by FallbackSttProvider"
        );

        cfg.stt_fallback_policy = "none".to_string();
        assert!(
            stt_local_unavailable_is_fatal_for_slot("local", &cfg),
            "local-only policy must halt on permanent local errors"
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
            key_to_action(&key, false, true, false, false),
            Some(UserAction::ConfigCycleCaptureDevice)
        );
    }

    /// ↑/↓/Tab/Shift+Tab route to picker cycling only when `picker_field_active`
    /// is `true` (UX-03 / issue #683).  Pane-cycle Tab is unaffected because it
    /// only fires when `in_config_editor` is `false`.
    #[test]
    fn config_editor_tab_scoped_to_picker_open_state() {
        let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        let shift_tab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);

        // picker NOT active → field navigation (existing behaviour preserved)
        assert_eq!(
            key_to_action(&tab, false, true, false, false),
            Some(UserAction::ConfigNextField)
        );
        assert_eq!(
            key_to_action(&shift_tab, false, true, false, false),
            Some(UserAction::ConfigPrevField)
        );
        assert_eq!(
            key_to_action(&down, false, true, false, false),
            Some(UserAction::ConfigNextField)
        );
        assert_eq!(
            key_to_action(&up, false, true, false, false),
            Some(UserAction::ConfigPrevField)
        );

        // picker IS active → cycle device picker (Tab scoped to picker-open state only)
        assert_eq!(
            key_to_action(&tab, false, true, false, true),
            Some(UserAction::ConfigPickerNext)
        );
        assert_eq!(
            key_to_action(&shift_tab, false, true, false, true),
            Some(UserAction::ConfigPickerPrev)
        );
        assert_eq!(
            key_to_action(&down, false, true, false, true),
            Some(UserAction::ConfigPickerNext)
        );
        assert_eq!(
            key_to_action(&up, false, true, false, true),
            Some(UserAction::ConfigPickerPrev)
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

        assert_eq!(
            key_to_action(&key, false, false, false, false),
            Some(UserAction::AnyKey)
        );
    }

    #[test]
    fn settings_shortcut_opens_settings_outside_text_overlays() {
        for key_code in [KeyCode::Char('s'), KeyCode::Char('S')] {
            let key = KeyEvent::new(key_code, KeyModifiers::NONE);

            assert_eq!(
                key_to_action(&key, false, false, false, false),
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
            key_to_action(&key, false, true, false, false),
            Some(UserAction::ConfigChar('x'))
        );
        assert_eq!(
            key_to_action(
                &KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                false,
                true,
                false,
                false
            ),
            Some(UserAction::Quit)
        );
        assert_eq!(
            key_to_action(
                &KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                false,
                true,
                false,
                false
            ),
            Some(UserAction::ConfigNextField)
        );
        assert_eq!(
            key_to_action(
                &KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
                false,
                true,
                false,
                false
            ),
            Some(UserAction::ConfigInput(InputRequest::GoToPrevChar))
        );
        assert_eq!(
            key_to_action(
                &KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
                false,
                true,
                false,
                false
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
    fn build_config_from_editor_normalizes_google_stt_fallback_policy() {
        let current = config::AppConfig::default();
        let cfg_path = Path::new(r"C:\Users\demo\.tui-translator\config.json");
        let mut editor =
            tui::ConfigEditorState::from_config(&current, cfg_path, ConfigEditorMode::Settings);
        editor.stt_provider = "google".to_string();
        editor.stt_fallback_policy = "google-when-keyed".to_string();

        let next = build_config_from_editor(&editor, &current).unwrap();

        assert_eq!(next.stt_provider, "google");
        assert_eq!(next.stt_fallback_policy, "none");
    }

    #[test]
    fn build_config_from_editor_persists_tts_route_fields() {
        let current = config::AppConfig::default();
        let cfg_path = Path::new(r"C:\Users\demo\.tui-translator\config.json");
        let mut editor =
            tui::ConfigEditorState::from_config(&current, cfg_path, ConfigEditorMode::Settings);
        editor.tts_routing = "both".to_string();
        editor.virtual_mic_device = "CABLE Input (VB-Audio Virtual Cable)".to_string();

        let next = build_config_from_editor(&editor, &current).unwrap();

        assert_eq!(next.tts_routing, config::TtsRouting::Both);
        assert_eq!(
            next.virtual_mic_device.as_deref(),
            Some("CABLE Input (VB-Audio Virtual Cable)")
        );
    }

    #[test]
    fn build_config_from_editor_rejects_invalid_tts_routing() {
        let current = config::AppConfig::default();
        let cfg_path = Path::new(r"C:\Users\demo\.tui-translator\config.json");
        let mut editor =
            tui::ConfigEditorState::from_config(&current, cfg_path, ConfigEditorMode::Settings);
        editor.tts_routing = "vmic".to_string();

        let err = build_config_from_editor(&editor, &current).unwrap_err();

        assert!(
            err.to_string().contains("tts_routing"),
            "invalid route error should name tts_routing: {err:#}"
        );
    }

    #[test]
    fn runtime_provider_error_allows_default_google_mode() {
        // Test that pure google mode has no runtime provider error.
        let mut cfg = config::AppConfig::default();
        cfg.stt_provider = "google".to_string();
        assert!(runtime_provider_error(&cfg).is_none());
    }

    #[test]
    fn missing_google_api_key_error_preserves_default_metrics_only_without_key() {
        // Pure google-only mode (no key) runs in metrics-only mode — no error.
        let mut cfg = config::AppConfig::default();
        cfg.stt_provider = "google".to_string();
        cfg.mt_provider = "google".to_string();
        // tts_enabled defaults to false.
        assert!(missing_google_api_key_error(&cfg).is_none());
    }

    #[test]
    fn missing_google_api_key_error_explains_local_stt_still_needs_google_mt() {
        let mut cfg = config::AppConfig::default();
        cfg.stt_provider = "local".to_string();
        cfg.mt_provider = "google".to_string();

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
        cfg.stt_provider = "google".to_string();
        cfg.mt_provider = "google".to_string();
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

    #[test]
    fn missing_google_api_key_error_uses_dual_slot_a_not_flat_fields() {
        let mut cfg = config::AppConfig::default();
        cfg.stt_provider = "google".to_string();
        cfg.mt_provider = "google".to_string();
        cfg.slots = Some(config::DualSlotConfig {
            slot_a: config::SlotConfig {
                stt_provider: "local".to_string(),
                mt_provider: "local".to_string(),
                target_language: "vi".to_string(),
            },
            slot_b: config::SlotConfig {
                stt_provider: "google".to_string(),
                mt_provider: "google".to_string(),
                target_language: "en".to_string(),
            },
        });

        assert!(
            missing_google_api_key_error(&cfg).is_none(),
            "slot A is fully local, so flat Google fields and slot B must not block startup"
        );
    }

    #[test]
    fn missing_google_api_key_error_reports_dual_slot_a_requirements() {
        let mut cfg = config::AppConfig::default();
        cfg.stt_provider = "local".to_string();
        cfg.mt_provider = "local".to_string();
        cfg.slots = Some(config::DualSlotConfig {
            slot_a: config::SlotConfig {
                stt_provider: "local".to_string(),
                mt_provider: "google".to_string(),
                target_language: "vi".to_string(),
            },
            slot_b: config::SlotConfig {
                stt_provider: "local".to_string(),
                mt_provider: "local".to_string(),
                target_language: "en".to_string(),
            },
        });

        let msg = missing_google_api_key_error(&cfg)
            .expect("slot A Google Translation should require google_api_key");
        assert!(msg.contains("Google Translation"));
        assert!(msg.contains("switch mt_provider to \"local\""));
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
            editor.stt_provider = "google".to_string();
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
            &state.slot_a_provider_name,
            &state.tts_enabled,
            &state.audio_consent,
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
    fn watcher_apply_status_skips_tui_round_trip_echo() {
        let status = config_apply_status_for_watcher_change(
            true,
            false,
            Some("stt_provider changed".to_string()),
        );
        assert!(
            status.is_none(),
            "watcher echo after a TUI write must not record a second apply"
        );
    }

    #[test]
    fn watcher_apply_status_uses_specific_restart_reason() {
        let status =
            config_apply_status_for_watcher_change(true, true, Some("stt_provider changed".into()))
                .expect("changed restart-required config should record status");
        assert_eq!(status.reason(), "stt_provider changed");
        assert!(status.is_persistent());
    }

    #[test]
    fn apply_runtime_config_updates_audio_consent_gate() {
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        let mut next = config::AppConfig::default();
        next.audio_archive.consent_given = true;

        apply_runtime_config(
            &current_config,
            &state.target_language,
            &state.source_language,
            &state.capture_device_label,
            &state.slot_a_provider_name,
            &state.tts_enabled,
            &state.audio_consent,
            &restart_required,
            &playback_service,
            next,
        );

        assert!(state.audio_consent.load(Ordering::Relaxed));
    }

    #[test]
    fn apply_runtime_config_rejected_capture_change_still_applies_hot_fields() {
        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        let mut next = config::AppConfig::default();
        next.audio_source = "alsa".to_string();
        next.target_language = "en".to_string();

        apply_runtime_config(
            &current_config,
            &state.target_language,
            &state.source_language,
            &state.capture_device_label,
            &state.slot_a_provider_name,
            &state.tts_enabled,
            &state.audio_consent,
            &restart_required,
            &playback_service,
            next,
        );

        assert_eq!(state.target_language(), "en");
        let effective = current_config.lock().unwrap().clone();
        assert_eq!(effective.target_language, "en");
        assert_eq!(
            effective.audio_source,
            config::AppConfig::default().audio_source,
            "rejected capture fields must not become the effective runtime config"
        );
        assert_eq!(
            effective.audio_file_path,
            config::AppConfig::default().audio_file_path,
            "rejected capture fields must keep the old effective source path"
        );
    }

    #[test]
    fn capture_hot_swap_status_predicate_matches_only_capture_statuses() {
        assert!(is_capture_hot_swap_status(
            "Capture change failed; still using previous audio stream: missing.wav"
        ));
        assert!(is_capture_hot_swap_status(
            "Capture config rejected: audio_file_path is required"
        ));
        assert!(
            !is_capture_hot_swap_status("Translation error: retry budget exhausted"),
            "successful capture hot-swap must not clear unrelated pipeline errors"
        );
    }

    #[test]
    fn capture_source_spec_from_config_maps_wasapi_device() {
        let mut cfg = config::AppConfig::default();
        cfg.capture_device = Some("Speakers (Realtek)".to_string());

        assert_eq!(
            capture_source_spec_from_config(&cfg).unwrap(),
            audio::CaptureSourceSpec::Wasapi {
                device: Some("Speakers (Realtek)".to_string())
            }
        );
    }

    #[test]
    fn capture_source_spec_from_config_rejects_file_without_path() {
        let mut cfg = config::AppConfig::default();
        cfg.audio_source = "file".to_string();
        cfg.audio_file_path = None;

        let err = capture_source_spec_from_config(&cfg).unwrap_err();
        assert!(
            err.to_string().contains("audio_file_path"),
            "error should mention missing audio_file_path; got: {err}"
        );
    }

    #[test]
    fn apply_runtime_config_updates_slot_a_provider_title() {
        let state = AppState::new();
        *state.slot_a_provider_name.lock().unwrap() = "google".to_string();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(config::AppConfig::default()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        let mut next = config::AppConfig::default();
        next.mt_provider = "local".to_string();

        apply_runtime_config(
            &current_config,
            &state.target_language,
            &state.source_language,
            &state.capture_device_label,
            &state.slot_a_provider_name,
            &state.tts_enabled,
            &state.audio_consent,
            &restart_required,
            &playback_service,
            next,
        );

        assert_eq!(state.slot_a_provider_name.lock().unwrap().as_str(), "local");
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
    fn settings_save_replaces_existing_google_api_key_from_editor() {
        let temp = TempDir::new().unwrap();
        let cfg_path = temp.path().join(".tui-translator").join("config.json");
        let mut existing = config::AppConfig::default();
        existing.google_api_key = Some("old-secret-key".to_string());
        config::write_config(&cfg_path, &existing).unwrap();

        let state = AppState::new();
        let restart_required = Arc::new(AtomicBool::new(false));
        let current_config = Arc::new(Mutex::new(existing.clone()));
        let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
        state.open_config_editor(ConfigEditorMode::Settings, &existing, &cfg_path);
        let _ = state.with_config_editor_mut(|editor| {
            editor.selected_field = 2; // Google API key.
            editor.cycle_active_field();
            for ch in "new-secret-key".chars() {
                editor.push_char(ch);
            }
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
        assert_eq!(persisted.google_api_key.as_deref(), Some("new-secret-key"));
        assert_eq!(
            current_config.lock().unwrap().google_api_key.as_deref(),
            Some("new-secret-key")
        );
        assert!(
            restart_required.load(Ordering::Relaxed),
            "changing provider credentials must keep the restart-required signal"
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
    fn config_save_persists_tts_route_and_virtual_mic_selection() {
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
            editor.tts_routing = "virtual_mic".to_string();
            editor.virtual_mic_device = "CABLE Input (VB-Audio Virtual Cable)".to_string();
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
        assert_eq!(persisted.tts_routing, config::TtsRouting::VirtualMic);
        assert_eq!(
            persisted.virtual_mic_device.as_deref(),
            Some("CABLE Input (VB-Audio Virtual Cable)")
        );
        assert!(restart_required.load(Ordering::Relaxed));
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
        editor.stt_fallback_policy = "none".to_string();

        // OK: unwrap in test assertion
        let result = build_config_from_editor(&editor, &current).expect("build config");

        assert_eq!(
            result.stt_fallback_policy, "none",
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
        editor.google_api_key = "test-google-api-key".to_string();

        let result = build_config_from_editor(&editor, &current).unwrap();

        assert_eq!(
            result.google_api_key.as_deref(),
            Some("test-google-api-key"),
            "build_config_from_editor must preserve the real (unmasked) API key"
        );
    }

    #[test]
    fn build_config_from_editor_round_trips_tts_and_fallback() {
        // Verify that a full round-trip preserves the exact fields.
        let cfg_path = Path::new(r"C:\Users\demo\.tui-translator\config.json");
        let mut cfg = config::AppConfig::default();
        cfg.tts_enabled = true;
        cfg.stt_fallback_policy = "none".to_string();

        let editor =
            tui::ConfigEditorState::from_config(&cfg, cfg_path, ConfigEditorMode::Settings);
        // OK: unwrap in test assertion
        let result = build_config_from_editor(&editor, &cfg).expect("build config");

        assert!(result.tts_enabled);
        assert_eq!(result.stt_fallback_policy, "none");
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

    // ── DM-03: Per-slot orchestrator and provider state (issue #379) ─────────

    #[test]
    fn slot_id_defaults_to_slot_a() {
        assert_eq!(
            pipeline::SlotId::default(),
            pipeline::SlotId::A,
            "SlotId::default() must return A so single-slot mode is zero-cost"
        );
        assert_eq!(pipeline::SlotId::A.label(), "A");
        assert_eq!(pipeline::SlotId::B.label(), "B");
    }

    #[test]
    fn slot_provider_config_uses_slot_specific_stt() {
        let mut cfg = config::AppConfig::default();
        cfg.stt_fallback_policy = "none".to_string();
        cfg.google_api_key = None;

        // "google" provider with "none" fallback — fatal for local, irrelevant
        // for google-keyed, but the function returns false for this combination.
        assert!(
            !stt_local_unavailable_is_fatal_for_slot("google", &cfg),
            "google provider with 'none' fallback is not fatal for local unavailability"
        );

        // "local" provider with "none" fallback is always fatal.
        assert!(
            stt_local_unavailable_is_fatal_for_slot("local", &cfg),
            "local provider with 'none' fallback must be fatal when local is unavailable"
        );

        // "local" with "google-when-keyed" and no key — fatal.
        cfg.stt_fallback_policy = "google-when-keyed".to_string();
        assert!(
            stt_local_unavailable_is_fatal_for_slot("local", &cfg),
            "local provider with 'google-when-keyed' fallback but no key must be fatal"
        );

        // "local" with "google-when-keyed" and a key — not fatal.
        cfg.google_api_key = Some("some-key".to_string());
        assert!(
            !stt_local_unavailable_is_fatal_for_slot("local", &cfg),
            "local provider with 'google-when-keyed' and a key must NOT be fatal"
        );
    }

    #[test]
    fn single_mode_creates_one_active_slot() {
        let slot = pipeline::SlotOrchestratorState::new(pipeline::SlotId::A);
        assert_eq!(slot.slot_id, pipeline::SlotId::A);
        // Default state: Idle STT, no errors, not halted.
        assert_eq!(*slot.stt_state.lock().unwrap(), metrics::SttState::Idle);
        assert!(!slot
            .pipeline_halted
            .load(std::sync::atomic::Ordering::Relaxed));
        assert!(slot.pipeline_error_msg.lock().unwrap().is_none());
        assert!(slot.auth_error_banner.lock().unwrap().is_none());
    }

    #[test]
    fn dual_mode_exposes_two_independent_slot_states() {
        let slot_a = pipeline::SlotOrchestratorState::new(pipeline::SlotId::A);
        let slot_b = pipeline::SlotOrchestratorState::new(pipeline::SlotId::B);

        assert_ne!(slot_a.slot_id, slot_b.slot_id);
        // Both start idle, not halted.
        assert!(!slot_a
            .pipeline_halted
            .load(std::sync::atomic::Ordering::Relaxed));
        assert!(!slot_b
            .pipeline_halted
            .load(std::sync::atomic::Ordering::Relaxed));

        // The Arc pointers are distinct — mutations to slot_a do not touch slot_b.
        assert!(!Arc::ptr_eq(
            &slot_a.pipeline_halted,
            &slot_b.pipeline_halted
        ));
        assert!(!Arc::ptr_eq(
            &slot_a.session_metrics,
            &slot_b.session_metrics
        ));
        assert!(!Arc::ptr_eq(&slot_a.subtitle_pane, &slot_b.subtitle_pane));
    }

    #[test]
    fn slot_halted_does_not_affect_other_slot() {
        let slot_a = pipeline::SlotOrchestratorState::new(pipeline::SlotId::A);
        let slot_b = pipeline::SlotOrchestratorState::new(pipeline::SlotId::B);

        // Halt slot A.
        slot_a
            .pipeline_halted
            .store(true, std::sync::atomic::Ordering::Relaxed);
        *slot_a.auth_error_banner.lock().unwrap() = Some("auth failed".to_string());

        // Slot B must remain unaffected.
        assert!(
            !slot_b
                .pipeline_halted
                .load(std::sync::atomic::Ordering::Relaxed),
            "halting slot A must not halt slot B"
        );
        assert!(
            // OK: test-only; poisoned mutex returns inner value
            slot_b.auth_error_banner.lock().unwrap().is_none(),
            "auth banner set on slot A must not appear on slot B"
        );
    }

    // DM-06 (issue #382): halt-aggregation and TTS-status wiring.

    /// `aggregate_halt_state` (used by the halt-aggregation background task) must
    /// derive the global halt from both per-slot flags, not slot A's flag alone.
    ///
    /// | A halted | B halted | global halted |
    /// |----------|----------|---------------|
    /// | false    | false    | false         |
    /// | true     | false    | false  | slot A auth fail; B still runs |
    /// | false    | true     | false  | slot B auth fail; A still runs |
    /// | true     | true     | true   | both halted; show global banner |
    #[test]
    fn aggregate_halt_state_dual_mode_requires_both_slots_halted() {
        // Mirrors the logic the background task runs every 100 ms.
        let a_halt = Arc::new(AtomicBool::new(false));
        let b_halt = Arc::new(AtomicBool::new(false));
        let global_halt = Arc::new(AtomicBool::new(false));

        let compute_global = || {
            let a = a_halt.load(Ordering::Relaxed);
            let b = b_halt.load(Ordering::Relaxed);
            pipeline::aggregate_halt_state(a, Some(b))
        };

        // Both healthy.
        assert!(!compute_global(), "both healthy: global must not be halted");

        // Only slot A halted.
        a_halt.store(true, Ordering::Relaxed);
        global_halt.store(compute_global(), Ordering::Relaxed);
        assert!(
            !global_halt.load(Ordering::Relaxed),
            "slot A halted, B healthy: global must NOT be halted"
        );

        // Only slot B halted (reset A first).
        a_halt.store(false, Ordering::Relaxed);
        b_halt.store(true, Ordering::Relaxed);
        global_halt.store(compute_global(), Ordering::Relaxed);
        assert!(
            !global_halt.load(Ordering::Relaxed),
            "slot B halted, A healthy: global must NOT be halted"
        );

        // Both halted.
        a_halt.store(true, Ordering::Relaxed);
        global_halt.store(compute_global(), Ordering::Relaxed);
        assert!(
            global_halt.load(Ordering::Relaxed),
            "both slots halted: global MUST be halted"
        );
    }

    /// In single-slot mode `aggregate_halt_state` preserves the original behaviour:
    /// the global halt mirrors slot A's halt exactly.
    #[test]
    fn aggregate_halt_state_single_mode_mirrors_slot_a() {
        assert!(!pipeline::aggregate_halt_state(false, None));
        assert!(pipeline::aggregate_halt_state(true, None));
    }

    /// In dual mode, slot A must use its own per-slot halt Arc so that setting it
    /// does not change `state.pipeline_halted` directly (the aggregation task
    /// handles that).  The two Arcs must be distinct.
    #[test]
    fn dual_mode_slot_a_halt_arc_is_independent_from_global() {
        let state = AppState::new();
        let slot_a = slot_a_ui_arcs_for_mode(config::SlotMode::Dual, &state);

        // Setting per-slot arc must NOT change the global one.
        slot_a.pipeline_halted.store(true, Ordering::Relaxed);
        assert!(
            !state.pipeline_halted.load(Ordering::Relaxed),
            "halting slot A's per-slot Arc must not change the global halt Arc"
        );
        assert!(
            !Arc::ptr_eq(&slot_a.pipeline_halted, &state.pipeline_halted),
            "slot A halt arc must be a distinct object from the global halt arc"
        );
    }

    #[test]
    fn dual_mode_slot_a_error_arcs_are_independent_from_global() {
        let state = AppState::new();
        let slot_a = slot_a_ui_arcs_for_mode(config::SlotMode::Dual, &state);

        *slot_a.pipeline_error_msg.lock().expect("lock slot_a_error") =
            Some("slot A failed".to_string());
        *slot_a.auth_error_banner.lock().expect("lock slot_a_auth") =
            Some("slot A auth failed".to_string());

        assert!(
            state
                .pipeline_error_msg
                .lock()
                .expect("lock global_error")
                .is_none(),
            "slot A pipeline error must not write the global error banner in dual mode"
        );
        assert!(
            state
                .auth_error_banner
                .lock()
                .expect("lock global_auth")
                .is_none(),
            "slot A auth error must not write the global auth banner in dual mode"
        );
        assert!(!Arc::ptr_eq(
            &slot_a.pipeline_error_msg,
            &state.pipeline_error_msg
        ));
        assert!(!Arc::ptr_eq(
            &slot_a.auth_error_banner,
            &state.auth_error_banner
        ));
    }

    #[test]
    fn single_mode_slot_a_error_arcs_share_global_state() {
        let state = AppState::new();
        let slot_a = slot_a_ui_arcs_for_mode(config::SlotMode::Single, &state);

        assert!(Arc::ptr_eq(
            &slot_a.pipeline_error_msg,
            &state.pipeline_error_msg
        ));
        assert!(Arc::ptr_eq(
            &slot_a.auth_error_banner,
            &state.auth_error_banner
        ));
    }

    #[test]
    fn format_slot_error_status_prefers_auth_error() {
        assert_eq!(
            format_slot_error_status(Some("bad key".to_string()), Some("network".to_string())),
            "auth: bad key"
        );
        assert_eq!(
            format_slot_error_status(None, Some("network".to_string())),
            "error: network"
        );
        assert!(format_slot_error_status(None, None).is_empty());
    }

    /// Slot A's `tts_status` Arc must be shared (not orphaned): writing to the
    /// orchestrator context's Arc is visible via the outer handle retained by
    /// the label-copier task.
    #[test]
    fn slot_a_tts_status_arc_is_shared_not_orphaned() {
        // Simulate main.rs: create the Arc, clone it into the context.
        let outer: Arc<Mutex<pipeline::SlotProviderStatus>> =
            Arc::new(Mutex::new(pipeline::SlotProviderStatus::Ok));
        let ctx_arc = Arc::clone(&outer);

        // Orchestrator writes a degraded status.
        *ctx_arc.lock().expect("lock ctx_arc") =
            pipeline::SlotProviderStatus::Degraded("test".to_string());

        // Outer handle (retained by label-copier task) must see the write.
        let status = outer.lock().expect("lock outer").clone();
        assert_eq!(
            status,
            pipeline::SlotProviderStatus::Degraded("test".to_string()),
            "write through ctx_arc must be visible via outer Arc clone"
        );
    }
}
