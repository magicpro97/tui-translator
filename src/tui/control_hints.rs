//! `ControlHintsBar` — single borderless row of keyboard hint labels.
//!
//! Issue #65: this bar is **always shown**, one row high, and never scrolls.
//! It replaces the hint text that was previously embedded in the compact
//! metrics strip (which now shows only metrics).
//!
//! WP-25.01 (#759): extracted from `src/tui/mod.rs` so the orchestrator
//! file can stay under the 1000-LOC gate.  The public API is unchanged;
//! `src/tui/mod.rs` re-exports the type for backwards compatibility.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

/// Single borderless row of keyboard hint labels.
pub struct ControlHintsBar {
    pub tts_on: bool,
}

impl Widget for &ControlHintsBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Adaptive label width (issue #60):
        //   <  80 cols → abbreviated
        //   ≥  80 cols → standard hints (issue #64/#65)
        // CTRL-01: the live `Mic ±N dB / TTS ±N dB` readout is only inlined at
        // ≥ 120 cols.  Narrower terminals keep the pre-PR hint text verbatim
        // so existing PTY snapshots (80×24, 110×30) still see "Q quit" at the
        // end of the row.
        //
        // Issue #854 follow-up: every label group is separated by a SINGLE
        // space.  When "B model" was added (#828 round 2) with the old
        // double-space separators, the standard rows overflowed their own
        // lower-bound width (88 chars at 80 cols, 126 at 120 cols), so
        // `set_stringn` clipped the trailing "Q quit" off the right edge and
        // the always-visible quit hint silently disappeared (regressing the
        // layout/exit/monochrome PTY tests).  Single spacing keeps the full
        // row — including "Q quit" — inside the minimum width of each branch.
        let text = if area.width < 80 {
            " ?  Spc  T  L  S  M  R  Tab  Q ".to_string()
        } else if area.width < 96 {
            let _ = self.tts_on;
            " ? help Space pause T audio L lang S settings M metrics R reload B model Q quit "
                .to_string()
        } else if area.width < 120 {
            let _ = self.tts_on;
            " ? help Space pause T audio L lang S settings M metrics R reload B model Tab pane Q quit "
                .to_string()
        } else {
            let _ = self.tts_on;
            format!(
                " ? help Space pause T audio L lang S settings M metrics R reload B model \
                 [/] mic {:+.0}dB  {{/}} tts {:+.0}dB Tab pane Q quit ",
                crate::audio::audio_gain::input_gain_db(),
                crate::audio::audio_gain::output_volume_db(),
            )
        };

        buf.set_stringn(
            area.x,
            area.y,
            &text,
            area.width as usize,
            Style::default().fg(Color::DarkGray),
        );
    }
}
