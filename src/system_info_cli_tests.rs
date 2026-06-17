//! Tests for the `--print-system-info` CLI handler (T15, #821).

use crate::sys_caps::{GpuKind, SysCaps};
use crate::system_info_cli::write_system_info;

fn caps(ram_gib: u64, cores: usize, gpu: GpuKind) -> SysCaps {
    SysCaps {
        total_memory_bytes: ram_gib * 1024 * 1024 * 1024,
        physical_cores: cores,
        gpu,
    }
}

#[test]
fn write_system_info_includes_ram_cores_recommended() {
    // Issue #821: the dump must contain `RAM:`, `Cores:`,
    // and `Recommended preset:` so scripted/CI users can
    // grep it.  We feed a hand-built `SysCaps` to keep the
    // test deterministic and CI-portable (no real
    // `SysCaps::detect()`).
    let mut buf: Vec<u8> = Vec::new();
    let c = caps(
        16,
        8,
        GpuKind::Metal {
            name: "Apple M1 Pro".to_string(),
            vram_bytes: 25_000_000_000,
        },
    );
    write_system_info(&mut buf, &c).expect("write_system_info ok");
    let out = String::from_utf8(buf).expect("utf-8 output");
    assert!(
        out.contains("RAM:"),
        "missing RAM: header in output:\n{out}"
    );
    assert!(
        out.contains("Cores:"),
        "missing Cores: header in output:\n{out}"
    );
    assert!(
        out.contains("Recommended preset:"),
        "missing Recommended preset: in output:\n{out}"
    );
    // 16 GiB on high-RAM resolves Auto to Best.
    assert!(
        out.contains("Best"),
        "expected Auto to resolve to Best on 16 GiB; got:\n{out}"
    );
    // The GPU kind must be reported.
    assert!(out.contains("metal"), "expected GPU=metal; got:\n{out}");
}

#[test]
fn write_system_info_low_ram_resolves_to_performance() {
    // Auto on 4 GiB (low RAM tier) must resolve to Performance.
    let mut buf: Vec<u8> = Vec::new();
    let c = caps(4, 4, GpuKind::None);
    write_system_info(&mut buf, &c).expect("write_system_info ok");
    let out = String::from_utf8(buf).expect("utf-8 output");
    assert!(
        out.contains("Performance"),
        "expected Auto to resolve to Performance on 4 GiB; got:\n{out}"
    );
    assert!(out.contains("low"), "expected RAM tier=low; got:\n{out}");
}

#[test]
fn write_system_info_gpu_none_renders_as_none() {
    // The `GpuKind::None` rendering arm must say "none",
    // not "unknown" — that arm is the only one reachable in
    // v3 (the Metal/Cuda variants are v3.1).
    let mut buf: Vec<u8> = Vec::new();
    let c = caps(8, 4, GpuKind::None);
    write_system_info(&mut buf, &c).expect("write_system_info ok");
    let out = String::from_utf8(buf).expect("utf-8 output");
    assert!(
        out.contains("GPU:       none"),
        "expected GPU: none line; got:\n{out}"
    );
}
