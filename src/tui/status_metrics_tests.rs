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
