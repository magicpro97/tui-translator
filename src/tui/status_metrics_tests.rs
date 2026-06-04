use super::*;
use anyhow::Result;

fn rendered_buffer_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
    let area = terminal.backend().buffer().area;
    let mut rows = Vec::with_capacity(area.height as usize);
    for y in 0..area.height {
        let row: String = (0..area.width)
            .map(|x| {
                terminal.backend().buffer()[(x, y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' ')
            })
            .collect();
        rows.push(row);
    }
    rows.join("\n")
}

fn render_status_strip(route: TtsRouteStatus, width: u16) -> Result<String> {
    use ratatui::{backend::TestBackend, Terminal};

    let backend = TestBackend::new(width, 3);
    let mut terminal = Terminal::new(backend)?;
    let stt = SttState::Listening;
    terminal.draw(|frame| {
        let strip = StatusMetricsStrip {
            stt: &stt,
            mt: &MtState::default(),
            tts_on: true,
            tts_route: route.clone(),
            target_language: "vi".to_string(),
            pairs: 0,
            audio_secs: 0.0,
            cost_usd: 0.0,
            elapsed: "00:00".to_string(),
            show_restart: false,
            expanded: false,
            cost_warning_usd: 0.0,
            cpu_pct: 0.0,
            ram_bytes: 0,
            net_kbps_tx: 0.0,
            net_kbps_rx: 0.0,
            e2e_latency_ms: None,
            loss_pct: 0.0,
            ram_warning: false,
            truncation_rate: 0.0,
            flicker_count: 0,
            mt_call_count: 0,
            local_cpu_pct: 0.0,
            local_active_threads: 0,
            recorder_bytes: 0,
            recorder_path: None,
            archive_bytes: 0,
            archive_path: None,
            archive_sealed: false,
            audio_consent: false,
            stt_source: SttSource::Local,
            slot_a_tts_status: "ok".to_string(),
            slot_b_tts_status: None,
            config_apply_status: None,
            config_apply_count: 0,
            readiness: crate::readiness::ReadinessState::Ready,
        };
        frame.render_widget(&strip, frame.area());
    })?;
    Ok(rendered_buffer_text(&terminal))
}

fn render_expanded_slot_status(slot_b_tts_status: Option<String>) -> Result<String> {
    use ratatui::{backend::TestBackend, Terminal};

    let backend = TestBackend::new(160, 11);
    let mut terminal = Terminal::new(backend)?;
    let stt = SttState::Listening;
    terminal.draw(|frame| {
        let strip = StatusMetricsStrip {
            stt: &stt,
            mt: &MtState::default(),
            tts_on: slot_b_tts_status.is_some(),
            tts_route: TtsRouteStatus::default(),
            target_language: "vi".to_string(),
            pairs: 0,
            audio_secs: 0.0,
            cost_usd: 0.0,
            elapsed: "00:00".to_string(),
            show_restart: false,
            expanded: true,
            cost_warning_usd: 0.0,
            cpu_pct: 0.0,
            ram_bytes: 0,
            net_kbps_tx: 0.0,
            net_kbps_rx: 0.0,
            e2e_latency_ms: None,
            loss_pct: 0.0,
            ram_warning: false,
            truncation_rate: 0.0,
            flicker_count: 0,
            mt_call_count: 0,
            local_cpu_pct: 0.0,
            local_active_threads: 0,
            recorder_bytes: 0,
            recorder_path: None,
            archive_bytes: 0,
            archive_path: None,
            archive_sealed: false,
            audio_consent: false,
            stt_source: SttSource::Local,
            slot_a_tts_status: "ok".to_string(),
            slot_b_tts_status,
            config_apply_status: None,
            config_apply_count: 0,
            readiness: crate::readiness::ReadinessState::Ready,
        };
        frame.render_widget(&strip, frame.area());
    })?;
    Ok(rendered_buffer_text(&terminal))
}

#[test]
fn status_bar_speakers_indicator() -> Result<()> {
    let rendered = render_status_strip(TtsRouteStatus::default(), 100)?;

    assert!(
        rendered.contains("Route:spk"),
        "speakers route should be visible in compact status strip; got:\n{rendered}"
    );
    Ok(())
}

#[test]
fn status_bar_virtual_mic_indicator() -> Result<()> {
    let rendered = render_status_strip(
        TtsRouteStatus {
            routing: TtsRouting::VirtualMic,
            virtual_mic_device: Some("CABLE Input (VB-Audio Virtual Cable)".to_string()),
        },
        100,
    )?;

    assert!(
        rendered.contains("Route:vmic:CABLE Input"),
        "virtual mic route and device should be visible; got:\n{rendered}"
    );
    Ok(())
}

#[test]
fn status_bar_missing_virtual_mic_warning() -> Result<()> {
    let rendered = render_status_strip(
        TtsRouteStatus {
            routing: TtsRouting::VirtualMic,
            virtual_mic_device: None,
        },
        160,
    )?;

    assert!(
        rendered.contains("missing virtual mic"),
        "missing virtual mic warning should be visible; got:\n{rendered}"
    );
    Ok(())
}

#[test]
fn expanded_metrics_shows_per_slot_tts_status_in_dual_mode() -> Result<()> {
    let rendered = render_expanded_slot_status(Some("degraded: auth timeout".to_string()))?;

    assert!(
        rendered.contains("TTS-A:"),
        "expanded dual-mode strip must contain 'TTS-A:'; got:\n{rendered}"
    );
    assert!(
        rendered.contains("TTS-B:"),
        "expanded dual-mode strip must contain 'TTS-B:'; got:\n{rendered}"
    );
    assert!(
        rendered.contains("degraded"),
        "slot B degraded status must be visible; got:\n{rendered}"
    );
    Ok(())
}

#[test]
fn expanded_metrics_hides_per_slot_tts_status_in_single_mode() -> Result<()> {
    let rendered = render_expanded_slot_status(None)?;

    assert!(
        !rendered.contains("TTS-A:"),
        "single-mode strip must NOT contain per-slot TTS labels; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("TTS-B:"),
        "single-mode strip must NOT contain per-slot TTS labels; got:\n{rendered}"
    );
    Ok(())
}

#[test]
fn app_state_default_tts_status_labels_are_ok() {
    let state = AppState::new();
    let slot_a = state
        .slot_a_tts_status_label
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let slot_b = state
        .slot_b_tts_status_label
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    assert_eq!(
        *slot_a, "ok",
        "slot_a_tts_status_label must default to 'ok'"
    );
    assert_eq!(
        *slot_b, "ok",
        "slot_b_tts_status_label must default to 'ok'"
    );
}

// ── Bug B (#715): long STT error wraps to multi-row, not clipped ─────────

fn render_status_strip_with_stt(
    stt: SttState,
    width: u16,
    height: u16,
    readiness: crate::readiness::ReadinessState,
) -> Result<String> {
    use ratatui::{backend::TestBackend, Terminal};

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| {
        let strip = StatusMetricsStrip {
            stt: &stt,
            mt: &MtState::default(),
            tts_on: true,
            tts_route: TtsRouteStatus::default(),
            target_language: "vi".to_string(),
            pairs: 0,
            audio_secs: 0.0,
            cost_usd: 0.0,
            elapsed: "00:00".to_string(),
            show_restart: false,
            expanded: false,
            cost_warning_usd: 0.0,
            cpu_pct: 0.0,
            ram_bytes: 0,
            net_kbps_tx: 0.0,
            net_kbps_rx: 0.0,
            e2e_latency_ms: None,
            loss_pct: 0.0,
            ram_warning: false,
            truncation_rate: 0.0,
            flicker_count: 0,
            mt_call_count: 0,
            local_cpu_pct: 0.0,
            local_active_threads: 0,
            recorder_bytes: 0,
            recorder_path: None,
            archive_bytes: 0,
            archive_path: None,
            archive_sealed: false,
            audio_consent: false,
            stt_source: SttSource::Local,
            slot_a_tts_status: "ok".to_string(),
            slot_b_tts_status: None,
            config_apply_status: None,
            config_apply_count: 0,
            readiness,
        };
        frame.render_widget(&strip, frame.area());
    })?;
    Ok(rendered_buffer_text(&terminal))
}

#[test]
fn status_strip_renders_long_stt_error_in_full() -> Result<()> {
    // #715: a long STT error must wrap to subsequent rows, not be clipped.
    let err = SttState::Error(
        "Some saved provider settings are not available in this build \
         (mt_provider=\"llm\"). Set unsupported providers to \"google\", save, and restart."
            .to_string(),
    );
    let text = render_status_strip_with_stt(err, 100, 6, crate::readiness::ReadinessState::Ready)?;
    assert!(
        text.contains("Some saved"),
        "head of error should render; got:\n{text}"
    );
    assert!(
        text.contains("restart"),
        "tail of error must appear on a wrapped row; got:\n{text}"
    );
    Ok(())
}

#[test]
fn status_strip_short_stt_error_stays_within_normal_height() -> Result<()> {
    let err = SttState::Error("short".to_string());
    let text = render_status_strip_with_stt(err, 100, 3, crate::readiness::ReadinessState::Ready)?;
    assert!(text.contains("short"), "got:\n{text}");
    Ok(())
}

#[test]
fn format_stt_span_preserves_full_error_text() {
    let long = "x".repeat(500);
    let span = format_stt_span(&SttState::Error(long.clone()), SttSource::Local);
    assert!(
        span.ends_with(&long),
        "format_stt_span must not pre-truncate; got len {}",
        span.len()
    );
}

#[test]
fn status_strip_expanded_height_grows_with_long_error() {
    let err = SttState::Error("x".repeat(250));
    let no_err = SttState::Idle;
    let with_err_strip = strip_for_height_check(&err);
    let no_err_strip = strip_for_height_check(&no_err);
    let h_err = with_err_strip.expanded_height(80);
    let h_ok = no_err_strip.expanded_height(80);
    assert!(
        h_err > h_ok,
        "expanded_height should grow when STT is in error (err={h_err}, ok={h_ok})",
    );
}

#[test]
fn status_strip_compact_height_grows_with_long_error() {
    let err = SttState::Error("x".repeat(250));
    let no_err = SttState::Idle;
    let h_err = strip_for_height_check(&err).compact_height(80);
    let h_ok = strip_for_height_check(&no_err).compact_height(80);
    assert!(
        h_err > h_ok,
        "compact_height should grow when STT is in error (err={h_err}, ok={h_ok})",
    );
}

#[test]
fn format_stt_error_lines_caps_at_max_rows() {
    let huge = "x ".repeat(2_000); // many words
    let lines = format_stt_error_lines(&huge, 20);
    assert!(
        lines.len() <= super::STT_ERROR_MAX_WRAPPED_ROWS,
        "got {} lines",
        lines.len()
    );
}

fn strip_for_height_check(stt: &SttState) -> StatusMetricsStrip<'_> {
    StatusMetricsStrip {
        stt,
        mt: Box::leak(Box::new(MtState::default())),
        tts_on: false,
        tts_route: TtsRouteStatus::default(),
        target_language: "vi".to_string(),
        pairs: 0,
        audio_secs: 0.0,
        cost_usd: 0.0,
        elapsed: "0".to_string(),
        show_restart: false,
        expanded: true,
        cost_warning_usd: 0.0,
        cpu_pct: 0.0,
        ram_bytes: 0,
        net_kbps_tx: 0.0,
        net_kbps_rx: 0.0,
        e2e_latency_ms: None,
        loss_pct: 0.0,
        ram_warning: false,
        truncation_rate: 0.0,
        flicker_count: 0,
        mt_call_count: 0,
        local_cpu_pct: 0.0,
        local_active_threads: 0,
        recorder_bytes: 0,
        recorder_path: None,
        archive_bytes: 0,
        archive_path: None,
        archive_sealed: false,
        audio_consent: false,
        stt_source: SttSource::Local,
        slot_a_tts_status: "ok".to_string(),
        slot_b_tts_status: None,
        config_apply_status: None,
        config_apply_count: 0,
        readiness: crate::readiness::ReadinessState::Ready,
    }
}

// ── Bug C (#716): readiness badge appears on the strip ───────────────────

#[test]
fn status_strip_renders_init_badge() -> Result<()> {
    let text = render_status_strip_with_stt(
        SttState::Idle,
        100,
        3,
        crate::readiness::ReadinessState::Init,
    )?;
    assert!(text.contains("INIT"), "expected INIT badge; got:\n{text}");
    Ok(())
}

#[test]
fn status_strip_renders_load_badge() -> Result<()> {
    let text = render_status_strip_with_stt(
        SttState::Idle,
        140,
        3,
        crate::readiness::ReadinessState::Loading {
            component: "llm-mt",
            percent: Some(42),
        },
    )?;
    assert!(text.contains("LOAD"), "expected LOAD badge; got:\n{text}");
    Ok(())
}

#[test]
fn status_strip_renders_ready_badge() -> Result<()> {
    let text = render_status_strip_with_stt(
        SttState::Idle,
        100,
        3,
        crate::readiness::ReadinessState::Ready,
    )?;
    assert!(text.contains("READY"), "expected READY badge; got:\n{text}");
    Ok(())
}

#[test]
fn status_strip_renders_error_badge() -> Result<()> {
    let text = render_status_strip_with_stt(
        SttState::Idle,
        100,
        3,
        crate::readiness::ReadinessState::Error("boom".to_string()),
    )?;
    assert!(text.contains("ERROR"), "expected ERROR badge; got:\n{text}");
    Ok(())
}

// Combined Bug B + Bug C: simultaneous wrapped error AND non-READY badge.
#[test]
fn status_strip_renders_wrapped_error_and_loading_badge_together() -> Result<()> {
    let err = SttState::Error(
        "Cloud provider quota exhausted; switch to local STT or wait \
         for quota reset window — see docs/quota.md for details."
            .to_string(),
    );
    let text = render_status_strip_with_stt(
        err,
        90,
        7,
        crate::readiness::ReadinessState::Loading {
            component: "llm-mt",
            percent: None,
        },
    )?;
    assert!(text.contains("LOAD"), "badge missing; got:\n{text}");
    assert!(
        text.contains("Cloud provider") && text.contains("details"),
        "wrapped error must show head AND tail; got:\n{text}"
    );
    Ok(())
}
