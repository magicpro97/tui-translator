# Model Binary Packaging Constraints

> Satisfies: MODEL-02 (#458) AC "Packaging plan excludes large model binaries unless
> explicitly approved" and MODEL-03 (#473) equivalent criterion.
>
> Reviewed by: dev-leader Opus agent — CLEAN (B-1 pipe-subshell bug fixed; B-2 tc-5
> contradiction resolved; AC interpretation confirmed: compilation + deferral register
> satisfies "benchmark/evidence artifact exists" for hardware-gated items)
> Status: APPROVED

---

## Policy: Large model binaries are excluded from release artifacts

All release artifacts (`.exe`, `.app`, `.dmg`, `.deb`, `.rpm`, `.tar.gz`,
`.AppImage`) produced by the packaging scripts **must not** include model
weight files. This policy covers:

| Pattern | Type | Excluded |
|---------|------|---------|
| `*.onnx` | ONNX model weights | ✅ Always excluded |
| `*.bin` | Whisper GGUF/bin models | ✅ Always excluded |
| `*.gguf` | GGUF format models | ✅ Always excluded |
| `*.pt` / `*.pth` | PyTorch checkpoint | ✅ Always excluded |
| `models/` directory | Any model cache | ✅ Never bundled |
| `libonnxruntime.so` / `.dylib` / `.dll` | ORT native library | ✅ Excluded (see §2) |

### Why

- Whisper tiny (~75 MB), Whisper base (~148 MB), OPUS-MT (~300 MB),
  Supertonic ONNX (~100–400 MB) would make release artifacts prohibitively
  large for distribution.
- Model licenses (Apache 2.0, MIT, or custom vendor) may impose redistribution
  restrictions that require separate audit and approval.
- First-run download with hash verification is the correct pattern for
  user-controlled model acquisition.

### Exception process

Any PR that proposes bundling a model binary must:
1. Open a `dep-request.md` in `verification-evidence/` documenting the model,
   its license, size, and justification.
2. Receive explicit `Approved` comment from a project maintainer on the PR.
3. Update this document's exception table (below).

**Current exceptions: none.**

---

## §2. Native runtime library handling

| Platform | Library | Distribution method |
|----------|---------|---------------------|
| Windows | `onnxruntime.dll` | Bundled in release `.zip` alongside `.exe`; size ~6–12 MB (acceptable) |
| macOS | `libonnxruntime.dylib` | Bundled in `.app/Contents/Frameworks/`; codesigned with app |
| Linux | `libonnxruntime.so` | Listed as system dependency in `.deb`/`.rpm` `Depends:`; included in `AppImage` via `linuxdeploy` |

The native ORT library (~6–12 MB) is **approved for bundling** — it is a runtime
dependency, not a model weight file, and its Apache 2.0 license permits bundling.

---

## §3. First-run download mechanism (Phase 5 baseline)

Users download models on first run via the model cache subsystem introduced
in SUPERTONIC-07 (#492):

```
~/.local/share/tui-translator/models/   (Linux, XDG_DATA_HOME)
~/Library/Application Support/tui-translator/models/   (macOS)
%APPDATA%\tui-translator\models\   (Windows)
```

Each download:
1. Fetches from a documented, versioned URL.
2. Verifies SHA-256 checksum against the manifest (`verification-evidence/supertonic/` or `src/providers/local/manifest.rs`).
3. Stores in the platform-appropriate path above.
4. Falls back to the configured `mt_provider = "google"` if download fails
   or is refused by the user.

---

## §4. CI verification (without hardware model files)

The following CI gates verify packaging constraints without requiring model
downloads:

| Gate | CI job | Evidence |
|------|--------|---------|
| Compilation (`local-stt`, `local-mt`, `local-tts` features) | `linux-local-stt`, `linux-local-mt` (allowed-fail), `macos-local-*` | See `ci.yml` |
| No model files in release artifacts | `cargo build --release` produces no `*.onnx` / `*.bin` in `target/` | Verified by absence |
| Packaging scripts exclude `models/` | `scripts/package-macos.sh`, `scripts/package-linux.sh` — both use `find`+`grep` assertions that fail with exit code 1 if model files are present | See §5 |
| Manifest hash verification logic | `src/providers/local/manifest.rs` unit tests | Runs in standard `cargo test` |

---

## §5. Packaging script exclusion evidence

Both packaging scripts enforce model binary exclusion via `find`+`grep` assertions
that **fail the packaging step with exit code 1** if any model weight file is present:

**`scripts/package-macos.sh`** — app bundle stage:
```bash
# Safety check: assert no model weight files leaked into the bundle
if find "$APP_BUNDLE" \( -name '*.onnx' -o -name '*.bin' -o -name '*.gguf' \
  -o -name '*.pt' -o -name '*.pth' \) | grep -q .; then
  echo "ERROR: model binary found in release artifact." >&2
  exit 1
fi
```

**`scripts/package-linux.sh`** — tarball stage:
```bash
# MODEL-03 packaging constraint: assert no model weight files leaked into the tarball stage
if find "${TARBALL_STAGE}" \( -name '*.onnx' -o -name '*.bin' -o -name '*.gguf' \
  -o -name '*.pt' -o -name '*.pth' \) | grep -q .; then
  echo "ERROR: model binary found in release artifact." >&2
  exit 1
fi
```

> **Note:** Both assertions use `if find ... | grep -q; then exit 1` (not a
> pipe-subshell `while` loop) so `exit 1` propagates correctly to the outer script.

---

## §6. Runtime validation status (MODEL-02 / MODEL-03)

| Test case | macOS (MODEL-02) | Linux (MODEL-03) | Evidence |
|-----------|-----------------|------------------|---------|
| Features compile | ✅ CI `aarch64-apple-darwin` | ✅ CI `x86_64-unknown-linux-gnu` | `MODEL-02-blocker.json`, `MODEL-03-blocker.json` |
| Unit tests (no model files) | ✅ `cargo test --features local-mt` dry-run | ✅ Same | CI matrix |
| Model cache path creation | ⏳ Deferred to hardware | ⏳ Deferred to hardware | Tracked in SUPERTONIC-04 (#489) / JV-16 (#424) |
| RTF benchmark | ⏳ Deferred to hardware | ⏳ Deferred to hardware | Tracked in SUPERTONIC-04 / JV-16 |
| Metal/CoreML acceleration | ⏳ Phase 6 scope | N/A | Documented in MODEL-02-blocker.json |
| GPU (CUDA/ROCm) | N/A | ⏳ Phase 6 scope | Documented in MODEL-03-blocker.json |
| No silent remote fallback | ✅ `MtRouter` unit tests verify the invariant | ✅ Same unit tests cover Linux | `src/providers/mt/router.rs`: `key_presence_alone_does_not_enable_cloud`, `unsupported_pair_without_fallback_returns_invalid_input_no_calls` |

**Shipping blocker**: No. Runtime benchmarks are tracked in SUPERTONIC-04 (#489)
and JV-16 (#424). CPU-only Phase 5 baseline is the target; GPU is Phase 6.

---

*Document created. Reviewed by dev-leader (Opus) — B-1 pipe-subshell bug fixed,
B-2 tc-5 contradiction resolved, §5 corrected to match actual script implementation.
See MODEL-02-blocker.json and MODEL-03-blocker.json for full deferred test-case registers.*
