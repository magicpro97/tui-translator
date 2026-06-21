//! Wire types for the Gemini 3.5 Live Translate protocol.
//!
//! Reference: <https://ai.google.dev/gemini-api/docs/live-api/live-translate>
//! (fetched 2026-06-20).
//!
//! Every struct in this file derives `Serialize` / `Deserialize` and uses
//! `#[serde(rename_all = "camelCase")]` so JSON round-trips against
//! Google's docs without manual renaming.  The wire is JSON over
//! WebSocket; no protobuf for Live API.
//!
//! # `translationConfig` is the load-bearing field
//!
//! The whole point of using `gemini-3.5-live-translate-preview` is that
//! the server handles ASR + translation in one continuous call.  The
//! `translationConfig` is what tells the server to enter that mode
//! instead of treating the request as a regular Live API agent session.
//!
//! In particular:
//!
//! - `inputAudioTranscription: {}` — empty struct that enables
//!   streaming transcripts of the *source* audio.  Used for the
//!   bilingual subtitle layout in tui-translator (left column =
//!   original language).
//! - `outputAudioTranscription: {}` — empty struct that enables
//!   streaming transcripts of the *translated* audio (even when
//!   audio output is disabled).  Used for the right column =
//!   target language.  This is the field the `gemini-live` Rust
//!   crate v0.1.8 forgot to expose, forcing us to use raw WSS.
//! - `translationConfig.targetLanguageCode` — BCP-47 target language.
//!   Source language is auto-detected by the model.
//! - `translationConfig.echoTargetLanguage` — controls whether the
//!   server emits output when the input is already in the target
//!   language.  Default false: tui-translator doesn't want double
//!   subtitles when the speaker happens to be using the target
//!   language already.
//!
//! # Style hint via system instruction
//!
//! The Live API does not expose a structured "style" field for
//! translation (unlike Cloud Translation v3's `glossaryConfig` etc).
//! We pass a short system instruction string instead.  This is a
//! pragmatic workaround; if Google adds a structured field we
//! should switch to it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::CloudError;

// ── Style hint (re-exported in mod.rs) ───────────────────────────────────────

/// Translation style hint.  Passed to the model as part of the system
/// instruction (see [`protocol::build_system_instruction`]).  The Live API
/// has no first-class style field, so this is best-effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TranslationStyle {
    #[default]
    Neutral,
    Formal,
    Casual,
    Technical,
    /// Preserve digits, code identifiers, units, dates verbatim.
    PreserveOriginalNumerics,
}

impl TranslationStyle {
    /// Phrase used in the system instruction.  Short and concrete so
    /// the model can latch onto it reliably.
    pub fn as_instruction_phrase(self) -> &'static str {
        match self {
            Self::Neutral => "Use a neutral, professional register.",
            Self::Formal => "Use a formal register; address the listener with respectful language.",
            Self::Casual => "Use a casual, friendly register; contractions are fine.",
            Self::Technical => "Use a precise technical register; prefer domain terminology over everyday words.",
            Self::PreserveOriginalNumerics => "Preserve all numbers, dates, code identifiers, and units verbatim; do not localize them.",
        }
    }
}

// ── Setup message (client → server, first frame) ────────────────────────────

/// `setup` message sent as the first WebSocket frame.  Wraps the typed
/// `SetupConfig` and the unmodeled `translationConfig` extension that
/// turns Live API into Live Translate.
///
/// We hand-build the JSON rather than reusing `gemini-live`'s typed
/// `SetupConfig` because that struct lacks a `translationConfig` field.
/// See ADR-0008-rev1 §"Verified evidence" for the failed-dependency
/// rationale.
#[derive(Debug, Clone, Serialize)]
pub struct SetupMessage {
    pub setup: SetupBody,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupBody {
    /// Always `models/gemini-3.5-live-translate-preview` for the
    /// live-translate path.  Pinning the id (not the family) keeps
    /// behaviour stable when Google rolls out 3.6 / 4.x.
    pub model: String,

    pub generation_config: GenerationConfig,

    /// Mandatory for Gemini 3.5 Live Translate.  Without this, the
    /// server treats the session as a regular agent (no translation).
    pub translation_config: TranslationConfig,

    /// Optional style hint passed to the model.  Many users skip
    /// this and rely on `style = Neutral`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<SystemInstruction>,

    /// Enable transcripts of the *input* audio so the TUI can show
    /// original-language subtitles alongside the translation.
    pub input_audio_transcription: InputTranscriptionConfig,

    /// Enable transcripts of the *output* (translated) audio even when
    /// audio output is suppressed.  Without this, the only way to
    /// surface translations is the audio stream — which we don't want.
    pub output_audio_transcription: OutputTranscriptionConfig,
}

/// A type alias so the field name in the JSON is unambiguous.
pub type InputTranscriptionConfig = EmptyObject;
pub type OutputTranscriptionConfig = EmptyObject;

/// Empty object used as a presence-activated config flag.
///
/// Wire format: `{"inputAudioTranscription": {}}` — the server reads
/// "field present" as the on-switch.  We can't model this with
/// `bool: true` because the spec mandates the empty object.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmptyObject {}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationConfig {
    /// We want text output, not audio.  The `echo_target_language`
    /// flag on `translationConfig` controls whether the *translated*
    /// audio stream is emitted (it isn't, by default).
    pub response_modalities: Vec<Modality>,

    /// Sampling temperature.  Low (0.0) gives deterministic output
    /// suitable for live captions; higher values produce more
    /// varied (and riskier) translations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Keep low for the same reason as temperature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Modality {
    Audio,
    Text,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationConfig {
    /// BCP-47 target language tag.  Source is auto-detected.
    pub target_language_code: String,

    /// Whether the server emits output audio when the input is
    /// already in the target language.  Default false: tui-translator
    /// suppresses the redundant translation rather than emitting it.
    #[serde(skip_serializing_if = "is_false_bool")]
    pub echo_target_language: bool,
}

fn is_false_bool(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemInstruction {
    pub parts: Vec<TextPart>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextPart {
    pub text: String,
}

// ── Realtime input (client → server, after setup) ───────────────────────────

/// One `realtimeInput` message.  Carries either a PCM chunk, a text
/// fragment, or a VAD/stream-end signal.  Exactly one field per message.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeInput {
    /// Base64-encoded 16 kHz mono 16-bit LE PCM (3200 bytes per 100 ms).
    /// `mime_type` is fixed to `audio/pcm;rate=16000`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioBlob>,

    /// Manual VAD: signal that user activity has started.  Requires
    /// `realtimeInputConfig.automaticActivityDetection.disabled = true`,
    /// which we set in `SetupBody` only for the v1 implementation.
    /// For now we use auto VAD and leave this unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_start: Option<EmptyObject>,

    /// Manual VAD: signal that user activity has ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_end: Option<EmptyObject>,

    /// Server-side auto VAD: end of stream notification.  Sent when
    /// the producer has no more audio to ship.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_stream_end: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioBlob {
    pub data: String, // base64
    pub mime_type: String,
}

// ── Server messages (server → client) ───────────────────────────────────────

/// One wire frame from the server.  Decoded into [`ServerEvent`]s by
/// the `events()` consumer; the user-facing [`CloudStreamEvent`] is
/// a stricter wrapper that hides fields we don't need.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerMessage {
    /// Empty object sent exactly once when the server has accepted
    /// our `setup` and is ready for `realtimeInput`.
    #[serde(default)]
    pub setup_complete: Option<EmptyObject>,

    /// Model output + transcription + turn metadata, all in one frame.
    #[serde(default)]
    pub server_content: Option<ServerContent>,

    /// Server is about to terminate the connection.  The client should
    /// reconnect.
    #[serde(default)]
    pub go_away: Option<GoAway>,

    /// Updated session resumption handle (we don't use this for v0.3.0
    /// since reconnect is a v0.4.0 concern).
    #[serde(default)]
    pub session_resumption_update: Option<serde_json::Value>,

    /// Per-frame token usage.  Sent frequently; we accumulate it
    /// into a per-session total and report to the cost dashboard.
    #[serde(default)]
    pub usage_metadata: Option<UsageMetadata>,

    /// API-level error.  We surface this as a `CloudStreamEvent::Error`
    /// and tear down the session.
    #[serde(default)]
    pub error: Option<ApiError>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerContent {
    /// Text/audio chunks generated by the model.  We don't consume
    /// the audio (text-only response_modality), but the field is
    /// still present in some setup combinations.
    #[serde(default)]
    pub model_turn: Option<ModelTurn>,

    /// True once the model has finished generating for the current
    /// turn.  Note: more model output may follow in subsequent frames.
    #[serde(default)]
    pub turn_complete: Option<bool>,

    /// True once the model has produced all output it will produce
    /// for this turn.  This is what we wait for before considering
    /// the segment "final" in the cost dashboard.
    #[serde(default)]
    pub generation_complete: Option<bool>,

    /// True if the model was interrupted by user activity.
    #[serde(default)]
    pub interrupted: Option<bool>,

    /// Transcript of the user's input (source language).
    #[serde(default)]
    pub input_transcription: Option<Transcription>,

    /// Transcript of the model's output (target language).
    #[serde(default)]
    pub output_transcription: Option<Transcription>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelTurn {
    /// For text-only modality, the text is in `parts[].text`.  The
    /// `inline_data` field is present only when audio output is on.
    #[serde(default)]
    pub parts: Vec<ModelTurnPart>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTurnPart {
    #[serde(default)]
    pub text: Option<String>,

    /// Base64-encoded audio bytes when `response_modalities` includes
    /// AUDIO.  We do not consume this for v0.3.0.
    #[serde(default)]
    pub inline_data: Option<InlineData>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InlineData {
    pub mime_type: Option<String>,
    /// Base64-encoded raw audio.
    pub data: Option<String>,
}

/// A partial or final transcription segment.  `finished=true` indicates
/// the server will not extend this segment further.  The `text` field
/// can be empty for "I'm starting to listen" markers.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Transcription {
    #[serde(default)]
    pub text: Option<String>,

    #[serde(default)]
    pub finished: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoAway {
    /// Protobuf-Duration string like "30s".  We display this to the
    /// user in a status-bar warning.
    #[serde(default)]
    pub time_left: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetadata {
    #[serde(default)]
    pub prompt_token_count: u32,
    #[serde(default)]
    pub cached_content_token_count: u32,
    #[serde(default)]
    pub response_token_count: u32,
    #[serde(default)]
    pub total_token_count: u32,
    /// Per-modality token counts (e.g. audio input vs text output).
    #[serde(default)]
    pub prompt_tokens_details: Vec<ModalityTokenCount>,
    #[serde(default)]
    pub response_tokens_details: Vec<ModalityTokenCount>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModalityTokenCount {
    pub modality: String,
    pub token_count: u32,
}

#[derive(Debug, Clone, Deserialize, thiserror::Error)]
#[error("api error: {message}")]
pub struct ApiError {
    pub message: String,
}

// ── High-level event type (decoded from ServerMessage) ───────────────────────

/// Application-facing event stream.  Built by the transport task from
/// raw `ServerMessage`s; users subscribe to it via
/// `CloudStreamSession::events()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudStreamEvent {
    /// The server accepted our `setup` and is ready for audio.
    Ready,

    /// Partial or final transcript of the *source* (input) audio.
    /// `finished = true` indicates no further extension.
    InputTranscript { text: String, finished: bool },

    /// Partial or final transcript of the *translated* (output) text.
    /// `finished = true` indicates no further extension.  The TUI
    /// uses this to render the target-language subtitle.
    OutputTranscript { text: String, finished: bool },

    /// Incremental token-usage update.  The transport task accumulates
    /// these into a per-session total exposed in the cost dashboard.
    Usage(UsageStats),

    /// The server is about to terminate the connection.  The transport
    /// task will attempt a single reconnect.
    GoAway { time_left_secs: Option<u32> },

    /// The WebSocket closed (cleanly or with an error).  The session
    /// is no longer usable; further `send_pcm` / `finish` calls will
    /// return `Err(SessionClosed)`.
    Closed { reason: String },
}

/// User-facing usage summary.  Summed across `usageMetadata` frames
/// by the transport task; reset on each new `Ready` event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageStats {
    pub audio_input_tokens: u32,
    pub text_input_tokens: u32,
    pub text_output_tokens: u32,
    pub total_tokens: u32,
}

impl UsageStats {
    /// Estimate cost in USD given Google AI Studio pricing (2026-06-20):
    ///   audio input: $3 / 1M tokens
    ///   text input:  $0.30 / 1M tokens
    ///   text output: $2 / 1M tokens
    pub fn estimated_cost_usd(&self) -> f64 {
        let audio = self.audio_input_tokens as f64 * 3.0 / 1_000_000.0;
        let text_in = self.text_input_tokens as f64 * 0.30 / 1_000_000.0;
        let text_out = self.text_output_tokens as f64 * 2.0 / 1_000_000.0;
        audio + text_in + text_out
    }
}

impl From<&UsageMetadata> for UsageStats {
    fn from(m: &UsageMetadata) -> Self {
        let mut s = Self {
            total_tokens: m.total_token_count,
            ..Default::default()
        };
        for d in &m.prompt_tokens_details {
            // Per Google docs the modality tag is e.g. "AUDIO" or
            // "TEXT".  We use exact match; unknown modalities are
            // ignored (will be added to `total_tokens` only).
            match d.modality.as_str() {
                "AUDIO" => s.audio_input_tokens = d.token_count,
                "TEXT" => s.text_input_tokens = d.token_count,
                _ => {}
            }
        }
        for d in &m.response_tokens_details {
            match d.modality.as_str() {
                "TEXT" => s.text_output_tokens = d.token_count,
                _ => {}
            }
        }
        s
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Build the system instruction text from the chosen style.  Short
/// and concrete; the model needs an unambiguous steer, not a
/// paragraph of theory.
pub fn build_system_instruction(style: TranslationStyle) -> SystemInstruction {
    SystemInstruction {
        parts: vec![TextPart {
            text: format!(
                "You are a real-time speech interpreter. {} \
                 Translate the speaker's words into the target language while \
                 preserving technical terms, named entities, and numbers.",
                style.as_instruction_phrase()
            ),
        }],
    }
}

/// Build a `realtimeInput.audio` frame for a chunk of 16-bit LE PCM.
///
/// The caller is responsible for chunk size (we recommend 100 ms =
/// 3200 bytes at 16 kHz mono).  We do not enforce a maximum here
/// because the wire format is permissive; the server may rate-limit
/// oversized frames.
pub fn build_audio_frame(pcm_le_i16: &[u8]) -> RealtimeInput {
    use base64::Engine;
    RealtimeInput {
        audio: Some(AudioBlob {
            data: base64::engine::general_purpose::STANDARD.encode(pcm_le_i16),
            mime_type: "audio/pcm;rate=16000".into(),
        }),
        activity_start: None,
        activity_end: None,
        audio_stream_end: None,
    }
}

/// Build a `realtimeInput` frame that signals end-of-stream to the
/// server's auto-VAD.
pub fn build_stream_end_frame() -> RealtimeInput {
    RealtimeInput {
        audio: None,
        activity_start: None,
        activity_end: None,
        audio_stream_end: Some(true),
    }
}

// ── Conversion from raw ServerMessage to high-level events ──────────────────

/// Convert a stream of `ServerMessage`s into a stream of
/// `CloudStreamEvent`s.  Used by the transport task; the user-facing
/// API just exposes the broadcast receiver.
pub fn into_events(msg: ServerMessage) -> Vec<CloudStreamEvent> {
    let mut out = Vec::new();
    if msg.setup_complete.is_some() {
        out.push(CloudStreamEvent::Ready);
    }
    if let Some(sc) = msg.server_content {
        if let Some(t) = sc.input_transcription {
            if let Some(text) = t.text {
                if !text.is_empty() {
                    out.push(CloudStreamEvent::InputTranscript {
                        text,
                        finished: t.finished.unwrap_or(false),
                    });
                }
            }
        }
        if let Some(t) = sc.output_transcription {
            if let Some(text) = t.text {
                if !text.is_empty() {
                    out.push(CloudStreamEvent::OutputTranscript {
                        text,
                        finished: t.finished.unwrap_or(false),
                    });
                }
            }
        }
    }
    if let Some(usage) = msg.usage_metadata {
        out.push(CloudStreamEvent::Usage(UsageStats::from(&usage)));
    }
    if let Some(g) = msg.go_away {
        // Protobuf duration string like "30s".  We try to parse the
        // seconds out; if it doesn't parse, we leave `time_left_secs`
        // as None.
        let secs = g.time_left.as_deref().and_then(parse_protobuf_seconds);
        out.push(CloudStreamEvent::GoAway {
            time_left_secs: secs,
        });
    }
    if let Some(err) = msg.error {
        out.push(CloudStreamEvent::Closed {
            reason: format!("api error: {}", err.message),
        });
    }
    out
}

/// Best-effort parser for protobuf Duration strings.  Accepts "30s",
/// "1m30s", "0.5s".  Returns None for anything that doesn't look
/// like a duration.
fn parse_protobuf_seconds(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Most common form: "Ns"
    if let Some(rest) = s.strip_suffix('s') {
        // Could be "30s", "1.5s"
        if let Ok(secs) = rest.parse::<f64>() {
            return Some(secs as u32);
        }
    }
    // "Nm" or "NmNs"
    if let Some(min_pos) = s.find('m') {
        let mins_str = &s[..min_pos];
        let rest = &s[min_pos + 1..];
        let mins: u32 = mins_str.parse().ok()?;
        let secs_part = rest.strip_suffix('s').unwrap_or(rest);
        let secs: f64 = secs_part.parse().ok()?;
        return Some(mins * 60 + secs as u32);
    }
    None
}

// ── Map internal CloudError from wire-level failure hints ───────────────────

/// Inspect a `ServerMessage` and return an `Err` if the message
/// represents a terminal failure.  Used by the transport task to
/// convert `error` fields into `CloudError::SetupFailed` /
/// `Auth` / `RateLimit`.
pub fn check_terminal_error(msg: &ServerMessage) -> Result<(), CloudError> {
    if let Some(err) = &msg.error {
        let lower = err.message.to_ascii_lowercase();
        if lower.contains("api key")
            || lower.contains("auth")
            || lower.contains("permission")
            || lower.contains("credential")
        {
            return Err(CloudError::Auth(err.message.clone()));
        }
        if lower.contains("quota") || lower.contains("rate") {
            return Err(CloudError::RateLimit(err.message.clone()));
        }
        if lower.contains("model") || lower.contains("not found") {
            return Err(CloudError::SetupFailed(err.message.clone()));
        }
        return Err(CloudError::Protocol(err.message.clone()));
    }
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── style hint ──────────────────────────────────────────────────────

    #[test]
    fn style_serde_round_trip() {
        let s = serde_json::to_string(&TranslationStyle::Formal).unwrap();
        assert_eq!(s, "\"formal\"");
        let v: TranslationStyle = serde_json::from_str(&s).unwrap();
        assert_eq!(v, TranslationStyle::Formal);
    }

    #[test]
    fn style_default_is_neutral() {
        assert_eq!(TranslationStyle::default(), TranslationStyle::Neutral);
    }

    // ── setup message ───────────────────────────────────────────────────

    #[test]
    fn setup_message_round_trip_omits_false_echo_flag() {
        let s = SetupMessage {
            setup: SetupBody {
                model: "models/gemini-3.5-live-translate-preview".into(),
                generation_config: GenerationConfig {
                    response_modalities: vec![Modality::Text],
                    temperature: Some(0.0),
                    top_p: None,
                },
                translation_config: TranslationConfig {
                    target_language_code: "vi".into(),
                    echo_target_language: false,
                },
                system_instruction: Some(build_system_instruction(TranslationStyle::Neutral)),
                input_audio_transcription: EmptyObject {},
                output_audio_transcription: EmptyObject {},
            },
        };
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        // The model name is the live-translate one.
        assert_eq!(
            v["setup"]["model"],
            "models/gemini-3.5-live-translate-preview"
        );
        // response_modalities is just TEXT, not AUDIO.
        assert_eq!(
            v["setup"]["generationConfig"]["responseModalities"],
            json!(["TEXT"])
        );
        // echoTargetLanguage omitted when false (skip_serializing_if).
        assert!(v["setup"]["translationConfig"]
            .as_object()
            .unwrap()
            .get("echoTargetLanguage")
            .is_none());
        // Both transcription flags present as empty objects.
        assert_eq!(v["setup"]["inputAudioTranscription"], json!({}));
        assert_eq!(v["setup"]["outputAudioTranscription"], json!({}));
    }

    // ── server message parsing ─────────────────────────────────────────

    #[test]
    fn server_message_parses_input_transcript() {
        let raw = json!({
            "serverContent": {
                "inputTranscription": {"text": "こんにちは", "finished": false}
            }
        });
        let msg: ServerMessage = serde_json::from_value(raw).unwrap();
        let events = into_events(msg);
        assert_eq!(
            events,
            vec![CloudStreamEvent::InputTranscript {
                text: "こんにちは".into(),
                finished: false,
            }]
        );
    }

    #[test]
    fn server_message_parses_output_transcript() {
        let raw = json!({
            "serverContent": {
                "outputTranscription": {"text": "Xin chào", "finished": true}
            }
        });
        let msg: ServerMessage = serde_json::from_value(raw).unwrap();
        let events = into_events(msg);
        assert_eq!(
            events,
            vec![CloudStreamEvent::OutputTranscript {
                text: "Xin chào".into(),
                finished: true,
            }]
        );
    }

    #[test]
    fn server_message_ignores_empty_transcript() {
        // An empty-text transcription update is a server "still listening"
        // marker; we don't surface it as a UI event.
        let raw = json!({
            "serverContent": {
                "inputTranscription": {"text": "", "finished": false}
            }
        });
        let msg: ServerMessage = serde_json::from_value(raw).unwrap();
        let events = into_events(msg);
        assert!(events.is_empty());
    }

    #[test]
    fn server_message_ready_event() {
        let raw = json!({"setupComplete": {}});
        let msg: ServerMessage = serde_json::from_value(raw).unwrap();
        let events = into_events(msg);
        assert_eq!(events, vec![CloudStreamEvent::Ready]);
    }

    #[test]
    fn server_message_combined_events() {
        // Real frames often contain model_turn + transcription + usage
        // in the same frame; we should emit all of them.
        let raw = json!({
            "serverContent": {
                "modelTurn": {
                    "parts": [{"text": "Xin chào"}]
                },
                "outputTranscription": {"text": "Xin chào", "finished": false},
                "turnComplete": false
            },
            "usageMetadata": {
                "promptTokenCount": 100,
                "responseTokenCount": 5,
                "totalTokenCount": 105,
                "promptTokensDetails": [
                    {"modality": "AUDIO", "tokenCount": 80},
                    {"modality": "TEXT", "tokenCount": 20}
                ],
                "responseTokensDetails": [
                    {"modality": "TEXT", "tokenCount": 5}
                ]
            }
        });
        let msg: ServerMessage = serde_json::from_value(raw).unwrap();
        let events = into_events(msg);
        // We expect: OutputTranscript (text) + Usage.  modelTurn is
        // captured but for the v0.3.0 UI we surface it via
        // outputTranscription, not the model_turn field.
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0],
            CloudStreamEvent::OutputTranscript { .. }
        ));
        let usage = match &events[1] {
            CloudStreamEvent::Usage(u) => u,
            _ => panic!("expected Usage, got {:?}", events[1]),
        };
        assert_eq!(usage.audio_input_tokens, 80);
        assert_eq!(usage.text_input_tokens, 20);
        assert_eq!(usage.text_output_tokens, 5);
        assert_eq!(usage.total_tokens, 105);
    }

    // ── usage → cost ────────────────────────────────────────────────────

    #[test]
    fn usage_cost_at_published_rates() {
        // 1h meeting: 30k audio tokens + 15k text output tokens.
        let u = UsageStats {
            audio_input_tokens: 30_000,
            text_input_tokens: 0,
            text_output_tokens: 15_000,
            total_tokens: 45_000,
        };
        let cost = u.estimated_cost_usd();
        // audio: 30_000 * 3.0 / 1M = 0.09
        // text out: 15_000 * 2.0 / 1M = 0.03
        // total: 0.12
        assert!((cost - 0.12).abs() < 1e-6, "cost was {cost}");
    }

    // ── error classification ────────────────────────────────────────────

    #[test]
    fn auth_error_classified() {
        let raw = json!({"error": {"message": "Invalid API key"}});
        let msg: ServerMessage = serde_json::from_value(raw).unwrap();
        let err = check_terminal_error(&msg).unwrap_err();
        assert!(matches!(err, CloudError::Auth(_)));
    }

    #[test]
    fn rate_limit_error_classified() {
        let raw = json!({"error": {"message": "Resource exhausted (rate limit)"}});
        let msg: ServerMessage = serde_json::from_value(raw).unwrap();
        let err = check_terminal_error(&msg).unwrap_err();
        assert!(matches!(err, CloudError::RateLimit(_)));
    }

    #[test]
    fn go_away_parsed_to_seconds() {
        let raw = json!({"goAway": {"timeLeft": "30s"}});
        let msg: ServerMessage = serde_json::from_value(raw).unwrap();
        let events = into_events(msg);
        assert_eq!(
            events,
            vec![CloudStreamEvent::GoAway {
                time_left_secs: Some(30)
            }]
        );
    }

    #[test]
    fn protobuf_duration_minutes_and_seconds() {
        assert_eq!(parse_protobuf_seconds("1m30s"), Some(90));
        assert_eq!(parse_protobuf_seconds("0.5s"), Some(0));
        assert_eq!(parse_protobuf_seconds("5s"), Some(5));
        assert_eq!(parse_protobuf_seconds("garbage"), None);
    }
}
