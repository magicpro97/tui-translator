/// WP-24 US-04 — multiline status-bar render tests (issue #729).
///
/// Verifies that long provider-error messages word-wrap across multiple
/// visual rows in the status strip so the terminal border never clips
/// the message tail.
use super::*;
use anyhow::Result;

/// Decompose the TestBackend buffer into one `String` per visual row so tests
/// can assert which row a substring appears on.
fn rendered_buffer_rows(
    terminal: &ratatui::Terminal<ratatui::backend::TestBackend>,
) -> Vec<String> {
    let area = terminal.backend().buffer().area;
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| {
                    terminal.backend().buffer()[(x, y)]
                        .symbol()
                        .chars()
                        .next()
                        .unwrap_or(' ')
                })
                .collect::<String>()
        })
        .collect()
}

/// Render a `StatusMetricsStrip` with the given `SttState` into a minimal-height
/// `TestBackend` terminal and return per-row strings.
///
/// Width is 80 (the documented compact-terminal target).  Height is intentionally
/// small (6 rows) so the strip cannot mask a regression by re-using extra vertical
/// space — if the wrap math is wrong, the assertions below will still fail.
fn render_strip_rows(stt: SttState) -> Result<Vec<String>> {
    use ratatui::{backend::TestBackend, Terminal};

    static MT: std::sync::LazyLock<MtState> = std::sync::LazyLock::new(MtState::default);

    let backend = TestBackend::new(80, 6);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| {
        let strip = StatusMetricsStrip {
            stt: &stt,
            mt: &MT,
            tts_on: false,
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
            readiness: crate::readiness::ReadinessState::Ready,
        };
        frame.render_widget(&strip, frame.area());
    })?;
    Ok(rendered_buffer_rows(&terminal))
}

/// AC2 + AC3 (WP-24 US-04 / issue #729):
///
/// The `mt_provider="llm"` saved-settings error must word-wrap in a standard
/// 80×24 terminal so that:
///
/// * The word `restart` (end of the message) appears on a **different** visual
///   row from the word `Some` (start of the message).
/// * The substring `mt_provider` is fully visible (not clipped mid-word).
#[test]
fn long_error_wraps_multiline_in_status_bar() -> Result<()> {
    let msg = "Some saved provider settings are not available in this build \
               (mt_provider=\"llm\"). Set unsupported providers to \"google\", \
               save, and restart.";

    let rows = render_strip_rows(SttState::Error(msg.to_string()))?;

    let some_row = rows
        .iter()
        .position(|r| r.contains("Some"))
        .unwrap_or_else(|| {
            panic!(
                "'Some' not found in rendered 80×24 output; got:\n{}",
                rows.join("\n")
            )
        });

    let restart_row = rows
        .iter()
        .position(|r| r.contains("restart"))
        .unwrap_or_else(|| {
            panic!(
                "'restart' not found in rendered 80×24 output; got:\n{}",
                rows.join("\n")
            )
        });

    assert_ne!(
        some_row,
        restart_row,
        "'Some' (row {some_row}) and 'restart' (row {restart_row}) must be on \
         different visual rows in an 80×24 terminal; got:\n{}",
        rows.join("\n")
    );

    let all_text = rows.join("\n");
    assert!(
        all_text.contains("mt_provider"),
        "substring 'mt_provider' must appear un-clipped in the 80×24 buffer; got:\n{all_text}",
    );

    Ok(())
}
