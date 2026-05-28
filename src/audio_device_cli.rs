use std::io::{self, Write};

use anyhow::{Context, Result};

use crate::{audio, config, config_json_path};

/// Return whether process arguments request audio device listing mode.
pub(crate) fn should_list_audio_devices() -> bool {
    std::env::args().skip(1).any(|arg| {
        arg == "--list-audio-devices"
            || arg == "--list-capture-devices"
            || arg == "list-audio-devices"
    })
}

/// Print configured WASAPI capture devices to stdout.
pub(crate) fn print_audio_devices_to_stdout() -> Result<()> {
    let cfg_path = config_json_path();
    let (cfg, _) = config::load_with_state(&cfg_path)
        .with_context(|| format!("failed to load config for {}", cfg_path.display()))?;
    let registry =
        audio::VirtualDevicePatternRegistry::with_custom_patterns(&cfg.virtual_device_patterns)
            .context("failed to load virtual_device_patterns from config")?;
    let devices = audio::list_capture_devices().context("failed to list audio capture devices")?;
    let mut stdout = io::stdout();
    write_audio_devices(&mut stdout, &devices, &registry)
        .context("failed to write audio device list")?;
    Ok(())
}

/// Write a human-readable audio device listing.
pub(crate) fn write_audio_devices(
    writer: &mut impl Write,
    devices: &[audio::CaptureDeviceInfo],
    registry: &audio::VirtualDevicePatternRegistry,
) -> io::Result<()> {
    writeln!(
        writer,
        "Audio capture devices for WASAPI loopback (Windows playback endpoints):"
    )?;
    writeln!(
        writer,
        "  [default] Windows default playback device (leave capture_device blank)"
    )?;
    if devices.is_empty() {
        writeln!(
            writer,
            "  No active playback devices were reported by Windows."
        )?;
    } else {
        for device in devices {
            let default_marker = if device.is_default {
                " (current Windows default)"
            } else {
                ""
            };
            let virtual_marker =
                if audio::classify_virtual_device_with_registry(&device.name, registry).is_some() {
                    " [VIRTUAL]"
                } else {
                    ""
                };
            writeln!(
                writer,
                "  - {}{}{}",
                device.name, virtual_marker, default_marker
            )?;
            writeln!(writer, "      endpoint_id: {}", device.id)?;
        }
    }
    Ok(())
}
