# Wave 1 — Semgrep Plan (Arbiter, R8 wave-close gate)

> Author: Opus arbiter.
> Purpose: Satisfy R8 — every W1 src/** touch needs either a semgrep run
> or an explicit waiver before wave-close.

## 1. W1 src/** touches that trigger this gate

Per `files_allowed.txt`, the src/** paths editable in W1 are:

```
src/audio/file_source.rs                  # issue #460
src/bin/audio_stability_proof.rs          # issue #503  (currently blocked)
src/metrics/loss.rs                       # issue #505
src/metrics/memory_guard.rs               # issue #506  (downgraded scope)
src/metrics/network.rs                    # issue #505
src/metrics/process.rs                    # issue #502
src/metrics/snapshot.rs                   # issue #501
```

R8 applies to any of these files whose post-implementation SHA differs from
the pre-implementation SHA recorded in `baseline-hashes.json`.

## 2. Primary command (use first)

```powershell
# Run from repo root on Windows PowerShell.
# Uses local rule packs only — does NOT contact registry (avoids the
# SSL/CERTIFICATE_VERIFY_FAILED failure already observed in
# verification-evidence\semgrep-current.txt).

# Step 0 — One-time: cache rule packs locally if not already present.
# Maintainer runs this once with network access; commits the cache.
#   semgrep --config p/rust --config p/secrets --dump-config > .semgrep/rust-secrets.yml
# Cache lives at: .semgrep/rust-secrets.yml  (repo-tracked)

# Step 1 — Wave-close run, src/** scoped to W1 touches.
$W1_TOUCHED = @(
  'src/audio/file_source.rs',
  'src/bin/audio_stability_proof.rs',
  'src/metrics/loss.rs',
  'src/metrics/memory_guard.rs',
  'src/metrics/network.rs',
  'src/metrics/process.rs',
  'src/metrics/snapshot.rs'
) | Where-Object { Test-Path $_ }

semgrep --config .semgrep/rust-secrets.yml `
        --error `
        --metrics off `
        --json `
        --output verification-evidence/waves/wave-1/semgrep-wave-close.json `
        $W1_TOUCHED

# Step 2 — Human-readable summary alongside the JSON.
semgrep --config .semgrep/rust-secrets.yml `
        --metrics off `
        --output verification-evidence/waves/wave-1/semgrep-wave-close.txt `
        $W1_TOUCHED
```

Exit-code policy: `--error` makes any finding fail the wave-close hook.
The orchestrator publishes both JSON and TXT artifacts into
`verification-evidence/waves/wave-1/` before closing the wave.

## 3. Fallback command (if `.semgrep/rust-secrets.yml` is missing)

If the local rule pack is not yet cached, attempt registry pull once with
the system CA bundle:

```powershell
$env:REQUESTS_CA_BUNDLE = "$env:USERPROFILE\AppData\Local\Programs\Python\Python312\Lib\site-packages\certifi\cacert.pem"
$env:SSL_CERT_FILE      = $env:REQUESTS_CA_BUNDLE

semgrep --config p/rust --config p/secrets `
        --error --metrics off --json `
        --output verification-evidence/waves/wave-1/semgrep-wave-close.json `
        $W1_TOUCHED
```

If this also fails (SSL or offline), proceed to the waiver template below.

## 4. Waiver template (use only if both 2 and 3 fail)

If semgrep cannot run, the wave-close gate accepts the following waiver
**only** when ALL three conditions hold:

1. The failure is environmental (network/SSL/registry), not a rule-engine
   crash on actual W1 code.
2. `cargo clippy --all-targets --all-features -- -D warnings` passes on W1
   touches.
3. `cargo deny check` passes (no new advisories or license violations).

Drop the following file into the wave-close evidence directory:

```markdown
<!-- verification-evidence/waves/wave-1/semgrep-waiver.md -->
# Wave 1 — Semgrep Waiver

Reason for waiver (pick one and justify):
- [ ] Registry unreachable (SSL/network) — attach stderr from attempted run.
- [ ] Rule pack cache absent and offline environment.
- [ ] Other (explain in detail).

Compensating evidence:
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` — link to CI run / paste output.
- [ ] `cargo deny check` — link / paste.
- [ ] Hand-grep for the high-signal patterns below across W1 touches:
      - `unsafe ` blocks (justify each new one).
      - `unwrap()` / `expect(` outside tests and main (forbidden per repo conventions).
      - Hard-coded secrets (`api_key`, `password`, `Bearer `, `AKIA`, `-----BEGIN`).
      - `std::process::exit` (forbidden per repo conventions).
      - `println!` in non-test src/** (forbidden — use `tracing`).
- [ ] Reviewer sign-off: <name> <date>

Files covered by this waiver:
- <list each W1-touched src/** file with its post-impl SHA>

Successor action: open a follow-up issue to restore the semgrep gate
(install rule cache, fix SSL trust store, etc.) within the next wave.
```

The waiver must be reviewed by the wave-close reviewer. A waiver is
**single-wave**; W2 must re-run semgrep or re-submit a fresh waiver.

## 5. Gate Zero hand-off

The orchestrator's wave-close hook MUST:

- Verify `verification-evidence/waves/wave-1/semgrep-wave-close.json` exists
  AND its `results` array is empty (no findings) AND `errors` array is
  empty, OR
- Verify `verification-evidence/waves/wave-1/semgrep-waiver.md` exists AND
  is signed by a reviewer.

Either condition satisfies R8. Absence of both blocks wave-close.
