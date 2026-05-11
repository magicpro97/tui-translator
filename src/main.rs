//! Entry point for TUI Translator.
//!
//! Phase 0 goal: open a terminal window, show a placeholder title, and exit
//! cleanly when the user presses `q` or `Ctrl+C`. Nothing here touches audio,
//! APIs, or configuration — those come in later phases.

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
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::io;

mod audio;
mod config;
mod metrics;
mod pipeline;
mod providers;
mod tui;

fn main() -> Result<()> {
    // Initialise structured logging.  RUST_LOG controls the verbosity.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tui_translator=info".into()),
        )
        .init();

    tracing::info!("tui-translator starting (Phase 0 skeleton)");

    run_tui()
}

/// Run the minimal terminal interface for Phase 0.
fn run_tui() -> Result<()> {
    let mut terminal_guard = TerminalGuard::enter()?;
    event_loop(terminal_guard.terminal_mut())
}

/// Main event loop: draw the UI, then handle keyboard input.
fn event_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    loop {
        terminal.draw(draw_ui)?;

        if let Event::Key(key) = event::read()? {
            if is_quit_key(&key) {
                tracing::info!("user requested shutdown");
                break;
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

/// Draw the placeholder Phase 0 UI.
fn draw_ui(frame: &mut ratatui::Frame) {
    let area = frame.size();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title bar
            Constraint::Min(1),    // content area
            Constraint::Length(3), // status bar
        ])
        .split(area);

    // Title bar
    let title = Paragraph::new("TUI Translator — Phase 0 Skeleton")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

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
    frame.render_widget(content, chunks[1]);

    // Status bar
    let status =
        Paragraph::new(" Status: ready  |  Phase: 0 — skeleton  |  Press q or Ctrl+C to quit ")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Left)
            .block(Block::default().borders(Borders::ALL));
    frame.render_widget(status, chunks[2]);
}
