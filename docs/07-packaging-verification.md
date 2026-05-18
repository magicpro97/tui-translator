# Packaging Verification — WP-14.01 and WP-14.04

**Issues:** [#90](https://github.com/magicpro97/tui-translator/issues/90) (WP-14.01),
[#93](https://github.com/magicpro97/tui-translator/issues/93) (WP-14.04)

This document records the build verification evidence, the static-linking
configuration added, and the portability audit for the `tui-translator.exe`
release binary.

---

## Static CRT Linking (Issue #90)

### What was configured

`.cargo/config.toml` now contains:

```toml
[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "target-feature=+crt-static"]
```

This tells the Rust compiler to link the Visual C++ Runtime statically into the
binary. The result is a single `.exe` that does not require the user to install
the VC++ Redistributable separately. The flag is scoped to the
`x86_64-pc-windows-msvc` target, so it does not affect GNU or non-Windows
builds. It **does** apply to all MSVC builds for that target, including debug,
clippy, test, and release runs on Windows CI.

### Build command (for CI / release workflow)

```powershell
cargo build --release --target x86_64-pc-windows-msvc
# artifact: target\x86_64-pc-windows-msvc\release\tui-translator.exe
```

### Dependency verification (run after a successful build)

Use any of the following tools on the resulting `.exe` to confirm it has no
dynamic DLL dependencies beyond the Windows OS itself (`kernel32.dll`,
`ntdll.dll`, etc.):

- **dumpbin** (part of Visual Studio Build Tools):
  ```
  dumpbin /DEPENDENTS target\x86_64-pc-windows-msvc\release\tui-translator.exe
  ```
  Expected output: only Windows system DLLs (`kernel32.dll`, `ntdll.dll`, etc.).
  `vcruntime140.dll` and `msvcp140.dll` must **not** appear.

- **Dependencies GUI** (free, <https://github.com/lucasg/Dependencies>):
  Open the `.exe` and confirm the dependency tree contains no VC++ runtime DLLs.

### CI-enforced gate (primary evidence path)

A dedicated `packaging` job has been added to `.github/workflows/ci.yml`. On
every push and pull-request the job:

1. Checks out the repository on a GitHub `windows-latest` runner (which
   provides `link.exe`, the Windows SDK, and Visual Studio Build Tools).
2. Installs the `x86_64-pc-windows-msvc` standard library via
   `dtolnay/rust-toolchain@stable` with `targets: x86_64-pc-windows-msvc`.
3. Runs `cargo build --release --target x86_64-pc-windows-msvc --bins`,
   picking up the `+crt-static` flag from `.cargo/config.toml`.
4. Confirms the single `.exe` artifact exists and logs its size.
5. Runs `dumpbin /DEPENDENTS` (located via `vswhere.exe`) and **fails the
   build** if `vcruntime140.dll` or `msvcp140.dll` appears in the import list.

This gate is intended to become the canonical proof path once the branch is
pushed and the job has completed successfully. Until that CI run exists, the
job definition is only a verification mechanism, not proof by itself.

### Local host blockers (recorded for transparency)

Two independent blockers prevent building the MSVC target on this workstation:

| # | Blocker | Evidence |
|---|---------|---------|
| 1 | `link.exe` (MSVC linker) not installed - Visual Studio Build Tools absent | `where.exe link.exe` -> `INFO: Could not find files for the given pattern(s).` |
| 2 | Disk `C:` is 100 % full (0 bytes free) | `Get-PSDrive C` -> `FreeGB: 0, UsedGB: 238.4` |

Because of blocker 2, even installing the MSVC toolchain sysroot with
`rustup target add x86_64-pc-windows-msvc` fails:

```
error: failed to extract package: There is not enough space on the disk. (os error 112)
```

These are host-environment constraints, not code or configuration issues.

### Historical local evidence - GNU release build

A GNU-toolchain release binary (from worktree `tt-wt-p1-status-metrics`) was
inspected with `objdump -p` to confirm the local portability baseline on this
host:

```
objdump -p tt-wt-p1-status-metrics/target/release/tui-translator.exe | grep "DLL Name:"
```

**Result (all DLL imports, sorted and deduplicated):**

```
DLL Name: api-ms-win-core-synch-l1-2-0.dll   <- Windows UCRT forwarder (built-in Win 10+)
DLL Name: api-ms-win-crt-environment-l1-1-0.dll
DLL Name: api-ms-win-crt-heap-l1-1-0.dll
DLL Name: api-ms-win-crt-locale-l1-1-0.dll
DLL Name: api-ms-win-crt-math-l1-1-0.dll
DLL Name: api-ms-win-crt-private-l1-1-0.dll
DLL Name: api-ms-win-crt-runtime-l1-1-0.dll
DLL Name: api-ms-win-crt-stdio-l1-1-0.dll
DLL Name: api-ms-win-crt-string-l1-1-0.dll
DLL Name: bcryptprimitives.dll                <- Windows system DLL
DLL Name: kernel32.dll                        <- Windows system DLL
DLL Name: ntdll.dll                           <- Windows system DLL
DLL Name: ole32.dll                           <- Windows system DLL
DLL Name: oleaut32.dll                        <- Windows system DLL
DLL Name: propsys.dll                         <- Windows system DLL
DLL Name: user32.dll                          <- Windows system DLL
```

**No `vcruntime140.dll` or `msvcp140.dll` present.**

This does **not** prove the MSVC `+crt-static` requirement from issue #90. The
GNU toolchain ignores the `[target.x86_64-pc-windows-msvc]` section in
`.cargo/config.toml`, so this output is only a historical local observation,
not closing evidence for the MSVC packaging requirement.

The `api-ms-win-crt-*` entries are Windows UCRT API-set forwarders - they are
present in every Windows 10 / 11 installation and do not require a separate
Redistributable. The MSVC-specific requirement still must be proven by a
successful CI `packaging` job on `x86_64-pc-windows-msvc`.

---

## Portability Audit (Issue #93)

The `.exe` must be runnable from any folder — it must not read or write files
relative to the current working directory at startup.

### Config file path

`src/main.rs`, function `config_json_path()`:

```rust
fn config_json_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("config.json")))
        .unwrap_or_else(|| std::path::PathBuf::from("config.json"))
}
```

The path is resolved relative to the **executable's own directory**, not the
working directory. A user can `cd` to any folder and run:

```
C:\path\to\tui-translator.exe
```

and the application will look for `config.json` in `C:\path\to\`, regardless
of the current directory. This is the correct portable behaviour described in
the user-facing guide ([USAGE.md](../USAGE.md), Step 4).

### Hardcoded path audit

A search of all `*.rs` source files for absolute Windows paths (`C:\`, `D:\`),
Unix home paths (`/home/`, `/usr/`), and `std::env::current_dir` found no
hard-coded absolute paths and no use of `current_dir`. All file access goes
through `config_json_path()` or through the hot-reload watcher, which is
seeded from the same function.

### Verdict

The application is fully portable. The only file it touches at runtime is
`config.json` in the directory that contains the `.exe`, which is exactly what
the end-user guide documents.

---

## Local Model Packaging Notes (Issue #236)

ZIP and Inno Setup packages must not include Whisper or MT model binaries. Ship
only `tui-translator.exe`, `config.example.json`, and docs. Operators who need
offline/local STT should prefetch models after install:

```powershell
.\tui-translator.exe --prefetch-local-stt-model tiny
.\tui-translator.exe --prefetch-local-stt-model tiny --yes
```

By default the verified model cache is `%USERPROFILE%\.tui-translator\models`.
For a portable ZIP or managed installer staging layout, run the same command
with `--model-cache-dir <dir>` and copy that verified cache into the user's
model cache during install. If the package uses a pinned vendor manifest, use
`--prefetch-local-stt-manifest <manifest.json>` with the same cache flag. The
manifest must match one of the built-in Whisper files that local STT can load.
The command resumes interrupted `.part` downloads, verifies SHA-256 before
writing `manifest.json`, and reuses already verified files on repeat runs.

---

## Checklist

| Check | Status |
|-------|--------|
| `.cargo/config.toml` sets `+crt-static` for msvc target | ✅ Done |
| Build command documented | ✅ Done |
| Dependency verification method documented | ✅ Done |
| CI `packaging` job added - builds MSVC target, runs dumpbin, asserts no VC++ DLLs | ✅ Added |
| CI `packaging` job completed successfully on this branch | ⏳ Pending |
| Local MSVC build blockers recorded with exact errors | ✅ Recorded |
| GNU release build inspected - portability baseline recorded as local history only | ✅ Recorded |
| `config_json_path()` uses `current_exe().parent()` | ✅ Confirmed portable |
| No absolute paths found in source | ✅ Audited clean |
| USAGE.md documents run-from-any-folder behaviour | ✅ Done |
| ZIP/Inno packages exclude large model binaries and document post-install prefetch | ✅ Done |
