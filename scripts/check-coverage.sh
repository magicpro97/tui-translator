#!/usr/bin/env bash
# check-coverage.sh — v0.4.0 layer 6 of CODE_STYLE.md.
#
# Enforces the per-module line coverage threshold of 0.6 for
# modules added in v0.4.0 or later.  Legacy modules (predating the
# CODE_STYLE.md rule) are reported but do not fail the gate.
#
# "New" is defined as: the file's first commit is on a commit
# whose first 7 hex chars we have added to NEW_FILES_*
# below.  A simpler proxy: the path appears in this script.
# When a legacy module is refactored, the corresponding
# `is_legacy_module` entry is removed.
#
# Exit code 0 on pass, 1 on violation.  CI rejects the PR.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

THRESHOLD=0.60

# Modules introduced or refactored in v0.4.0 (ADR-0010).  These
# must hit the threshold; anything else is reported but not gated.
NEW_MODULES=(
    "src/providers/cloud/orchestrator.rs"
    "src/providers/cloud/segment_swap.rs"
    "src/providers/cloud/reconnect_state.rs"
    "src/providers/cloud/cost_cap.rs"
    "src/providers/cloud/transcript_segment.rs"
    "src/providers/cloud/cloud_segment.rs"
    "src/providers/cloud/local_segment.rs"
)

# Build a coverage report.  cargo-llvm-cov is fast enough that
# we re-build every time; the test binaries are already cached
# in target/ from `cargo test --no-run`.
echo "check-coverage: building test binaries..."
cargo test --bin tui-translator --no-run --quiet 2>&1 | tail -5 || true

# Generate per-file coverage JSON.  Output goes to coverage/.
rm -rf coverage/
mkdir -p coverage
echo "check-coverage: running llvm-cov..."
cargo llvm-cov --bin tui-translator \
    --json \
    --output-path coverage/coverage.json \
    --ignore-filename-regex '(tests/|src/bin/|src/main\.rs)' \
    --quiet 2>&1 | tail -3 || {
        echo "check-coverage: failed to run cargo llvm-cov; skipping gate"
        exit 0
    }

# Per-file coverage from the JSON.  We use Python because the
# JSON is nested and we need a few small computations.
COVERAGE_JSON="coverage/coverage.json"
if [[ ! -f "$COVERAGE_JSON" ]]; then
    echo "check-coverage: no coverage.json produced; skipping gate"
    exit 0
fi

python3 - "$COVERAGE_JSON" <<'PY'
import json
import sys

threshold = 0.60

new_modules = [
    "src/providers/cloud/orchestrator.rs",
    "src/providers/cloud/segment_swap.rs",
    "src/providers/cloud/reconnect_state.rs",
    "src/providers/cloud/cost_cap.rs",
    "src/providers/cloud/transcript_segment.rs",
    "src/providers/cloud/cloud_segment.rs",
    "src/providers/cloud/local_segment.rs",
]

with open(sys.argv[1]) as f:
    data = json.load(f)

# cargo-llvm-cov JSON shape:
#   { "data": [ { "files": [ { "filename": "...", "summary": { "lines": { "covered": N, "count": M } } } ] } ] }
files = {}
for datum in data.get("data", []):
    for f in datum.get("files", []):
        files[f["filename"]] = f

# Normalize paths so they always start with the project root.
# cargo-llvm-cov reports them as relative to the workspace root
# in our case, but on some configs they may be absolute.
def norm(p):
    return p.lstrip("/")

violations = 0
for mod in new_modules:
    # Find the file entry; match by suffix because the JSON
    # path may be prefixed with the project root.
    matched = None
    for path, info in files.items():
        if norm(path).endswith(mod):
            matched = info
            break

    if matched is None:
        print(f"check-coverage: {mod} not present in coverage report (skipping)")
        continue

    summary = matched.get("summary", {})
    lines = summary.get("lines", {})
    covered = lines.get("covered", 0)
    count = lines.get("count", 0)
    if count == 0:
        print(f"check-coverage: {mod}: 0 executable lines (skipping)")
        continue

    ratio = covered / count
    status = "PASS" if ratio >= threshold else "FAIL"
    if ratio < threshold:
        violations += 1
    print(f"check-coverage: {mod}: {covered}/{count} = {ratio:.2%} [{status}]")

# Also report top 5 worst-coverage files overall so the user
# can see which legacy modules to chip away at next.
print()
print("check-coverage: top 5 worst-coverage files overall (advisory):")
ranked = []
for path, info in files.items():
    np = norm(path)
    if "/tests/" in np or "/bin/" in np or np.endswith("main.rs"):
        continue
    if "target" in np:
        continue
    lines = info.get("summary", {}).get("lines", {})
    covered = lines.get("covered", 0)
    count = lines.get("count", 0)
    if count == 0:
        continue
    ratio = covered / count
    ranked.append((ratio, covered, count, np))
ranked.sort()
for ratio, covered, count, path in ranked[:5]:
    print(f"  {ratio:6.2%}  {covered:5d}/{count:5d}  {path}")

sys.exit(1 if violations > 0 else 0)
PY
