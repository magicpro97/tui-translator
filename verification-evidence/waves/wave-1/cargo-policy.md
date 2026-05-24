# Wave 1 — Cargo.toml / Cargo.lock Policy (Arbiter)

> Author: Opus arbiter.
> Question: May W1 implementation tentacles edit `Cargo.toml` or `Cargo.lock`?

## Ruling: **NO** in Wave 1.

`Cargo.toml` and `Cargo.lock` are **not present** in
`verification-evidence/waves/wave-1/files_allowed.txt`. Under R8 + Gate Zero,
that absence is dispositive: a W1 implementation tentacle MUST NOT modify
either file. There is no "additive caveat" that overrides allow-list closure
in W1.

## Why no extension at this time

Planner A floated an additive-only allow-list extension. Arbiter declines:

1. Gate Zero says allow-lists are closed unless an arbiter ruling backed by
   explicit acceptance evidence extends them. The acceptance matrix is
   missing (see `dispatch-groups.md §Blockers`), so the evidence threshold
   is not met.
2. None of the **non-downgraded** W1 issues (#499, #501, #502, #505, #506,
   #509, #510, and the non-QA8 docs/evidence/workflow issues) require a new
   dependency. Specifically:
   - `#501 snapshot.rs` extension uses the already-vendored `serde` /
     `serde_json` / `sysinfo` crates.
   - `#502 process.rs` already pulls platform handle/FD/thread metrics via
     `sysinfo` (present) plus stdlib; on Windows the existing `windows-sys`
     dep covers GDI/handle counters.
   - `#505 loss.rs`/`network.rs` use stdlib + `tokio` (present).
   - `#506 memory_guard.rs` at the downgraded scope (OOM watcher + panic
     hook) needs only `sysinfo` + stdlib `std::panic`. **Crash-dump capture
     and symbolication are deferred to QA8-08b precisely because they need
     new deps** (`minidumper` / `crash-handler`); deferring keeps W1
     dependency-clean.
3. CI-01 (#461) and the two workflow issues (#509, #510) edit YAML only.

## How agents must handle "I need a new dep" in W1

If a W1 implementation tentacle discovers it needs a crate not already in
`Cargo.toml`:

1. **Stop**. Do not edit `Cargo.toml`.
2. Emit a `verification-evidence/waves/wave-1/dep-request-<issue>.md`
   documenting:
   - the crate, version, and feature flags requested,
   - the precise code path that requires it,
   - the alternative attempted (stdlib / vendored crate) and why it failed.
3. Open or update the issue with a `needs-dep` label and stop the
   implementation attempt for that issue.
4. The arbiter (or wave planner) then either:
   - confirms a downgrade keeps W1 dep-clean (preferred), or
   - opens a successor issue carrying the dep into a later wave whose
     allow-list explicitly lists `Cargo.toml` and `Cargo.lock`.

## Forward-looking rule

If a future wave needs new dependencies, the wave-planner MUST list both
`Cargo.toml` and `Cargo.lock` in that wave's `files_allowed.txt`, and the
implementation tentacle MUST follow these gates **in that wave**:

- Additive-only: no `version` downgrades, no feature removals from existing
  entries, no profile changes.
- One dep per issue unless the issue title explicitly bundles deps.
- `cargo tree --duplicates` MUST show no new duplicate-major-version
  conflicts vs the pre-edit baseline.
- `cargo deny check` MUST pass with the current `deny.toml`.
- `cargo audit` MUST report no new vulnerabilities for the added crate
  family.
- License gate: new crate's SPDX license MUST be on the existing allow
  list in `deny.toml` (currently MIT, Apache-2.0, BSD variants, ISC,
  Unicode-DFS).

None of those gates apply to W1 because Cargo.toml is off-limits in W1.
