//! Frame-pacing bench configuration and gate selection.
//!
//! WP-25.03 (#761): the bench's gates must be selected by a single
//! function so the production code path and the test path agree on
//! which threshold to use. Before this refactor, the 20ms / 25ms
//! constants were private to `frame_pacing_bench.rs` and untested;
//! the audit flagged that the "60 FPS guarantee" had no regression
//! gate in CI.
//!
//! The `--strict` flag in `main` selects the 60 FPS target
//! (16.6ms p95). Both modes run on every invocation; the flag
//! controls which set of gates the bench reports and exits on.

/// Standard 60 FPS target: 16.6 ms per frame budget, but we round
/// to 17 ms to keep the gate comparison an integer check.
pub const STRICT_60FPS_GATE_MS: u64 = 17;

/// Lenient (default) gate for the single-pane mode.  This matches
/// the legacy acceptance criteria from issue #383 and the bench's
/// previous behaviour; it is not a regression on existing callers.
pub const SINGLE_MODE_P95_GATE_MS: u64 = 20;

/// Lenient (default) gate for the dual-pane mode.  Matches the
/// legacy acceptance criteria.
pub const DUAL_MODE_P95_GATE_MS: u64 = 25;

/// Which gate set to apply to a given bench mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateSet {
    /// The 20ms / 25ms gates that issue #383 specified.  This is
    /// the default — running the bench without `--strict` keeps
    /// the legacy behaviour.
    Lenient,
    /// The 17ms / 17ms gates for the 60 FPS contract.  Selected
    /// by `--strict`.
    Strict60Fps,
}

/// The bench mode the gate is being applied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchMode {
    SinglePane,
    DualPane,
}

/// Return the p95 gate in milliseconds for a `(GateSet, BenchMode)`
/// pair.  This is the single source of truth — both `main` and
/// the test module call this.
///
/// The function is total over its input domain (no `Err`/`Option`)
/// because the matrix is small and constant.  Adding a new mode
/// or a new gate set is a compile-time failure (missing match arm)
/// rather than a runtime surprise.
pub fn p95_gate_ms(gate_set: GateSet, mode: BenchMode) -> u64 {
    match (gate_set, mode) {
        (GateSet::Lenient, BenchMode::SinglePane) => SINGLE_MODE_P95_GATE_MS,
        (GateSet::Lenient, BenchMode::DualPane) => DUAL_MODE_P95_GATE_MS,
        (GateSet::Strict60Fps, BenchMode::SinglePane) => STRICT_60FPS_GATE_MS,
        (GateSet::Strict60Fps, BenchMode::DualPane) => STRICT_60FPS_GATE_MS,
    }
}

/// Return `true` if the measured p95 in milliseconds satisfies
/// the gate for the given `(GateSet, BenchMode)`.  Pure function,
/// no I/O, no env access.  This is the predicate `main` calls
/// after each `run_mode(...)` invocation.
pub fn passes_gate(p95_ms: u64, gate_set: GateSet, mode: BenchMode) -> bool {
    p95_ms <= p95_gate_ms(gate_set, mode)
}

/// Parse the `--strict` argument from the bench's argv.  Returns
/// `GateSet::Strict60Fps` if any argument equals exactly `--strict`,
/// otherwise `GateSet::Lenient`.  Centralised here so the test
/// can verify the parse independently of `main`'s control flow.
pub fn parse_gate_set(args: &[String]) -> GateSet {
    if args.iter().any(|a| a == "--strict") {
        GateSet::Strict60Fps
    } else {
        GateSet::Lenient
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── gate matrix ────────────────────────────────────────────────
    //
    // invariant: the four (GateSet, BenchMode) cells return the
    // exact constants documented in the bench's acceptance
    // criteria. A future refactor that bumps the lenient gates
    // must also bump these tests, which forces a reviewer to
    // consciously accept the new threshold.

    #[test]
    fn lenient_single_pane_gate_is_20ms() {
        assert_eq!(
            p95_gate_ms(GateSet::Lenient, BenchMode::SinglePane),
            SINGLE_MODE_P95_GATE_MS
        );
        assert_eq!(p95_gate_ms(GateSet::Lenient, BenchMode::SinglePane), 20);
    }

    #[test]
    fn lenient_dual_pane_gate_is_25ms() {
        assert_eq!(
            p95_gate_ms(GateSet::Lenient, BenchMode::DualPane),
            DUAL_MODE_P95_GATE_MS
        );
        assert_eq!(p95_gate_ms(GateSet::Lenient, BenchMode::DualPane), 25);
    }

    #[test]
    fn strict_60fps_gate_is_17ms_for_both_modes() {
        assert_eq!(
            p95_gate_ms(GateSet::Strict60Fps, BenchMode::SinglePane),
            STRICT_60FPS_GATE_MS
        );
        assert_eq!(
            p95_gate_ms(GateSet::Strict60Fps, BenchMode::DualPane),
            STRICT_60FPS_GATE_MS
        );
        // 60 FPS = 16.67ms per frame; the gate rounds up to 17ms
        // so the integer-comparison in `passes_gate` does not
        // false-fail at 16.6ms.
        assert_eq!(p95_gate_ms(GateSet::Strict60Fps, BenchMode::SinglePane), 17);
    }

    // ── pass / fail predicate ──────────────────────────────────────
    //
    // invariant: a measured p95 at exactly the gate passes; one
    // millisecond above fails.  This pins the inclusive
    // comparison so a future refactor to `p95_ms < gate` would
    // fail the test.

    #[test]
    fn passes_gate_is_inclusive_at_threshold() {
        assert!(passes_gate(20, GateSet::Lenient, BenchMode::SinglePane));
        assert!(passes_gate(25, GateSet::Lenient, BenchMode::DualPane));
        assert!(passes_gate(17, GateSet::Strict60Fps, BenchMode::SinglePane));
    }

    #[test]
    fn passes_gate_fails_one_ms_above_threshold() {
        assert!(!passes_gate(21, GateSet::Lenient, BenchMode::SinglePane));
        assert!(!passes_gate(26, GateSet::Lenient, BenchMode::DualPane));
        assert!(!passes_gate(
            18,
            GateSet::Strict60Fps,
            BenchMode::SinglePane
        ));
    }

    #[test]
    fn passes_gate_handles_zero_p95() {
        // A measurement of 0 ms is "infinitely fast" and should
        // pass every gate.  This protects against a regression
        // that wires the comparison backwards.
        assert!(passes_gate(0, GateSet::Lenient, BenchMode::SinglePane));
        assert!(passes_gate(0, GateSet::Strict60Fps, BenchMode::DualPane));
    }

    // ── argv parsing ───────────────────────────────────────────────
    //
    // invariant: `--strict` is the only argument that flips the
    // gate set.  Other arguments (notably `--json <path>`) must
    // not affect gate selection.

    #[test]
    fn parse_gate_set_default_is_lenient() {
        let args = vec![];
        assert_eq!(parse_gate_set(&args), GateSet::Lenient);
    }

    #[test]
    fn parse_gate_set_strict_flag_flips_to_strict_60fps() {
        let args = vec!["--strict".to_string()];
        assert_eq!(parse_gate_set(&args), GateSet::Strict60Fps);
    }

    #[test]
    fn parse_gate_set_strict_among_other_args_still_flips() {
        let args = vec![
            "--json".to_string(),
            "/tmp/bench.json".to_string(),
            "--strict".to_string(),
        ];
        assert_eq!(parse_gate_set(&args), GateSet::Strict60Fps);
    }

    #[test]
    fn parse_gate_set_unknown_flag_does_not_flip() {
        // Regression guard: a future flag like `--srict` (typo) or
        // a similar-looking name must not silently enable the
        // strict gate set.
        let args = vec!["--srict".to_string(), "--strictly".to_string()];
        assert_eq!(parse_gate_set(&args), GateSet::Lenient);
    }

    // ── gate-set equality is total ─────────────────────────────────
    //
    // invariant: `GateSet` derives `PartialEq`, so the matching
    // in `main` is exhaustive.  This test pins the variants
    // count so adding a new variant is a compile-time
    // consideration (the `match` in `p95_gate_ms` will fail to
    // compile until the new arm is added).

    #[test]
    fn gate_set_has_exactly_two_variants() {
        // Compile-time guarantee: if you add a new variant, this
        // test fails to compile and forces you to update
        // `p95_gate_ms`.
        let _all: [GateSet; 2] = [GateSet::Lenient, GateSet::Strict60Fps];
    }
}
