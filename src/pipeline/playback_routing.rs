//! TTS playback routing types and helpers.
//!
//! Extracted from `playback.rs` to keep module size within STD-02 limits.
//! The parent module re-exports public items so external consumers are
//! unaffected.

use crate::config::TtsRouting;

/// A concrete destination for synthesized TTS audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackSinkTarget {
    /// Speaker output, optionally pinned to a configured playback device.
    Speakers { output_device: Option<String> },
    /// Virtual microphone render endpoint, addressed by exact device name.
    VirtualMic { device: String },
}

impl PlaybackSinkTarget {
    pub(super) fn output_device(&self) -> Option<&str> {
        match self {
            Self::Speakers { output_device } => output_device.as_deref(),
            Self::VirtualMic { device } => Some(device.as_str()),
        }
    }

    fn description(&self) -> String {
        match self {
            Self::Speakers {
                output_device: Some(device),
            } => format!("speakers device '{device}'"),
            Self::Speakers {
                output_device: None,
            } => "speakers (system default)".to_string(),
            Self::VirtualMic { device } => format!("virtual mic device '{device}'"),
        }
    }
}

/// Resolved TTS playback route built from user configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackRoutePlan {
    label: &'static str,
    targets: Vec<PlaybackSinkTarget>,
}

impl PlaybackRoutePlan {
    /// Resolve a route from config fields.
    pub fn from_config(
        routing: TtsRouting,
        tts_output_device: Option<&str>,
        virtual_mic_device: Option<&str>,
    ) -> std::io::Result<Self> {
        match routing {
            TtsRouting::Speakers => Ok(Self::speakers(tts_output_device)),
            TtsRouting::VirtualMic => match virtual_mic_device {
                Some(device) => Ok(Self::virtual_mic(device)),
                None => {
                    tracing::warn!(
                        "tts_routing \"virtual_mic\" configured but virtual_mic_device \
                         is unset; falling back to speakers"
                    );
                    Ok(Self::speakers(tts_output_device))
                }
            },
            TtsRouting::Both => {
                let device = virtual_mic_device.ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "tts_routing \"both\" requires virtual_mic_device",
                    )
                })?;
                Ok(Self::both(tts_output_device, device))
            }
        }
    }

    /// Route to speakers only, preserving pre-VMIC behaviour.
    pub fn speakers(output_device: Option<&str>) -> Self {
        Self {
            label: "speakers",
            targets: vec![PlaybackSinkTarget::Speakers {
                output_device: output_device.map(str::to_owned),
            }],
        }
    }

    /// Route exclusively to a virtual microphone render endpoint.
    pub fn virtual_mic(device: &str) -> Self {
        Self {
            label: "virtual_mic",
            targets: vec![PlaybackSinkTarget::VirtualMic {
                device: device.to_string(),
            }],
        }
    }

    /// Route to both speakers and a virtual microphone render endpoint.
    pub fn both(output_device: Option<&str>, virtual_mic_device: &str) -> Self {
        Self {
            label: "both",
            targets: vec![
                PlaybackSinkTarget::Speakers {
                    output_device: output_device.map(str::to_owned),
                },
                PlaybackSinkTarget::VirtualMic {
                    device: virtual_mic_device.to_string(),
                },
            ],
        }
    }

    /// Human-readable route label used by startup/status logs.
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// Sink targets for this route.
    pub fn targets(&self) -> &[PlaybackSinkTarget] {
        &self.targets
    }
}

pub(super) fn build_sinks_for_targets<T, F>(
    targets: &[PlaybackSinkTarget],
    mut make_sink: F,
) -> std::io::Result<Vec<T>>
where
    F: FnMut(&PlaybackSinkTarget) -> Result<T, String>,
{
    if targets.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "TTS playback route must contain at least one sink",
        ));
    }

    let mut sinks = Vec::with_capacity(targets.len());
    for target in targets {
        match make_sink(target) {
            Ok(sink) => sinks.push(sink),
            Err(err) => {
                return Err(std::io::Error::other(format!(
                    "failed to initialise TTS sink for {}: {err}",
                    target.description()
                )));
            }
        }
    }
    Ok(sinks)
}

pub(super) fn play_to_audio_sinks(
    sinks: &[Box<dyn crate::pipeline::audio_sink::AudioSink>],
    audio_bytes: Vec<u8>,
) {
    if sinks.is_empty() {
        return;
    }

    for sink in sinks.iter().take(sinks.len() - 1) {
        sink.play_bytes(audio_bytes.clone());
    }
    if let Some(sink) = sinks.last() {
        sink.play_bytes(audio_bytes);
    }
}
