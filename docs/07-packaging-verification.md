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

### Local verification blocker (as of this worktree)

The local development host used for this lane has:

- Active toolchain: `stable-x86_64-pc-windows-gnu`
- The `x86_64-pc-windows-msvc` toolchain is installed but `link.exe` (MSVC
  linker, part of Visual Studio Build Tools) is **not present**.
- Error observed: `error: linker 'link.exe' not found — the msvc targets depend
  on the msvc linker but link.exe was not found`

**Consequence:** A full release build against the msvc target cannot be
completed on this local host. The static-linking flag is in place; proof must
be produced in a CI environment (for example GitHub Actions on
`windows-latest`) that has the Windows SDK and Build Tools installed, or on a
developer machine with Visual Studio 2019 / 2022 or the "Build Tools for Visual
Studio" package.

The repository CI workflow already runs on `windows-latest`, which normally
provides the MSVC toolchain and linker. The blocker described here is specific
to the local workstation used during this implementation pass.

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

## Checklist

| Check | Status |
|-------|--------|
| `.cargo/config.toml` sets `+crt-static` for msvc target | ✅ Done |
| Build command documented | ✅ Done |
| Dependency verification method documented | ✅ Done |
| Local MSVC build blocker recorded with exact error | ✅ Recorded |
| `config_json_path()` uses `current_exe().parent()` | ✅ Confirmed portable |
| No absolute paths found in source | ✅ Audited clean |
| USAGE.md documents run-from-any-folder behaviour | ✅ Done |
