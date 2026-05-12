//! Shared PTY harness for Layer-3 terminal behaviour tests (issues #104–#107).
//!
//! Spawns the `tui-translator` binary inside a ConPTY (Windows Pseudo Console)
//! using `portable-pty`, feeds PTY output through a `vt100` terminal emulator,
//! and exposes helpers for asserting on visible screen content.
//!
//! ## Design
//! - A background OS thread reads raw bytes from the PTY master and feeds them
//!   to a [`vt100::Parser`]; the test thread polls the parsed screen state via
//!   an `Arc<Mutex<vt100::Parser>>`.
//! - The background reader also responds to ANSI DSR cursor-position queries
//!   (`\x1b[6n`) so that crossterm inside the child does not block waiting for
//!   a cursor-position report from the host.  Windows ConPTY does not intercept
//!   DSR queries automatically; the harness must answer them.
//! - All public `wait_*` methods are bounded by a `Duration`; no call can block
//!   indefinitely.
//! - [`PtySession::drop`] kills the child process unconditionally so test
//!   failures never leave orphan processes.
//!
//! ## Why `RUST_LOG=off`
//! The `tracing-subscriber` formatter writes structured log lines to *stderr*,
//! which on a ConPTY slave is the same file descriptor as stdout.  Injecting
//! `RUST_LOG=off` keeps the byte stream clean so the vt100 parser sees only
//! ratatui's escape-sequence output.

#![allow(dead_code)]

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Path to the compiled `tui-translator` binary, injected by Cargo.
///
/// `cargo test --test pty` builds the binary as a side effect, so this
/// path is always valid when the tests run.
pub const BINARY: &str = env!("CARGO_BIN_EXE_tui-translator");

/// Generous timeout for the first complete TUI frame to appear on screen.
///
/// WASAPI loopback initialisation can take up to 5 s when the default render
/// device responds slowly.  An additional 7-second buffer covers process
/// startup, runtime init, and first ratatui draw.
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(12);

/// Timeout for the process to exit after a quit keystroke has been sent.
pub const EXIT_TIMEOUT: Duration = Duration::from_secs(8);

// ── PtySession ────────────────────────────────────────────────────────────────

/// A running `tui-translator` session managed inside a PTY.
pub struct PtySession {
    /// PTY master handle used only for `resize()`.
    master: Box<dyn MasterPty + Send>,
    /// Write end of the PTY (shared with the DSR-responder background thread).
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Child process handle.
    pub(super) child: Box<dyn portable_pty::Child + Send + Sync>,
    /// Shared terminal emulator state continuously updated by the background
    /// reader thread.
    parser: Arc<Mutex<vt100::Parser>>,
    /// Terminal rows configured at spawn time.
    pub rows: u16,
    /// Terminal columns configured at spawn time.
    pub cols: u16,
    /// Counter of raw bytes received from the PTY master since spawn.
    bytes_rx: Arc<AtomicUsize>,
    /// First raw bytes received (up to 256) for diagnostic purposes.
    raw_capture: Arc<Mutex<Vec<u8>>>,
}

impl PtySession {
    /// Spawn the binary in a PTY of the given dimensions.
    ///
    /// `extra_env` is a slice of `(key, value)` pairs appended to the child's
    /// environment.  `RUST_LOG=off` is always prepended.
    ///
    /// The working directory is set to `std::env::temp_dir()` so no accidental
    /// `config.json` from the repository root is loaded.
    pub fn spawn(cols: u16, rows: u16, extra_env: &[(&str, &str)]) -> Result<Self, String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("openpty({cols}×{rows}): {e}"))?;

        let mut cmd = CommandBuilder::new(BINARY);
        // Silence tracing output so it doesn't pollute the PTY byte stream.
        cmd.env("RUST_LOG", "off");
        // Neutral working directory to avoid picking up a real config.json.
        cmd.cwd(std::env::temp_dir());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("spawn_command: {e}"))?;

        // Obtain the reader *before* taking the writer (order matters on some
        // implementations).
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("try_clone_reader: {e}"))?;

        let raw_writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("take_writer: {e}"))?;
        // Wrap in Arc<Mutex<>> so the background reader thread can share it
        // for DSR (cursor-position query) responses.
        let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(raw_writer));

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
        let bytes_rx = Arc::new(AtomicUsize::new(0));
        let raw_capture: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));

        // Background reader: drain PTY output, keep vt100 screen in sync, and
        // respond to ANSI DSR cursor-position queries so crossterm doesn't block.
        {
            let parser_bg = Arc::clone(&parser);
            let bytes_bg = Arc::clone(&bytes_rx);
            let raw_bg = Arc::clone(&raw_capture);
            let writer_bg = Arc::clone(&writer);
            std::thread::spawn(move || {
                let mut buf = vec![0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            bytes_bg.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
                            {
                                let mut cap = raw_bg.lock().unwrap();
                                if cap.len() < 256 {
                                    let take = n.min(256 - cap.len());
                                    cap.extend_from_slice(&buf[..take]);
                                }
                            }
                            // Respond to ANSI DSR cursor-position queries.
                            // crossterm sends \x1b[6n ("Device Status Report") to
                            // ask for the current cursor position; the terminal must
                            // reply with \x1b[row;colR ("Cursor Position Report").
                            // Windows ConPTY does not handle this automatically when
                            // used via portable-pty, so the harness responds on
                            // behalf of the terminal (cursor at row 1, col 1).
                            let data = &buf[..n];
                            if data.windows(4).any(|w| w == b"\x1b[6n") {
                                if let Ok(mut w) = writer_bg.lock() {
                                    let _ = w.write_all(b"\x1b[1;1R");
                                    let _ = w.flush();
                                }
                            }
                            let mut p = parser_bg.lock().unwrap();
                            p.process(data);
                        }
                    }
                }
            });
        }

        Ok(Self {
            master: pair.master,
            writer,
            child,
            parser,
            rows,
            cols,
            bytes_rx,
            raw_capture,
        })
    }

    // ── Screen inspection ─────────────────────────────────────────────────────

    /// Block until `needle` appears anywhere on the virtual screen, or until
    /// `timeout` elapses.  Returns `true` when found.
    ///
    /// On timeout, dumps the current screen content to stderr to aid diagnosis.
    pub fn wait_for_text(&self, needle: &str, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.screen_contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        // Diagnostic: show what is actually on screen when the wait times out.
        let bytes = self.bytes_rx.load(std::sync::atomic::Ordering::Relaxed);
        let raw_preview = self.raw_capture.lock().unwrap().clone();
        eprintln!(
            "[PTY timeout] waited {:.1}s for {:?}; bytes_rx={bytes}; raw[0..16]={:?}; screen ({} rows):",
            timeout.as_secs_f32(),
            needle,
            &raw_preview[..raw_preview.len().min(16)],
            self.rows,
        );
        for (i, row) in self.all_rows().iter().enumerate() {
            eprintln!("  row {:02}: {:?}", i, row);
        }
        false
    }

    /// Return `true` if `needle` is present anywhere on the current screen.
    pub fn screen_contains(&self, needle: &str) -> bool {
        let p = self.parser.lock().unwrap();
        let screen = p.screen();
        (0..self.rows).any(|row| Self::extract_row_from(screen, row).contains(needle))
    }

    /// Return the visible text content of a single screen row (0-indexed).
    pub fn row_text(&self, row: u16) -> String {
        let p = self.parser.lock().unwrap();
        Self::extract_row_from(p.screen(), row)
    }

    /// Dump the entire screen as a `Vec<String>`, one entry per row.
    pub fn all_rows(&self) -> Vec<String> {
        let p = self.parser.lock().unwrap();
        let screen = p.screen();
        (0..self.rows)
            .map(|row| Self::extract_row_from(screen, row))
            .collect()
    }

    /// Total number of raw bytes received from the PTY master so far.
    pub fn bytes_received(&self) -> usize {
        self.bytes_rx.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Collect all cell characters on `row` into a single `String`.
    ///
    /// Cells belonging to the right half of a wide character return an empty
    /// string from `vt100::Cell::contents()`; those are simply skipped.
    fn extract_row_from(screen: &vt100::Screen, row: u16) -> String {
        let (_rows, cols) = screen.size();
        (0..cols)
            .filter_map(|col| screen.cell(row, col))
            .map(|cell| cell.contents().to_owned())
            .collect()
    }

    // ── Input ─────────────────────────────────────────────────────────────────

    /// Write raw bytes to the PTY stdin (child's keyboard input).
    pub fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut w = self
            .writer
            .lock()
            .map_err(|e| format!("writer lock: {e}"))?;
        w.write_all(bytes).map_err(|e| format!("PTY write: {e}"))?;
        w.flush().map_err(|e| format!("PTY flush: {e}"))
    }

    /// Press 'q' twice with a 300 ms gap so the event loop can process the
    /// first keypress (draw the session-summary overlay) before the second
    /// keypress dismisses it.
    ///
    /// If both bytes are written in a single [`send`] call they can both be
    /// consumed by the event loop's `try_recv` drain pass, leaving the
    /// secondary `recv_timeout` wait with nothing to receive.
    pub fn quit_cleanly(&mut self) -> Result<(), String> {
        self.send(b"q")?;
        // One full event-loop cycle (50 ms draw period) plus generous margin.
        std::thread::sleep(Duration::from_millis(300));
        self.send(b"q")
    }

    // ── PTY control ───────────────────────────────────────────────────────────

    /// Resize the PTY window and signal the child to redraw.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("PTY resize: {e}"))
    }

    // ── Process lifecycle ─────────────────────────────────────────────────────

    /// Return `true` if the child process has not yet exited.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Poll for process exit within `timeout`; return the exit code on success
    /// or `None` on timeout.
    pub fn wait_exit(&mut self, timeout: Duration) -> Option<u32> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            match self.child.try_wait() {
                Ok(Some(status)) => return Some(status.exit_code()),
                Ok(None) => {}
                Err(_) => break,
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    /// Kill the child process unconditionally (best-effort).
    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

impl Drop for PtySession {
    /// Kill the child process on drop so test failures never leave orphan
    /// processes running in the background.
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
