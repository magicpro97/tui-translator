//! US-10 (issue #735) — Google TTS 5 000-character chunking gate.
//!
//! Verifies that [`GoogleTtsProvider::synthesise`] splits texts that exceed
//! 4 900 characters into multiple `text:synthesize` HTTP requests and
//! concatenates the returned MP3 bytes in order.
//!
//! The HTTP layer is mocked with a lightweight `tokio::net::TcpListener`
//! server; the real Google TTS API is never contacted.
//!
//! # Running
//! ```sh
//! cargo test --test google_tts_chunking_test
//! ```

// Suppress dead-code warnings for items that come from the provider bridge
// but are not directly referenced by these focused tests.
#![allow(dead_code)]

// Mirror `crate::providers` so that `GoogleTtsProvider` and friends resolve.
// The #[path] includes within providers/mod.rs in turn bring in
// `google/tts.rs`, whose test-build `_segmentation` shim satisfies the
// `split_all_at_boundaries` call site without needing a full `mod pipeline`.
#[path = "../src/providers/mod.rs"]
mod providers;

use providers::google::tts::GoogleTtsProvider;
use providers::TtsProvider;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// ── Mock HTTP server ──────────────────────────────────────────────────────────

/// Spawn a minimal HTTP server that counts `POST /v1/text:synthesize` calls
/// and returns a fixed `{"audioContent":"<fake_audio>"}` JSON body for each.
///
/// Returns the server's base URL (e.g. `"http://127.0.0.1:NNNNN"`) and a
/// shared counter that increments on every accepted request.
async fn spawn_mock_tts_server() -> (String, Arc<AtomicUsize>) {
    let call_count = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock TTS server listener");
    let port = listener.local_addr().expect("get local addr").port();
    let counter = Arc::clone(&call_count);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let cnt = Arc::clone(&counter);
            tokio::spawn(async move {
                // Read the full HTTP request so reqwest can flush its send
                // buffer before we write the response (avoids ECONNRESET).
                let mut buf = vec![0u8; 131_072];
                let _ = socket.read(&mut buf).await;

                cnt.fetch_add(1, Ordering::SeqCst);

                // Return a minimal valid SynthesizeResponse.
                // base64("FAKE") = "RkFLRQ==" (4 decoded bytes per call).
                let fake_audio = STANDARD.encode(b"FAKE");
                let json_body = format!(r#"{{"audioContent":"{}"}}"#, fake_audio);
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: application/json\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n\
                     {}",
                    json_body.len(),
                    json_body
                );
                socket.write_all(http_resp.as_bytes()).await.ok();
                socket.shutdown().await.ok();
            });
        }
    });

    (format!("http://127.0.0.1:{port}"), call_count)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// A 12 000-character text must be split into exactly **3** HTTP calls, each
/// carrying at most 4 900 characters (12 000 ÷ 4 900 = 2.449 → 3 chunks).
///
/// This is the primary gate for US-10 / issue #735.
#[tokio::test]
async fn google_tts_chunking_splits_12000_char_text_into_3_calls() {
    let (base_url, call_count) = spawn_mock_tts_server().await;

    let provider = GoogleTtsProvider::new("test-key")
        .expect("construct provider with dummy key")
        .with_base_url(base_url);

    // 12 000 ASCII characters, no punctuation → hard-splits at 4 900 each.
    let text = "A".repeat(12_000);

    let result = provider.synthesise(&text, "en-US").await;
    assert!(
        result.is_ok(),
        "synthesise must succeed for valid text; err = {:?}",
        result.err()
    );

    // Give async server tasks time to finish incrementing the counter.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let count = call_count.load(Ordering::SeqCst);
    assert_eq!(
        count, 3,
        "12 000-char text must produce exactly 3 HTTP calls \
         (max 4 900 chars/chunk); got {count}"
    );

    // Audio bytes: 3 chunks × base64-decode(b"FAKE") = 3 × 4 bytes = 12 bytes.
    let tts = result.unwrap();
    assert_eq!(
        tts.audio_bytes.len(),
        12,
        "audio_bytes must be the concatenation of 3 × 4-byte fake chunks; \
         got {} bytes",
        tts.audio_bytes.len()
    );
    assert_eq!(tts.mime_type, "audio/mpeg");
}

/// A text within the 4 900-character limit must produce exactly **1** HTTP call.
///
/// Regression guard: chunking must not introduce extra calls for short text.
#[tokio::test]
async fn google_tts_chunking_short_text_makes_one_call() {
    let (base_url, call_count) = spawn_mock_tts_server().await;

    let provider = GoogleTtsProvider::new("test-key")
        .expect("construct provider with dummy key")
        .with_base_url(base_url);

    let text = "Hello, this is a short sentence well within the chunk limit.";

    let result = provider.synthesise(text, "en-US").await;
    assert!(
        result.is_ok(),
        "synthesise must succeed for short text; err = {:?}",
        result.err()
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let count = call_count.load(Ordering::SeqCst);
    assert_eq!(
        count, 1,
        "short text must produce exactly 1 HTTP call; got {count}"
    );
}

/// A text of exactly 4 900 characters (the boundary value) must produce
/// exactly **1** HTTP call — it must not be split.
///
/// Boundary-value gate: `split_all_at_boundaries(text, 4900)` with a
/// 4 900-char input must return a single-element `Vec`.
#[tokio::test]
async fn google_tts_chunking_exactly_max_chars_makes_one_call() {
    let (base_url, call_count) = spawn_mock_tts_server().await;

    let provider = GoogleTtsProvider::new("test-key")
        .expect("construct provider with dummy key")
        .with_base_url(base_url);

    let text = "B".repeat(4_900);

    let result = provider.synthesise(&text, "en-US").await;
    assert!(
        result.is_ok(),
        "synthesise must succeed for boundary text; err = {:?}",
        result.err()
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let count = call_count.load(Ordering::SeqCst);
    assert_eq!(
        count, 1,
        "4 900-char text must produce exactly 1 HTTP call; got {count}"
    );
}
