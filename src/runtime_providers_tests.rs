use crate::{providers, runtime_providers};

fn voice(name: &str, language: &str) -> providers::VoiceSelection {
    providers::VoiceSelection {
        name: name.to_string(),
        language: language.to_string(),
        gender: providers::VoiceGender::Unspecified,
    }
}

#[test]
fn filtered_voice_catalog_prefers_target_language_prefix() {
    let catalog = vec![
        voice("en-a", "en-US"),
        voice("vi-a", "vi-VN"),
        voice("vi-b", "vi"),
    ];

    assert_eq!(
        runtime_providers::filtered_voice_catalog(&catalog, "vi-VN"),
        vec![voice("vi-a", "vi-VN"), voice("vi-b", "vi")]
    );
}

#[test]
fn filtered_voice_catalog_falls_back_to_full_catalog() {
    let catalog = vec![voice("en-a", "en-US"), voice("ja-a", "ja-JP")];

    assert_eq!(
        runtime_providers::filtered_voice_catalog(&catalog, "vi-VN"),
        catalog
    );
}

#[test]
fn next_voice_selection_cycles_through_filtered_catalog() {
    let voices = vec![voice("vi-a", "vi-VN"), voice("vi-b", "vi-VN")];

    assert_eq!(
        runtime_providers::next_voice_selection(&voices, None),
        Some(voice("vi-a", "vi-VN"))
    );
    assert_eq!(
        runtime_providers::next_voice_selection(&voices, Some("vi-a")),
        Some(voice("vi-b", "vi-VN"))
    );
    assert_eq!(
        runtime_providers::next_voice_selection(&voices, Some("vi-b")),
        None
    );
    assert_eq!(
        runtime_providers::next_voice_selection(&voices, Some("unknown")),
        Some(voice("vi-a", "vi-VN"))
    );
}

#[tokio::test]
async fn offline_stt_provider_yields_empty_non_final_result_without_network() {
    use providers::SttProvider;

    let provider = runtime_providers::OfflineSttProvider;
    let chunk = providers::PcmChunk {
        samples: vec![0i16; 320],
        sequence_number: 0,
    };
    let result = provider
        .transcribe(&chunk, "ja-JP")
        .await
        .expect("offline STT must never error");
    assert_eq!(result.text, "");
    assert_eq!(result.confidence, None);
    assert!(
        !result.is_final,
        "offline STT yields a non-final empty segment so the pipeline stays idle"
    );
}
