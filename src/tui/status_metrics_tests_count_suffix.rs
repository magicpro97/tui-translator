//! WP-25.02 (#767 follow-up): US-02b count-suffix tests for the
//! readiness badge.  Extracted from `src/tui/status_metrics_tests.rs`
//! so the parent file stays under the 600-LOC engineering-standards
//! gate.
use crate::tui::status_metrics_tests::render_status_strip_with_stt;
use crate::tui::SttState;
use anyhow::Result;

// ── US-02b: `(n/N)` count suffix in the LOAD badge ───────────────────────
/// US-02b: when the interpreter aggregator embeds a subsystem-ready count in
/// `percent`, the badge renders as `LOAD (n/N)`.
#[test]
fn status_strip_load_badge_shows_count_suffix_when_loading() -> Result<()> {
    // `percent: Some(3)` → `loading_count_suffix()` returns `Some((3, 7))`.
    let text = render_status_strip_with_stt(
        SttState::Idle,
        160,
        3,
        crate::readiness::ReadinessState::Loading {
            component: "stt",
            percent: Some(3),
        },
    )?;
    assert!(
        text.contains("(3/7)"),
        "badge must contain `(3/7)` suffix; got:\n{text}"
    );
    Ok(())
}

/// US-02b: zero subsystems ready is still a valid count — badge shows `(0/N)`.
#[test]
fn status_strip_load_badge_shows_zero_count_suffix() -> Result<()> {
    let text = render_status_strip_with_stt(
        SttState::Idle,
        160,
        3,
        crate::readiness::ReadinessState::Loading {
            component: "stt",
            percent: Some(0),
        },
    )?;
    assert!(
        text.contains("(0/7)"),
        "badge must contain `(0/7)` suffix for n=0; got:\n{text}"
    );
    Ok(())
}

/// US-02b: when no aggregator count is available (`percent: None`), render
/// plain `LOAD` with no `(n/N)` suffix.
#[test]
fn status_strip_load_badge_plain_when_percent_none() -> Result<()> {
    let text = render_status_strip_with_stt(
        SttState::Idle,
        160,
        3,
        crate::readiness::ReadinessState::Loading {
            component: "stt",
            percent: None,
        },
    )?;
    assert!(
        text.contains("LOAD"),
        "LOAD badge must be present; got:\n{text}"
    );
    assert!(
        !text.contains("(/"),
        "plain LOAD must not contain `(n/N)` suffix; got:\n{text}"
    );
    Ok(())
}

/// US-02b: non-Loading states (`Ready`, `Error`) must never show a count suffix.
#[test]
fn status_strip_load_badge_omits_suffix_when_not_loading() -> Result<()> {
    let ready_text = render_status_strip_with_stt(
        SttState::Idle,
        160,
        3,
        crate::readiness::ReadinessState::Ready,
    )?;
    assert!(
        !ready_text.contains("(/"),
        "READY badge must not contain count suffix; got:\n{ready_text}"
    );

    let error_text = render_status_strip_with_stt(
        SttState::Idle,
        160,
        3,
        crate::readiness::ReadinessState::Error("fail".to_string()),
    )?;
    assert!(
        !error_text.contains("(/"),
        "ERROR badge must not contain count suffix; got:\n{error_text}"
    );
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
