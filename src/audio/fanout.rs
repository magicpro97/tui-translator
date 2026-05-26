//! Bounded mpsc fanout for the audio-capture output (DM-02, issue #378).
//!
//! # Design
//!
//! After capture, the raw [`AudioChunk`] stream is split into **two independent
//! bounded slots** (A and B) so that a slow downstream consumer on one slot can
//! never stall the audio thread or starve the other slot.
//!
//! ```text
//!  ┌─────────────────┐   mpsc::Receiver<AudioChunk>
//!  │  audio capture  │─────────────────────────────────────────────────────┐
//!  └─────────────────┘                                                     │
//!                                                                          ▼
//!                                                              ┌──────────────────┐
//!                                                              │  fanout task     │
//!                                                              │  (tokio::spawn)  │
//!                                                              └──────┬──────┬───┘
//!                                              try_send ──────────────┘      └─── try_send
//!                                                    │                             │
//!                                        bounded(64) │                             │ bounded(64)
//!                                                    ▼                             ▼
//!                                             slot A receiver               slot B receiver
//!                                            (e.g. STT pipeline)         (e.g. archive / soak)
//! ```
//!
//! # Back-pressure policy
//!
//! The fanout task uses [`try_send`](tokio::sync::mpsc::Sender::try_send) for
//! each slot.  When a slot's queue is full the chunk is **dropped for that slot
//! only** and the corresponding [`FanoutDropCounters`] entry is incremented
//! atomically.  The other slot is unaffected.
//!
//! The audio-capture OS thread never awaits and never blocks on downstream
//! consumers.  There are **no broadcast channels** and **no unbounded channels**
//! in the data path.
//!
//! # Slot index
//!
//! Slot A is index 0; slot B is index 1.  The constants [`SLOT_A`] and [`SLOT_B`]
//! are provided for readability.  Future phases (DM-03 and beyond) may attach
//! additional consumers to the receivers stored in [`FanoutHandle`].

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use tokio::sync::mpsc;

use crate::audio::AudioChunk;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Number of [`AudioChunk`]s buffered per fanout slot before back-pressure
/// drops begin.  At 10 ms per chunk this is 640 ms of headroom per slot.
pub const FANOUT_SLOT_CAPACITY: usize = 64;

/// Slot index for the primary consumer (STT pipeline in v1).
pub const SLOT_A: usize = 0;
/// Slot index for the secondary consumer (archive / soak / DM-03).
pub const SLOT_B: usize = 1;

// ── FanoutDropCounters ────────────────────────────────────────────────────────

/// Atomic per-slot drop counters.
///
/// Shared via `Arc` between the fanout task and its callers.  All operations
/// use [`Ordering::Relaxed`] — monotonic counters that are only ever read for
/// diagnostics and tests; happens-before ordering is not required.
#[derive(Debug, Default)]
pub struct FanoutDropCounters {
    drops: [AtomicU64; 2],
}

impl FanoutDropCounters {
    /// Create a zeroed counter set wrapped in an `Arc`.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Increment the drop counter for `slot`.
    ///
    /// Silently no-ops when `slot >= 2`.
    #[inline]
    pub fn increment(&self, slot: usize) {
        if let Some(c) = self.drops.get(slot) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Return the number of drops recorded for `slot`.
    pub fn drops(&self, slot: usize) -> u64 {
        self.drops
            .get(slot)
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Return `true` when both slots have recorded zero drops.
    pub fn all_zero(&self) -> bool {
        self.drops[0].load(Ordering::Relaxed) == 0 && self.drops[1].load(Ordering::Relaxed) == 0
    }
}

// ── FanoutHandle ──────────────────────────────────────────────────────────────

/// Receivers and shared counters returned by [`start_fanout`].
///
/// Downstream consumers receive their chunk stream from `slot_a` and `slot_b`
/// respectively.  The `counters` field gives read access to the per-slot drop
/// totals for metrics and testing.
pub struct FanoutHandle {
    /// Receiver for slot A (primary consumer).
    pub slot_a: mpsc::Receiver<AudioChunk>,
    /// Receiver for slot B (secondary consumer).
    pub slot_b: mpsc::Receiver<AudioChunk>,
    /// Shared atomic drop counters; one entry per slot.
    pub counters: Arc<FanoutDropCounters>,
}

// ── start_fanout ──────────────────────────────────────────────────────────────

/// Attach a dual-slot bounded fanout to an existing [`AudioChunk`] stream.
///
/// Spawns a Tokio task that drains `source` and fans each chunk out to two
/// independent bounded queues (capacity [`FANOUT_SLOT_CAPACITY`] each).
/// Returns a [`FanoutHandle`] whose receivers and counters are ready for the
/// caller to use.
///
/// # Behaviour
///
/// * Each chunk is cloned once only when both slots are still open (small heap
///   copy: about 320 bytes for a 10 ms, 16 kHz mono chunk).
/// * Delivery to each slot is attempted with `try_send`.  A full slot
///   causes a drop **for that slot only**; the other slot is unaffected.
/// * When `source` is closed (capture task exited) the fanout task exits and
///   both slot senders are dropped, which signals EOF to downstream receivers.
///
/// # Panics
///
/// Never panics — all channel errors are handled gracefully.
pub fn start_fanout(source: mpsc::Receiver<AudioChunk>) -> FanoutHandle {
    let (tx_a, rx_a) = mpsc::channel::<AudioChunk>(FANOUT_SLOT_CAPACITY);
    let (tx_b, rx_b) = mpsc::channel::<AudioChunk>(FANOUT_SLOT_CAPACITY);
    let counters = FanoutDropCounters::new();
    let counters_task = Arc::clone(&counters);

    tokio::spawn(async move {
        fanout_loop(source, tx_a, tx_b, counters_task).await;
    });

    FanoutHandle {
        slot_a: rx_a,
        slot_b: rx_b,
        counters,
    }
}

// ── Internal fanout loop ──────────────────────────────────────────────────────

/// Core fanout loop.  Extracted to a named `async fn` for testability.
///
/// Reads chunks from `source` and attempts `try_send` to each slot.
/// Exits when `source` is closed (returns `None`).
pub(crate) async fn fanout_loop(
    mut source: mpsc::Receiver<AudioChunk>,
    tx_a: mpsc::Sender<AudioChunk>,
    tx_b: mpsc::Sender<AudioChunk>,
    counters: Arc<FanoutDropCounters>,
) {
    let mut tx_a = Some(tx_a);
    let mut tx_b = Some(tx_b);

    while let Some(chunk) = source.recv().await {
        match (tx_a.is_some(), tx_b.is_some()) {
            (true, true) => {
                try_send_slot(&mut tx_a, chunk.clone(), SLOT_A, "A", &counters);
                try_send_slot(&mut tx_b, chunk, SLOT_B, "B", &counters);
            }
            (true, false) => try_send_slot(&mut tx_a, chunk, SLOT_A, "A", &counters),
            (false, true) => try_send_slot(&mut tx_b, chunk, SLOT_B, "B", &counters),
            (false, false) => {}
        }
    }

    tracing::debug!("fanout: source closed — fanout task exiting");
}

fn try_send_slot(
    tx: &mut Option<mpsc::Sender<AudioChunk>>,
    chunk: AudioChunk,
    slot: usize,
    label: &str,
    counters: &FanoutDropCounters,
) {
    let result = match tx.as_ref() {
        Some(sender) => sender.try_send(chunk),
        None => return,
    };
    match result {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            tracing::debug!("fanout: slot {label} full — dropping chunk");
            counters.increment(slot);
            // QA8-07 (#505): mirror fanout drops into the global
            // backpressure telemetry so the QA8-05 runner reads a
            // single object. No-op when telemetry is not installed.
            super::backpressure_hook::fanout_drop(slot);
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            tracing::debug!("fanout: slot {label} closed permanently");
            *tx = None;
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    fn make_chunk(n: usize) -> AudioChunk {
        // 10 ms at 16 kHz = 160 samples
        AudioChunk::new(vec![n as i16; 160])
    }

    // ── Counter tests ─────────────────────────────────────────────────────────

    #[test]
    fn drop_counters_start_at_zero() {
        let c = FanoutDropCounters::new();
        assert_eq!(c.drops(SLOT_A), 0);
        assert_eq!(c.drops(SLOT_B), 0);
        assert!(c.all_zero());
    }

    #[test]
    fn increment_only_affects_target_slot() {
        let c = FanoutDropCounters::new();
        c.increment(SLOT_A);
        c.increment(SLOT_A);
        c.increment(SLOT_B);
        assert_eq!(c.drops(SLOT_A), 2);
        assert_eq!(c.drops(SLOT_B), 1);
        assert!(!c.all_zero());
    }

    #[test]
    fn out_of_range_slot_does_not_panic() {
        let c = FanoutDropCounters::new();
        // slot index 99 must silently no-op, never panic.
        c.increment(99);
        assert_eq!(c.drops(SLOT_A), 0);
        assert_eq!(c.drops(SLOT_B), 0);
    }

    // ── Core isolation tests (deterministic — uses fanout_loop directly) ───────
    //
    // These tests call `fanout_loop` directly with hand-crafted channels so
    // that no concurrent tasks or timing windows are required.  Isolation is
    // proved by giving the *active* slot a capacity large enough that it never
    // fills, while the *stalled* slot has the spec capacity (FANOUT_SLOT_CAPACITY).
    // This is the authoritative DM-02 unit evidence.

    /// DM-02 core acceptance criterion:
    /// When slot B is completely stalled (never drained), try_send to slot B
    /// fails after 64 items and the drop counter for B is incremented.  Slot A
    /// (given enough capacity) receives every chunk without drops.
    #[tokio::test]
    async fn stalled_slot_b_does_not_drop_slot_a() {
        // Slot A: large capacity so it never fills (no timing dependency).
        // Slot B: spec capacity — stalled receiver means it fills and drops.
        let large_cap = FANOUT_SLOT_CAPACITY * 4;
        let (source_tx, source_rx) = mpsc::channel::<AudioChunk>(large_cap);
        let (tx_a, mut rx_a) = mpsc::channel::<AudioChunk>(large_cap);
        let (tx_b, _rx_b_stalled) = mpsc::channel::<AudioChunk>(FANOUT_SLOT_CAPACITY);
        let counters = Arc::new(FanoutDropCounters::default());

        // Pre-fill source with enough chunks to overflow slot B.
        let total = FANOUT_SLOT_CAPACITY + 32; // 96 chunks
        for i in 0..total {
            source_tx
                .try_send(make_chunk(i))
                .expect("source must accept chunk");
        }
        drop(source_tx); // signal EOF so fanout_loop exits

        // Run the fanout loop synchronously — no concurrent tasks required.
        fanout_loop(source_rx, tx_a, tx_b, Arc::clone(&counters)).await;

        // Drain whatever slot A received.
        let mut received_a = 0usize;
        while rx_a.try_recv().is_ok() {
            received_a += 1;
        }

        // Slot A must have zero drops (large cap means try_send always succeeds).
        assert_eq!(
            counters.drops(SLOT_A),
            0,
            "slot A must not drop any chunks when only slot B is stalled"
        );
        // Slot B must have dropped the overflow (total − capacity).
        let expected_b_drops = (total - FANOUT_SLOT_CAPACITY) as u64;
        assert_eq!(
            counters.drops(SLOT_B),
            expected_b_drops,
            "slot B must drop exactly {expected_b_drops} chunks"
        );
        // Slot A received every chunk.
        assert_eq!(received_a, total, "slot A must receive all {total} chunks");
    }

    /// Mirror of the above: stalling slot A must not drop slot B.
    #[tokio::test]
    async fn stalled_slot_a_does_not_drop_slot_b() {
        let large_cap = FANOUT_SLOT_CAPACITY * 4;
        let (source_tx, source_rx) = mpsc::channel::<AudioChunk>(large_cap);
        let (tx_a, _rx_a_stalled) = mpsc::channel::<AudioChunk>(FANOUT_SLOT_CAPACITY);
        let (tx_b, mut rx_b) = mpsc::channel::<AudioChunk>(large_cap);
        let counters = Arc::new(FanoutDropCounters::default());

        let total = FANOUT_SLOT_CAPACITY + 32;
        for i in 0..total {
            source_tx
                .try_send(make_chunk(i))
                .expect("source must accept chunk");
        }
        drop(source_tx);

        fanout_loop(source_rx, tx_a, tx_b, Arc::clone(&counters)).await;

        let mut received_b = 0usize;
        while rx_b.try_recv().is_ok() {
            received_b += 1;
        }

        assert_eq!(counters.drops(SLOT_B), 0, "slot B must have zero drops");
        let expected_a_drops = (total - FANOUT_SLOT_CAPACITY) as u64;
        assert_eq!(counters.drops(SLOT_A), expected_a_drops);
        assert_eq!(received_b, total);
    }

    #[tokio::test]
    async fn closed_slot_b_does_not_drop_or_block_slot_a() {
        let large_cap = FANOUT_SLOT_CAPACITY * 4;
        let (source_tx, source_rx) = mpsc::channel::<AudioChunk>(large_cap);
        let (tx_a, mut rx_a) = mpsc::channel::<AudioChunk>(large_cap);
        let (tx_b, rx_b) = mpsc::channel::<AudioChunk>(FANOUT_SLOT_CAPACITY);
        drop(rx_b);
        let counters = Arc::new(FanoutDropCounters::default());

        let total = FANOUT_SLOT_CAPACITY + 32;
        for i in 0..total {
            source_tx
                .try_send(make_chunk(i))
                .expect("source must accept chunk");
        }
        drop(source_tx);

        fanout_loop(source_rx, tx_a, tx_b, Arc::clone(&counters)).await;

        let mut received_a = 0usize;
        while rx_a.try_recv().is_ok() {
            received_a += 1;
        }

        assert_eq!(counters.drops(SLOT_A), 0);
        assert_eq!(
            counters.drops(SLOT_B),
            0,
            "closed slot B must not be counted as backpressure drops"
        );
        assert_eq!(received_a, total);
    }

    // ── Both slots draining: no drops ─────────────────────────────────────────

    /// When both slot receivers are drained and the total send count is below
    /// the slot capacity, no drops are expected on either slot.
    #[tokio::test]
    async fn both_slots_draining_no_drops() {
        let (source_tx, source_rx) = mpsc::channel::<AudioChunk>(128);
        let handle = start_fanout(source_rx);

        let counters = Arc::clone(&handle.counters);
        let mut rx_a = handle.slot_a;
        let mut rx_b = handle.slot_b;

        // Send fewer than the slot capacity so neither queue fills.
        let total_chunks = FANOUT_SLOT_CAPACITY / 2;
        for i in 0..total_chunks {
            source_tx.send(make_chunk(i)).await.unwrap();
        }

        sleep(Duration::from_millis(50)).await;

        let mut received_a = 0usize;
        while rx_a.try_recv().is_ok() {
            received_a += 1;
        }
        let mut received_b = 0usize;
        while rx_b.try_recv().is_ok() {
            received_b += 1;
        }

        assert!(
            counters.all_zero(),
            "no drops expected when both slots drain"
        );
        assert_eq!(received_a, total_chunks);
        assert_eq!(received_b, total_chunks);
    }

    // ── Source EOF propagates to both slot receivers ──────────────────────────

    #[tokio::test]
    async fn source_eof_closes_both_slots() {
        let (source_tx, source_rx) = mpsc::channel::<AudioChunk>(8);
        let handle = start_fanout(source_rx);
        let mut rx_a = handle.slot_a;
        let mut rx_b = handle.slot_b;

        // Send a couple of chunks then drop the source sender to signal EOF.
        source_tx.send(make_chunk(0)).await.unwrap();
        source_tx.send(make_chunk(1)).await.unwrap();
        drop(source_tx);

        // Give the fanout task time to drain and exit.
        sleep(Duration::from_millis(50)).await;

        // Drain whatever arrived.
        while rx_a.try_recv().is_ok() {}
        while rx_b.try_recv().is_ok() {}

        // Both channels should now report EOF (recv returns None).
        assert!(
            rx_a.recv().await.is_none(),
            "slot A should be closed after source EOF"
        );
        assert!(
            rx_b.recv().await.is_none(),
            "slot B should be closed after source EOF"
        );
    }

    // ── Soak/sim: artificial latency on slot B ────────────────────────────────

    /// Acceptance soak evidence (DM-02):
    ///
    /// Simulates a burst of 200 audio chunks from the capture thread with
    /// a stalled slow consumer on slot B (simulating artificial latency that
    /// causes the slot B queue to overflow).
    ///
    /// Slot A is given a large capacity (fast consumer model), so it receives
    /// every chunk.  Slot B overflows by `SEND_COUNT − FANOUT_SLOT_CAPACITY`
    /// chunks.
    ///
    /// Expected outcome:
    /// * `drops(SLOT_A) == 0`
    /// * `drops(SLOT_B) == SEND_COUNT − FANOUT_SLOT_CAPACITY`
    /// * Slot A received all `SEND_COUNT` chunks.
    ///
    /// The test calls `fanout_loop` directly for full determinism — no
    /// concurrent tasks, no timing windows, no scheduler dependency.
    #[tokio::test]
    async fn soak_artificial_latency_on_slot_b() {
        const SEND_COUNT: usize = 200;

        // Slot A: large capacity (fast consumer, absorbs all chunks).
        // Slot B: spec capacity (64) — stalled receiver models the slow/high-latency
        //         consumer that cannot drain fast enough.
        let large_cap = SEND_COUNT + 64;
        let (source_tx, source_rx) = mpsc::channel::<AudioChunk>(large_cap);
        let (tx_a, mut rx_a) = mpsc::channel::<AudioChunk>(large_cap);
        let (tx_b, _rx_b_slow) = mpsc::channel::<AudioChunk>(FANOUT_SLOT_CAPACITY);
        let counters = Arc::new(FanoutDropCounters::default());

        // Fill source with SEND_COUNT chunks (simulates burst from capture thread).
        for i in 0..SEND_COUNT {
            source_tx
                .try_send(make_chunk(i))
                .expect("source must accept chunk");
        }
        drop(source_tx);

        // Run fanout loop — deterministic, no race conditions.
        fanout_loop(source_rx, tx_a, tx_b, Arc::clone(&counters)).await;

        // Count what slot A received.
        let mut received_a = 0usize;
        while rx_a.try_recv().is_ok() {
            received_a += 1;
        }

        // Slot A drops == 0 (fast consumer, large cap).
        assert_eq!(
            counters.drops(SLOT_A),
            0,
            "slot A drops must be 0 under artificial latency on slot B"
        );
        // Slot B drops exactly the overflow count.
        let expected_b_drops = (SEND_COUNT - FANOUT_SLOT_CAPACITY) as u64;
        assert_eq!(
            counters.drops(SLOT_B),
            expected_b_drops,
            "slot B must drop exactly {expected_b_drops} chunks, got {}",
            counters.drops(SLOT_B)
        );
        // Slot A received every chunk.
        assert_eq!(
            received_a, SEND_COUNT,
            "fast slot A must receive all {SEND_COUNT} chunks, got {received_a}"
        );
    }
}
