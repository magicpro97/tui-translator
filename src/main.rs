//! Entry point for TUI Translator.
//!
//! Phase 0 goal: open a terminal window, show the placeholder loading screen
//! (issue #26), and exit cleanly when the user presses `q` or `Ctrl+C`.
//! A live audio-level bar (issue #31) is fed by the audio capture foundation
//! and grows/shrinks with captured audio energy in real time.
//!
//! The bilingual subtitle pane (issues #54–57) is wired in here: Up/Down
//! arrows scroll the pane; End jumps back to the auto-follow bottom.

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Gauge, Paragraph},
    Terminal,
};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
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
use tui::{subtitle_inner_area, AppState, SubtitlePane, AUDIO_LEVEL_SCALE};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tui_translator=info".into()),
        )
        .init();

    tracing::info!("tui-translator starting (Phase 0)");

    // Load configuration, falling back to built-in defaults if config.json is absent.
    let cfg_path = config_json_path();
    let cfg = config::load(&cfg_path)?;
    let restart_required = Arc::new(AtomicBool::new(false));

    // Start the hot-reload watcher; keep the receiver alive for the process lifetime.
    let _config_rx = match config::start_watcher(&cfg_path, cfg, restart_required.clone()) {
        Ok(rx) => Some(rx),
        Err(err) => {
            tracing::warn!("config hot-reload unavailable: {err:#}");
            None
        }
    };

    let state = AppState::new();

    // Build a multi-threaded Tokio runtime for the background audio capture task.
    // The TUI event loop runs on the main thread; audio runs on worker threads.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    match rt.block_on(audio::start_capture(DEFAULT_SILENCE_THRESHOLD)) {
        Ok(mut stream) => {
            overwrite_device_name(&state.device_name, &stream.info.device_name);

            let level_tx = state.audio_level.clone();
            rt.spawn(async move {
                while let Some(chunk) = stream.receiver.recv().await {
                    // Encode RMS as integer to allow atomic storage.
                    let encoded =
                        (chunk.rms_energy().clamp(0.0, 1.0) * AUDIO_LEVEL_SCALE as f32) as u32;
                    level_tx.store(encoded, Ordering::Relaxed);
                }
            });
        }
        Err(err) => {
            tracing::error!("audio capture failed to start: {err}");
        }
    }

    let result = run_tui(&state, &restart_required);

    // Cancel background tasks without waiting for them to finish.
    rt.shutdown_background();

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

fn overwrite_device_name(slot: &Arc<Mutex<String>>, next_name: &str) {
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

/// Run the minimal terminal interface for Phase 0.
fn run_tui(state: &AppState, restart_required: &Arc<AtomicBool>) -> Result<()> {
    let mut terminal_guard = TerminalGuard::enter()?;
    event_loop(terminal_guard.terminal_mut(), state, restart_required)
}

/// Main event loop: draw the UI, then poll for keyboard input.
///
/// Uses `event::poll` with a 50 ms timeout so the audio-level bar refreshes
/// at ~20 fps regardless of user input.  Crossterm resize events are handled
/// implicitly: the next `terminal.draw()` call picks up the new size
/// automatically, so no explicit resize handler is needed.
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &AppState,
    restart_required: &Arc<AtomicBool>,
) -> Result<()> {
    loop {
        let level = state.level_ratio();
        let dev_name = state.device_name_str();
        let show_restart_notice = restart_required.load(Ordering::Relaxed);
        let pane_area = subtitle_inner_area(terminal.size()?);

        {
            let mut pane = state
                .subtitle_pane
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            pane.clamp_scroll(pane_area.width, pane_area.height);
        }

        terminal.draw(|frame| {
            let pane = state
                .subtitle_pane
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            draw_ui(frame, level, &dev_name, &pane, show_restart_notice);
        })?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if is_quit_key(&key) {
                        tracing::info!("user requested shutdown");
                        break;
                    }
                    handle_scroll_key(&key, state, pane_area);
                }
                // Resize: ratatui auto-reflows on the next draw; no action needed.
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    Ok(())
}

fn is_quit_key(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q'))
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

/// Translate scroll keys into pane scroll commands.
fn handle_scroll_key(key: &KeyEvent, state: &AppState, pane_area: Rect) {
    let mut pane = state
        .subtitle_pane
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    match key.code {
        KeyCode::Up => pane.scroll_up(pane_area.width, pane_area.height),
        KeyCode::Down => pane.scroll_down(pane_area.width, pane_area.height),
        KeyCode::Home => pane.scroll_to_top(pane_area.width, pane_area.height),
        KeyCode::End => pane.scroll_to_bottom(),
        _ => {}
    }
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

/// Draw the Phase 0 loading UI.
///
/// - `audio_level`: current RMS energy as a ratio in `[0.0, 1.0]`.
/// - `device_name`: human-readable name of the active capture device.
/// - `subtitle_pane`: the stateful bilingual subtitle widget.
fn draw_ui(
    frame: &mut ratatui::Frame,
    audio_level: f64,
    device_name: &str,
    subtitle_pane: &SubtitlePane,
    show_restart_notice: bool,
) {
    let area = frame.size();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title bar
            Constraint::Length(3), // audio level bar
            Constraint::Min(1),    // subtitle pane
            Constraint::Length(3), // status bar
        ])
        .split(area);

    // Title bar — placeholder title per issue #26: "TUI Translator — Loading…"
    let title = Paragraph::new("TUI Translator \u{2014} Loading\u{2026}")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    // Live audio-level bar — reacts to captured audio (issue #31).
    // Colour: dark grey = silent, green = quiet, yellow = moderate, red = loud.
    let bar_color = if audio_level < 0.001 {
        Color::DarkGray
    } else if audio_level < 0.3 {
        Color::Green
    } else if audio_level < 0.7 {
        Color::Yellow
    } else {
        Color::Red
    };
    let bar_title = format!(" Audio \u{2014} {device_name} ");
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(bar_title.as_str()),
        )
        .gauge_style(Style::default().fg(bar_color))
        .ratio(audio_level.clamp(0.0, 1.0));
    frame.render_widget(gauge, chunks[1]);

    // Bilingual subtitle pane (issues #54–57).
    frame.render_widget(subtitle_pane, chunks[2]);

    // Status bar
    let mut status_text = " Status: loading  |  Phase: 0 — skeleton ".to_string();
    if show_restart_notice {
        status_text.push_str(" |  ⚠ Restart required for some settings");
    }
    status_text.push_str(" |  ↑↓ scroll  |  End: follow  |  q / Ctrl+C: quit ");

    let status = Paragraph::new(status_text)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Left)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(status, chunks[3]);
}
