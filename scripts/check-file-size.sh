#!/usr/bin/env bash
# check-file-size.sh — v0.4.0 layer 4 of CODE_STYLE.md.
#
# Enforces the per-file LOC caps defined in §1.1 of
# `docs/architecture/CODE_STYLE.md`:
#
#   - Module entry (mod.rs) and module file: 1500 LOC
#   - Test sibling (*_tests.rs):  unbounded
#   - Integration test (tests/<name>.rs):  2000 LOC
#   - CLI subcommand (src/bin/<name>.rs):  500 LOC
#   - main.rs: 2500 LOC
#
# Legacy files (e.g. src/main.rs today is 7682) are exempted via
# the LEGACY_OVERRIDES block at the bottom; each entry must
# reference a tracked ADR/PR.  When the file is refactored, the
# override is removed in the same commit.
#
# Exit code 0 on pass, 1 on violation.  CI rejects the PR.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# Per-class caps (lines).
CAP_MODULE=1500
CAP_INTEGRATION_TEST=2000
CAP_BIN=500
CAP_MAIN=2500

# Files we always skip (cargo-generated, build artefacts).
SKIP_PATHS=(
    "target/"
)

# A violation prints one line per offending file:
#   FILE_TOO_LARGE: <path> = <loc> LOC (cap = <cap>, class = <class>)
violations=0

classify() {
    local rel="$1"
    if [[ "$rel" == "src/main.rs" ]]; then
        echo "main"
    elif [[ "$rel" == src/bin/* ]]; then
        echo "bin"
    elif [[ "$rel" == tests/*_tests.rs || "$rel" == src/**/*_tests.rs ]]; then
        # The convention is `*_tests.rs` sibling; tests in
        # `tests/` are integration tests, not siblings.
        echo "test_sibling"
    elif [[ "$rel" == tests/* ]]; then
        echo "integration_test"
    elif [[ "$rel" == src/*_tests.rs ]]; then
        echo "test_sibling"
    else
        echo "module"
    fi
}

cap_for_class() {
    local cls="$1"
    case "$cls" in
        main) echo "$CAP_MAIN" ;;
        bin) echo "$CAP_BIN" ;;
        integration_test) echo "$CAP_INTEGRATION_TEST" ;;
        test_sibling) echo "0" ;;  # 0 = no cap
        module) echo "$CAP_MODULE" ;;
    esac
}

# ----------------------------------------------------------------------------
# Legacy overrides.
#
# Each entry is a single case in the dispatch below.  Bypass is
# auditable: the case comment forces you to name the ADR that
# is going to fix it.
# ----------------------------------------------------------------------------
is_legacy_override() {
    local rel="$1"
    case "$rel" in
        # Standard cap is 1500; current is 7682.  Tracked in ADR-0010 §PR-A.
        "src/main.rs") return 0 ;;
        # Standard cap is 1500; current is 5453.  Tracked in ADR-0010 §migration.
        "src/config/mod.rs") return 0 ;;
        # Standard cap is 1500; current is 5374.  Tracked in ADR-0010 §migration.
        "src/tui/mod.rs") return 0 ;;
        # Standard cap is 1500; current is 4285.  Tracked in ADR-0010 §migration.
        "src/pipeline/mod.rs") return 0 ;;
        # bin cap is 500; current is 2446.  Tracked in ADR-0010 §migration.
        "src/bin/eval_session.rs") return 0 ;;
        # bin cap is 500; current is 2115.  Tracked in ADR-0010 §migration.
        "src/bin/mt_bench.rs") return 0 ;;
        # bin cap is 500; current is 541.  Tracked in CODE_STYLE.md §9.
        "src/bin/quality_benchmark.rs") return 0 ;;
        # bin cap is 500; current is 590.  Tracked in CODE_STYLE.md §9.
        "src/bin/llm_mt_bench.rs") return 0 ;;
        # bin cap is 500; current is 586.  Tracked in CODE_STYLE.md §9.
        "src/bin/audio_stability_proof.rs") return 0 ;;
        # bin cap is 500; current is 568.  Tracked in CODE_STYLE.md §9.
        "src/bin/qa8_slo_gate_checker.rs") return 0 ;;
        # bin cap is 500; current is 534.  Tracked in CODE_STYLE.md §9.
        "src/bin/frame_pacing_bench.rs") return 0 ;;
        # integration_test cap is 2000; current is 2531.  Tracked in CODE_STYLE.md §9.
        "tests/soak/run_soak.rs") return 0 ;;
        # integration_test cap is 2000; current is 2513.  Tracked in CODE_STYLE.md §9.
        "tests/snapshot.rs") return 0 ;;
        *) return 1 ;;
    esac
}

# Walk all .rs files in src/ and tests/.
while IFS= read -r -d '' file; do
    rel="${file#./}"
    # Apply skip paths.
    skip=0
    for p in "${SKIP_PATHS[@]}"; do
        if [[ "$rel" == $p* ]]; then
            skip=1; break
        fi
    done
    [[ $skip -eq 1 ]] && continue

    cls=$(classify "$rel")
    cap=$(cap_for_class "$cls")
    [[ "$cap" -eq 0 ]] && continue  # unbounded (test_sibling)

    # Legacy override?
    if is_legacy_override "$rel"; then
        continue
    fi

    loc=$(wc -l < "$rel")
    if (( loc > cap )); then
        echo "FILE_TOO_LARGE: $rel = $loc LOC (cap = $cap, class = $cls)"
        violations=$((violations + 1))
    fi
done < <(find src tests -type f -name '*.rs' -print0 2>/dev/null)

if (( violations > 0 )); then
    echo
    echo "Found $violations file(s) over the per-class cap."
    echo "See docs/architecture/CODE_STYLE.md §1.1 for the rules."
    echo "If this is a legacy file, add a LEGACY_OVERRIDES entry in this script"
    echo "and reference a tracking ADR / PR.  New files must respect the cap."
    exit 1
fi

echo "check-file-size: all files within caps"
exit 0
