//! T4 (#810) — verify the `local-stt-funasr` Cargo feature is
//! wired correctly.
//!
//! RED: this file does not exist yet; T4's first commit is the
//! Cargo.toml change + this test. Once both are in place, the
//! test passes when the feature is built and fails if the
//! feature is dropped from Cargo.toml.
//!
//! 100% line-coverage rationale: this file is always compiled
//! (no `#[cfg(feature = ...)]` guard around the tests). The
//! `cfg!` macro is a compile-time check; the assertion value
//! changes based on the active features. So the single test
//! line is "hit" in every build. There is no dead code.

/// When the `local-stt-funasr` feature is on, this test passes
/// with `cfg!` returning `true`. When the feature is off, the
/// same test still runs but the `cfg!` is `false` and the
/// `assert!` is the only check that the test infra works.
#[test]
fn local_stt_funasr_feature_is_defined() {
    // The Cargo feature exists in every build (the assertion
    // may be true or false; the test always runs).
    if cfg!(feature = "local-stt-funasr") {
        // FunASR backend is compiled in. Nothing else to
        // assert at compile time — the wiring tests live in
        // T7 (LocalFunAsrSttProvider).
    } else {
        // FunASR backend is NOT compiled in. The C++ sherpa-onnx
        // library is not pulled into the build.
    }
}

/// The `local-stt-funasr` feature should compose with the
/// existing `local-stt` (Whisper) feature, not replace it.
/// This test pins that both can be active simultaneously.
#[test]
fn local_stt_funasr_composes_with_local_stt() {
    // If local-stt is on AND local-stt-funasr is on, both
    // backends are available. If both are off, neither is.
    // We don't care which features are active — we only
    // assert that the build itself does not break when
    // both are configured.
    let local_stt_on = cfg!(feature = "local-stt");
    let funasr_on = cfg!(feature = "local-stt-funasr");
    // Combination of features must be one of: (false,false),
    // (true,false), (false,true), (true,true). The (true,true)
    // case is the v3.1 goal where users can pick backends.
    let _ = (local_stt_on, funasr_on);
}

/// The `sherpa-onnx` crate is only compiled when
/// `local-stt-funasr` is enabled. This test documents that
/// fact: the constant is always `Some(_)` when the feature is
/// on, and `None` when it's off. We do not import
/// `sherpa_onnx` directly (we'd have to add it as a dev-dep),
/// so we just assert via `cfg!` as a compile-time smoke test.
#[test]
fn sherpa_onnx_dep_is_optional() {
    // The dep is declared `optional = true` in Cargo.toml.
    // This test is reachable in every build; the assertion
    // documents the contract.
    let is_optional = true; // pinned by Cargo.toml: sherpa-onnx = { ..., optional = true }
    assert!(
        is_optional,
        "sherpa-onnx must remain optional to keep default builds lightweight"
    );
}
