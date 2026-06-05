// IMPORTANT: This file is include-d from playback.rs inside mod imp.
// Do NOT add inner-doc //! comments or use statements here — they conflict
// with the parent module imports. Types are reachable directly from imp scope.

/// Event loop for virtual-mic routes.
///
/// `primary_sink` is `Some(sink)` when the device is available at startup, or
/// `None` when it was already unavailable.  On the first
/// [`SubsystemHealth::Failed`] event from `health_rx`, the loop falls back to
/// the system-default speaker and emits a single [`tracing::warn!`].
/// Subsequent `Failed` events are silently ignored (idempotent).
fn run_vmic_loop(
    rx: mpsc::Receiver<PlaybackCmd>,
    enabled: Arc<AtomicBool>,
    primary_sink: Option<Box<dyn AudioSink>>,
    mut health_rx: tokio::sync::watch::Receiver<SubsystemHealth>,
    device: &str,
) {
    /// Active routing target for this loop iteration.
    enum Active {
        Vmic(Box<dyn AudioSink>),
        Fallback(RodioSink),
        /// Device was lost and speaker fallback also failed.
        Lost,
    }

    let mut active = match primary_sink {
        Some(s) => Active::Vmic(s),
        // Sink unavailable at startup: try speaker fallback immediately.
        None => match RodioSink::try_new(None) {
            Ok(r) => Active::Fallback(r),
            Err(e) => {
                tracing::warn!(
                    device = %device,
                    error = %e,
                    "tts-playback: vmic unavailable and speaker fallback also failed; \
                     audio disabled"
                );
                Active::Lost
            }
        },
    };

    // Set to `true` after the first device-loss warn to prevent repeats.
    let mut warned_fallback = false;

    loop {
        // Non-blocking poll for device-loss events.
        if health_rx.has_changed().unwrap_or(false) {
            let health = health_rx.borrow_and_update().clone();
            if matches!(health, SubsystemHealth::Failed { .. }) && !warned_fallback {
                warned_fallback = true;
                tracing::warn!(
                    device = %device,
                    "tts-playback: virtual-mic device lost; falling back to system speaker"
                );
                active = match RodioSink::try_new(None) {
                    Ok(r) => Active::Fallback(r),
                    Err(e) => {
                        tracing::warn!(
                            device = %device,
                            error = %e,
                            "tts-playback: speaker fallback also failed; audio disabled"
                        );
                        Active::Lost
                    }
                };
            }
        }

        let cmd = match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(cmd) => cmd,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                tracing::info!("tts-playback: stopping");
                return;
            }
        };

        match cmd {
            PlaybackCmd::Play(bytes) => {
                if !enabled.load(Ordering::SeqCst) {
                    continue;
                }
                match &active {
                    Active::Vmic(s) => s.play_bytes(bytes),
                    Active::Fallback(r) => r.play_bytes(bytes),
                    Active::Lost => {}
                }
            }
            PlaybackCmd::Shutdown => {
                tracing::info!("tts-playback: stopping");
                return;
            }
        }
    }
}