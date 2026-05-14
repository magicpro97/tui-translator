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
//! ## Log routing
//! Since issue #183, `init_tracing()` in `src/main.rs` routes tracing output
//! to a log file (`tui-translator.log` in the OS temp directory) rather than
//! stderr.  Log lines therefore never enter the ConPTY byte stream, so the
//! harness no longer needs to suppress logging via `RUST_LOG=off`.  Tests
//! that want to exercise logging explicitly can pass `("RUST_LOG", "...")` in
//! `extra_env`.

#![allow(dead_code)]

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

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
    /// Per-session sandbox that holds the fixture-backed config.json used by
    /// most PTY tests so they never touch real WASAPI devices on the host.
    _sandbox: TempDir,
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
    /// All raw PTY bytes received so far, retained for diagnostic and assertion
    /// purposes.  The buffer grows without bound; callers that only need a
    /// preview should read a bounded slice rather than cloning the whole vec.
    raw_capture: Arc<Mutex<Vec<u8>>>,
}

impl PtySession {
    /// Spawn the binary in a PTY of the given dimensions.
    ///
    /// `extra_env` is a slice of `(key, value)` pairs appended to the child's
    /// environment.
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

        let sandbox = tempfile::tempdir().map_err(|e| format!("tempdir: {e}"))?;
        let mut cmd = CommandBuilder::new(BINARY);
        // Most PTY tests assert the steady-state UI and intentionally bypass
        // the first-run setup flow unless they override this flag explicitly.
        cmd.env("TUI_TRANSLATOR_SKIP_ONBOARDING", "1");
        // Neutral working directory inside the sandbox so the child never
        // discovers a user or repo-local config.json by accident.
        cmd.cwd(sandbox.path());
        if should_inject_fixture_config(extra_env) {
            let cfg_path = write_fixture_backed_config(sandbox.path())?;
            cmd.env("TUI_TRANSLATOR_CONFIG", cfg_path.as_os_str());
        }
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
                // Rolling tail: carry the last 3 bytes of the previous read so
                // that `\x1b[6n` is detected even when it straddles two reads.
                let mut dsr_tail: Vec<u8> = Vec::with_capacity(3);
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            bytes_bg.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
                            let data = &buf[..n];
                            {
                                let mut cap = raw_bg.lock().unwrap();
                                cap.extend_from_slice(data);
                            }
                            // Respond to ANSI DSR cursor-position queries.
                            // crossterm sends \x1b[6n ("Device Status Report") to
                            // ask for the current cursor position; the terminal must
                            // reply with \x1b[row;colR ("Cursor Position Report").
                            // Windows ConPTY does not handle this automatically when
                            // used via portable-pty, so the harness responds on
                            // behalf of the terminal (cursor at row 1, col 1).
                            //
                            // The sequence is 4 bytes and may be split across two
                            // reads.  Prepend the last 3 bytes of the previous read
                            // before searching so split sequences are still caught.
                            let check: Vec<u8> =
                                dsr_tail.iter().chain(data.iter()).copied().collect();
                            if check.windows(4).any(|w| w == b"\x1b[6n") {
                                if let Ok(mut w) = writer_bg.lock() {
                                    let _ = w.write_all(b"\x1b[1;1R");
                                    let _ = w.flush();
                                }
                            }
                            // Update rolling tail: keep at most the last 3 bytes.
                            let tail_start = data.len().saturating_sub(3);
                            dsr_tail.clear();
                            dsr_tail.extend_from_slice(&data[tail_start..]);

                            let mut p = parser_bg.lock().unwrap();
                            p.process(data);
                        }
                    }
                }
            });
        }

        Ok(Self {
            _sandbox: sandbox,
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
        // Take only a small preview inside the lock to avoid cloning the
        // (potentially large) unbounded capture buffer.
        let raw_preview: Vec<u8> = {
            let cap = self.raw_capture.lock().unwrap();
            cap[..cap.len().min(32)].to_vec()
        };
        eprintln!(
            "[PTY timeout] waited {:.1}s for {:?}; bytes_rx={bytes}; raw[0..32]={:?}; screen ({} rows):",
            timeout.as_secs_f32(),
            needle,
            raw_preview,
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

    /// Return a snapshot of all raw PTY bytes captured so far.
    ///
    /// Useful for asserting on terminal-cleanup escape sequences (e.g.
    /// `\x1b[?1049l` for leave-alternate-screen, `\x1b[?25h` for cursor-show)
    /// that are emitted by the app's [`TerminalGuard`] drop implementation.
    pub fn raw_bytes_captured(&self) -> Vec<u8> {
        self.raw_capture.lock().unwrap().clone()
    }

    /// Block until `sequence` appears anywhere in the accumulated raw PTY
    /// bytes, or until `timeout` elapses.  Returns `true` when found.
    ///
    /// This is the right tool for asserting on terminal-cleanup sequences
    /// emitted after process exit, since the background reader may still be
    /// draining the PTY pipe when `wait_exit` returns.
    pub fn wait_for_raw_sequence(&self, sequence: &[u8], timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            {
                let cap = self.raw_capture.lock().unwrap();
                if cap.windows(sequence.len()).any(|w| w == sequence) {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
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

    /// Resize the PTY window, update the internal dimension fields, and
    /// resize the `vt100` parser to match.
    ///
    /// Updating the parser is essential so that post-resize screen inspection
    /// uses the correct row/column counts, and so the virtual screen buffer
    /// properly renders the new-size redraw that the child emits in response
    /// to the resize event.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), String> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("PTY resize: {e}"))?;
        self.rows = rows;
        self.cols = cols;
        self.parser.lock().unwrap().set_size(rows, cols);
        Ok(())
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

fn write_fixture_backed_config(dir: &std::path::Path) -> Result<PathBuf, String> {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("soak")
        .join("soak_audio.wav");
    let fixture = fixture
        .canonicalize()
        .map_err(|e| format!("canonicalize soak fixture: {e}"))?;
    let cfg_path = dir.join("config.json");
    let payload = format!(
        concat!(
            "{{\n",
            "  \"google_api_key\": \"pty-test-key\",\n",
            "  \"source_language\": \"ja-JP\",\n",
            "  \"target_language\": \"vi\",\n",
            "  \"tts_enabled\": false,\n",
            "  \"audio_source\": \"file\",\n",
            "  \"audio_file_path\": \"{}\"\n",
            "}}\n"
        ),
        fixture.display().to_string().replace('\\', "\\\\")
    );
    std::fs::write(&cfg_path, payload).map_err(|e| format!("write PTY config: {e}"))?;
    Ok(cfg_path)
}

fn should_inject_fixture_config(extra_env: &[(&str, &str)]) -> bool {
    let has_explicit_config = extra_env
        .iter()
        .any(|(key, _)| *key == "TUI_TRANSLATOR_CONFIG");
    let disables_onboarding = extra_env
        .iter()
        .any(|(key, value)| *key == "TUI_TRANSLATOR_SKIP_ONBOARDING" && *value == "0");
    !has_explicit_config && !disables_onboarding
}

impl Drop for PtySession {
    /// Kill the child process on drop so test failures never leave orphan
    /// processes running in the background.
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
