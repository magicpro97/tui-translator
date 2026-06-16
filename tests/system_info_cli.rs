//! Integration test for `--print-system-info` (#821).
//!
//! Spawns the compiled `tui-translator` binary with the flag
//! and asserts the three required headers appear on stdout.

// ── Binary-level integration: the compiled `tui-translator`
// binary, when invoked with `--print-system-info`, prints the
// three required headers and exits 0.  This is the test
// called out in #821.
#[test]
fn print_system_info_outputs_human_readable_summary() {
    use std::process::Command;
    let bin = env!("CARGO_BIN_EXE_tui-translator");
    let out = Command::new(bin)
        .arg("--print-system-info")
        .output()
        .expect("spawn tui-translator --print-system-info");
    assert!(
        out.status.success(),
        "binary exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("RAM:"), "missing RAM: in stdout:\n{stdout}");
    assert!(
        stdout.contains("Cores:"),
        "missing Cores: in stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("Recommended preset:"),
        "missing Recommended preset: in stdout:\n{stdout}"
    );
}
