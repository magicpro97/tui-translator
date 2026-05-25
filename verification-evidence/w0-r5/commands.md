# w0-r5-family-file-map — commands log

Read-only research tentacle. No `src/**`, docs, Cargo files, GitHub issues, or
issue comments were modified.

## Inputs consumed

- `verification-evidence/board-snapshot-20260524.json`
  SHA256: `290FB421CDAC960618640169CD36876621ECA9F8E328AF876C3280B7EC19949E`
- `verification-evidence/w0-r4/low-confidence-resolution.json`
- `verification-evidence/board-count-rest-20260524.json`
- `verification-evidence/board-count-graphql-20260524.json`
  Issue-set SHA256: `3ACD5BFD9051F2E9E01AA9B7F991D26F69E05E9D45B299A261F297C1FA6ED1BC`

## Commands

```powershell
# Hash the W0-R1 snapshot to lock it as the R5 input.
Get-FileHash verification-evidence\board-snapshot-20260524.json -Algorithm SHA256

# Enumerate every Todo-open row's per-issue files_touched_hint:
node -e "const d=JSON.parse(require('fs').readFileSync('verification-evidence/board-snapshot-20260524.json','utf8'));for(const r of d.rows){console.log(r.number+'|'+r.family+'|'+(r.files_touched_hint||[]).join(';')+'|'+r.confidence+'|HG='+r.human_gate);}"

# Enumerate titles to derive family tokens for rows where snapshot has family='unknown':
node -e "const d=JSON.parse(require('fs').readFileSync('verification-evidence/board-snapshot-20260524.json','utf8'));for(const r of d.rows){console.log(r.number+'|'+r.title);}"

# Enumerate concrete src/**.rs files that actually exist today (sanity-check ownership claims):
Get-ChildItem src -Recurse -File -Filter *.rs | ForEach-Object { $_.FullName.Replace((Get-Location).Path+'\','').Replace('\','/') }

# Validate the produced artifact parses as JSON and report shape:
node -e "const d=JSON.parse(require('fs').readFileSync('verification-evidence/family-file-map.json','utf8'));console.log('issues:',d.issues.length,'shared_paths:',d.shared_paths.length,'blocked:',d.blocked_collisions.length,'readiness:',d.readiness);"
```

## Method

1. Each of the 82 Todo-open issues was assigned a family token derived from
   the title prefix (`WP-19`, `WP-20..23`, `VERIF-OPS`, `LF`, `DM`, `HC`,
   `SM`, `MACOS-WBS`, `CTRL`, `MODEL`, `QA`, `TEST`, `CI`, `SEC`, `REL`,
   `LINUX-WBS`, `PROC`, `XPLAT`, `PARITY`, `UX`, `I18N`, `CFG`, `STD`,
   `SUPERTONIC`, `QA8`).
2. `files_touched_hint` values were taken from the W0-R1 snapshot row and,
   where the W0-R4 audit recorded a more specific list (issues #366,
   #450–#463), the R4 values were used instead.
3. Hints were classified as one of:
   - concrete repo path (file exists today OR is an explicit new file);
   - broad glob (e.g. `src/audio/**`, `src/providers/**`) — treated as
     planning-stage touchpoint, not an ownership claim;
   - evidence path (`verification-evidence/**`) — additive per-issue
     evidence, never collides.
4. Every concrete `src/**` path was assigned exactly one owning family. Where
   multiple issues or families touch the same concrete path, an explicit
   serialization rule was recorded in `shared_paths` and indexed in
   `collision_resolution`.
5. `Cargo.toml` and `.github/workflows/*.yml` were given additive /
   serialized merge rules rather than exclusive owners because they are
   merge-friendly manifest files.

## Output

- `verification-evidence/family-file-map.json` — R5 artifact
  (schema_version 1, 82 issues, 14 shared-src serialized paths, 0 blocked
  collisions, readiness=true with caveats).

## Outputs NOT produced

- No edits to `src/**`, `docs/**`, `Cargo.toml`, GitHub issue bodies, issue
  labels, comments, or project state.
- No edits to `verification-evidence/board-snapshot-20260524.json` — the W0-R5
  research did not require adding factual W0-R5 notes to the snapshot.
