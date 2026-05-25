//! In-memory frame recorder for the deterministic simulation harness.
//!
//! Production TUI tests spawn a real ConPTY (see `tests/pty/harness.rs`)
//! which is expensive and Windows-only. The [`FrameRecorder`] provides
//! an equivalent recorder that lives entirely in memory: tests feed
//! either raw VT bytes (the same byte stream that would be written to
//! the PTY) or a rendered [`ratatui::buffer::Buffer`], and the recorder
//! stores one [`RecordedFrame`] per snapshot with a virtual timestamp
//! supplied by the caller.
//!
//! Golden-frame assertions are then plain string equality between
//! `frame.screen` and a committed expected value (or use of `insta`).
//! No real OS PTY, no real WASAPI, no real time.

use std::time::Duration;

use ratatui::buffer::Buffer;

/// A single recorded TUI frame.
#[derive(Debug, Clone)]
pub struct RecordedFrame {
    /// Virtual time (relative to simulation start) at which the frame
    /// was captured.
    pub virtual_time: Duration,
    /// Plain-text screen contents at capture time. Rows are joined by
    /// `\n`; trailing whitespace on each row is preserved so layout
    /// drift is visible in golden-frame diffs.
    pub screen: String,
}

/// In-memory VT100-backed recorder.
///
/// Use [`FrameRecorder::feed_bytes`] to stream PTY-style escape
/// sequences, or [`FrameRecorder::record_buffer`] to capture a
/// rendered ratatui buffer directly. Either way, call
/// [`FrameRecorder::snapshot`] (or the `record_*` helpers) to append a
/// frame to [`FrameRecorder::frames`].
pub struct FrameRecorder {
    parser: vt100::Parser,
    frames: Vec<RecordedFrame>,
}

impl std::fmt::Debug for FrameRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameRecorder")
            .field("frames", &self.frames.len())
            .finish()
    }
}

impl FrameRecorder {
    /// Construct a recorder for a `rows × cols` virtual terminal.
    ///
    /// # Panics
    /// Panics if either dimension is zero — a zero-sized terminal is a
    /// caller bug, not a runtime condition.
    pub fn new(rows: u16, cols: u16) -> Self {
        assert!(
            rows > 0 && cols > 0,
            "FrameRecorder requires non-zero dimensions"
        );
        Self {
            parser: vt100::Parser::new(rows, cols, 0),
            frames: Vec::new(),
        }
    }

    /// Feed raw bytes into the embedded VT parser. The buffer is *not*
    /// snapshotted yet; call [`FrameRecorder::snapshot`] when the test
    /// is ready to commit a frame.
    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Capture the current screen contents into a [`RecordedFrame`].
    pub fn snapshot(&mut self, virtual_time: Duration) {
        let screen = self.parser.screen().contents();
        self.frames.push(RecordedFrame {
            virtual_time,
            screen,
        });
    }

    /// Convenience: feed `bytes` and snapshot in one call.
    pub fn record_bytes(&mut self, virtual_time: Duration, bytes: &[u8]) {
        self.feed_bytes(bytes);
        self.snapshot(virtual_time);
    }

    /// Capture a rendered ratatui buffer directly, bypassing the VT
    /// parser. Use this when the test renders widgets to a buffer
    /// instead of writing escape sequences to a PTY.
    pub fn record_buffer(&mut self, virtual_time: Duration, buffer: &Buffer) {
        self.frames.push(RecordedFrame {
            virtual_time,
            screen: buffer_to_string(buffer),
        });
    }

    /// All frames captured so far, in order.
    pub fn frames(&self) -> &[RecordedFrame] {
        &self.frames
    }

    /// Current VT screen contents without committing a frame.
    pub fn current_screen(&self) -> String {
        self.parser.screen().contents()
    }
}

/// Render a [`Buffer`] to a stable multi-line string for golden-frame
/// comparisons. Rows are joined by `\n` and each row preserves the
/// printable characters in cell order (no styling).
pub fn buffer_to_string(buffer: &Buffer) -> String {
    let area = buffer.area();
    let mut out = String::with_capacity((area.width as usize + 1) * area.height as usize);
    for y in 0..area.height {
        for x in 0..area.width {
            // ratatui 0.30 exposes Buffer::cell(Position) -> Option<&Cell>.
            if let Some(cell) = buffer.cell(ratatui::layout::Position::new(x, y)) {
                out.push_str(cell.symbol());
            }
        }
        if y + 1 < area.height {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn feed_bytes_then_snapshot_captures_text() {
        let mut rec = FrameRecorder::new(3, 20);
        rec.feed_bytes(b"hello\r\nworld");
        rec.snapshot(Duration::from_millis(50));
        let frames = rec.frames();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].virtual_time, Duration::from_millis(50));
        assert!(
            frames[0].screen.contains("hello"),
            "screen should contain fed text, got {:?}",
            frames[0].screen
        );
        assert!(frames[0].screen.contains("world"));
    }

    #[test]
    fn multiple_snapshots_preserve_order_and_time() {
        let mut rec = FrameRecorder::new(2, 10);
        rec.record_bytes(Duration::from_millis(10), b"a");
        rec.record_bytes(Duration::from_millis(20), b"b");
        rec.record_bytes(Duration::from_millis(30), b"c");
        let times: Vec<_> = rec.frames().iter().map(|f| f.virtual_time).collect();
        assert_eq!(
            times,
            vec![
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(30),
            ]
        );
    }

    #[test]
    fn record_buffer_produces_row_joined_string() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 2));
        buf.set_string(0, 0, "abcd", ratatui::style::Style::default());
        buf.set_string(0, 1, "1234", ratatui::style::Style::default());
        let mut rec = FrameRecorder::new(2, 4);
        rec.record_buffer(Duration::ZERO, &buf);
        assert_eq!(rec.frames()[0].screen, "abcd\n1234");
    }

    #[test]
    fn deterministic_under_repeat() {
        let make = || {
            let mut rec = FrameRecorder::new(2, 8);
            rec.record_bytes(Duration::from_millis(5), b"foo\r\nbar");
            rec.frames()[0].screen.clone()
        };
        assert_eq!(make(), make());
    }

    #[test]
    #[should_panic(expected = "non-zero")]
    fn zero_dim_panics() {
        let _ = FrameRecorder::new(0, 80);
    }
}
