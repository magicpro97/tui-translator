# Windows crash-dump, panic, and symbolication workflow

> Roadmap marker: **QA8-08** (issue #506).
> This document is the operational runbook for collecting and reading
> crash evidence from a Windows tui-translator install. The kernel-level
> minidump is produced by Windows Error Reporting (WER); the panic
> sidecar and rolling log are produced by `tui-translator` itself.

## 1. What evidence is captured

| Artefact | Producer | Location | When written |
|----------|----------|----------|--------------|
| `panic-<unix_ms>-<pid>.json` | tui-translator panic hook | `%LOCALAPPDATA%\tui-translator\dumps\` | Any Rust panic |
| `panic-log.txt` (+ rotated `.old`) | tui-translator panic hook | same | Any Rust panic |
| `tui-translator.exe.<pid>.dmp` | Windows Error Reporting | `%LOCALAPPDATA%\CrashDumps\` | Unhandled SEH exception (e.g. 0xc0000409 stack-cookie, 0xc0000005 AV), abort, stack overflow |
| `tui-translator.log` (+ rolled) | tracing-subscriber | `%TEMP%` | Always; last-N tracing spans available alongside the dump |
| `tui-translator.pdb` | MSVC linker (release CI) | GitHub Release asset (separate from the user-facing zip) | Built per release tag |

The Rust panic sidecar is a normalised JSON node consumable by the
`crash-root-cause` agent. It is intentionally **separate** from the WER
minidump because pure-Rust panics (e.g. `unwrap()` in a Tokio task) do
not always trigger WER, while WER captures the C/C++ side
(WASAPI/ORT/whisper.cpp) that Rust panic hooks cannot see.

## 2. Override the dump directory

```powershell
# Override at process startup (per-session)
$env:TUI_TRANSLATOR_DUMP_DIR = "D:\soak-evidence\dumps"
.\tui-translator.exe

# Or persist for the current user
[Environment]::SetEnvironmentVariable(
  "TUI_TRANSLATOR_DUMP_DIR",
  "D:\soak-evidence\dumps",
  "User"
)
```

The variable accepts any absolute path. The directory is created if
missing. Blank / whitespace values are ignored and the default
`%LOCALAPPDATA%\tui-translator\dumps\` is used instead.

## 3. Enable Windows Error Reporting LocalDumps

WER LocalDumps writes a per-process minidump on any crashing termination
(SEH unhandled exception, abort, fail-fast). Configure it **once per
machine** before running a soak or a release-candidate validation:

```powershell
# Run from an elevated PowerShell.
$key = "HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\tui-translator.exe"
New-Item -Path $key -Force | Out-Null
New-ItemProperty -Path $key -Name "DumpFolder"  -Value "$env:LOCALAPPDATA\CrashDumps" -PropertyType ExpandString -Force | Out-Null
New-ItemProperty -Path $key -Name "DumpCount"   -Value 20 -PropertyType DWord -Force | Out-Null
# DumpType 2 = full memory dump. Use 1 (mini) on size-constrained machines.
New-ItemProperty -Path $key -Name "DumpType"    -Value 2  -PropertyType DWord -Force | Out-Null
```

Verify dumps are appearing after a forced crash:

```powershell
Get-ChildItem $env:LOCALAPPDATA\CrashDumps\tui-translator.exe.*.dmp |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 5 Name, Length, LastWriteTime
```

## 4. Force a panic in a controlled environment

To exercise the Rust panic sidecar pathway without a real crash, run
the application with a deliberate panic via cargo (development only):

```powershell
cd C:\path\to\tui-translator
$env:RUST_BACKTRACE = "full"
$env:TUI_TRANSLATOR_DUMP_DIR = "$pwd\verification-evidence\panic-sentinel"
cargo run --bin tui-translator -- --intentionally-panic-for-qa
```

Even if the CLI does not expose an `--intentionally-panic-for-qa` flag,
any panic raised by the application during development writes a
sidecar. Inspect the result:

```powershell
Get-ChildItem $env:TUI_TRANSLATOR_DUMP_DIR
Get-Content $env:TUI_TRANSLATOR_DUMP_DIR\panic-log.txt
```

A sample sidecar (formatted):

```json
{
  "kind": "tui-translator.panic",
  "app_version": "0.1.4",
  "timestamp_unix_ms": 1730000000123,
  "pid": 14820,
  "thread": "tokio-runtime-worker",
  "location": "src/pipeline/orchestrator.rs:412:9",
  "message": "stt window flushed before initialisation",
  "backtrace": "<scrubbed stack frames>"
}
```

`google_api_key` shaped values and the literal `google_api_key=...` /
`"google_api_key":"..."` forms are replaced with `[REDACTED]` before the
sidecar reaches disk.

## 5. Symbolicate a WER minidump

### 5.1 With WinDbg / cdb

1. Download the matching `tui-translator-<tag>-pdb.zip` symbol bundle
   from the GitHub Release page and extract `tui-translator.pdb` to a
   local folder, e.g. `C:\symbols\tui-translator\<tag>\`.
2. Open the dump:
   ```powershell
   & "C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\cdb.exe" `
     -z "$env:LOCALAPPDATA\CrashDumps\tui-translator.exe.12345.dmp" `
     -y "C:\symbols\tui-translator\<tag>;srv*C:\symbols\ms*https://msdl.microsoft.com/download/symbols" `
     -c "!analyze -v; ~*kn; q"
   ```
3. The `!analyze -v` output prints the exception code (e.g.
   `0xc0000409` STATUS_STACK_BUFFER_OVERRUN), the faulting frame, and
   the suggested bucket. `~*kn` prints every thread stack — paste the
   tui-translator frames into the issue evidence.

### 5.2 With minidump-stackwalk (rust-minidump)

For CI symbolication without a Microsoft toolchain:

```powershell
# One-time setup (cargo install does not require admin)
cargo install --locked minidump-stackwalk
cargo install --locked dump_syms

dump_syms tui-translator.pdb > tui-translator.sym
# Lay out a Breakpad symbol store
$store = "C:\sym-store"
$mod   = (Select-String -Path tui-translator.sym -Pattern '^MODULE ').Line.Split(' ')
$dir   = Join-Path $store (Join-Path $mod[4] $mod[3])  # name\hash
New-Item -ItemType Directory -Force -Path $dir | Out-Null
Move-Item tui-translator.sym (Join-Path $dir "tui-translator.sym")

minidump-stackwalk --symbols-path=$store `
  "$env:LOCALAPPDATA\CrashDumps\tui-translator.exe.12345.dmp"
```

The walker emits JSON (use `--json`) or human-readable backtraces ideal
for attaching to a GitHub issue.

## 6. Scrub before sharing evidence

`tui-translator` already scrubs Google API keys from the sidecar JSON.
The WER minidump can still contain process-memory secrets. Before
attaching a `.dmp` to a public issue:

```powershell
# Strip the full-memory dump down to a minidump (no heap memory)
# This makes the file smaller and removes most heap-resident secrets.
$src = "$env:LOCALAPPDATA\CrashDumps\tui-translator.exe.12345.dmp"
$dst = "C:\evidence\tui-translator-stack-only.dmp"
& "$env:WINDIR\System32\Procdump.exe" -ma -mp $src $dst
```

Prefer the panic sidecar (`panic-*.json`) for public attachments; share
the full minidump only via private channels.

## 7. CI / release packaging

* `cargo build --release` on Windows MSVC produces `tui-translator.pdb`
  next to `tui-translator.exe` even with `strip = true` (which only
  strips line-info from the executable). The `release` workflow uploads
  the PDB as a GitHub Release asset (`tui-translator-<tag>-pdb.zip`)
  separate from the user-facing zip, so end users still download a
  small archive but support engineers have everything needed for
  symbolication.
* The CI build also stores the PDB as a workflow artefact so commits
  preceding the tag can still be symbolicated.

## 8. Acceptance evidence

A successful QA8-08 evidence bundle contains:

* A `panic-*.json` sidecar produced by the panic hook during a forced
  panic.
* A WER minidump produced by an SEH exception (e.g. divide-by-zero
  process probe).
* A `!analyze -v` or `minidump-stackwalk` text output naming a Rust
  function in the faulting frame, proving symbolication works.
* A confirmation that `panic-log.txt` contains the matching summary
  line.

Drop the four artefacts into
`verification-evidence/qa8/QA8-08-<rc-tag>/` and reference them from
the issue closure comment.
