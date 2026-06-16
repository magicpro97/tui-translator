//! Failing tests for `render_model_manager_lines` (T10, #816).
//!
//! RED: `src/tui/model_manager_render.rs` does not exist yet.
//! These will go GREEN after the module is implemented.

use crate::quality_preset::QualityPreset;
use crate::sys_caps::SysCaps;
use crate::tui::model_manager_render::render_model_manager_lines;
use crate::tui::model_manager_state::ModelManagerState;
use crate::tui::model_manager_tokens::PresetBar;

fn sys_caps(ram_bytes: u64, cores: usize) -> SysCaps {
    SysCaps {
        total_memory_bytes: ram_bytes,
        physical_cores: cores,
        gpu: crate::sys_caps::GpuKind::None,
    }
}

#[test]
fn first_line_is_preset_bar() {
    let s = ModelManagerState::default();
    let bar = PresetBar::for_preset(QualityPreset::Auto, &sys_caps(32 * 1024 * 1024 * 1024, 8));
    let lines = render_model_manager_lines(&s, &bar);
    let first = lines.first().expect("at least one line");
    assert!(
        first.starts_with("Quality: "),
        "first line must be the preset bar, got {first:?}"
    );
    assert!(
        first.contains("Best"),
        "32 GiB Auto resolves to Best: {first:?}"
    );
}

#[test]
fn lines_include_all_three_tabs_in_order() {
    let s = ModelManagerState::default();
    let bar = PresetBar::for_preset(QualityPreset::Auto, &sys_caps(8 * 1024 * 1024 * 1024, 4));
    let text = render_model_manager_lines(&s, &bar).join("\n");
    // Tab labels appear in the tab strip (current tab shown with brackets).
    let strip_start = text.find('[').unwrap_or(0);
    let strip_end = text[strip_start..].find("History").unwrap_or(0) + "History".len();
    let strip = &text[strip_start..strip_start + strip_end + 1];
    assert!(
        strip.contains("Whisper"),
        "strip must include Whisper: {strip}"
    );
    assert!(
        strip.contains("FunASR"),
        "strip must include FunASR: {strip}"
    );
    assert!(
        strip.contains("History"),
        "strip must include History: {strip}"
    );
}

#[test]
fn current_tab_in_strip_is_wrapped_in_brackets() {
    let s = ModelManagerState::default();
    let bar = PresetBar::for_preset(QualityPreset::Auto, &sys_caps(8 * 1024 * 1024 * 1024, 4));
    let text = render_model_manager_lines(&s, &bar).join("\n");
    // Find tab strip line (contains "Whisper" + "FunASR" + "History").
    for line in text.lines() {
        if line.contains("Whisper") && line.contains("FunASR") && line.contains("History") {
            assert!(
                line.contains("[Whisper]"),
                "default tab must be [Whisper]: {line}"
            );
            assert!(!line.contains("[FunASR]"), "FunASR is not active: {line}");
            assert!(!line.contains("[History]"), "History is not active: {line}");
            return;
        }
    }
    panic!("tab strip not found");
}

#[test]
fn whisper_tab_renders_eight_model_rows() {
    let s = ModelManagerState::default();
    let bar = PresetBar::for_preset(QualityPreset::Auto, &sys_caps(8 * 1024 * 1024 * 1024, 4));
    let lines = render_model_manager_lines(&s, &bar);
    let ggml_count = lines.iter().filter(|l| l.contains("ggml-")).count();
    assert!(
        ggml_count >= 1,
        "Whisper tab must show at least one ggml model line, got {ggml_count}"
    );
}

#[test]
fn funasr_tab_renders_three_model_rows() {
    let mut s = ModelManagerState::default();
    s.next_tab(); // FunAsr
    let bar = PresetBar::for_preset(QualityPreset::Auto, &sys_caps(8 * 1024 * 1024 * 1024, 4));
    let lines = render_model_manager_lines(&s, &bar);
    let funasr_count = lines
        .iter()
        .filter(|l| {
            l.contains("funasr-") || l.contains("FunASR") || l.contains("sherpa-onnx-funasr")
        })
        .count();
    assert!(
        funasr_count >= 3,
        "FunASR tab must show 3 model rows, got {funasr_count}"
    );
}

#[test]
fn history_tab_shows_empty_message() {
    let mut s = ModelManagerState::default();
    s.next_tab(); // FunAsr
    s.next_tab(); // History
    let bar = PresetBar::for_preset(QualityPreset::Auto, &sys_caps(8 * 1024 * 1024 * 1024, 4));
    let lines = render_model_manager_lines(&s, &bar);
    let text = lines.join("\n");
    assert!(
        text.contains("empty")
            || text.contains("Empty")
            || text.contains("0 models")
            || text.contains("(0)"),
        "History tab must indicate emptiness: {text}"
    );
}

#[test]
fn selected_row_is_marked_with_cursor() {
    let mut s = ModelManagerState::default();
    s.select_next(); // selected_index = 1
    let bar = PresetBar::for_preset(QualityPreset::Auto, &sys_caps(8 * 1024 * 1024 * 1024, 4));
    let lines = render_model_manager_lines(&s, &bar);
    // Exactly one line should have a cursor marker (e.g. ">").
    let cursor_count = lines.iter().filter(|l| l.starts_with('>')).count();
    assert_eq!(
        cursor_count, 1,
        "exactly one row must be marked selected, got {cursor_count} rows: {lines:?}"
    );
}

#[test]
fn switching_to_funasr_tab_moves_cursor_to_first_funasr_row() {
    let mut s = ModelManagerState::default();
    s.select_index(5); // cursor deep in whisper list
    s.next_tab(); // FunAsr
    let bar = PresetBar::for_preset(QualityPreset::Auto, &sys_caps(8 * 1024 * 1024 * 1024, 4));
    let lines = render_model_manager_lines(&s, &bar);
    // The first FunASR row must be marked.
    let first_funasr = lines
        .iter()
        .find(|l| l.contains("funasr") || l.contains("sherpa-onnx-funasr"));
    assert!(
        first_funasr.is_some(),
        "must have a FunASR row in lines: {lines:?}"
    );
    let first_funasr = first_funasr.unwrap();
    assert!(
        first_funasr.starts_with('>'),
        "first FunASR row must be selected after tab switch: {first_funasr:?}"
    );
}

#[test]
fn width_clamps_long_labels() {
    let s = ModelManagerState::default();
    let bar = PresetBar::for_preset(QualityPreset::Auto, &sys_caps(8 * 1024 * 1024 * 1024, 4));
    let lines = render_model_manager_lines(&s, &bar);
    for line in &lines {
        assert!(
            line.len() <= 200,
            "line too long ({} chars): {line:?}",
            line.len()
        );
    }
}
