//! Google Gemini 3.5 Live Translate provider.
//!
//! This file implements [`CloudStreamProvider`] for the Gemini 3.5 Live
//! Translate endpoint.  We hand-roll the WebSocket client on top of
//! `tokio-tungstenite` because the third-party `gemini-live` crate
//! (v0.1.8) does not expose the `translationConfig` field that makes
//! the Live API behave as a translator.
//!
//! # Architecture
//!
//! ```text
//! caller ──► CloudStreamSession ──(mpsc)──► transport task
//!                                           │
//!                                           ├─► WSS write loop
//!                                           │
//!                                           ├─► WSS read loop
//!                                           │
//!                                           └─► (broadcast) ──► events()
//! ```
//!
//! On `open()` we spawn a single transport task that owns the
//! WebSocket.  The task:
//!
//! 1. Sends the `setup` frame.
//! 2. Waits for `setupComplete`.
//! 3. Pumps `realtimeInput` audio frames to the server.
//! 4. Decodes `serverContent` / `usageMetadata` / `goAway` / `error`
//!    frames into `CloudStreamEvent`s and broadcasts them.
//! 5. On `EndOfStream` from the audio channel, sends the
//!    `audioStreamEnd` frame and reads until the server closes.
//! 6. Closes the WS gracefully on drop of all `CloudStreamSession`
//!    handles.
//!
//! # Error mapping
//!
//! All wire failures (network, auth, parse) are surfaced via
//! `CloudError`.  Terminal `error` frames are mapped to
//! `CloudError::Auth` / `RateLimit` / `SetupFailed` / `Protocol`
//! by [`protocol::check_terminal_error`].

use std::sync::Arc;
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info, warn};

use super::config::CloudConfig;
use super::protocol::{
    build_audio_frame, build_stream_end_frame, build_system_instruction, into_events,
    check_terminal_error, EmptyObject, RealtimeInput, ServerMessage, SetupMessage,
    TranslationStyle,
};
use super::{
    AudioCommand, CloudError, CloudStreamEvent, CloudStreamProvider, CloudStreamSession,
    CloudVendor,
};

/// The exact model id we pin to.  This is a preview model released
/// 2026-06-09; see `docs/adr/0008-rev1-adopt-gemini-live-translate.md`
/// for the rationale.
pub const GEMINI_LIVE_TRANSLATE_MODEL: &str = "models/gemini-3.5-live-translate-preview";

/// Production WSS endpoint for the Gemini Developer API (AI Studio
/// / MakerSuite).  Vertex AI uses a different host; add a separate
/// variant when we want to support it.
const WSS_URL: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

/// Channel buffer for the audio input side.  We size it so that the
/// transport task can survive a brief scheduler hiccup without
/// blocking the producer.  At 16 kHz mono, 16 frames = 1.6 s of
/// audio; well above the typical jitter on a desktop.
const AUDIO_CHANNEL_DEPTH: usize = 16;

/// Channel buffer for the event broadcast side.  Events are
/// single-line text; 64 is plenty for the TUI to fall behind on a
/// slow frame and still catch up.
const EVENT_CHANNEL_DEPTH: usize = 64;

/// Provider for Google Gemini 3.5 Live Translate.
///
/// Constructed once per `AppConfig`; cheap to clone (the heavy state
/// lives behind `Arc` in the WebSocket task).
#[derive(Debug, Clone)]
pub struct GeminiLiveTranslateProvider {
    cfg: Arc<CloudConfig>,
}

impl GeminiLiveTranslateProvider {
    /// Build a provider from a validated [`CloudConfig`].  Caller is
    /// responsible for `cfg.validate()` having passed.
    pub fn new(cfg: CloudConfig) -> Self {
        Self { cfg: Arc::new(cfg) }
    }

    /// Cheap access to the underlying config (used by tests and by
    /// `--print-system-info`).
    pub fn config(&self) -> &CloudConfig {
        &self.cfg
    }
}

impl CloudStreamProvider for GeminiLiveTranslateProvider {
    fn vendor(&self) -> CloudVendor {
        CloudVendor::GeminiLiveTranslate
    }

    fn open(&self) -> Result<CloudStreamSession, CloudError> {
        let api_key = self.cfg.resolve_api_key().map_err(CloudError::Auth)?;
        let (audio_tx, audio_rx) = mpsc::channel::<AudioCommand>(AUDIO_CHANNEL_DEPTH);
        let (event_tx, _) = tokio::sync::broadcast::channel::<CloudStreamEvent>(EVENT_CHANNEL_DEPTH);
        let (close_tx, close_rx) = watch::channel(false);

        let cfg = Arc::clone(&self.cfg);
        let event_tx_for_task = event_tx.clone();
        let _handle: JoinHandle<()> = tokio::spawn(transport_task(
            cfg,
            api_key,
            audio_rx,
            event_tx_for_task,
            close_rx,
        ));

        Ok(CloudStreamSession::new(audio_tx, event_tx, close_tx))
    }
}

// ── Transport task ──────────────────────────────────────────────────────────

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn transport_task(
    cfg: Arc<CloudConfig>,
    api_key: String,
    mut audio_rx: mpsc::Receiver<AudioCommand>,
    event_tx: tokio::sync::broadcast::Sender<CloudStreamEvent>,
    mut close_rx: watch::Receiver<bool>,
) {
    debug!(target: "tui_translator::cloud", "transport task starting");
    let result = run_session(cfg, api_key, &mut audio_rx, &event_tx, &mut close_rx).await;
    if let Err(e) = result {
        warn!(target: "tui_translator::cloud", "session ended with error: {e}");
        let _ = event_tx.send(CloudStreamEvent::Closed {
            reason: e.to_string(),
        });
    } else {
        debug!(target: "tui_translator::cloud", "session ended cleanly");
    }
}

async fn run_session(
    cfg: Arc<CloudConfig>,
    api_key: String,
    audio_rx: &mut mpsc::Receiver<AudioCommand>,
    event_tx: &tokio::sync::broadcast::Sender<CloudStreamEvent>,
    close_rx: &mut watch::Receiver<bool>,
) -> Result<(), CloudError> {
    let ws = connect_ws(&api_key).await?;
    let (mut write, mut read) = ws.split();

    // 1. Send setup, wait for setupComplete
    let setup = build_setup(&cfg);
    let setup_json = serde_json::to_string(&setup).map_err(|e| {
        CloudError::Internal(format!("failed to serialize setup: {e}"))
    })?;
    write
        .send(Message::Text(setup_json))
        .await
        .map_err(CloudError::from)?;
    debug!(target: "tui_translator::cloud", "setup sent, awaiting setupComplete");

    // 2. Wait for setupComplete.  Until it arrives, any server frames
    //    are unexpected (and we treat them as `SetupFailed`).
    loop {
        let frame = read
            .next()
            .await
            .ok_or_else(|| CloudError::SetupFailed("ws closed before setupComplete".into()))?
            .map_err(CloudError::from)?;
        let text = frame_to_text(frame)?;
        let msg: ServerMessage = serde_json::from_str(&text)
            .map_err(|e| CloudError::Protocol(format!("setup response parse: {e}; body={text}")))?;
        check_terminal_error(&msg)?;
        if msg.setup_complete.is_some() {
            break;
        }
        // Ignore other frames during setup handshake (e.g. early
        // usageMetadata).  Continue reading.
    }
    info!(target: "tui_translator::cloud", "session ready (setupComplete received)");

    // 3. Drive the session: pump audio, drain events, watch for
    //    close signals.  We use a simple `tokio::select!` over the
    //    three relevant futures.
    let mut saw_close = false;
    while !saw_close {
        tokio::select! {
            biased;

            // 3a. Audio input.
            cmd = audio_rx.recv() => {
                match cmd {
                    Some(AudioCommand::Pcm(pcm)) => {
                        let frame = build_audio_frame(&pcm);
                        let s = serde_json::to_string(&frame)
                            .map_err(|e| CloudError::Internal(format!("frame serialize: {e}")))?;
                        write.send(Message::Text(s)).await.map_err(CloudError::from)?;
                    }
                    Some(AudioCommand::EndOfStream) => {
                        let frame = build_stream_end_frame();
                        let s = serde_json::to_string(&frame)
                            .map_err(|e| CloudError::Internal(format!("end-of-stream serialize: {e}")))?;
                        write.send(Message::Text(s)).await.map_err(CloudError::from)?;
                        debug!(target: "tui_translator::cloud", "sent audioStreamEnd; awaiting server close");
                    }
                    None => {
                        // Audio channel closed (all CloudStreamSession
                        // clones dropped).  Send end-of-stream
                        // gracefully and wait for the server to
                        // close.
                        let frame = build_stream_end_frame();
                        if let Ok(s) = serde_json::to_string(&frame) {
                            let _ = write.send(Message::Text(s)).await;
                        }
                        // Best-effort WS close.
                        let _ = write.close().await;
                        saw_close = true;
                    }
                }
            }

            // 3b. Server messages.
            frame = read.next() => {
                let frame = match frame {
                    Some(Ok(f)) => f,
                    Some(Err(e)) => return Err(CloudError::from(e)),
                    None => {
                        // Server closed the WS cleanly.
                        return Ok(());
                    }
                };
                if matches!(frame, Message::Close(_)) {
                    return Ok(());
                }
                let text = match frame_to_text(frame) {
                    Ok(s) => s,
                    Err(e) => {
                        // Non-text frames (binary ping/pong, etc.)
                        // are best-effort ignored.
                        debug!(target: "tui_translator::cloud", "ignoring non-text frame: {e}");
                        continue;
                    }
                };
                let msg: ServerMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(target: "tui_translator::cloud", "frame parse failed: {e}; body={text}");
                        continue;
                    }
                };
                if let Err(e) = check_terminal_error(&msg) {
                    return Err(e);
                }
                for ev in into_events(msg) {
                    let is_close = matches!(ev, CloudStreamEvent::Closed { .. });
                    // Ignore send errors — receivers may have been
                    // dropped, that's fine.
                    let _ = event_tx.send(ev);
                    if is_close {
                        return Ok(());
                    }
                }
            }

            // 3c. Close signal.
            _ = close_rx.changed() => {
                if *close_rx.borrow() {
                    let _ = write.close().await;
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

// ── WS connect ──────────────────────────────────────────────────────────────

async fn connect_ws(api_key: &str) -> Result<WsStream, CloudError> {
    let mut req = WSS_URL
        .into_client_request()
        .map_err(|e| CloudError::SetupFailed(format!("invalid WSS URL: {e}")))?;
    // API key goes in the `x-goog-api-key` header on the upgrade
    // request; we also accept `?key=...` query param but the header
    // is the recommended path per Google docs (2026-06-20).
    let header_val = HeaderValue::from_str(api_key)
        .map_err(|e| CloudError::Auth(format!("invalid API key header: {e}")))?;
    req.headers_mut().insert("x-goog-api-key", header_val);

    let connect_result = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async(req),
    )
    .await
    .map_err(|_| CloudError::Network("ws connect timeout (10s)".into()))?;

    let (ws, _resp) = connect_result
        .map_err(|e| CloudError::Network(format!("ws connect: {e}")))?;
    info!(target: "tui_translator::cloud", "ws connected");
    Ok(ws)
}

/// Convert a WS frame to its UTF-8 text payload.  Returns
/// `Protocol(...)` for non-text frames (binary, ping, pong, close).
fn frame_to_text(frame: Message) -> Result<String, CloudError> {
    match frame {
        Message::Text(s) => Ok(s),
        Message::Binary(_) => Err(CloudError::Protocol("unexpected binary frame".into())),
        Message::Ping(_) | Message::Pong(_) => Err(CloudError::Protocol("ping/pong".into())),
        Message::Close(_) => Err(CloudError::SessionClosed("server sent close".into())),
        Message::Frame(_) => Err(CloudError::Protocol("raw frame".into())),
    }
}

// ── Setup message construction ──────────────────────────────────────────────

fn build_setup(cfg: &CloudConfig) -> SetupMessage {
    use super::protocol::{GenerationConfig, Modality, SetupBody, TranslationConfig};

    let style = cfg.style;
    let system_instruction = if style == TranslationStyle::Neutral {
        // Empty instruction is fine; the model defaults to neutral.
        None
    } else {
        Some(build_system_instruction(style))
    };

    SetupMessage {
        setup: SetupBody {
            model: GEMINI_LIVE_TRANSLATE_MODEL.to_string(),
            generation_config: GenerationConfig {
                response_modalities: vec![Modality::Text],
                temperature: Some(0.0),
                top_p: Some(0.95),
            },
            translation_config: TranslationConfig {
                target_language_code: cfg.target_language.clone(),
                echo_target_language: cfg.echo_target_language,
            },
            system_instruction,
            input_audio_transcription: EmptyObject {},
            output_audio_transcription: EmptyObject {},
        },
    }
}

/// Public re-export of [`build_setup`] for the standalone cloud
/// binary's `--dry-run` mode and for integration tests.  The
/// name has a `_public` suffix so it is distinct from the
/// private helper used by the transport task.
pub fn build_setup_public(cfg: &CloudConfig) -> SetupMessage {
    build_setup(cfg)
}

// Suppress unused-import warning for `RealtimeInput` which is not
// directly named in this file but is the type returned by
// `build_audio_frame` / `build_stream_end_frame`.  Rust doesn't
// require the import but clippy on stricter modes might.
const _: Option<RealtimeInput> = None;

// ── Tests (offline) ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cfg() -> CloudConfig {
        CloudConfig {
            vendor: super::super::CloudVendor::GeminiLiveTranslate,
            api_key: Some("test-key".into()),
            api_key_env: None,
            target_language: "vi".into(),
            style: TranslationStyle::Neutral,
            echo_target_language: false,
            track_usage: true,
        }
    }

    #[test]
    fn provider_vendor_reports_gemini() {
        let p = GeminiLiveTranslateProvider::new(test_cfg());
        assert_eq!(p.vendor(), CloudVendor::GeminiLiveTranslate);
    }

    #[test]
    fn build_setup_for_translate_target_vi() {
        let cfg = test_cfg();
        let s = build_setup(&cfg);
        // The pinned model id must be present.
        assert_eq!(
            s.setup.model,
            "models/gemini-3.5-live-translate-preview"
        );
        // The target language must round-trip.
        assert_eq!(s.setup.translation_config.target_language_code, "vi");
        // echo_target_language defaults to false; the field is
        // omitted in the JSON because of `skip_serializing_if`.
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert!(v["setup"]["translationConfig"]
            .as_object()
            .unwrap()
            .get("echoTargetLanguage")
            .is_none());
    }

    #[test]
    fn build_setup_with_non_neutral_style_includes_system_instruction() {
        let mut cfg = test_cfg();
        cfg.style = TranslationStyle::Technical;
        let s = build_setup(&cfg);
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        let text = v["setup"]["systemInstruction"]["parts"][0]["text"]
            .as_str()
            .expect("system instruction text");
        // Should mention "technical register".
        assert!(text.contains("technical"), "got: {text}");
    }

    #[test]
    fn build_setup_with_echo_target_language_includes_flag() {
        let mut cfg = test_cfg();
        cfg.echo_target_language = true;
        let s = build_setup(&cfg);
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(
            v["setup"]["translationConfig"]["echoTargetLanguage"],
            serde_json::Value::Bool(true)
        );
    }

    #[test]
    fn build_audio_frame_encodes_pcm_as_base64() {
        // 3200 bytes of arbitrary PCM payload.  We don't care about
        // the actual sample values here — just that the encoder
        // correctly base64-encodes them.  At 16 kHz mono, 3200 bytes
        // = 100 ms.
        let pcm: Vec<u8> = (0u16..3200u16).map(|i| (i & 0xff) as u8).collect();
        let frame = build_audio_frame(&pcm);
        // mime type is fixed
        assert_eq!(frame.audio.as_ref().unwrap().mime_type, "audio/pcm;rate=16000");
        // base64 length: 3200 bytes raw → 4 * ceil(3200/3) = 4268 chars.
        let b64 = &frame.audio.as_ref().unwrap().data;
        assert_eq!(b64.len(), 4268);
        // End-of-stream signal is suppressed on the audio frame.
        assert!(frame.audio_stream_end.is_none());
    }

    #[test]
    fn frame_to_text_rejects_binary() {
        let f = Message::Binary(vec![1, 2, 3]);
        let err = frame_to_text(f).unwrap_err();
        assert!(matches!(err, CloudError::Protocol(_)));
    }

    #[test]
    fn frame_to_text_accepts_text() {
        let f = Message::Text("hello".into());
        let s = frame_to_text(f).unwrap();
        assert_eq!(s, "hello");
    }

    /// Integration-style test: build the full setup JSON and parse it
    /// back as `serde_json::Value`, then assert each top-level
    /// shape requirement from Google's docs.
    #[test]
    fn setup_message_matches_google_docs_shape() {
        let cfg = test_cfg();
        let s = build_setup(&cfg);
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        // The wire format has `{"setup": {...}}` at the top.
        assert!(v.get("setup").is_some());
        let setup = &v["setup"];
        // model is a string.
        assert!(setup["model"].is_string());
        // generationConfig.responseModalities is a non-empty array.
        assert!(setup["generationConfig"]["responseModalities"]
            .as_array()
            .unwrap()
            .len() > 0);
        // translationConfig.targetLanguageCode is set.
        assert!(setup["translationConfig"]["targetLanguageCode"].is_string());
        // Both transcription configs are empty objects.
        assert_eq!(setup["inputAudioTranscription"], serde_json::json!({}));
        assert_eq!(setup["outputAudioTranscription"], serde_json::json!({}));
    }
}

// Suppress `unused_import` for `RealtimeInput` which is exercised only
// through `build_audio_frame` / `build_stream_end_frame`.
#[allow(dead_code)]
const _REALTIME_INPUT_MARKER: fn() -> Option<RealtimeInput> = || None;
