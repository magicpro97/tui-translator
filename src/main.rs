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

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

mod audio;
mod config;
mod metrics;
mod pipeline;
mod providers;
mod tui;

use audio::DEFAULT_SILENCE_THRESHOLD;
use metrics::{
    spawn_process_metrics_task, LatencyHistogram, LossMetrics, MetricsSnapshot, NetworkMetrics,
    ProcessSnapshot,
};
use tui::{
    draw_session_summary, draw_ui, subtitle_inner_area, AppState, UserAction, AUDIO_LEVEL_SCALE,
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
fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tui_translator=info".into()),
        )
        .init();

    tracing::info!("tui-translator starting");

    // Issue #88 — install the Windows console control handler before the TUI
    // enters alternate-screen mode so a forced close triggers our cleanup.
    windows_signal::install();

    // Load configuration, falling back to built-in defaults if config.json is absent.
    let cfg_path = config_json_path();
    let cfg = config::load(&cfg_path)?;
    let current_config = Arc::new(Mutex::new(cfg.clone()));
    let restart_required = Arc::new(AtomicBool::new(false));

    // Start the hot-reload watcher; keep the receiver alive for the process lifetime.
    let config_rx = match config::start_watcher(&cfg_path, cfg, restart_required.clone()) {
        Ok(rx) => Some(rx),
        Err(err) => {
            tracing::warn!("config hot-reload unavailable: {err:#}");
            None
        }
    };

    let state = AppState::new();
    state.set_target_language(
        current_config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .target_language
            .clone(),
    );
    // Initialise source_language from the loaded config so AppState and the
    // orchestrator start with the same value.
    overwrite_source_language(
        &state.source_language,
        &current_config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .source_language
            .clone(),
    );
    state.set_tts_enabled(
        current_config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .tts_enabled,
    );
    let playback_service: SharedPlaybackService = Arc::new(Mutex::new(None));
    {
        let current_cfg = current_config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        sync_playback_service_state(&playback_service, &current_cfg, current_cfg.tts_enabled);
    }

    // Build a multi-threaded Tokio runtime for background tasks.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    if let Some(mut config_rx) = config_rx {
        let current_config = Arc::clone(&current_config);
        let target_language = Arc::clone(&state.target_language);
        let source_language = Arc::clone(&state.source_language);
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
                let te = Arc::clone(&tts_enabled);
                let rr = Arc::clone(&restart_required);
                let ps = Arc::clone(&playback_service);
                tokio::task::spawn_blocking(move || {
                    apply_runtime_config(&cc, &tl, &sl, &te, &rr, &ps, next_cfg);
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

    // Issue #79: start the process-metrics polling task and get its receiver.
    let (process_tx, process_rx) = tokio::sync::watch::channel(ProcessSnapshot::default());
    spawn_process_metrics_task(process_tx);

    match rt.block_on(audio::start_capture(DEFAULT_SILENCE_THRESHOLD)) {
        Ok(stream) => {
            overwrite_device_name(&state.device_name, &stream.info.device_name);
            *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
                metrics::SttState::Listening;

            // ── Issue #84 — wire orchestrator or fall back to metrics-only ────
            let api_key = current_config
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .google_api_key
                .clone();

            if let Some(key) = api_key {
                // Build Google providers and start the orchestrator.
                // Reuse the Arc already held in AppState so hot-reload writes
                // are visible to the running orchestrator.
                let source_language = Arc::clone(&state.source_language);

                let stt_provider = match providers::google::stt::GoogleSttProvider::new(key.clone())
                {
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
                            },
                        );
                    }
                };
                // Issue #71–#76: wire cost reporter so MT API usage is billed
                // against the shared CostCounter.
                let mt_provider = match providers::google::mt::GoogleMtProvider::new(key.clone()) {
                    Ok(p) => p.with_cost_reporter(
                        Arc::clone(&state.cost_counter) as Arc<dyn providers::CostReporter>
                    ),
                    Err(err) => {
                        tracing::error!("failed to create MT provider: {err}");
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
                            },
                        );
                    }
                };
                // Issue #71–#76: wire cost reporter so TTS API usage is billed
                // against the shared CostCounter.
                let tts_provider = match providers::google::tts::GoogleTtsProvider::new(key) {
                    Ok(p) => p.with_cost_reporter(
                        Arc::clone(&state.cost_counter) as Arc<dyn providers::CostReporter>
                    ),
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
                            },
                        );
                    }
                };

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
                    playback: Arc::clone(&playback_service),
                    shutdown: Arc::clone(&orchestrator_shutdown),
                    e2e_latency: Arc::clone(&e2e_latency),
                    network_metrics: Arc::clone(&network_metrics),
                    loss_metrics: Arc::clone(&loss_metrics),
                };

                orchestrator_join = Some(rt.spawn(pipeline::run_orchestrator(
                    stream.receiver,
                    stt_provider,
                    mt_provider,
                    tts_provider,
                    ctx,
                )));
            } else {
                // No API key configured — run metrics-only audio task.
                tracing::info!(
                    "no google_api_key configured; running without STT/MT/TTS (issue #84)"
                );
                spawn_metrics_only_audio_task(&rt, stream, &state);
                orchestrator_join = None;
            }
        }
        Err(err) => {
            *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
                metrics::SttState::Error(err.to_string());
            tracing::error!("audio capture failed to start: {err}");
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
    } = args;

    // ── Issues #61, #82: metrics observability background task ───────────────
    // Publish a fresh `MetricsSnapshot` to the watch channel every second so the
    // UI can read it lock-free via `state.metrics_snapshot()` (issue #82).
    {
        let metrics_src = Arc::clone(&state.session_metrics);
        let metrics_tx = Arc::clone(&state.metrics_tx);
        let subtitle_pane = Arc::clone(&state.subtitle_pane);
        let cost_counter = Arc::clone(&state.cost_counter);
        let mut process_rx = process_rx;
        rt.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
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

                let mut snapshot = MetricsSnapshot {
                    audio_seconds_sent: session.audio_seconds_sent,
                    chars_translated: session.chars_translated,
                    // Issue #71–#76: derive running cost from shared CostCounter so
                    // STT, MT, and TTS costs are all included.
                    estimated_cost_usd: cost_counter.current_estimate_usd(),
                    line_pairs_shown,
                    session_start: session.session_start,
                    ..MetricsSnapshot::default()
                };

                // Issue #79: apply CPU/RAM from the process-metrics task.
                snapshot.apply_process(&proc_snap);
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
        let keyboard_shutdown = Arc::clone(&keyboard_shutdown);
        rt.spawn(async move {
            tokio::task::spawn_blocking(move || {
                keyboard_task(key_tx, lang_flag, keyboard_shutdown)
            })
            .await
            .ok();
        });
    }

    let result = run_tui(
        state,
        restart_required,
        cfg_path,
        current_config,
        playback_service,
        &keyboard_shutdown,
        key_rx,
    );

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

/// Returns the path to `config.json`, resolved relative to the running
/// executable so the file stays portable regardless of the working directory.
fn config_json_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("config.json")))
        .unwrap_or_else(|| std::path::PathBuf::from("config.json"))
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

fn apply_runtime_config(
    current_config: &Arc<Mutex<config::AppConfig>>,
    target_language: &Arc<std::sync::Mutex<String>>,
    source_language: &Arc<std::sync::Mutex<String>>,
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
    // Sync the backend first; only set the UI flag to match what actually succeeded.
    let service_ok = sync_playback_service_state(playback_service, &next_cfg, next_cfg.tts_enabled);
    tts_enabled.store(next_cfg.tts_enabled && service_ok, Ordering::Relaxed);
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
        .audio_seconds_sent += audio_secs;
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

/// Run the terminal interface.  Enters the alternate screen, runs the event
/// loop, then returns.  The [`TerminalGuard`] restores the terminal on drop.
fn run_tui(
    state: &AppState,
    restart_required: &Arc<AtomicBool>,
    cfg_path: &Path,
    current_config: &Arc<Mutex<config::AppConfig>>,
    playback_service: &SharedPlaybackService,
    keyboard_shutdown: &Arc<AtomicBool>,
    key_rx: mpsc::Receiver<UserAction>,
) -> Result<()> {
    let mut terminal_guard = TerminalGuard::enter()?;
    let result = event_loop(
        terminal_guard.terminal_mut(),
        state,
        restart_required,
        cfg_path,
        current_config,
        playback_service,
        key_rx,
    );
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
    restart_required: &Arc<AtomicBool>,
    cfg_path: &Path,
    current_config: &Arc<Mutex<config::AppConfig>>,
    playback_service: &SharedPlaybackService,
    key_rx: mpsc::Receiver<UserAction>,
) -> Result<()> {
    loop {
        // ── Issue #88 — check for Windows forced-close signal ─────────────────
        if FORCED_SHUTDOWN.load(Ordering::Relaxed) {
            tracing::info!("forced shutdown signal received; exiting event loop (issue #88)");
            break;
        }

        let expanded = state.metrics_expanded.load(Ordering::Relaxed);
        let show_restart = restart_required.load(Ordering::Relaxed);
        let pane_area = subtitle_inner_area(terminal.size()?, expanded);

        {
            let mut pane = state
                .subtitle_pane
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            pane.clamp_scroll(pane_area.width, pane_area.height);
        }

        let level = state.level_ratio();
        let dev_name = state.device_name_str();
        let cost_warning_usd = current_config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cost_warning_usd;
        terminal.draw(|frame| {
            draw_ui(
                frame,
                state,
                &dev_name,
                level,
                show_restart,
                cost_warning_usd,
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
                    handle_action(
                        &action,
                        state,
                        pane_area,
                        restart_required,
                        cfg_path,
                        current_config,
                        playback_service,
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
/// `in_lang_prompt` routes character input to the language-change prompt rather
/// than to the normal command set (issue #64).
fn key_to_action(key: &KeyEvent, in_lang_prompt: bool) -> Option<UserAction> {
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
                let in_prompt = lang_prompt_active.load(Ordering::Relaxed);
                if let Some(action) = key_to_action(&key, in_prompt) {
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
    pane_area: ratatui::layout::Rect,
    restart_required: &Arc<AtomicBool>,
    cfg_path: &Path,
    current_config: &Arc<Mutex<config::AppConfig>>,
    playback_service: &SharedPlaybackService,
) {
    match action {
        // Scrolling
        UserAction::ScrollUp => state
            .subtitle_pane
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .scroll_up(pane_area.width, pane_area.height),
        UserAction::ScrollDown => state
            .subtitle_pane
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .scroll_down(pane_area.width, pane_area.height),
        UserAction::ScrollTop => state
            .subtitle_pane
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .scroll_to_top(pane_area.width, pane_area.height),
        UserAction::ScrollBottom => state
            .subtitle_pane
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .scroll_to_bottom(),

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

        // ? — show / hide help (issue #66)
        UserAction::ToggleHelp => {
            let show_help = !state.show_help.load(Ordering::Relaxed);
            state.show_help.store(show_help, Ordering::Relaxed);
            if show_help {
                state.lang_prompt_active.store(false, Ordering::Relaxed);
                *state.lang_input.lock().unwrap_or_else(|p| p.into_inner()) = String::new();
            }
        }

        // Escape — dismiss open overlay (issue #64)
        UserAction::DismissOverlay => {
            if state.lang_prompt_active.load(Ordering::Relaxed) {
                state.lang_prompt_active.store(false, Ordering::Relaxed);
                *state.lang_input.lock().unwrap_or_else(|p| p.into_inner()) = String::new();
            } else if state.show_help.load(Ordering::Relaxed) {
                state.show_help.store(false, Ordering::Relaxed);
            }
        }

        // L — open language prompt (issue #64)
        UserAction::PromptLanguage => {
            if !state.lang_prompt_active.load(Ordering::Relaxed) {
                state.show_help.store(false, Ordering::Relaxed);
                *state.lang_input.lock().unwrap_or_else(|p| p.into_inner()) = String::new();
                state.lang_prompt_active.store(true, Ordering::Relaxed);
            }
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
                state.set_target_language(next_language.to_string());
                {
                    let mut current = current_config.lock().unwrap_or_else(|p| p.into_inner());
                    current.target_language = next_language.to_string();
                }
                tracing::info!("target language changed to {next_language}");
            }
            state.lang_prompt_active.store(false, Ordering::Relaxed);
            *state.lang_input.lock().unwrap_or_else(|p| p.into_inner()) = String::new();
        }
        UserAction::LangCancel => {
            state.lang_prompt_active.store(false, Ordering::Relaxed);
            *state.lang_input.lock().unwrap_or_else(|p| p.into_inner()) = String::new();
        }

        UserAction::AnyKey => {}

        // R — signal config reload (issue #64)
        UserAction::ReloadConfig => match config::load(cfg_path) {
            Ok(next_cfg) => {
                apply_runtime_config(
                    current_config,
                    &state.target_language,
                    &state.source_language,
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
        metrics::cost::format_cost_display(metrics.estimated_cost_usd)
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
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;
    use std::io::Write;
    use tempfile::NamedTempFile;

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

        assert_eq!(key_to_action(&key, false), Some(UserAction::AnyKey));
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
}
