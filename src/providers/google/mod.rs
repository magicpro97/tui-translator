//! Google Cloud providers.
//!
//! - [`stt`] — `GoogleSttProvider` (Speech-to-Text REST API).
//! - [`mt`] — `GoogleMtProvider` (Translation REST API).
//! - [`tts`] — `GoogleTtsProvider` (Text-to-Speech REST API).

#![allow(dead_code)]
#![allow(async_fn_in_trait)]

pub mod mt;
pub mod stt;
pub mod tts;
