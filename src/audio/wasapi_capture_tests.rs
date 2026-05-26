//! Unit tests for `wasapi_capture` (extracted from `wasapi_capture.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).

use super::*;

// ── resolve_capture_device_name — selection / fallback logic ─────────────

/// `None` input (no `capture_device` key in config) must map to `None` so
/// the caller opens the Windows default render endpoint.
#[test]
fn resolve_device_name_none_uses_default() {
    assert_eq!(resolve_capture_device_name(None), None);
}

/// A blank string (user cleared the field) must also map to `None` so the
/// blank-means-default contract is upheld.
#[test]
fn resolve_device_name_empty_string_uses_default() {
    assert_eq!(resolve_capture_device_name(Some("")), None);
}

/// A whitespace-only string (e.g. accidental space in config.json) must
/// also be treated as absent and fall back to the default device.
#[test]
fn resolve_device_name_whitespace_uses_default() {
    assert_eq!(resolve_capture_device_name(Some("   ")), None);
}

/// A name surrounded by whitespace must be trimmed and returned so the
/// downstream lookup matches the exact Windows device name.
#[test]
fn resolve_device_name_trims_surrounding_whitespace() {
    assert_eq!(
        resolve_capture_device_name(Some("  Speakers (HDA Audio)  ")),
        Some("Speakers (HDA Audio)"),
    );
}

/// A name with no surrounding whitespace is returned verbatim.
#[test]
fn resolve_device_name_exact_name_is_returned_unchanged() {
    assert_eq!(
        resolve_capture_device_name(Some("Headphones (USB Audio)")),
        Some("Headphones (USB Audio)"),
    );
}

fn loopback_devices_or_skip() -> Option<Vec<CaptureDeviceInfo>> {
    match list_loopback_devices() {
        Ok(devices) if !devices.is_empty() => Some(devices),
        _ => None,
    }
}

fn default_loopback_device_or_skip() -> Option<CaptureDeviceInfo> {
    loopback_devices_or_skip()?
        .into_iter()
        .find(|device| device.is_default)
}

fn non_default_loopback_device_or_skip() -> Option<CaptureDeviceInfo> {
    loopback_devices_or_skip()?
        .into_iter()
        .find(|device| !device.is_default)
}

fn assert_selects_endpoint(requested: Option<&str>, expected: &CaptureDeviceInfo) {
    let (device, selected_name) =
        select_render_device(requested).expect("render device selection should succeed");
    let selected_id = device
        .get_id()
        .expect("selected render device must expose a stable endpoint id");
    assert_eq!(selected_name, expected.name);
    assert_eq!(selected_id, expected.id);
}

/// No `capture_device` selection must preserve the existing Windows default
/// render endpoint behavior.
#[test]
fn select_render_device_none_uses_windows_default() {
    let Some(default_device) = default_loopback_device_or_skip() else {
        return;
    };

    assert_selects_endpoint(None, &default_device);
    eprintln!(
        "[wasapi-selection] default selection: {}",
        default_device.name
    );
}

/// Blank `capture_device` config values must also fall back to the Windows
/// default render endpoint.
#[test]
fn select_render_device_blank_uses_windows_default() {
    let Some(default_device) = default_loopback_device_or_skip() else {
        return;
    };

    assert_selects_endpoint(Some("   "), &default_device);
    eprintln!(
        "[wasapi-selection] blank selection fallback: {}",
        default_device.name
    );
}

/// A valid explicit playback-device name must return that exact stable
/// endpoint, not just any endpoint with a matching display label.
#[test]
fn select_render_device_explicit_name_uses_requested_endpoint() {
    let Some(default_device) = default_loopback_device_or_skip() else {
        return;
    };

    assert_selects_endpoint(Some(&default_device.name), &default_device);
    eprintln!(
        "[wasapi-selection] explicit selection: {}",
        default_device.name
    );
}

/// When a non-default endpoint exists, explicit selection must not silently
/// fall back to the Windows default device.
#[test]
fn select_render_device_non_default_explicit_name_does_not_use_default() {
    let Some(non_default_device) = non_default_loopback_device_or_skip() else {
        eprintln!(
            "[wasapi-selection] skipping non-default selection proof: only default endpoint is active"
        );
        return;
    };

    assert_selects_endpoint(Some(&non_default_device.name), &non_default_device);
    eprintln!(
        "[wasapi-selection] explicit non-default selection: {}",
        non_default_device.name
    );
}

// ── format_device_names — label formatting ────────────────────────────────

/// A single device name is returned verbatim (no trailing comma or separator).
#[test]
fn format_device_names_single_entry() {
    assert_eq!(
        format_device_names(&["Speakers (Realtek High Definition Audio)".into()]),
        "Speakers (Realtek High Definition Audio)",
    );
}

/// Multiple device names are joined by `", "`.
#[test]
fn format_device_names_multiple_entries_are_comma_separated() {
    let names = vec![
        "Speakers (Realtek HD Audio)".to_string(),
        "Headphones (USB Audio Device)".to_string(),
        "CABLE Input (VB-Audio Virtual Cable)".to_string(),
    ];
    assert_eq!(
        format_device_names(&names),
        "Speakers (Realtek HD Audio), Headphones (USB Audio Device), CABLE Input (VB-Audio Virtual Cable)",
    );
}

#[test]
fn unsupported_pcm_depth_zero_fills_once_per_chunk() {
    let data = VecDeque::from(vec![0_u8; 12]);
    let mono = raw_bytes_to_mono_f32(&data, 2, 24);

    assert_eq!(mono, vec![0.0, 0.0]);
}

#[test]
fn format_device_names_reports_empty_device_list() {
    assert_eq!(
        format_device_names(&[]),
        "no active playback devices reported by Windows"
    );
}

// ── Issue #196 regression tests ───────────────────────────────────────────

/// `no_default_render_device_error` must produce an operator-actionable
/// message that includes both a Windows UI hint and the CLI escape hatch.
#[test]
fn no_default_render_device_error_is_operator_actionable() {
    let err = no_default_render_device_error("HRESULT 0x80070490 (ERROR_NOT_FOUND)");
    let msg = err.to_string();
    assert!(
        msg.contains("no default audio render device"),
        "must state that no default device was found; got: {msg}"
    );
    assert!(
        msg.contains("Windows Sound Settings"),
        "must mention Windows Sound Settings for GUI recovery; got: {msg}"
    );
    assert!(
        msg.contains("--list-capture-devices"),
        "must suggest the CLI discovery flag; got: {msg}"
    );
    assert!(
        msg.contains("capture_device"),
        "must mention the config key so the operator knows where to set it; got: {msg}"
    );
}

/// The raw WASAPI diagnostic string must be preserved verbatim inside the
/// error so it appears in logs and is useful for support.
#[test]
fn no_default_render_device_error_preserves_wasapi_diagnostic() {
    let raw = "HRESULT 0xDEADBEEF some-windows-error";
    let err = no_default_render_device_error(raw);
    assert!(
        err.to_string().contains(raw),
        "raw WASAPI error must be embedded for diagnostics"
    );
}

/// `find_render_device_by_name` must return `Err` (not panic) when the
/// requested device does not exist.  This exercises the WASAPI COM path;
/// the test skips gracefully if COM cannot be initialised (headless CI).
#[test]
fn find_render_device_by_name_unknown_returns_err_not_panic() {
    // Best-effort COM initialisation — skip rather than fail if unavailable.
    if initialize_mta().is_err() {
        return;
    }
    let result = find_render_device_by_name("____nonexistent_device_issue_196____");
    assert!(
        result.is_err(),
        "an unknown device name must produce Err, not panic"
    );
    // `wasapi::Device` does not implement Debug so we cannot use unwrap_err().
    let msg = match result {
        Err(e) => e.to_string(),
        Ok(_) => unreachable!(),
    };
    assert!(
        msg.contains("was not found"),
        "error should report the device was not found; got: {msg}"
    );
    assert!(
        msg.contains("Capture device"),
        "error should point operators to the capture-device setting; got: {msg}"
    );
}
