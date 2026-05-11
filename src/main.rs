//! Entry point for TUI Translator.
//!
//! Phase 0 goal: open a terminal window, show the placeholder loading screen
//! (issue #26), and exit cleanly when the user presses `q` or `Ctrl+C`.
//! A live audio-level bar (issue #31) is fed by the audio capture foundation
//! and grows/shrinks with captured audio energy in real time.

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Gauge, Paragraph},
    Terminal,
};
use std::{
    io,
    sync::{atomic::Ordering, Arc, Mutex},
    time::Duration,
};

mod audio;
mod config;
mod metrics;
mod pipeline;
mod providers;
mod tui;

use audio::DEFAULT_SILENCE_THRESHOLD;
use tui::{AppState, AUDIO_LEVEL_SCALE};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tui_translator=info".into()),
        )
        .init();

    tracing::info!("tui-translator starting (Phase 0)");

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

    let result = run_tui(&state);

    // Cancel background tasks without waiting for them to finish.
    rt.shutdown_background();

    result
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
fn run_tui(state: &AppState) -> Result<()> {
    let mut terminal_guard = TerminalGuard::enter()?;
    event_loop(terminal_guard.terminal_mut(), state)
}

/// Main event loop: draw the UI, then poll for keyboard input.
///
/// Uses `event::poll` with a 50 ms timeout so the audio-level bar refreshes
/// at ~20 fps regardless of user input.
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &AppState,
) -> Result<()> {
    loop {
        let level = state.level_ratio();
        let dev_name = state.device_name_str();

        terminal.draw(|frame| draw_ui(frame, level, &dev_name))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if is_quit_key(&key) {
                    tracing::info!("user requested shutdown");
                    break;
                }
            }
        }
    }
    Ok(())
}

fn is_quit_key(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q'))
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
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

/// Draw the placeholder Phase 0 loading UI.
///
/// - `audio_level`: current RMS energy as a ratio in `[0.0, 1.0]`.
/// - `device_name`: human-readable name of the active capture device.
fn draw_ui(frame: &mut ratatui::Frame, audio_level: f64, device_name: &str) {
    let area = frame.size();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title bar
            Constraint::Length(3), // audio level bar
            Constraint::Min(1),    // content area
            Constraint::Length(3), // status bar
        ])
        .split(area);

    // Title bar — placeholder title per issue #26: "TUI Translator — Loading…"
    let title = Paragraph::new("TUI Translator — Loading\u{2026}")
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
    let bar_title = format!(" Audio — {device_name} ");
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(bar_title.as_str()),
        )
        .gauge_style(Style::default().fg(bar_color))
        .ratio(audio_level.clamp(0.0, 1.0));
    frame.render_widget(gauge, chunks[1]);

    // Content placeholder
    let content = Paragraph::new(
        "Audio capture, speech-to-text, and translation are not yet implemented.\n\
          This window proves Phase 0: the project builds and the TUI opens cleanly.\n\n\
          Press  q  or  Ctrl+C  to quit.",
    )
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .title(" Subtitles ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White)),
    );
    frame.render_widget(content, chunks[2]);

    // Status bar
    let status =
        Paragraph::new(" Status: loading  |  Phase: 0 — skeleton  |  Press q or Ctrl+C to quit ")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Left)
            .block(Block::default().borders(Borders::ALL));
    frame.render_widget(status, chunks[3]);
}
