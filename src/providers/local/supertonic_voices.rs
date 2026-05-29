//! Supertonic TTS voice catalog (SUPERTONIC-09 / issue #494).
//!
//! Defines the 10 built-in Supertonic voices (M1–M5, F1–F5), voice metadata,
//! catalog lookup, and single-active-voice parity with CTRL-02 (#455) and
//! CTRL-03 (#456).
//!
//! Hot-swap policy: calling [`SupertonicVoiceCatalog::set_active`] takes
//! effect on the *next* synthesis call; the in-flight utterance completes.
//!
//! Custom voices require explicit user consent; attempting to load one without
//! a recorded consent record returns [`VoiceError::CustomVoiceNotConsented`].

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

const SUPPORTED_LANGUAGES: &[&str] = &["ja", "vi", "en"];

/// Built-in Supertonic voice identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SupertonicVoiceId {
    /// Built-in male voice 1.
    M1,
    /// Built-in male voice 2.
    M2,
    /// Built-in male voice 3.
    M3,
    /// Built-in male voice 4.
    M4,
    /// Built-in male voice 5.
    M5,
    /// Built-in female voice 1.
    F1,
    /// Built-in female voice 2.
    F2,
    /// Built-in female voice 3.
    F3,
    /// Built-in female voice 4.
    F4,
    /// Built-in female voice 5.
    F5,
}

impl SupertonicVoiceId {
    /// Return the display-only gender hint for this voice.
    pub fn gender(self) -> VoiceGender {
        match self {
            Self::M1 | Self::M2 | Self::M3 | Self::M4 | Self::M5 => VoiceGender::Male,
            Self::F1 | Self::F2 | Self::F3 | Self::F4 | Self::F5 => VoiceGender::Female,
        }
    }

    /// Return the stable zero-based catalog index for this voice.
    pub fn index(self) -> usize {
        match self {
            Self::M1 => 0,
            Self::M2 => 1,
            Self::M3 => 2,
            Self::M4 => 3,
            Self::M5 => 4,
            Self::F1 => 5,
            Self::F2 => 6,
            Self::F3 => 7,
            Self::F4 => 8,
            Self::F5 => 9,
        }
    }
}

impl fmt::Display for SupertonicVoiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::M1 => "M1",
            Self::M2 => "M2",
            Self::M3 => "M3",
            Self::M4 => "M4",
            Self::M5 => "M5",
            Self::F1 => "F1",
            Self::F2 => "F2",
            Self::F3 => "F3",
            Self::F4 => "F4",
            Self::F5 => "F5",
        };
        f.write_str(name)
    }
}

impl FromStr for SupertonicVoiceId {
    type Err = VoiceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            raw if raw.eq_ignore_ascii_case("M1") => Ok(Self::M1),
            raw if raw.eq_ignore_ascii_case("M2") => Ok(Self::M2),
            raw if raw.eq_ignore_ascii_case("M3") => Ok(Self::M3),
            raw if raw.eq_ignore_ascii_case("M4") => Ok(Self::M4),
            raw if raw.eq_ignore_ascii_case("M5") => Ok(Self::M5),
            raw if raw.eq_ignore_ascii_case("F1") => Ok(Self::F1),
            raw if raw.eq_ignore_ascii_case("F2") => Ok(Self::F2),
            raw if raw.eq_ignore_ascii_case("F3") => Ok(Self::F3),
            raw if raw.eq_ignore_ascii_case("F4") => Ok(Self::F4),
            raw if raw.eq_ignore_ascii_case("F5") => Ok(Self::F5),
            other => Err(VoiceError::UnknownVoiceId(other.to_string())),
        }
    }
}

/// Display-only gender hint for a Supertonic voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoiceGender {
    /// Voice presents as male.
    Male,
    /// Voice presents as female.
    Female,
}

/// Metadata for a built-in Supertonic voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupertonicVoiceMeta {
    /// Stable built-in voice identifier.
    pub id: SupertonicVoiceId,
    /// Human-readable voice label shown in the UI.
    pub display_name: &'static str,
    /// Languages supported by this voice.
    pub supported_languages: &'static [&'static str],
}

/// Static catalog of built-in Supertonic voices.
pub const BUILTIN_VOICES: &[SupertonicVoiceMeta] = &[
    SupertonicVoiceMeta {
        id: SupertonicVoiceId::M1,
        display_name: "Male Voice 1",
        supported_languages: SUPPORTED_LANGUAGES,
    },
    SupertonicVoiceMeta {
        id: SupertonicVoiceId::M2,
        display_name: "Male Voice 2",
        supported_languages: SUPPORTED_LANGUAGES,
    },
    SupertonicVoiceMeta {
        id: SupertonicVoiceId::M3,
        display_name: "Male Voice 3",
        supported_languages: SUPPORTED_LANGUAGES,
    },
    SupertonicVoiceMeta {
        id: SupertonicVoiceId::M4,
        display_name: "Male Voice 4",
        supported_languages: SUPPORTED_LANGUAGES,
    },
    SupertonicVoiceMeta {
        id: SupertonicVoiceId::M5,
        display_name: "Male Voice 5",
        supported_languages: SUPPORTED_LANGUAGES,
    },
    SupertonicVoiceMeta {
        id: SupertonicVoiceId::F1,
        display_name: "Female Voice 1",
        supported_languages: SUPPORTED_LANGUAGES,
    },
    SupertonicVoiceMeta {
        id: SupertonicVoiceId::F2,
        display_name: "Female Voice 2",
        supported_languages: SUPPORTED_LANGUAGES,
    },
    SupertonicVoiceMeta {
        id: SupertonicVoiceId::F3,
        display_name: "Female Voice 3",
        supported_languages: SUPPORTED_LANGUAGES,
    },
    SupertonicVoiceMeta {
        id: SupertonicVoiceId::F4,
        display_name: "Female Voice 4",
        supported_languages: SUPPORTED_LANGUAGES,
    },
    SupertonicVoiceMeta {
        id: SupertonicVoiceId::F5,
        display_name: "Female Voice 5",
        supported_languages: SUPPORTED_LANGUAGES,
    },
];

/// Errors returned by the Supertonic voice catalog.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum VoiceError {
    /// The requested built-in voice identifier is not known.
    #[error("unknown Supertonic voice id: {0}")]
    UnknownVoiceId(String),
    /// A custom voice was requested without a recorded consent record.
    #[error("custom Supertonic voices require explicit user consent")]
    CustomVoiceNotConsented,
    /// A voice does not support the requested language.
    #[error("voice {voice} does not support language {language}")]
    UnsupportedLanguage {
        /// Voice identifier or provider-specific name.
        voice: String,
        /// Requested language tag.
        language: String,
    },
}

/// Single-active-voice catalog for Supertonic built-in voices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupertonicVoiceCatalog {
    active: SupertonicVoiceId,
}

impl Default for SupertonicVoiceCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl SupertonicVoiceCatalog {
    /// Create a new catalog with `M1` as the active voice.
    pub fn new() -> Self {
        Self {
            active: SupertonicVoiceId::M1,
        }
    }

    /// Return the built-in Supertonic voice catalog.
    pub fn list_builtin() -> &'static [SupertonicVoiceMeta] {
        BUILTIN_VOICES
    }

    /// Return the current active voice.
    pub fn active(&self) -> SupertonicVoiceId {
        self.active
    }

    /// Set the single active voice.
    ///
    /// The new selection is applied on the next synthesis request; any
    /// utterance already in flight continues with the previously selected voice.
    pub fn set_active(&mut self, id: SupertonicVoiceId) -> Result<(), VoiceError> {
        self.active = id;
        Ok(())
    }

    /// Parse a built-in voice name and make it active.
    pub fn set_active_by_name(&mut self, name: &str) -> Result<(), VoiceError> {
        let id = SupertonicVoiceId::from_str(name)?;
        self.set_active(id)
    }

    /// Return whether `voice` supports the requested language.
    pub fn supports_language(&self, voice: SupertonicVoiceId, lang: &str) -> bool {
        let wanted = lang.trim();
        BUILTIN_VOICES
            .get(voice.index())
            .map(|meta| {
                meta.supported_languages
                    .iter()
                    .any(|supported| supported.eq_ignore_ascii_case(wanted))
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_voices_has_ten_entries() {
        assert_eq!(SupertonicVoiceCatalog::list_builtin().len(), 10);
    }

    #[test]
    fn five_male_five_female_voices() {
        let male = BUILTIN_VOICES
            .iter()
            .filter(|meta| meta.id.gender() == VoiceGender::Male)
            .count();
        let female = BUILTIN_VOICES
            .iter()
            .filter(|meta| meta.id.gender() == VoiceGender::Female)
            .count();

        assert_eq!(male, 5);
        assert_eq!(female, 5);
    }

    #[test]
    fn m1_is_default_voice() {
        let catalog = SupertonicVoiceCatalog::new();
        assert_eq!(catalog.active(), SupertonicVoiceId::M1);
    }

    #[test]
    fn set_active_voice_succeeds() {
        let mut catalog = SupertonicVoiceCatalog::new();
        let result = catalog.set_active(SupertonicVoiceId::F4);

        assert_eq!(result, Ok(()));
        assert_eq!(catalog.active(), SupertonicVoiceId::F4);
    }

    #[test]
    fn set_active_unknown_fails() {
        let mut catalog = SupertonicVoiceCatalog::new();
        let result = catalog.set_active_by_name("Z9");

        assert_eq!(result, Err(VoiceError::UnknownVoiceId("Z9".to_string())));
        assert_eq!(catalog.active(), SupertonicVoiceId::M1);
    }

    #[test]
    fn display_name_format() {
        assert_eq!(BUILTIN_VOICES[0].display_name, "Male Voice 1");
        assert_eq!(BUILTIN_VOICES[9].display_name, "Female Voice 5");
    }

    #[test]
    fn from_str_case_insensitive() {
        assert_eq!(SupertonicVoiceId::from_str("m1"), Ok(SupertonicVoiceId::M1));
        assert_eq!(SupertonicVoiceId::from_str("F5"), Ok(SupertonicVoiceId::F5));
        assert_eq!(SupertonicVoiceId::from_str("f2"), Ok(SupertonicVoiceId::F2));
    }
}
