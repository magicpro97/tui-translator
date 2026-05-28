use super::*;
use anyhow::Result;
use std::{path::PathBuf, sync::atomic::Ordering};

fn render_expanded_storage_strip(
    recorder_bytes: u64,
    recorder_path: Option<PathBuf>,
    archive_bytes: u64,
    archive_path: Option<PathBuf>,
    archive_sealed: bool,
    audio_consent: bool,
) -> Result<String> {
    use ratatui::{backend::TestBackend, Terminal};

    let stt = SttState::Idle;
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on: false,
        tts_route: TtsRouteStatus::default(),
        target_language: "vi".to_string(),
        pairs: 0,
        audio_secs: 0.0,
        cost_usd: 0.0,
        elapsed: "0:00".to_string(),
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
        recorder_bytes,
        recorder_path,
        archive_bytes,
        archive_path,
        archive_sealed,
        audio_consent,
        stt_source: SttSource::Local,
        slot_a_tts_status: "ok".to_string(),
        slot_b_tts_status: None,
        config_apply_status: None,
        config_apply_count: 0,
    };
    let backend = TestBackend::new(120, 9);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| {
        frame.render_widget(&strip, frame.area());
    })?;
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
    Ok(rows.join("\n"))
}

#[test]
fn storage_row_with_consent_shows_archive_path_and_bytes() -> Result<()> {
    let archive_path = PathBuf::from(r"C:\tui\archive.wav");
    let recorder_path = PathBuf::from(r"C:\tui\session.jsonl");
    let rendered = render_expanded_storage_strip(
        1_234_567,
        Some(recorder_path),
        98_765_432,
        Some(archive_path),
        false,
        true,
    )?;
    assert!(
        rendered.contains("audio archive:"),
        "storage row must contain 'audio archive:' label when consent is given; got:\n{rendered}"
    );
    assert!(
        rendered.contains("archive.wav"),
        "storage row must show archive path when consent is given; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("consent revoked"),
        "storage row must NOT show 'consent revoked' when consent is given; got:\n{rendered}"
    );
    assert!(
        rendered.contains("transcripts:"),
        "storage row must always show transcripts label; got:\n{rendered}"
    );
    assert!(
        rendered.contains("session.jsonl"),
        "storage row must show recorder path; got:\n{rendered}"
    );
    Ok(())
}

#[test]
fn storage_row_without_consent_hides_archive_bytes_and_path() -> Result<()> {
    let archive_path = PathBuf::from(r"C:\tui\archive.wav");
    let recorder_path = PathBuf::from(r"C:\tui\session.jsonl");
    let rendered = render_expanded_storage_strip(
        1_234_567,
        Some(recorder_path),
        98_765_432,
        Some(archive_path),
        false,
        false,
    )?;
    assert!(
        rendered.contains("consent revoked"),
        "storage row must show '(consent revoked)' when audio consent is false; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("archive.wav"),
        "storage row must NOT show archive path when consent is revoked; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("94.2 MB"),
        "storage row must NOT show archive byte count when consent is revoked; got:\n{rendered}"
    );
    assert!(
        rendered.contains("transcripts:"),
        "storage row must still show transcripts label even when consent is revoked; got:\n{rendered}"
    );
    Ok(())
}

#[test]
fn storage_row_sealed_archive_shows_sealed_label() -> Result<()> {
    let archive_path = PathBuf::from(r"C:\tui-translator\audio-archive\sealed.wav");
    let rendered =
        render_expanded_storage_strip(0, None, 50 * 1024 * 1024, Some(archive_path), true, true)?;
    assert!(
        rendered.contains("(sealed)"),
        "sealed archive must display '(sealed)' label; got:\n{rendered}"
    );
    Ok(())
}

#[test]
fn storage_row_no_recorder_shows_dash() -> Result<()> {
    let rendered = render_expanded_storage_strip(0, None, 0, None, false, false)?;
    assert!(
        rendered.contains("transcripts:"),
        "storage row must show transcripts label even when recording is disabled; got:\n{rendered}"
    );
    assert!(
        rendered.contains('\u{2014}'),
        "disabled recorder path should show em-dash placeholder; got:\n{rendered}"
    );
    Ok(())
}

#[test]
fn format_storage_bytes_edge_cases() {
    assert_eq!(format_storage_bytes(0), "0 B");
    assert_eq!(format_storage_bytes(1), "1 B");
    assert_eq!(format_storage_bytes(1023), "1023 B");
    assert_eq!(format_storage_bytes(1024), "1.0 KB");
    assert_eq!(format_storage_bytes(1024 * 1024), "1.0 MB");
    assert_eq!(format_storage_bytes(1024 * 1024 * 1024), "1.0 GB");
    assert_eq!(format_storage_bytes(1536), "1.5 KB");
}

#[test]
fn audio_consent_default_is_false() {
    let state = AppState::new();
    assert!(
        !state.audio_consent.load(Ordering::Relaxed),
        "audio_consent must default to false (no consent on fresh state)"
    );
}

#[test]
fn set_audio_consent_updates_atomic() {
    let state = AppState::new();
    state.set_audio_consent(true);
    assert!(state.audio_consent.load(Ordering::Relaxed));
    state.set_audio_consent(false);
    assert!(!state.audio_consent.load(Ordering::Relaxed));
}
