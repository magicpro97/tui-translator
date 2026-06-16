//! `--print-system-info` CLI handler (T15, #821).
//!
//! Prints a one-shot human-readable summary of the host's hardware
//! capabilities and the wizard's recommended quality preset.  This
//! is the third surface channel for `SysCaps` (the other two being
//! the TUI's `HardwareSurvey` step and the `PresetBar` rendered in
//! the ModelManager overlay), aimed at shell-scripting / CI users
//! who want to probe the machine without running the full TUI.
//!
//! Output format (all on stdout, LF newlines, UTF-8, no ANSI):
//!
//! ```text
//! tui-translator system info
//!   RAM:       <gib> GiB (<tier>)
//!   Cores:     <n> (<tier>)
//!   GPU:       <kind>
//!   Recommended preset: <resolved>  (Auto resolves from <configured> on <ram_tier>)
//! ```
//!
//! Exit code: 0.  No side effects.

use std::io::{self, Write};

use anyhow::Result;

use crate::quality_preset::QualityPreset;
use crate::sys_caps::{GpuKind, SysCaps};

/// Return whether process arguments request the system-info
/// dump mode.  Accepts `--print-system-info` and the
/// `print-system-info` alias for parity with the other
/// `should_list_*` helpers in the codebase.
pub(crate) fn should_print_system_info() -> bool {
    std::env::args().skip(1).any(|arg| {
        arg == "--print-system-info" || arg == "print-system-info" || arg == "--system-info"
    })
}

/// Detect [`SysCaps`] and print the human-readable summary to
/// stdout.  Returns `Ok(())` on success; on I/O failure the
/// error propagates to the caller (which will print a
/// diagnostic and exit non-zero, matching the other CLI helpers).
pub(crate) fn print_system_info_to_stdout() -> Result<()> {
    let caps = SysCaps::detect();
    let mut stdout = io::stdout();
    write_system_info(&mut stdout, &caps).map_err(anyhow::Error::from)
}

/// Pure writer — split out from [`print_system_info_to_stdout`]
/// so the format and the `GpuKind::None` rendering arm are both
/// testable without touching a real stdout or a real
/// `SysCaps::detect()`.
pub(crate) fn write_system_info(writer: &mut impl Write, caps: &SysCaps) -> io::Result<()> {
    writeln!(writer, "tui-translator system info")?;
    writeln!(
        writer,
        "  RAM:       {:.1} GiB ({})",
        caps.total_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        ram_tier_name(caps.ram_tier()),
    )?;
    writeln!(
        writer,
        "  Cores:     {} ({})",
        caps.physical_cores,
        cpu_tier_name(caps.cpu_tier()),
    )?;
    writeln!(writer, "  GPU:       {}", gpu_kind_name(caps.gpu.clone()))?;
    // Recommended preset = Auto resolved against the host.
    let configured = QualityPreset::Auto;
    let resolved = configured.resolve_for(caps);
    writeln!(
        writer,
        "  Recommended preset: {}  (Auto resolves to {} on {} RAM)",
        resolved,
        resolved,
        ram_tier_name(caps.ram_tier()),
    )?;
    Ok(())
}

fn ram_tier_name(tier: crate::sys_caps::RamTier) -> &'static str {
    use crate::sys_caps::RamTier;
    match tier {
        RamTier::Low => "low",
        RamTier::Medium => "medium",
        RamTier::High => "high",
    }
}

fn cpu_tier_name(tier: crate::sys_caps::CpuTier) -> &'static str {
    use crate::sys_caps::CpuTier;
    match tier {
        CpuTier::Low => "low",
        CpuTier::Medium => "medium",
        CpuTier::High => "high",
    }
}

fn gpu_kind_name(kind: GpuKind) -> String {
    // Build a human-readable string in the form `<variant> (<name>,
    // <vram>)` when a GPU is present, or just `none` for the
    // no-GPU case.  Pulled out from `write_system_info` so the
    // per-file coverage gate sees all three variant arms even
    // when the host has no GPU.
    match kind {
        GpuKind::None => "none".to_string(),
        GpuKind::Metal { name, vram_bytes } => format!("metal ({name}, {})", vram_gib(vram_bytes)),
        GpuKind::Cuda { name, vram_bytes } => format!("cuda ({name}, {})", vram_gib(vram_bytes)),
    }
}

/// Render a byte count as a one-decimal GiB value (e.g. 16.0 GiB)
/// so the `--print-system-info` line stays compact.
fn vram_gib(bytes: u64) -> String {
    let gib = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    format!("{gib:.1} GiB")
}

// Tests live in `src/system_info_cli_tests.rs`, included at the
// crate root as `mod system_info_cli_tests;` from `main.rs`.
