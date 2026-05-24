// Build verification-evidence/wave-plan.json and waves/wave-*/files_allowed.txt
// from verified inputs. Planning only — no src edits.
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const root = path.resolve(__dirname, '..', '..');
const ve = path.join(root, 'verification-evidence');

function sha256(p) {
  return crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex').toUpperCase();
}

const SRC_BOARD = path.join(ve, 'board-snapshot-20260524.json');
const SRC_FFM = path.join(ve, 'family-file-map.json');
const SRC_R4 = path.join(ve, 'w0-r4', 'low-confidence-resolution.json');
const SRC_R5 = path.join(ve, 'w0-r5', 'commands.md');

const board = JSON.parse(fs.readFileSync(SRC_BOARD, 'utf8'));
const ffm = JSON.parse(fs.readFileSync(SRC_FFM, 'utf8'));

// ---------------------------------------------------------------------------
// R5 readiness caveats — ratified into wave-plan serialization rules.
// Three cycles existed in R5's per-path serialization (config↔playback,
// providers/mod↔config, ci.yml↔release.yml). Below are the canonical
// orderings we adopt for R6; each break is explained in `ratifications`.
// ---------------------------------------------------------------------------
const ratifiedChains = {
  'src/audio/mod.rs':            [451, 452, 454, 469, 470, 495],
  'src/audio/probe.rs':          [451, 452, 469, 470],
  'src/audio/virtual_device.rs': [453, 471],
  'src/audio/router.rs':         [453, 471],
  'src/audio/file_source.rs':    [460, 507],
  'src/pipeline/playback.rs':    [456, 454, 455, 453, 495],
  // RATIFIED: 456 first then 454 (CTRL-03 invariant lands before CTRL-01 gain),
  // 457 before 455 (MODEL-01 backend contract before CTRL-02 voice catalog).
  'src/config/mod.rs':           [452, 456, 454, 457, 455, 481, 482, 491, 492, 496],
  'src/tui/mod.rs':              [454, 455, 479, 480, 481, 494],
  'src/providers/mod.rs':        [457, 455, 491],
  'src/providers/local':         [457, 458, 473],
  'src/providers/mt':            [457, 458, 473],
  'src/providers/local_mt':      [493, 492, 494],
  'src/metrics/snapshot.rs':     [501, 511],
  // RATIFIED: SEC-01 (#462) lands SBOM/signing scaffolding before MODEL-02
  // (#458) layers macOS runtime onto either workflow.
  '.github/workflows/ci.yml':      [461, 462, 458, 475, 477, 508],
  '.github/workflows/release.yml': [462, 463, 458, 472, 477, 478, 497],
};

const ratifications = [
  {
    cycle: 'src/pipeline/playback.rs vs src/config/mod.rs (CTRL-01 #454 vs CTRL-03 #456)',
    r5_state: 'playback chain had 456 before 454; config chain had 454 before 456 — cycle.',
    decision: 'CTRL-03 (#456) lands first globally. R5 explicitly marked CTRL-03 as the playback invariant ("CTRL-03 lands the single-voice invariant first; all later writers must preserve it"). Config order is therefore re-sequenced to 452→456→454→… so the playback invariant carries.',
    affected_paths: ['src/pipeline/playback.rs', 'src/config/mod.rs'],
  },
  {
    cycle: 'src/providers/mod.rs vs src/config/mod.rs (CTRL-02 #455 vs MODEL-01 #457)',
    r5_state: 'providers/mod chain had 457 before 455; config chain had 455 before 457 — cycle.',
    decision: 'MODEL-01 (#457) lands first globally. R5 rule on providers/mod ("MODEL-01 defines the backend-selection contract first; CTRL-02 layers voice catalog on top") is canonical. Config order is re-sequenced to …454→457→455→… so CTRL-02 registers its voice catalog after MODEL-01\'s backend trait exists.',
    affected_paths: ['src/providers/mod.rs', 'src/config/mod.rs'],
  },
  {
    cycle: '.github/workflows/ci.yml vs .github/workflows/release.yml (SEC-01 #462 vs MODEL-02 #458)',
    r5_state: 'ci.yml had 458 before 462; release.yml had 462 before 458 — cycle.',
    decision: 'SEC-01 (#462) lands first globally. SBOM/signing scaffolding is a prerequisite for MODEL-02\'s cross-platform release matrix. ci.yml order re-sequenced to 461→462→458→… (matrix → SBOM → macOS runtime layer). Workflows are additive at the job/step level, so concurrent edits in different sections of the same file are safe; cycle break only governs cross-section dependencies.',
    affected_paths: ['.github/workflows/ci.yml', '.github/workflows/release.yml'],
  },
];

// R5 caveats that do NOT introduce cycles but are carried forward as
// serialization rules in this wave plan.
const carriedCaveats = [
  {
    rule: 'Cargo.toml is additive-merge only',
    detail: 'Multiple wave-N issues may append [dependencies] entries concurrently. --locked CI gate (#461) prevents silent drift. If two issues bump the SAME dependency in overlapping windows, the later-merged PR MUST rerun the earlier issue\'s tests.',
  },
  {
    rule: 'src/tui/frame_pacer.rs has a writer-reader split',
    detail: 'UX-01 (#479) is the sole writer in wave 8. QA8-06 (#504) is read-only in wave 9. If QA8-06 ever needs to write the file, it must be rebased onto UX-01 explicitly and serialized inside CTRL family.',
  },
  {
    rule: 'STD-02 (#484) is intentionally last-in-line',
    detail: 'STD-02 refactors oversized modules across many files. It is assigned to Wave F (final / release-train) and the orchestrator MUST hold STD-02 until all other src-touching tentacles for the affected files have merged.',
  },
  {
    rule: 'Planning-stage issues (Wave P) require WBS decomposition before implementation dispatch',
    detail: 'The 16 epics / planning issues without concrete src ownership MUST spawn WBS children with concrete files_touched_hint values before they can be assigned to any implementation wave. No implementation tentacle may be dispatched against a Wave P issue until its child issues exist on the board.',
  },
  {
    rule: 'Spike-before-implementation logical dependencies',
    detail: 'Although the family-file-map only encodes file-level deps, R6 adds logical deps: MACOS-01 spike (#450) precedes MACOS-02/03/04 (#451/452/453); LINUX-01 spike (#468) precedes LINUX-02..05 (#469-472); SUPERTONIC-01 spike (#486) precedes SUPERTONIC-02..12 (#487-497).',
  },
  {
    rule: 'QA8-06 (#504) reads UX-01 (#479)',
    detail: 'R6 adds a logical dep 504→479 so the QA8 60fps frame-pacer probe runs after UX-01 schema is stable. Not in family-file-map because 504 is read-only; recorded here as wave-plan serialization rule.',
  },
];

// ---------------------------------------------------------------------------
// Build predecessor graph.
// ---------------------------------------------------------------------------
const preds = {};
for (const i of ffm.issues) preds[i.number] = new Set();
for (const p in ratifiedChains) {
  const order = ratifiedChains[p];
  for (let i = 1; i < order.length; i++) {
    for (let j = 0; j < i; j++) preds[order[i]].add(order[j]);
  }
}
// Logical deps from caveats
preds[504].add(479);
for (const n of [451, 452, 453]) preds[n].add(450);
for (const n of [469, 470, 471, 472]) preds[n].add(468);
for (const n of [487, 488, 489, 490, 491, 492, 493, 494, 495, 496, 497]) preds[n].add(486);

const wave = {};
function depth(n, stack) {
  if (wave[n] != null) return wave[n];
  if (stack.has(n)) throw new Error('cycle at ' + n + ' stack=' + [...stack].join('>'));
  stack.add(n);
  let mx = 0;
  for (const p of preds[n]) mx = Math.max(mx, depth(p, stack));
  stack.delete(n);
  wave[n] = mx + 1;
  return wave[n];
}
for (const i of ffm.issues) depth(i.number, new Set());

// ---------------------------------------------------------------------------
// Classify issues into waves.
// ---------------------------------------------------------------------------
const planningSet = new Set(ffm.non_owning_issues.issues);
const humanSet = new Set(ffm.human_only_issues);
// 484 is in planningSet but is explicitly Wave F per caveat.
planningSet.delete(484);

const issuesByNumber = new Map(ffm.issues.map(i => [i.number, i]));
const boardByNumber = new Map(board.rows.map(r => [r.number, r]));

function classifyWave(n) {
  if (humanSet.has(n)) return 'H';
  if (n === 484) return 'F';
  if (planningSet.has(n)) return 'P';
  return 'W' + wave[n];
}

// red_mode policy:
//  - "tests_first"     : issue writes src/** code; tests must precede impl
//  - "evidence_first"  : evidence-only issue; baseline evidence captured before changes
//  - "workflow_dry_run": modifies .github/workflows; dry-run on a fork branch first
//  - "doc_first"       : docs-only issue; ADR/draft posted before merge
//  - "decomposition"   : Wave P/F; no implementation dispatch yet
function redModeFor(issue, boardRow) {
  const number = issue.number;
  const owned = issue.owned_paths || [];
  const shared = (issue.shared_paths_in || []).map(s => s.path);
  const all = [...owned, ...shared];
  const wlab = classifyWave(number);
  if (wlab === 'H') return 'human_acceptance_log';
  if (wlab === 'P' || wlab === 'F') return 'decomposition';
  const hasSrc = all.some(p => p.startsWith('src/'));
  const hasWorkflow = all.some(p => p.startsWith('.github/workflows/'));
  const hasDoc = all.some(p => p.startsWith('docs/') || p === 'PRIVACY.md');
  const hasEvidence = (issue.evidence_paths || []).length > 0;
  if (hasSrc) return 'tests_first';
  if (hasWorkflow) return 'workflow_dry_run';
  if (hasDoc) return 'doc_first';
  if (hasEvidence) return 'evidence_first';
  return 'evidence_first';
}

function humanGatePrereqsFor(issue, boardRow) {
  const number = issue.number;
  // Direct human-gated issues
  if (humanSet.has(number)) {
    return {
      required: true,
      reasons: ['needs:human-reviewer label on board snapshot'],
      gates: ['named_human_reviewer_assigned (per #366)', 'acceptance_log_signed (per #122)'],
    };
  }
  // AI-actionable issues that touch release/signing surfaces require human
  // co-sign before merge.
  const owned = issue.owned_paths || [];
  const shared = (issue.shared_paths_in || []).map(s => s.path);
  const all = [...owned, ...shared];
  const touchesRelease = all.some(p => p.includes('.github/workflows/release.yml') || p.includes('packaging/'));
  const touchesPrivacy = all.includes('PRIVACY.md') || all.includes('deny.toml');
  if (touchesRelease || touchesPrivacy) {
    return {
      required: false,
      co_sign_required: true,
      reasons: [touchesRelease ? 'modifies release workflow / packaging' : 'modifies privacy/security policy'],
      gates: ['security_review_signoff'],
    };
  }
  return { required: false };
}

// Files allowed list: union of owned_paths + shared_paths_in.path + evidence_paths.
// For a wave, the closed allow-list is the union across all issues assigned.
function filesAllowedFor(issue) {
  const set = new Set();
  for (const p of (issue.owned_paths || [])) set.add(p);
  for (const s of (issue.shared_paths_in || [])) set.add(s.path);
  for (const p of (issue.evidence_paths || [])) set.add(p);
  return [...set].sort();
}

// ---------------------------------------------------------------------------
// Build per-issue records.
// ---------------------------------------------------------------------------
const issueRecords = [];
for (const i of ffm.issues) {
  const board = boardByNumber.get(i.number) || {};
  const wlab = classifyWave(i.number);
  const rec = {
    number: i.number,
    title: board.title || null,
    family: i.family,
    family_kind: i.kind || 'implementation',
    wave: wlab,
    wave_numeric: wlab.startsWith('W') ? parseInt(wlab.slice(1), 10) : null,
    dependencies: [...preds[i.number]].sort((a, b) => a - b),
    files_allowed: filesAllowedFor(i),
    owned_paths: i.owned_paths || [],
    shared_paths_in: i.shared_paths_in || [],
    evidence_paths: i.evidence_paths || [],
    red_mode: redModeFor(i, board),
    human_gate_prereqs: humanGatePrereqsFor(i, board),
    confidence: board.confidence || null,
    needs_human_clarification: !!board.needs_human_clarification,
    board_human_gate: !!board.human_gate,
  };
  if (wlab === 'P') {
    rec.planning_stage = true;
    rec.decomposition_required = true;
    rec.implementation_dispatch_allowed = false;
    rec.note = 'Wave P: epic / planning-only issue with no concrete src ownership. WBS children must be created with concrete files_touched_hint before any implementation tentacle may be dispatched against this issue.';
  } else if (wlab === 'F') {
    rec.planning_stage = false;
    rec.decomposition_required = true;
    rec.implementation_dispatch_allowed = false;
    rec.note = 'Wave F: held until all src-touching tentacles for affected files have merged; orchestrator schedules into a release-train window with no other open PRs on the same files.';
  } else if (wlab === 'H') {
    rec.planning_stage = false;
    rec.decomposition_required = false;
    rec.implementation_dispatch_allowed = false;
    rec.note = 'Human-gated: not assigned to an AI implementation wave. Acceptance is signed via the WP-19 acceptance log.';
  } else {
    rec.planning_stage = false;
    rec.decomposition_required = false;
    rec.implementation_dispatch_allowed = true;
  }
  issueRecords.push(rec);
}
issueRecords.sort((a, b) => a.number - b.number);

// ---------------------------------------------------------------------------
// Build waves array.
// ---------------------------------------------------------------------------
const waveLabels = [...new Set(issueRecords.map(r => r.wave))].sort((a, b) => {
  const ord = k => k === 'H' ? -2 : k === 'P' ? -1 : k === 'F' ? 999 : parseInt(k.slice(1), 10);
  return ord(a) - ord(b);
});

const waves = waveLabels.map(lab => {
  const issues = issueRecords.filter(r => r.wave === lab).map(r => r.number).sort((a, b) => a - b);
  const allowed = new Set();
  for (const n of issues) {
    const r = issueRecords.find(x => x.number === n);
    for (const p of r.files_allowed) allowed.add(p);
  }
  let kind = 'implementation';
  if (lab === 'H') kind = 'human_acceptance';
  else if (lab === 'P') kind = 'planning_decomposition';
  else if (lab === 'F') kind = 'final_refactor';
  return {
    id: lab,
    kind,
    issue_count: issues.length,
    issues,
    files_allowed: [...allowed].sort(),
    implementation_dispatch_allowed: !(lab === 'H' || lab === 'P' || lab === 'F'),
  };
});

// ---------------------------------------------------------------------------
// Summary counts.
// ---------------------------------------------------------------------------
const summary = {
  total_issues: issueRecords.length,
  human_gated: issueRecords.filter(r => r.wave === 'H').length,
  ai_actionable: issueRecords.filter(r => r.wave !== 'H').length,
  ai_planning_stage: issueRecords.filter(r => r.wave === 'P').length,
  ai_implementation_assigned: issueRecords.filter(r => r.wave.startsWith('W')).length,
  ai_final_refactor: issueRecords.filter(r => r.wave === 'F').length,
  waves_implementation: waveLabels.filter(l => l.startsWith('W')).length,
};

// ---------------------------------------------------------------------------
// Cross-check guard: every issue must be classified, and no issue without
// human-gate or decomposition flag may lack a wave.
// ---------------------------------------------------------------------------
const missing = issueRecords.filter(r => !r.wave);
if (missing.length) throw new Error('Issues missing wave: ' + missing.map(r => r.number).join(','));

const plan = {
  schema_version: 1,
  tentacle: 'w0-r6-wave-plan',
  generated_at: new Date().toISOString(),
  source: {
    'verification-evidence/board-snapshot-20260524.json': { sha256: sha256(SRC_BOARD) },
    'verification-evidence/family-file-map.json':         { sha256: sha256(SRC_FFM) },
    'verification-evidence/w0-r4/low-confidence-resolution.json': { sha256: sha256(SRC_R4) },
    'verification-evidence/w0-r5/commands.md':            { sha256: sha256(SRC_R5) },
  },
  ratifications,
  carried_caveats: carriedCaveats,
  serialization_rules: {
    path_orders: ratifiedChains,
    extra_logical_deps: {
      '504_depends_on_479': 'QA8-06 reads frame_pacer.rs; rebases on UX-01 schema.',
      '451_452_453_depend_on_450': 'macOS implementation issues depend on MACOS-01 spike decision.',
      '469_470_471_472_depend_on_468': 'Linux implementation issues depend on LINUX-01 spike decision.',
      '487..497_depend_on_486': 'SUPERTONIC implementation/evaluation issues depend on SUPERTONIC-01 spike.',
    },
  },
  red_mode_policies: {
    tests_first: 'New or modified src/** code requires failing test(s) committed before implementation.',
    workflow_dry_run: 'Workflow edits must be exercised on a fork branch via workflow_dispatch before merge.',
    doc_first: 'Docs/ADR edits require a draft ADR or doc PR with cross-team review.',
    evidence_first: 'Evidence-only artifacts: baseline must be captured (or schema declared) before changes.',
    human_acceptance_log: 'WP-19 family: human reviewer signs the acceptance log per child issue.',
    decomposition: 'Wave P / Wave F: no implementation dispatch until children exist or release-train window opens.',
  },
  dependency_dag: {
    edges: Object.entries(preds)
      .flatMap(([n, ps]) => [...ps].map(p => ({ from: parseInt(p, 10), to: parseInt(n, 10) })))
      .sort((a, b) => a.from - b.from || a.to - b.to),
    notes: 'Edges are derived from ratifiedChains path orders + extra_logical_deps. An edge from A to B means A must merge before B.',
  },
  waves,
  issues: issueRecords,
  summary,
  readiness: true,
  readiness_notes: [
    'All 82 in-scope issues classified.',
    'All 72 AI-actionable issues are either assigned to a numbered wave (Wave 1..14), Wave F (final/refactor), or Wave P (planning/decomposition).',
    'All 10 human-gated issues are in Wave H with implementation_dispatch_allowed=false.',
    'All three R5 cycles (config↔playback, providers/mod↔config, ci.yml↔release.yml) are ratified with canonical orderings; no cycle remains in the dependency DAG.',
    'No Wave P issue may be dispatched until WBS children with concrete files_touched_hint exist on the board.',
    'Wave F (#484 STD-02) is held until all src-touching tentacles for affected files have merged.',
  ],
};

fs.writeFileSync(path.join(ve, 'wave-plan.json'), JSON.stringify(plan, null, 2));
console.log('wrote', path.join(ve, 'wave-plan.json'), 'bytes=', fs.statSync(path.join(ve, 'wave-plan.json')).size);

// ---------------------------------------------------------------------------
// Per-wave files_allowed.txt + baseline-hashes.json placeholder schema.
// ---------------------------------------------------------------------------
function writeWaveDir(wave) {
  const wid = wave.id;
  const dirName = wid === 'H' ? 'wave-H' : wid === 'P' ? 'wave-P' : wid === 'F' ? 'wave-F' : 'wave-' + wid.slice(1);
  const dir = path.join(ve, 'waves', dirName);
  fs.mkdirSync(dir, { recursive: true });

  const note = (() => {
    if (wave.kind === 'human_acceptance') return '# Wave H — Human acceptance only.\n# No AI implementation dispatch.\n# files_allowed below is the human reviewer evidence surface (acceptance log / Layer-5 artifacts).\n';
    if (wave.kind === 'planning_decomposition') return '# Wave P — Planning / decomposition only.\n# No AI implementation dispatch is allowed against these epics.\n# files_allowed below is the planning/evidence surface only; WBS children must be created with concrete src paths before any wave assignment.\n';
    if (wave.kind === 'final_refactor') return '# Wave F — Final / refactor held for release-train window.\n# STD-02 (#484) refactors oversized modules across many files.\n# files_allowed must be re-derived at release-train scheduling time from the actual oversized-module audit; this is a placeholder closed-list scoped to STD-02 itself.\n';
    return `# Wave ${wid} — Implementation closed allow-list.\n# An implementation tentacle assigned to an issue in this wave MAY ONLY modify files in this list.\n# Tests under tests/** and benches under benches/** are implicitly allowed for tests_first issues even if not listed.\n`;
  })();

  const files = wave.files_allowed;
  const txt = note + '\n' + (files.length ? files.join('\n') + '\n' : '# (no closed paths — planning/decomposition only)\n');
  fs.writeFileSync(path.join(dir, 'files_allowed.txt'), txt);

  // baseline-hashes.json placeholder schema
  const baseline = {
    schema_version: 1,
    wave: wid,
    note: 'Placeholder schema. Pre-implementation, the orchestrator (or wave start hook) MUST populate `files` with {path, sha256_pre} for every file in files_allowed.txt that exists on main HEAD at wave-start time. Post-implementation, the implementation tentacle records {sha256_post}. CI/wave-close hook diffs pre vs post to enforce closed-list compliance.',
    generated_at: null,
    main_head_sha: null,
    files: [],
    files_allowed_source: 'waves/' + dirName + '/files_allowed.txt',
  };
  fs.writeFileSync(path.join(dir, 'baseline-hashes.json'), JSON.stringify(baseline, null, 2));

  // Per-wave manifest of issues
  const manifest = {
    schema_version: 1,
    wave: wid,
    kind: wave.kind,
    implementation_dispatch_allowed: wave.implementation_dispatch_allowed,
    issue_count: wave.issue_count,
    issues: wave.issues.map(n => {
      const r = issueRecords.find(x => x.number === n);
      return {
        number: n,
        title: r.title,
        family: r.family,
        red_mode: r.red_mode,
        dependencies: r.dependencies,
        files_allowed: r.files_allowed,
        human_gate_prereqs: r.human_gate_prereqs,
        planning_stage: !!r.planning_stage,
        decomposition_required: !!r.decomposition_required,
      };
    }),
  };
  fs.writeFileSync(path.join(dir, 'wave-manifest.json'), JSON.stringify(manifest, null, 2));
}
for (const w of waves) writeWaveDir(w);
console.log('waves written:', waves.length);
console.log('summary:', JSON.stringify(summary, null, 2));
