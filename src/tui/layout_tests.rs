use super::*;
use ratatui::layout::Rect;

fn rect(w: u16, h: u16) -> Rect {
    Rect::new(0, 0, w, h)
}

#[test]
fn layout_profile_detects_too_small() {
    assert_eq!(LayoutProfile::detect(rect(0, 0)), LayoutProfile::TooSmall);
    assert_eq!(LayoutProfile::detect(rect(19, 30)), LayoutProfile::TooSmall);
    assert_eq!(LayoutProfile::detect(rect(80, 9)), LayoutProfile::TooSmall);
    assert!(!LayoutProfile::detect(rect(15, 5)).is_renderable());
}

#[test]
fn layout_profile_detects_compact() {
    // 60x20 is the canonical compact size from UX-01 acceptance criteria.
    assert_eq!(LayoutProfile::detect(rect(60, 20)), LayoutProfile::Compact);
    assert_eq!(LayoutProfile::detect(rect(20, 10)), LayoutProfile::Compact);
    assert_eq!(LayoutProfile::detect(rect(79, 50)), LayoutProfile::Compact);
    assert!(!LayoutProfile::detect(rect(60, 20)).is_dual_pane());
}

#[test]
fn layout_profile_detects_normal() {
    // 80x24 is the canonical normal size.
    assert_eq!(LayoutProfile::detect(rect(80, 24)), LayoutProfile::Normal);
    assert_eq!(LayoutProfile::detect(rect(119, 40)), LayoutProfile::Normal);
    assert!(!LayoutProfile::detect(rect(80, 24)).is_dual_pane());
}

#[test]
fn layout_profile_detects_wide() {
    // 120x40 is the canonical wide size.
    assert_eq!(LayoutProfile::detect(rect(120, 40)), LayoutProfile::Wide);
    assert_eq!(LayoutProfile::detect(rect(200, 50)), LayoutProfile::Wide);
    assert!(LayoutProfile::detect(rect(120, 40)).is_dual_pane());
}

#[test]
fn layout_profile_is_monotone() {
    // Property: growing either dimension never returns a smaller profile.
    let widths: [u16; 10] = [0, 10, 19, 20, 40, 79, 80, 100, 119, 120];
    let heights: [u16; 6] = [0, 5, 9, 10, 24, 50];
    for &w1 in &widths {
        for &w2 in &widths {
            for &h1 in &heights {
                for &h2 in &heights {
                    if w1 <= w2 && h1 <= h2 {
                        let p1 = LayoutProfile::detect(rect(w1, h1));
                        let p2 = LayoutProfile::detect(rect(w2, h2));
                        assert!(
                            p1 <= p2,
                            "monotonicity violated: detect({w1}x{h1})={p1:?} > detect({w2}x{h2})={p2:?}",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn layout_profile_chunks_stay_within_frame() {
    use ratatui::layout::{Constraint, Direction, Layout};

    let sizes: [(u16, u16); 6] = [
        (60, 20),
        (80, 24),
        (120, 40),
        (200, 50),
        (20, 10),
        (240, 80),
    ];
    for &(w, h) in &sizes {
        let area = rect(w, h);
        let profile = LayoutProfile::detect(area);
        assert!(
            profile.is_renderable(),
            "{w}x{h} should be renderable but classified as {profile:?}"
        );
        let metrics_h = expanded_metrics_height(false, false);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(metrics_h),
                Constraint::Length(1),
            ])
            .split(area);
        for (idx, c) in chunks.iter().enumerate() {
            assert!(
                c.x + c.width <= area.x + area.width && c.y + c.height <= area.y + area.height,
                "chunk[{idx}] {c:?} escapes frame {area:?} at {w}x{h} ({profile:?})"
            );
        }
    }
}
