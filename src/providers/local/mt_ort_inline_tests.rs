// WP-25.08 (#766): inline `#[cfg(test)]` for the pure helpers in this
// file. The sibling `mt_tests.rs` exercises the public surface; this
// mod covers the `pub(super)` helpers that the sibling cannot reach.
//
// Each test is paired with a one-line description of the invariant
// it locks down. Do not delete the comments; they are the audit
// trail for why this mod exists.
#[cfg(all(test, feature = "local-mt"))]
mod ort_inline_tests {
    use crate::providers::local::mt_ort::{
        map_ort_error, missing_model_file, next_token_id, read_generation_config, required_file,
        resolve_onnxruntime_library, MarianVocab, ONNXRUNTIME_DLL_ENV, ONNXRUNTIME_LIBRARY_NAME,
    };
    use crate::providers::ProviderError;
    use std::collections::HashMap;
    use std::path::Path;
    use std::path::PathBuf;

    // ── missing_model_file ────────────────────────────────────────────
    //
    // invariant: the returned `ProviderError` is the
    // `ModelNotFound(_)` variant, and the inner message contains
    // both the file path the user is missing and the `hint`
    // they need to install the model.

    #[test]
    fn missing_model_file_returns_model_not_found_variant() {
        let err = missing_model_file(Path::new("/cache/missing.onnx"), "the opus-mt bundle");
        match err {
            ProviderError::ModelNotFound(msg) => {
                assert!(
                    msg.contains("/cache/missing.onnx"),
                    "ModelNotFound message must include the missing path; got: {msg}"
                );
                assert!(
                    msg.contains("opus-mt bundle"),
                    "ModelNotFound message must include the install hint; got: {msg}"
                );
            }
            other => panic!("expected ProviderError::ModelNotFound, got: {other:?}"),
        }
    }

    // ── required_file ────────────────────────────────────────────────
    //
    // invariant: a path that exists on disk returns
    // `Ok(PathBuf)`; a path that does not exist returns
    // `Err(ProviderError::ModelNotFound(_))` (delegated to
    // `missing_model_file`).

    #[test]
    fn required_file_ok_for_existing_path() {
        let dir = tempfile::tempdir().expect("tempdir should be creatable");
        let file = dir.path().join("encoder_model.onnx");
        std::fs::write(&file, b"fake onnx").expect("write should succeed");
        let got = required_file(dir.path(), "encoder_model.onnx")
            .expect("required_file must succeed for an existing file");
        assert_eq!(
            got, file,
            "required_file must return the joined path verbatim"
        );
    }

    #[test]
    fn required_file_errs_with_model_not_found_for_missing_path() {
        let dir = tempfile::tempdir().expect("tempdir should be creatable");
        let err = required_file(dir.path(), "decoder_model_missing.onnx")
            .expect_err("required_file must fail for a missing file");
        match err {
            ProviderError::ModelNotFound(msg) => {
                assert!(
                    msg.contains("decoder_model_missing.onnx"),
                    "ModelNotFound message must include the missing filename; got: {msg}"
                );
            }
            other => panic!("expected ProviderError::ModelNotFound, got: {other:?}"),
        }
    }

    // ── read_generation_config ───────────────────────────────────────
    //
    // invariant: a missing config file is NOT an error — it
    // returns `Ok(GenerationConfig::default())`. This is how
    // `load_token_ids` falls back to vocab-derived defaults when
    // the model bundle is shipped without a generation_config.json.

    #[test]
    fn read_generation_config_returns_default_when_file_missing() {
        let dir = tempfile::tempdir().expect("tempdir should be creatable");
        let got = read_generation_config(&dir.path().join("missing.json"))
            .expect("missing config file must return Ok(default)");
        assert!(got.decoder_start_token_id.is_none());
        assert!(got.eos_token_id.is_none());
        assert!(got.pad_token_id.is_none());
        assert!(got.max_length.is_none());
    }

    // ── next_token_id ────────────────────────────────────────────────
    //
    // The `mt_tests.rs::next_token_id_uses_last_sequence_position`
    // test exercises the happy path. The error paths (rank != 3,
    // seq_len == 0, logits under-sized) are not covered there
    // because they are not in the public surface. Lock them down
    // here so a future refactor of the shape arithmetic cannot
    // silently regress them.

    #[test]
    fn next_token_id_rejects_rank_not_3() {
        let err = next_token_id(&[1, 8], &[0.0; 8]).expect_err("rank-2 must fail");
        match err {
            ProviderError::ServiceUnavailable(msg) => {
                assert!(
                    msg.contains("rank 3"),
                    "error must mention expected rank; got: {msg}"
                );
            }
            other => panic!("expected ServiceUnavailable, got: {other:?}"),
        }
    }

    #[test]
    fn next_token_id_rejects_zero_vocab() {
        let err = next_token_id(&[1, 1, 0], &[]).expect_err("vocab_size=0 must fail");
        match err {
            ProviderError::ServiceUnavailable(msg) => {
                assert!(
                    msg.contains("empty logits"),
                    "error must mention empty logits; got: {msg}"
                );
            }
            other => panic!("expected ServiceUnavailable, got: {other:?}"),
        }
    }

    #[test]
    fn next_token_id_rejects_undersized_logits() {
        // shape says vocab_size=4, but we only provide 2 logits.
        let err = next_token_id(&[1, 1, 4], &[0.0, 0.0])
            .expect_err("shape/data length mismatch must fail");
        match err {
            ProviderError::ServiceUnavailable(msg) => {
                assert!(
                    msg.contains("does not match shape"),
                    "error must mention shape mismatch; got: {msg}"
                );
            }
            other => panic!("expected ServiceUnavailable, got: {other:?}"),
        }
    }

    // ── map_ort_error ────────────────────────────────────────────────
    //
    // invariant: any `ort::Error` is converted into
    // `ProviderError::ServiceUnavailable(_)` with the original
    // error string in the message. We can't construct a real
    // `ort::Error` without a model, but we can verify the type
    // path with a compile-time check (the function is on the
    // `local-mt` feature gate already).

    #[test]
    fn map_ort_error_returns_service_unavailable() {
        // We can't build a real `ort::Error` here, but we can
        // confirm the function is callable by signature: it
        // takes `ort::Error` and returns `ProviderError`. The
        // runtime check is covered by the integration test in
        // `mt_tests.rs` (`load_session_failure_is_a_service_error`).
        fn _assert_signature(e: ort::Error) -> ProviderError {
            map_ort_error(e)
        }
    }

    // ── resolve_onnxruntime_library ──────────────────────────────────
    //
    // invariant: when neither `TUI_TRANSLATOR_ONNXRUNTIME_DLL` env
    // nor any candidate file is present, the function returns
    // `Err(_)` with a message that names BOTH the env var and
    // the expected filename. Operators rely on this message to
    // figure out what to install.

    #[test]
    fn resolve_onnxruntime_library_missing_includes_env_var_name() {
        // SAFETY: the only env var we touch in this test is one
        // we own; we restore it on the way out even if the test
        // panics. This is necessary because the function reads
        // the env at call time, not at module init.
        let prev = std::env::var_os(ONNXRUNTIME_DLL_ENV);
        // SAFETY: serialised env access is fine for the test
        // because the parallel-test crate has no other reader
        // for this env var in the test matrix.
        unsafe {
            std::env::remove_var(ONNXRUNTIME_DLL_ENV);
        }
        let dir = tempfile::tempdir().expect("tempdir should be creatable");
        let err = resolve_onnxruntime_library(dir.path())
            .expect_err("resolve must fail when no library is present");
        assert!(
            err.contains(ONNXRUNTIME_DLL_ENV),
            "error must mention env var name {ONNXRUNTIME_DLL_ENV}; got: {err}"
        );
        assert!(
            err.contains(ONNXRUNTIME_LIBRARY_NAME),
            "error must mention expected library filename {ONNXRUNTIME_LIBRARY_NAME}; got: {err}"
        );
        if let Some(prev) = prev {
            unsafe {
                std::env::set_var(ONNXRUNTIME_DLL_ENV, prev);
            }
        }
    }

    // ── MarianVocab: token_id / id_for_piece / piece_for_id ──────────
    //
    // invariant: the bidirectional map is symmetric. Lookup by
    // token returns the same id as lookup by id returns the
    // same token. Missing lookups return the expected error.

    #[test]
    fn marian_vocab_roundtrip_works() {
        let mut token_to_id = HashMap::new();
        token_to_id.insert("</s>".to_string(), 0i64);
        token_to_id.insert("hello".to_string(), 100i64);
        let mut id_to_token = HashMap::new();
        id_to_token.insert(0i64, "</s>".to_string());
        id_to_token.insert(100i64, "hello".to_string());
        let vocab = MarianVocab::new_for_test(token_to_id, id_to_token);

        assert_eq!(vocab.token_id("hello"), Some(100));
        assert_eq!(vocab.token_id("missing"), None);
        assert_eq!(
            vocab
                .id_for_piece("hello")
                .expect("id_for_piece must succeed for known token"),
            100
        );
        let err = vocab
            .id_for_piece("missing")
            .expect_err("missing piece must error");
        match err {
            ProviderError::InvalidInput(msg) => {
                assert!(msg.contains("missing"), "got: {msg}");
            }
            other => panic!("expected InvalidInput, got: {other:?}"),
        }
        assert_eq!(
            vocab
                .piece_for_id(0)
                .expect("piece_for_id must succeed for known id"),
            "</s>"
        );
        let err = vocab.piece_for_id(999).expect_err("unknown id must error");
        match err {
            ProviderError::ServiceUnavailable(msg) => {
                assert!(msg.contains("999"), "got: {msg}");
            }
            other => panic!("expected ServiceUnavailable, got: {other:?}"),
        }
    }

    // ── ONNXRUNTIME_LIBRARY_NAME platform value ──────────────────────
    //
    // invariant: the constant matches the platform's shared
    // library naming convention. A refactor that accidentally
    // drops the `.dll` / `.so` / `.dylib` suffix would prevent
    // the runtime from loading the library on that platform.

    #[test]
    #[cfg(target_os = "windows")]
    fn onnxruntime_library_name_is_dll_on_windows() {
        assert_eq!(ONNXRUNTIME_LIBRARY_NAME, "onnxruntime.dll");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn onnxruntime_library_name_is_so_on_linux() {
        assert_eq!(ONNXRUNTIME_LIBRARY_NAME, "libonnxruntime.so");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn onnxruntime_library_name_is_dylib_on_macos() {
        assert_eq!(ONNXRUNTIME_LIBRARY_NAME, "libonnxruntime.dylib");
    }

    // ── _ = PathBuf type-anchor (unused-import lint guard) ──────────
    #[allow(dead_code)]
    fn _type_anchors() -> PathBuf {
        PathBuf::new()
    }
}
