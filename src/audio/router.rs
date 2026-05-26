//! HC-03B: CaptureRouter — switchable upstream forwarder (issue #436).
//!
//! # Design
//!
//! The orchestrator holds a single **fixed** `mpsc::Receiver<AudioChunk>` for
//! its entire lifetime.  `CaptureRouter` sits between the active capture stream
//! and the orchestrator as a channel-indirection layer that can be hot-swapped
//! without restarting the orchestrator:
//!
//! ```text
//!  Capture A  ──►  CaptureRouter  ──►  archive writer (if configured)
//!                                 └──►  orchestrator Receiver  (FIXED)
//!
//!  Capture B  ──►  CaptureRouter  (after hot-swap; same downstream channel)
//! ```
//!
//! # Swap protocol
//!
//! 1. Open the new source via `open_source` while keeping the old receiver
//!    attached.  If opening fails, forwarding continues from the old stream.
//! 2. After the new source is ready, drain the old upstream with a
//!    `DRAIN_TIMEOUT_MS` bounded timeout using non-blocking `try_recv`.  Chunks
//!    that cannot be forwarded because the downstream is full are counted in
//!    [`RouterMetrics::dropped_during_swap`].
//! 3. Resume forwarding from the new source; the downstream receiver is unchanged.
//! 4. The optional archive writer remains attached throughout — it receives every
//!    chunk *before* the orchestrator (design requirement).
//!
//! # Borrow-safety note
//!
//! The forwarding loop uses a `poll_action` helper that takes separate `&mut`
//! references to the swap-command receiver and the current audio receiver,
//! encapsulating the `tokio::select!` call.  This pattern ensures all borrows
//! inside the helper are fully released when it returns, allowing the caller to
//! mutate both references freely in the action-processing code.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::Result;
use tokio::sync::{mpsc, oneshot};

use super::{AudioArchiveWriter, AudioChunk, CaptureInfo, CaptureStream};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Capacity of the bounded channel the router writes into (orchestrator side).
///
/// The *receiver* end is passed to `run_orchestrator` and remains stable
/// across all hot-swaps.  64 slots × ~10 ms/chunk ≈ 640 ms headroom.
pub const ROUTER_CHANNEL_CAPACITY: usize = 64;

/// Maximum time (ms) to drain the old capture stream before dropping it.
///
/// Uses non-blocking `try_recv`; chunks not forwarded due to a full downstream
/// are counted as [`RouterMetrics::dropped_during_swap`].
const DRAIN_TIMEOUT_MS: u64 = 200;

// ── CaptureSourceSpec ─────────────────────────────────────────────────────────

/// Describes which capture source to open for a hot-swap operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureSourceSpec {
    /// WASAPI loopback capture.  `None` → system default playback device.
    Wasapi { device: Option<String> },
    /// Loop a WAV fixture file indefinitely (soak / CI / file-replay testing).
    File { path: String },
}

impl CaptureSourceSpec {
    /// Human-readable label used in log messages and status display.
    pub fn label(&self) -> String {
        match self {
            Self::Wasapi { device: None } => "wasapi:default".to_string(),
            Self::Wasapi { device: Some(d) } => format!("wasapi:{d}"),
            Self::File { path } => format!("file:{path}"),
        }
    }
}

// ── RouterState ───────────────────────────────────────────────────────────────

/// Operational state of the [`CaptureRouter`] task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterState {
    /// A capture stream is connected and forwarding audio.
    Active,
    /// No upstream source is connected; the router is idle.
    NoUpstream,
}

// ── RouterMetrics ─────────────────────────────────────────────────────────────

/// Atomic counters exposed by the [`CaptureRouterHandle`].
#[derive(Debug, Default)]
pub struct RouterMetrics {
    /// Chunks dropped because the downstream channel was full during a swap or drain.
    dropped_during_swap: AtomicU64,
    /// Total number of successful hot-swaps performed.
    swap_count: AtomicU64,
}

impl RouterMetrics {
    /// Create a zeroed counter set wrapped in an `Arc`.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Increment `dropped_during_swap` by `n`.
    pub fn record_swap_drops(&self, n: u64) {
        self.dropped_during_swap.fetch_add(n, Ordering::Relaxed);
    }

    /// Increment the successful-swap counter.
    pub fn record_swap(&self) {
        self.swap_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Chunks dropped because the downstream was full during a swap window.
    pub fn dropped_during_swap(&self) -> u64 {
        self.dropped_during_swap.load(Ordering::Relaxed)
    }

    /// Total number of completed hot-swaps.
    pub fn swap_count(&self) -> u64 {
        self.swap_count.load(Ordering::Relaxed)
    }
}

// ── Internal command types ────────────────────────────────────────────────────

/// Internal swap request sent from [`CaptureRouterHandle::hot_swap`] to the task.
pub(crate) struct SwapRequest {
    pub(crate) spec: CaptureSourceSpec,
    pub(crate) silence_threshold: f32,
    pub(crate) reply: oneshot::Sender<Result<CaptureInfo>>,
}

/// Action returned by [`poll_action`] — separates receive from processing so
/// the Rust borrow checker can verify that borrows are fully released before
/// the caller mutates any field.
enum RouterAction {
    Chunk(AudioChunk),
    SwapCmd(SwapRequest),
    UpstreamClosed,
    Shutdown,
}

// ── CaptureRouterHandle ────────────────────────────────────────────────────────

/// External handle for requesting hot-swaps from the [`CaptureRouter`] task.
///
/// Cheaply cloneable; all clones share the same command channel.
#[derive(Clone)]
pub struct CaptureRouterHandle {
    swap_tx: mpsc::Sender<SwapRequest>,
    /// Shared metrics; readable from any clone of the handle.
    pub metrics: Arc<RouterMetrics>,
}

impl CaptureRouterHandle {
    /// Hot-swap the capture source without restarting the orchestrator.
    ///
    /// Sends a swap request to the router task, which:
    /// 1. Opens the new source described by `spec` while the old stream remains active.
    /// 2. Drains the old stream with a bounded timeout after the new source is ready.
    /// 3. Resumes forwarding to the fixed orchestrator receiver.
    ///
    /// Returns the [`CaptureInfo`] of the new source on success.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the router task has stopped or the new source
    /// cannot be opened (device not found, invalid file format, etc.).
    pub async fn hot_swap(
        &self,
        spec: CaptureSourceSpec,
        silence_threshold: f32,
    ) -> Result<CaptureInfo> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.swap_tx
            .send(SwapRequest {
                spec,
                silence_threshold,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("CaptureRouter task has stopped"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("CaptureRouter task stopped before replying"))?
    }

    /// Shared read access to the router's atomic metrics.
    pub fn metrics(&self) -> &Arc<RouterMetrics> {
        &self.metrics
    }
}

// ── CaptureRouter (task-internal struct) ─────────────────────────────────────

/// Internal task state for the router.  Constructed and spawned by [`start_router`].
struct CaptureRouter {
    swap_rx: mpsc::Receiver<SwapRequest>,
    downstream_tx: mpsc::Sender<AudioChunk>,
    state: RouterState,
    metrics: Arc<RouterMetrics>,
    archive: Option<AudioArchiveWriter>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Spawn the capture router task.
///
/// Returns `(handle, orchestrator_rx)`.  Pass `orchestrator_rx` directly to
/// `run_orchestrator`; it remains valid across all subsequent hot-swaps.
///
/// # Parameters
///
/// - `initial_stream`: the first capture stream to forward from.
/// - `silence_threshold`: forwarded to `start_capture*` helpers on every swap.
/// - `archive`: optional WAV archive writer.  Every chunk is written to the
///   archive **before** being forwarded to the orchestrator.
pub fn start_router(
    initial_stream: CaptureStream,
    silence_threshold: f32,
    archive: Option<AudioArchiveWriter>,
) -> (CaptureRouterHandle, mpsc::Receiver<AudioChunk>) {
    let metrics = RouterMetrics::new();
    let (swap_tx, swap_rx) = mpsc::channel::<SwapRequest>(4);
    let (downstream_tx, downstream_rx) = mpsc::channel::<AudioChunk>(ROUTER_CHANNEL_CAPACITY);

    let task = CaptureRouter {
        swap_rx,
        downstream_tx,
        state: RouterState::Active,
        metrics: Arc::clone(&metrics),
        archive,
    };

    // The `current` receiver is passed separately so run_router can keep it
    // as a plain local variable, avoiding borrow-checker conflicts with the
    // rest of the task struct.
    tokio::spawn(run_router(
        task,
        Some(initial_stream.receiver),
        silence_threshold,
    ));

    let handle = CaptureRouterHandle { swap_tx, metrics };
    (handle, downstream_rx)
}

// ── Router run loop ───────────────────────────────────────────────────────────

/// Core forwarding loop.
///
/// `current` is deliberately kept as a plain local variable (not a struct
/// field) so it can be mutably borrowed by [`poll_action`] independently of
/// the rest of `router`.  This avoids borrow-checker conflicts that arise when
/// `select!` holds a `&mut self.field` while the arm body also needs `&mut self`.
async fn run_router(
    mut router: CaptureRouter,
    mut current: Option<mpsc::Receiver<AudioChunk>>,
    _silence_threshold: f32,
) {
    tracing::info!("CaptureRouter: task started");

    loop {
        if router.downstream_tx.is_closed() {
            tracing::info!("CaptureRouter: downstream closed — stopping");
            break;
        }

        // poll_action borrows `router.swap_rx` and `current` independently.
        // When it returns, ALL borrows inside it are fully released, so we
        // can mutate `current` and all router fields freely below.
        let action = poll_action(&mut router.swap_rx, &mut current).await;

        match action {
            RouterAction::Chunk(chunk) => {
                forward_chunk(&mut router.archive, &router.downstream_tx, chunk);
            }

            RouterAction::SwapCmd(req) => {
                let spec_label = req.spec.label();
                tracing::info!(source = %spec_label, "CaptureRouter: swap requested");

                // Open-before-drain preserves the old stream if the target
                // device/file is invalid.  This keeps the app live instead of
                // entering a silent NoUpstream state on failed hot-swap.
                match open_source(&req.spec, req.silence_threshold).await {
                    Ok(new_stream) => {
                        let info = new_stream.info.clone();
                        if let Some(old_rx) = current.take() {
                            drain_old(
                                old_rx,
                                &mut router.archive,
                                &router.downstream_tx,
                                &router.metrics,
                            )
                            .await;
                        }
                        tracing::info!(
                            source = %spec_label,
                            device = %info.device_name,
                            "CaptureRouter: hot-swap complete"
                        );
                        current = Some(new_stream.receiver);
                        router.state = RouterState::Active;
                        router.metrics.record_swap();
                        let _ = req.reply.send(Ok(info));
                    }
                    Err(err) => {
                        tracing::error!(
                            source = %spec_label,
                            error = %err,
                            "CaptureRouter: failed to open new source — preserving existing upstream"
                        );
                        router.state = if current.is_some() {
                            RouterState::Active
                        } else {
                            RouterState::NoUpstream
                        };
                        let _ = req.reply.send(Err(err));
                    }
                }
            }

            RouterAction::UpstreamClosed => {
                tracing::info!("CaptureRouter: upstream closed — NoUpstream");
                router.state = RouterState::NoUpstream;
                current = None;
            }

            RouterAction::Shutdown => {
                tracing::info!("CaptureRouter: handle dropped — draining remaining upstream");
                // The swap handle was dropped, but the upstream stream may still
                // have chunks in flight.  Drain them so no data is lost before
                // stopping.  This is a graceful-shutdown path, not a hot-swap.
                if let Some(mut remaining) = current.take() {
                    while let Some(chunk) = remaining.recv().await {
                        forward_chunk(&mut router.archive, &router.downstream_tx, chunk);
                        if router.downstream_tx.is_closed() {
                            break;
                        }
                    }
                }
                break;
            }
        }
    }

    tracing::info!("CaptureRouter: task stopped");
}

// ── poll_action helper ────────────────────────────────────────────────────────

/// Wait for the next router event: a chunk from the upstream, a swap command,
/// an upstream-closed signal, or a shutdown signal.
///
/// By encapsulating `tokio::select!` in a standalone async function that
/// takes `&mut` references to the two channels, we ensure that every borrow
/// inside this function is **fully released** when the function returns an
/// owned [`RouterAction`].  The caller can then mutate `swap_rx` and `current`
/// freely without any residual borrow conflicts.
async fn poll_action(
    swap_rx: &mut mpsc::Receiver<SwapRequest>,
    current: &mut Option<mpsc::Receiver<AudioChunk>>,
) -> RouterAction {
    if let Some(rx) = current.as_mut() {
        tokio::select! {
            biased; // check swap command before chunks so swaps are never starved

            cmd = swap_rx.recv() => match cmd {
                Some(req) => RouterAction::SwapCmd(req),
                None      => RouterAction::Shutdown,
            },

            chunk = rx.recv() => match chunk {
                Some(c) => RouterAction::Chunk(c),
                None    => RouterAction::UpstreamClosed,
            },
        }
    } else {
        // No upstream — block until a swap command arrives or the handle drops.
        match swap_rx.recv().await {
            Some(req) => RouterAction::SwapCmd(req),
            None => RouterAction::Shutdown,
        }
    }
}

// ── Forward helper ────────────────────────────────────────────────────────────

/// Write `chunk` to the archive writer (if active), then `try_send` to the
/// orchestrator's downstream channel.
///
/// During normal forwarding, downstream backpressure is intentionally not
/// counted as a swap drop; [`drain_old`] records swap-boundary drops explicitly.
/// The archive always receives the chunk **before** the orchestrator
/// (`forward_chunk` enforces this ordering invariant).
#[inline]
fn forward_chunk(
    archive: &mut Option<AudioArchiveWriter>,
    downstream_tx: &mpsc::Sender<AudioChunk>,
    chunk: AudioChunk,
) {
    // Archive first — preserves ordering invariant.
    if let Some(w) = archive.as_mut() {
        if let Err(err) = w.append_chunk(&chunk) {
            tracing::warn!("CaptureRouter: archive write error — disabling archive: {err:#}");
            w.disable();
        }
    }

    // Non-blocking send — never stall the router task.
    let _ = downstream_tx.try_send(chunk);
}

// ── Drain helper ──────────────────────────────────────────────────────────────

/// Drain `old_rx` within [`DRAIN_TIMEOUT_MS`].
///
/// Uses non-blocking `try_recv` to avoid stalling the downstream channel.
/// Chunks not fitting in the downstream are counted as `dropped_during_swap`.
/// Archive writes are performed for every successfully drained chunk to maintain
/// the "archive before orchestrator" invariant across the swap boundary.
async fn drain_old(
    mut old_rx: mpsc::Receiver<AudioChunk>,
    archive: &mut Option<AudioArchiveWriter>,
    downstream_tx: &mpsc::Sender<AudioChunk>,
    metrics: &RouterMetrics,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(DRAIN_TIMEOUT_MS);

    while tokio::time::Instant::now() < deadline {
        match old_rx.try_recv() {
            Ok(chunk) => {
                // Archive-before-orchestrator for drain chunks.
                if let Some(w) = archive.as_mut() {
                    if let Err(err) = w.append_chunk(&chunk) {
                        tracing::warn!("CaptureRouter: archive write error during drain: {err:#}");
                        w.disable();
                    }
                }
                if downstream_tx.try_send(chunk).is_err() {
                    metrics.record_swap_drops(1);
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(mpsc::error::TryRecvError::Disconnected) => break,
        }
    }
    // Drop old_rx here — capture stream is released.
}

// ── Source opener ─────────────────────────────────────────────────────────────

/// Open a [`CaptureStream`] as described by `spec`.
async fn open_source(spec: &CaptureSourceSpec, silence_threshold: f32) -> Result<CaptureStream> {
    match spec {
        CaptureSourceSpec::Wasapi { device } => {
            super::start_capture_with_device(device.as_deref(), silence_threshold).await
        }
        CaptureSourceSpec::File { path } => {
            super::start_file_capture(path, silence_threshold).await
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
