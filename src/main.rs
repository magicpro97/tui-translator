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
use tui::{
    draw_session_summary, draw_ui, subtitle_inner_area, AppState, UserAction, AUDIO_LEVEL_SCALE,
};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tui_translator=info".into()),
        )
        .init();

    tracing::info!("tui-translator starting");

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
    state.set_tts_enabled(
        current_config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .tts_enabled,
    );

    // Build a multi-threaded Tokio runtime for background tasks.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    if let Some(mut config_rx) = config_rx {
        let current_config = Arc::clone(&current_config);
        let target_language = Arc::clone(&state.target_language);
        let tts_enabled = Arc::clone(&state.tts_enabled);
        let restart_required = Arc::clone(&restart_required);
        rt.spawn(async move {
            while config_rx.changed().await.is_ok() {
                let next_cfg = config_rx.borrow().clone();
                apply_runtime_config(
                    &current_config,
                    &target_language,
                    &tts_enabled,
                    &restart_required,
                    next_cfg,
                );
            }
        });
    }

    match rt.block_on(audio::start_capture(DEFAULT_SILENCE_THRESHOLD)) {
        Ok(mut stream) => {
            overwrite_device_name(&state.device_name, &stream.info.device_name);
            *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
                metrics::SttState::Listening;

            let level_tx = state.audio_level.clone();
            let paused = Arc::clone(&state.paused);
            let session_metrics = Arc::clone(&state.session_metrics);
            let stt_state = Arc::clone(&state.stt_state);
            rt.spawn(async move {
                loop {
                    let Some(chunk) = stream.receiver.recv().await else {
                        mark_audio_capture_stopped(&level_tx, &stt_state);
                        break;
                    };
                    handle_audio_chunk(chunk, &paused, &level_tx, &session_metrics);
                }
            });
        }
        Err(err) => {
            *state.stt_state.lock().unwrap_or_else(|p| p.into_inner()) =
                metrics::SttState::Error(err.to_string());
            tracing::error!("audio capture failed to start: {err}");
        }
    }

    // ── Issue #61: metrics observability background task ─────────────────────
    // Publish a fresh `SessionMetrics` snapshot to the watch channel every second
    // so the UI can read it lock-free via `state.metrics_snapshot()`.
    {
        let metrics_src = Arc::clone(&state.session_metrics);
        let metrics_tx = Arc::clone(&state.metrics_tx);
        let subtitle_pane = Arc::clone(&state.subtitle_pane);
        rt.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let mut snapshot = metrics_src
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                snapshot.line_pairs_shown = subtitle_pane
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .pair_count() as u64;
                snapshot.recalculate_cost();
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
        &state,
        &restart_required,
        &cfg_path,
        &current_config,
        &keyboard_shutdown,
        key_rx,
    );

    // Cancel background tasks without waiting for them to finish.
    rt.shutdown_background();

    // ── Issue #64: print session summary to stdout after terminal is restored ─
    // The alternate screen was left when `run_tui` returned (TerminalGuard::drop).
    // Only print when the user quit intentionally (not on error paths).
    if result.is_ok() {
        print_session_summary_to_stdout(&state);
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

fn apply_runtime_config(
    current_config: &Arc<Mutex<config::AppConfig>>,
    target_language: &Arc<std::sync::Mutex<String>>,
    tts_enabled: &Arc<AtomicBool>,
    restart_required: &Arc<AtomicBool>,
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
    tts_enabled.store(next_cfg.tts_enabled, Ordering::Relaxed);
}

fn handle_audio_chunk(
    chunk: audio::AudioChunk,
    paused: &Arc<AtomicBool>,
    level_tx: &Arc<AtomicU32>,
    session_metrics: &Arc<Mutex<metrics::SessionMetrics>>,
) {
    if paused.load(Ordering::Relaxed) {
        level_tx.store(0, Ordering::Relaxed);
        return;
    }

    let encoded = (chunk.rms_energy().clamp(0.0, 1.0) * AUDIO_LEVEL_SCALE as f32) as u32;
    level_tx.store(encoded, Ordering::Relaxed);

    let mut metrics = session_metrics.lock().unwrap_or_else(|p| p.into_inner());
    metrics.audio_seconds_sent += f64::from(chunk.duration_ms) / 1000.0;
    metrics.recalculate_cost();
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
    key_rx: mpsc::Receiver<UserAction>,
) -> Result<()> {
    loop {
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
        terminal.draw(|frame| {
            draw_ui(frame, state, &dev_name, level, show_restart);
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
            state.toggle_tts();
            tracing::info!(
                "TTS toggled: {}",
                if state.tts_enabled.load(Ordering::Relaxed) {
                    "on"
                } else {
                    "off"
                }
            );
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
                    &state.tts_enabled,
                    restart_required,
                    next_cfg,
                );
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
    println!("  Characters:      {}", metrics.chars_translated);
    println!("  Estimated cost:  ${:.4}", metrics.estimated_cost_usd);
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

        handle_action(
            &UserAction::LangApply,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            Path::new("config.json"),
            &current_config,
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

        handle_action(
            &UserAction::ReloadConfig,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            config_file.path(),
            &current_config,
        );

        assert_eq!(state.target_language(), "en");
        assert!(state.tts_enabled.load(Ordering::Relaxed));
        assert_eq!(current_config.lock().unwrap().target_language, "en");
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

        handle_action(
            &UserAction::PromptLanguage,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            Path::new("config.json"),
            &current_config,
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

        handle_action(
            &UserAction::ToggleHelp,
            &state,
            Rect::new(0, 0, 80, 24),
            &restart_required,
            Path::new("config.json"),
            &current_config,
        );

        assert!(state.show_help.load(Ordering::Relaxed));
        assert!(!state.lang_prompt_active.load(Ordering::Relaxed));
        assert!(state.lang_input.lock().unwrap().is_empty());
    }
}
