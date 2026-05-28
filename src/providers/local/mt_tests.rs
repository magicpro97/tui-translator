use super::*;

#[test]
fn primary_language_subtag_accepts_bcp47_regions() {
    assert_eq!(primary_language_subtag("ja-JP"), "ja");
    assert_eq!(primary_language_subtag("VI"), "vi");
    assert_eq!(primary_language_subtag("zh_Hant_TW"), "zh");
}

#[cfg(not(feature = "local-mt"))]
#[tokio::test]
async fn empty_input_returns_empty_output_without_local_mt_feature() {
    let provider = LocalOpusMtProvider::stub_for_test(OpusMtLanguagePair::JapaneseToVietnamese);

    let result = provider.translate("   ", "ja-JP", "vi").await.unwrap();

    assert_eq!(result.translated_text, "");
    assert_eq!(result.detected_source_language.as_deref(), Some("ja"));
}

#[cfg(not(feature = "local-mt"))]
#[tokio::test]
async fn unsupported_target_language_is_rejected_before_model_call() {
    let provider = LocalOpusMtProvider::stub_for_test(OpusMtLanguagePair::JapaneseToVietnamese);

    let err = provider
        .translate("おはようございます", "ja-JP", "en")
        .await
        .unwrap_err();

    assert!(matches!(err, ProviderError::InvalidInput(_)));
}

#[cfg(not(feature = "local-mt"))]
#[tokio::test]
async fn non_empty_input_requires_local_mt_feature_in_default_build() {
    let provider = LocalOpusMtProvider::stub_for_test(OpusMtLanguagePair::JapaneseToVietnamese);

    let err = provider
        .translate("おはようございます", "ja-JP", "vi")
        .await
        .unwrap_err();

    assert!(matches!(err, ProviderError::Unimplemented(_)));
}

#[cfg(feature = "local-mt")]
#[test]
fn next_token_id_uses_last_sequence_position() {
    use super::super::mt_ort::next_token_id;

    let shape = vec![1, 2, 3];
    let logits = vec![9.0, 1.0, 0.0, -1.0, 5.0, 4.0];

    assert_eq!(next_token_id(&shape, &logits).unwrap(), 1);
}

#[cfg(feature = "local-mt")]
#[test]
fn marian_vocab_maps_sentencepiece_pieces_to_model_ids() {
    use super::super::mt_ort::MarianVocab;
    use std::collections::HashMap;

    let token_to_id = HashMap::from([
        ("</s>".to_string(), 0),
        ("<pad>".to_string(), 64501),
        ("▁おはようございます".to_string(), 27586),
        ("▁Chào".to_string(), 1428),
    ]);
    let id_to_token = token_to_id
        .iter()
        .map(|(token, id)| (*id, token.clone()))
        .collect();
    let vocab = MarianVocab::new_for_test(token_to_id, id_to_token);

    assert_eq!(vocab.id_for_piece("▁おはようございます").unwrap(), 27586);
    assert_eq!(vocab.piece_for_id(1428).unwrap(), "▁Chào");
    assert!(matches!(
        vocab.id_for_piece("missing").unwrap_err(),
        ProviderError::InvalidInput(_)
    ));
}

#[cfg(feature = "local-mt")]
#[tokio::test]
async fn local_mt_lazy_loads_so_empty_input_does_not_require_model_files() {
    let dir = tempfile::tempdir().unwrap();
    let provider = LocalOpusMtProvider::new_japanese_to_vietnamese_from_dir(dir.path())
        .expect("constructor should not load model files eagerly");

    let result = provider.translate("   ", "ja-JP", "vi").await.unwrap();

    assert_eq!(result.translated_text, "");
    assert_eq!(result.detected_source_language.as_deref(), Some("ja"));
}

#[cfg(feature = "local-mt")]
#[tokio::test]
async fn local_mt_missing_model_is_reported_on_first_non_empty_translate() {
    let dir = tempfile::tempdir().unwrap();
    let provider = LocalOpusMtProvider::new_japanese_to_vietnamese_from_dir(dir.path())
        .expect("constructor should defer model loading");

    let err = provider
        .translate("おはようございます", "ja-JP", "vi")
        .await
        .unwrap_err();

    assert!(
        matches!(err, ProviderError::ModelNotFound(_)),
        "expected ModelNotFound, got {err:?}"
    );
}

#[cfg(feature = "local-mt")]
#[tokio::test]
#[ignore = "requires exported OPUS-MT ja-vi ONNX files outside the repo"]
async fn real_opus_mt_ja_vi_fixture_translates_non_empty() {
    let dir = std::env::var_os("TUI_TRANSLATOR_OPUS_MT_JA_VI_DIR")
        .expect("set TUI_TRANSLATOR_OPUS_MT_JA_VI_DIR to exported opus-mt-ja-vi directory");
    let provider = LocalOpusMtProvider::new_japanese_to_vietnamese_from_dir(PathBuf::from(dir))
        .expect("real local OPUS-MT provider should load");

    let empty = provider
        .translate("   ", "ja-JP", "vi")
        .await
        .expect("empty local OPUS-MT input should succeed");
    assert_eq!(empty.translated_text, "");

    let result = provider
        .translate("おはようございます", "ja-JP", "vi")
        .await
        .expect("real local OPUS-MT translation should succeed");

    eprintln!("OPUS-MT ja->vi fixture: {}", result.translated_text);
    assert!(
        !result.translated_text.trim().is_empty(),
        "expected non-empty Vietnamese output"
    );
    assert_eq!(result.detected_source_language.as_deref(), Some("ja"));
}
