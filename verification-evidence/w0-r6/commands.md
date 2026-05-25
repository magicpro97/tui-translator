# w0-r6-wave-plan — commands log

Planning-only tentacle. No `src/**`, docs, Cargo files, GitHub issues, or
issue comments were modified.

## Inputs consumed

- `verification-evidence/board-snapshot-20260524.json`
  SHA256: `893A3C52AB12BFE563EDD968CA417061818E651BBE6DA294CE043FFE588BBF92`
- `verification-evidence/family-file-map.json`
  SHA256: `B36CC3F3FCD63F2E002A266911AF78998B9F8D03B152FD233D62291BB3E7E611`
- `verification-evidence/w0-r4/low-confidence-resolution.json`
  SHA256: `1F959788667C48A2FC8EBAEC1C979251A1C0AA8F3133B884A871681A11DAAFF7`
- `verification-evidence/w0-r5/commands.md`
  SHA256: `5E367CF4BF074D17F2CE6BEA655DF0AE63AFAE007B621AC97C12992FAA3D215D`

> Both `board-snapshot-20260524.json` and `family-file-map.json` are kept
> trackable via explicit `!`-exceptions in `.gitignore` so the wave-plan
> driver below is reproducible from a fresh PR checkout (the generic
> `verification-evidence/*.json` ignore rule would otherwise strip them).
> The `family-file-map.json` hash above corresponds to the version where
> the placeholder syntax for the release-candidate tag was migrated from
> `<rc-tag>` (invalid on Windows paths) to `{RC_TAG}`; see PR #512 review
> remediation for the rationale.

## R5 readiness caveats — disposition

R5's `family-file-map.json` set `readiness=true` but flagged five caveats. R6
ratifies each into an explicit serialization rule rather than escalating
BLOCKED:

1. **Cross-family shared `src/**` orderings must be ratified.** R6 detected
   three cycles produced by R5's per-path orderings:
   - `src/pipeline/playback.rs` vs `src/config/mod.rs` (#454 vs #456).
   - `src/providers/mod.rs` vs `src/config/mod.rs` (#455 vs #457).
   - `.github/workflows/ci.yml` vs `.github/workflows/release.yml` (#458 vs
     #462).

   For each cycle R6 picked the canonical ordering using R5's own narrative
   rules (CTRL-03 invariant first on playback; MODEL-01 contract first on
   providers/mod; SEC-01 SBOM scaffolding first on workflows) and
   re-sequenced the conflicting chain. See `wave-plan.json` →
   `ratifications` for the full justification per cycle.

2. **Wave P (planning-stage) issues need WBS decomposition before
   implementation dispatch.** Ratified: the 16 planning-stage issues are
   placed in `Wave P` with `implementation_dispatch_allowed: false` and
   `decomposition_required: true`.

3. **`Cargo.toml` additive-merge rule.** Ratified verbatim; recorded in
   `wave-plan.json` → `carried_caveats`.

4. **`src/tui/frame_pacer.rs` writer-reader split (UX-01 writes,
   QA8-06 reads).** Ratified: R6 adds an explicit logical edge
   `504 → 479` so QA8-06 lands in a strictly later wave than UX-01.

5. **STD-02 (#484) last-in-line.** Ratified: #484 is assigned to
   `Wave F` (final / release-train), held until all src-touching
   tentacles for the affected files have merged.

## Commands

```powershell
# Hash all inputs to lock provenance.
Get-FileHash verification-evidence\board-snapshot-20260524.json -Algorithm SHA256
Get-FileHash verification-evidence\family-file-map.json -Algorithm SHA256
Get-FileHash verification-evidence\w0-r4\low-confidence-resolution.json -Algorithm SHA256
Get-FileHash verification-evidence\w0-r5\commands.md -Algorithm SHA256

# Build the plan deterministically. The script is the source of truth;
# wave-plan.json is regeneratable.
node verification-evidence\w0-r6\build-wave-plan.js

# Verify the plan loads, counts add up, and no cycle remains.
node -e "const p=JSON.parse(require('fs').readFileSync('verification-evidence/wave-plan.json','utf8'));console.log('readiness:',p.readiness,JSON.stringify(p.summary));"
```

## Method

1. **Predecessor graph.** For every shared path with a serialized writer
   list (after R6 cycle-break ratification), insert an edge from each
   earlier writer to every later writer. Add five logical edges:
   - `504 → 479` (QA8-06 reads UX-01's frame_pacer schema).
   - `450 → {451,452,453}` (MACOS-01 spike precedes implementation).
   - `468 → {469,470,471,472}` (LINUX-01 spike precedes implementation).
   - `486 → {487..497}` (SUPERTONIC-01 spike precedes evaluation +
     implementation).

2. **Wave depth.** Toposort: `wave(n) = 1 + max(wave(p) for p in preds(n))`.
   Issues with no predecessors land in Wave 1. Highest wave = W14.

3. **Classification overrides.**
   - 10 human-gated issues (`board.human_gate == true`) → `Wave H`,
     `implementation_dispatch_allowed: false`.
   - 16 non-owning planning/epic issues from
     `family-file-map.non_owning_issues.issues` → `Wave P`,
     `decomposition_required: true`, `implementation_dispatch_allowed:
     false`. (`#484` STD-02 was originally in this list but is reassigned
     to Wave F per R5 caveat #5.)
   - `#484` → `Wave F` (final / release-train).

4. **`red_mode` policy assignment.** Per issue:
   - `tests_first` if it touches any `src/**`.
   - `workflow_dry_run` if it touches any `.github/workflows/**` and no
     `src/**`.
   - `doc_first` if it touches `docs/**` or `PRIVACY.md` and no `src/**`.
   - `evidence_first` if it only writes `verification-evidence/**`.
   - `human_acceptance_log` for Wave H.
   - `decomposition` for Wave P and Wave F.

5. **`human_gate_prereqs`.**
   - `required: true` for Wave H issues; lists named-reviewer (#366) and
     acceptance-log-signed (#122) gates.
   - `co_sign_required: true` for AI issues that touch
     `.github/workflows/release.yml`, `packaging/`, `PRIVACY.md`, or
     `deny.toml`.
   - `required: false` for all other AI issues.

6. **`files_allowed` per issue / per wave.** Union of `owned_paths` +
   `shared_paths_in[].path` + `evidence_paths`. Per-wave allow-list is the
   union of its members'. Tests under `tests/**` and benches under
   `benches/**` are implicitly allowed for `tests_first` issues (noted in
   each `files_allowed.txt` header).

## Outputs produced

- `verification-evidence/wave-plan.json` — single canonical plan (schema
  version 1, 82 issues, 17 waves: 14 implementation + Wave H + Wave P +
  Wave F, readiness=true).
- `verification-evidence/waves/wave-{1..14,H,P,F}/files_allowed.txt` —
  closed allow-list per wave with header notes.
- `verification-evidence/waves/wave-{1..14,H,P,F}/baseline-hashes.json` —
  placeholder schema. Pre-implementation, the wave-start hook MUST
  populate `{path, sha256_pre}` for every file that exists on `main` HEAD
  at wave-start time. Post-implementation, the tentacle records
  `sha256_post` and the wave-close hook diffs.
- `verification-evidence/waves/wave-{...}/wave-manifest.json` — per-wave
  view of issues, dependencies, files_allowed, red_mode, and human-gate
  prereqs.
- `verification-evidence/w0-r6/build-wave-plan.js` — deterministic builder
  (`node` re-runs reproduce identical output).
- `verification-evidence/w0-r6/commands.md` — this file.

## Outputs NOT produced

- No edits to `src/**`, `docs/**`, `Cargo.toml`, `Cargo.lock`,
  `.github/workflows/**`, GitHub issues, issue labels, project state, or
  comments.
- No edits to any prior verification-evidence input
  (`board-snapshot-20260524.json`, `family-file-map.json`,
  `w0-r4/**`, `w0-r5/**`).
- No commits and no pushes.

## Summary

| Bucket | Count |
|--------|-------|
| Total in-scope issues | 82 |
| Wave H (human-gated)  | 10 |
| Wave P (planning, decomposition required) | 16 |
| Wave 1..14 (AI implementation, dispatch allowed) | 55 |
| Wave F (final refactor, release-train hold) | 1 |
| **AI-actionable subtotal** | **72** |

Readiness: `true`. No cycles, no unassigned issues, no Wave P/F issues
allow implementation dispatch.
