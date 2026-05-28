//! Non-blocking session recorder handle: public API for queueing transcript
//! segments and lifecycle commands. The background writer task and low-level
//! file/path helpers live in [`super::recorder_writer`].
//!
//! Extracted from `session/mod.rs` for STD-02 LOC compliance (#484).
//! Public API and behavior are unchanged; symbols are re-exported through
//! `crate::session`.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use thiserror::Error;
use tokio::{
    fs::{self, OpenOptions},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use super::recorder_writer::{
    is_valid_path_component, prune_session_dirs, run_writer, sanitize_session_id_for_fs,
    segment_file_name, validate_slot_suffix, write_record_line, WriterMessage, WriterRuntime,
    RECORDER_QUEUE_CAPACITY,
};
use super::{SessionHeader, SessionLogRecord, TranscriptSegment};

/// Runtime configuration for transcript session recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecorderConfig {
    /// Whether transcript JSONL recording is enabled.
    pub enabled: bool,
    /// Parent directory under which per-session subdirectories
    /// (`<directory>/<session-id>/<segment>.jsonl`) are written.
    pub directory: PathBuf,
    /// Maximum number of session directories to keep, including the new one.
    pub max_sessions: Option<usize>,
    /// LF-06 per-session byte cap.  When the active segment file exceeds this
    /// many bytes, the writer task seals it and starts a new segment under
    /// the same session directory.  `0` disables segment rollover.
    pub per_session_bytes_cap: u64,
    /// Optional slot suffix appended to segment file names for dual-slot sessions
    /// (e.g. `"a"` → `00001-a.jsonl`, `"b"` → `00001-b.jsonl`).
    /// `None` (default) produces standard `00001.jsonl` segments.
    pub slot_suffix: Option<String>,
}

impl SessionRecorderConfig {
    /// Create an enabled recorder config using `directory`.
    pub fn enabled(directory: impl Into<PathBuf>) -> Self {
        Self {
            enabled: true,
            directory: directory.into(),
            max_sessions: None,
            per_session_bytes_cap: 0,
            slot_suffix: None,
        }
    }

    /// Create an enabled recorder config and cap retained sessions.
    pub fn enabled_with_max_sessions(directory: impl Into<PathBuf>, max_sessions: usize) -> Self {
        Self {
            enabled: true,
            directory: directory.into(),
            max_sessions: Some(max_sessions),
            per_session_bytes_cap: 0,
            slot_suffix: None,
        }
    }

    /// Builder: set the per-session byte cap for segment rollover.
    pub fn with_per_session_bytes_cap(mut self, cap: u64) -> Self {
        self.per_session_bytes_cap = cap;
        self
    }

    /// Builder: set the slot suffix for dual-slot sessions.
    ///
    /// Suffix must be 1–8 ASCII alphanumeric characters (e.g. `"a"`, `"b"`).
    /// Produces segment files like `00001-a.jsonl` instead of `00001.jsonl`.
    ///
    /// # Errors
    ///
    /// Returns an error if the suffix contains path-traversal characters or
    /// is otherwise unsafe to use as a file-name component.
    pub fn with_slot_suffix(mut self, suffix: impl Into<String>) -> anyhow::Result<Self> {
        let suffix = suffix.into();
        validate_slot_suffix(&suffix)?;
        self.slot_suffix = Some(suffix);
        Ok(self)
    }

    /// Create a disabled recorder config. The directory is kept for tests and diagnostics.
    pub fn disabled(directory: impl Into<PathBuf>) -> Self {
        Self {
            enabled: false,
            directory: directory.into(),
            max_sessions: None,
            per_session_bytes_cap: 0,
            slot_suffix: None,
        }
    }
}

/// Non-blocking transcript JSONL recorder.
///
/// The hot path calls [`record_segment`](Self::record_segment), which uses
/// `try_send` into a bounded channel. Disk I/O runs on a Tokio task and writes
/// each line through the Tokio file handle; this is not an fsync durability
/// guarantee.
///
/// HC-03: [`seal_and_reopen`](Self::seal_and_reopen) flushes the current JSONL
/// file and opens a new one at a different path without consuming `self` — safe
/// to call while `OrchestratorContext` owns the recorder by value.
pub struct SessionRecorder {
    session_id: Option<String>,
    session_dir: Option<PathBuf>,
    active_path: Arc<Mutex<Option<PathBuf>>>,
    sender: Option<mpsc::Sender<WriterMessage>>,
    writer: Option<JoinHandle<()>>,
    last_error: Arc<Mutex<Option<String>>>,
    /// Monotonically non-decreasing count of bytes successfully handed to the
    /// OS for the JSONL file since the recorder was started (including the
    /// header line). Zero when the recorder is disabled.
    bytes_written: Arc<AtomicU64>,
}

impl SessionRecorder {
    /// Return a recorder that never creates files and ignores segments.
    pub fn disabled() -> Self {
        Self {
            session_id: None,
            session_dir: None,
            active_path: Arc::new(Mutex::new(None)),
            sender: None,
            writer: None,
            last_error: Arc::new(Mutex::new(None)),
            bytes_written: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Start a recorder and write the header line before returning.
    pub async fn start(
        config: SessionRecorderConfig,
        header: SessionHeader,
    ) -> anyhow::Result<Self> {
        if !config.enabled {
            return Ok(Self::disabled());
        }

        fs::create_dir_all(&config.directory).await.map_err(|err| {
            anyhow::anyhow!(
                "failed to create session directory {}: {err}",
                config.directory.display()
            )
        })?;
        if let Some(max_sessions) = config.max_sessions {
            prune_session_dirs(&config.directory, max_sessions)?;
        }

        // LF-06: validate the session-id as a path component; fall back to the
        // legacy filesystem sanitizer when the caller supplied a string with
        // non-component characters so existing flows keep working.
        let session_subdir_name = if is_valid_path_component(&header.session_id) {
            header.session_id.clone()
        } else {
            sanitize_session_id_for_fs(&header.session_id)
        };

        let session_dir = config.directory.join(&session_subdir_name);
        fs::create_dir_all(&session_dir).await.map_err(|err| {
            anyhow::anyhow!(
                "failed to create per-session directory {}: {err}",
                session_dir.display()
            )
        })?;

        let path = session_dir.join(segment_file_name(1, config.slot_suffix.as_deref()));
        let session_id = header.session_id.clone();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map_err(|err| {
                anyhow::anyhow!("failed to create session log {}: {err}", path.display())
            })?;

        let header_record = SessionLogRecord::SessionHeader(header);
        let header_byte_count =
            write_record_line(&mut file, &header_record)
                .await
                .map_err(|err| {
                    anyhow::anyhow!("failed to write session header {}: {err}", path.display())
                })?;

        let bytes_written = Arc::new(AtomicU64::new(header_byte_count as u64));
        let active_path = Arc::new(Mutex::new(Some(path.clone())));

        let (tx, rx) = mpsc::channel(RECORDER_QUEUE_CAPACITY);
        let last_error = Arc::new(Mutex::new(None));
        let writer = tokio::spawn(run_writer(
            file,
            rx,
            WriterRuntime {
                last_error: Arc::clone(&last_error),
                session_dir: session_dir.clone(),
                active_path: Arc::clone(&active_path),
                initial_segment_bytes: header_byte_count as u64,
                per_session_bytes_cap: config.per_session_bytes_cap,
                bytes_written: Arc::clone(&bytes_written),
                slot_suffix: config.slot_suffix,
            },
        ));

        Ok(Self {
            session_id: Some(session_id),
            session_dir: Some(session_dir),
            active_path,
            sender: Some(tx),
            writer: Some(writer),
            last_error,
            bytes_written,
        })
    }

    /// Return the active JSONL segment path when recording is enabled.
    pub fn path(&self) -> Option<PathBuf> {
        self.active_path
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Return the per-session directory that holds JSONL segments.
    pub fn session_dir(&self) -> Option<&Path> {
        self.session_dir.as_deref()
    }

    /// Return the active session id when recording is enabled.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Return `true` when this recorder writes transcript files.
    pub fn is_enabled(&self) -> bool {
        self.sender.is_some()
    }

    /// Total bytes successfully handed to the OS for the JSONL file since the
    /// recorder started, including the session header.  Monotonically non-decreasing
    /// within a session.  Returns `0` when the recorder is disabled or before
    /// the first write completes.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }

    /// Shared atomic handle for the bytes-written counter.
    ///
    /// Callers that outlive the [`SessionRecorder`] (e.g. the metrics-publisher
    /// task) should clone this `Arc` before the recorder is moved elsewhere.
    pub fn bytes_written_arc(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.bytes_written)
    }

    /// Queue one final transcript segment without waiting for disk I/O.
    pub fn record_segment(&self, segment: TranscriptSegment) -> Result<(), SessionRecorderError> {
        let Some(sender) = &self.sender else {
            return Ok(());
        };

        if let Some(message) = self.last_error() {
            return Err(SessionRecorderError::WriterStopped(message));
        }

        let segment_id = segment.segment_id;
        match sender.try_send(WriterMessage::Record(SessionLogRecord::TranscriptSegment(
            segment,
        ))) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                Err(SessionRecorderError::QueueFull { segment_id })
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(SessionRecorderError::WriterStopped(
                self.last_error()
                    .unwrap_or_else(|| "session recorder writer stopped".to_string()),
            )),
        }
    }

    /// Return the last asynchronous writer error, if any.
    pub fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Seal the current JSONL segment and open a new one at `new_session_dir`.
    ///
    /// The writer task:
    /// 1. Flushes the active segment file (making it parseable through the last
    ///    line written).
    /// 2. Creates `new_session_dir` if it does not exist.
    /// 3. Opens `new_session_dir/00001.jsonl` (fails with `WriterStopped` when
    ///    that file already exists).
    /// 4. Writes `new_header` as the first JSONL line of the new file.
    /// 5. Returns the new file path on `Ok`.
    ///
    /// The recorder continues to accept [`record_segment`](Self::record_segment)
    /// calls. Records that the writer processes before the seal message stay
    /// in the old file; records submitted after this method returns are written
    /// to the new file. Callers that need strict old/new ordering must await
    /// this method before queueing records for the new session.
    ///
    /// Does nothing and returns `Err(WriterStopped)` when the recorder is
    /// disabled.
    ///
    /// # HC-03 note
    ///
    /// This method does not consume `self`, so it is safe to call while
    /// `OrchestratorContext` owns the recorder by value.
    pub async fn seal_and_reopen(
        &mut self,
        new_session_dir: PathBuf,
        new_header: SessionHeader,
    ) -> Result<PathBuf, SessionRecorderError> {
        let Some(sender) = &self.sender else {
            return Err(SessionRecorderError::WriterStopped(
                "recorder is disabled".to_string(),
            ));
        };
        let new_session_id = new_header.session_id.clone();
        let (done_tx, done_rx) = oneshot::channel();
        sender
            .send(WriterMessage::SealAndReopen {
                new_session_dir: new_session_dir.clone(),
                new_header,
                done_tx,
            })
            .await
            .map_err(|_| {
                SessionRecorderError::WriterStopped("writer channel closed".to_string())
            })?;
        match done_rx.await {
            Ok(Ok(new_path)) => {
                // Update the recorder's session_dir so path() / session_dir()
                // / session_id() reflect the new location.
                self.session_dir = Some(new_session_dir);
                self.session_id = Some(new_session_id);
                Ok(new_path)
            }
            Ok(Err(msg)) => Err(SessionRecorderError::WriterStopped(msg)),
            Err(_) => Err(SessionRecorderError::WriterStopped(
                "writer did not confirm seal".to_string(),
            )),
        }
    }

    /// Drop the sender and wait for the writer task to flush all queued records.
    pub async fn shutdown(mut self) -> Result<(), SessionRecorderError> {
        drop(self.sender.take());
        if let Some(writer) = self.writer.take() {
            writer
                .await
                .map_err(|err| SessionRecorderError::WriterStopped(err.to_string()))?;
        }
        if let Some(message) = self.last_error() {
            Err(SessionRecorderError::WriterStopped(message))
        } else {
            Ok(())
        }
    }
}

/// Errors surfaced synchronously from the non-blocking recorder handle.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionRecorderError {
    /// The bounded writer queue is full; this segment was dropped.
    #[error("session recorder queue is full; dropped transcript segment {segment_id}")]
    QueueFull {
        /// Segment that could not be queued.
        segment_id: u64,
    },
    /// The writer task has stopped, usually after a disk I/O error.
    #[error("session recorder writer stopped: {0}")]
    WriterStopped(String),
}
