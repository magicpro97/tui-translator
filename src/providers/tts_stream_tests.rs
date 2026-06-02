//! Unit tests for `TtsStreamProvider` default implementation
//! (extracted from `mod.rs` for the module-budget refactor; behaviour unchanged).

use super::*;
use tokio::sync::mpsc;

struct BufferOnlyProvider {
    bytes: Vec<u8>,
    mime: String,
}

impl TtsProvider for BufferOnlyProvider {
    async fn synthesise(
        &self,
        _text: &str,
        _language_code: &str,
    ) -> Result<TtsResult, ProviderError> {
        Ok(TtsResult {
            audio_bytes: self.bytes.clone(),
            mime_type: self.mime.clone(),
        })
    }
}

impl TtsStreamProvider for BufferOnlyProvider {}

#[tokio::test]
async fn default_stream_impl_emits_single_final_chunk() {
    let provider = BufferOnlyProvider {
        bytes: vec![1, 2, 3, 4],
        mime: "audio/mpeg".to_string(),
    };
    let (tx, mut rx) = mpsc::channel(4);
    provider.synthesise_stream("hello", "en-US", &tx).await;
    drop(tx);

    let first = rx.recv().await.expect("one chunk expected");
    let chunk = first.expect("synthesis succeeded");
    assert_eq!(chunk.audio_bytes, vec![1, 2, 3, 4]);
    assert_eq!(chunk.mime_type, "audio/mpeg");
    assert_eq!(chunk.sequence_number, 0);
    assert!(
        chunk.is_final,
        "buffer-only providers MUST emit is_final=true"
    );
    assert!(rx.recv().await.is_none(), "no further chunks after final");
}

struct FailingProvider;

impl TtsProvider for FailingProvider {
    async fn synthesise(
        &self,
        _text: &str,
        _language_code: &str,
    ) -> Result<TtsResult, ProviderError> {
        Err(ProviderError::ServiceUnavailable("down".to_string()))
    }
}

impl TtsStreamProvider for FailingProvider {}

#[tokio::test]
async fn default_stream_impl_propagates_error_once() {
    let (tx, mut rx) = mpsc::channel(4);
    FailingProvider.synthesise_stream("x", "en-US", &tx).await;
    drop(tx);

    let first = rx.recv().await.expect("one error expected");
    assert!(matches!(first, Err(ProviderError::ServiceUnavailable(_))));
    assert!(rx.recv().await.is_none(), "no further messages after error");
}

#[tokio::test]
async fn default_stream_impl_tolerates_dropped_receiver() {
    let provider = BufferOnlyProvider {
        bytes: vec![0xAA, 0xBB],
        mime: "audio/mpeg".to_string(),
    };
    let (tx, rx) = mpsc::channel(1);
    drop(rx);
    provider.synthesise_stream("y", "en-US", &tx).await;
}
