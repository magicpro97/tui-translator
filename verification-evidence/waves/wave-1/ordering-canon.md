# Wave 1 — Ordering Canon (Arbiter ruling)

> Author: Opus arbiter (Wave 1 dispatch arbitration).
> Scope: Resolves the apparent conflict between `wave-manifest.json`
> (`"dependencies": []` for every W1 issue) and `wave-plan.json`
> (`serialization_rules.path_orders` + `extra_logical_deps`).

## 1. Authoritative source

The canonical source for **intra-wave ordering** is:

```
verification-evidence/wave-plan.json
  ├── serialization_rules.path_orders     ← per-file write order across waves
  ├── serialization_rules.extra_logical_deps  ← named logical constraints
  └── dependency_dag.edges                ← cross-issue dependency graph
```

The `wave-manifest.json` `dependencies` field is **NOT** the source of truth
for intra-wave ordering. Its semantics (confirmed by the wave-plan's
`ratifiedChains` rationale and `cycle_breaks` decisions) are:

> `wave-manifest.dependencies` lists **cross-wave blocking prerequisites only**
> (issues in an earlier wave that must merge before this issue dispatches).
> An empty list means "no cross-wave blocker." It does **not** assert that
> intra-wave issues can run in arbitrary order.

Therefore there is **no real conflict**: the manifest is silent on intra-wave
order; the wave-plan supplies it. When the two disagree in interpretation,
**wave-plan wins**.

## 2. W1 application — QA8 metrics chain

### 2.1 What the canon says directly

From `wave-plan.json → serialization_rules.path_orders`:

| File                              | Order (issues, left→right = earlier→later) |
|-----------------------------------|--------------------------------------------|
| `src/metrics/snapshot.rs`         | `[501, 511]`  (511 is W2; W1 writer = 501 only) |
| `src/audio/file_source.rs`        | `[460, 507]`  (507 is W2; W1 writer = 460 only) |
| `.github/workflows/ci.yml`        | `[461, 462, 458, 475, 477, 508]` (W1 writer = 461 only) |

Every other W1-touched src/workflow file has **exactly one W1 writer**
(`process.rs`→502, `loss.rs`/`network.rs`→505, `memory_guard.rs`→506,
`audio_stability_proof.rs`→503, the four workflow files each have a single
issue). The canon therefore imposes **no path-order serialization between any
two W1 issues** by file conflict.

From `wave-plan.json → dependency_dag.edges` filtered to QA8 (499–510):
no edges exist between W1 QA8 issues. The only QA8 edges touch issues 504,
507, 508 which are **outside W1**.

From `extra_logical_deps`: no rule names W1 QA8 issues.

### 2.2 Arbiter-added logical ordering (API coupling)

Although the canon imposes no file-write conflict, **API coupling** between
`src/metrics/snapshot.rs` (#501) and the probe modules (#502 `process.rs`,
#505 `loss.rs`/`network.rs`, #506 `memory_guard.rs`) is real:

- `snapshot.rs` (current HEAD) already aggregates `ProcessSnapshot`,
  `NetworkMetrics`, and `LossMetrics` via `pub` fields and `apply_*`
  helpers. Any change to the probe struct shapes in #502/#505/#506 forces a
  follow-on change to `snapshot.rs` (#501).
- `src/bin/audio_stability_proof.rs` (#503) consumes the v2 soak schema
  defined by #501 and probes published by #502/#505/#506.

The wave-plan does not encode this coupling because each file has a unique
W1 writer (no file-level conflict). The arbiter therefore adds the following
**non-binding-on-canon, binding-on-dispatch** logical chain for W1 only:

```
                 #501 (schema/contract in snapshot.rs)
                  │     defines v2 evidence schema + telemetry export contract
                  ▼
   ┌──────────────┼──────────────┐
   │              │              │
 #502           #505           #506
(process)    (loss+network)  (memory_guard)
   │              │              │
   └──────────────┼──────────────┘
                  ▼
                #503 (8h soak runner v2 with fault injection)
                  consumes schema (501) + probes (502/505/506)
```

Rationale: #501 is the only `red_mode: "tests_first"` issue whose deliverable
is a **schema contract** (`QA8-03-soak-schema-v2.json` co-located in
`files_allowed`). Schema-first prevents probe authors from inventing
incompatible shapes. #503 is downstream of everything (it is the integrator
that emits the schema-conforming evidence after running probes for 8 h).

### 2.3 Canonical W1 QA8 dispatch order

```
T0 (now, parallel):   #499, #500
T1 (after #501 PR is in-flight or merged):  #501
T2 (parallel, after #501): #502, #505, #506
T3 (after T2 + scope ruling):              #503
```

`#509` and `#510` (workflow-only, files are `.github/workflows/*.yml`
disjoint from everything) dispatch in parallel with T0.

### 2.4 Non-QA8 issues — order

All non-QA8 W1 issues have `dependencies: []`, disjoint allow-lists, no
shared `path_orders` entries, and no `extra_logical_deps` referencing them
**within W1**. They dispatch fully in parallel at T0:

- `#384, #450, #459, #461, #468, #474, #476, #486, #460`

`extra_logical_deps` rules `451_452_453_depend_on_450`, `469_470_..._depend_on_468`,
and `487..497_depend_on_486` apply to **W2+ issues**, not W1.

## 3. Tie-break rule (forward-looking)

If a future wave reveals a real conflict between `wave-manifest.dependencies`
and `wave-plan.serialization_rules`:

1. **wave-plan.json wins** for ordering.
2. The wave-manifest must be regenerated to reflect the omitted dependency.
3. The orchestrator must run a wave-replan before dispatch.
4. The arbiter records the resolution in `verification-evidence/waves/<wave>/ordering-canon.md`
   with the same structure as this document.
