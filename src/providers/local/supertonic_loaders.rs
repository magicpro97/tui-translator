//! File loaders and text-preprocessing utilities for the Supertonic TTS pipeline.
//!
//! Split from `supertonic_provider` to stay under the 600-line LOC gate (STD-01).

use std::path::Path;

use super::supertonic_provider::SupertonicError;

// ─── Config types (shared with supertonic_provider) ──────────────────────────

/// Parsed `tts.json` hyperparameters.
#[derive(Debug, Clone)]
pub(super) struct SupertonicTtsConfig {
    pub(super) sample_rate: u32,
    pub(super) base_chunk_size: usize,
    pub(super) chunk_compress_factor: usize,
    pub(super) latent_dim: usize,
}

impl SupertonicTtsConfig {
    /// Latent frame dimension: `latent_dim × chunk_compress_factor` (= 144).
    pub(super) fn latent_frame_dim(&self) -> usize {
        self.latent_dim * self.chunk_compress_factor
    }

    /// Samples per latent chunk: `base_chunk_size × chunk_compress_factor` (= 3072).
    pub(super) fn chunk_size(&self) -> usize {
        self.base_chunk_size * self.chunk_compress_factor
    }

    /// Number of latent frames required to represent `wav_samples` audio samples.
    pub(super) fn latent_len_for_samples(&self, wav_samples: usize) -> usize {
        wav_samples.div_ceil(self.chunk_size())
    }
}

// ─── File loaders ─────────────────────────────────────────────────────────────

/// Parse `tts.json` into a [`SupertonicTtsConfig`].
pub(super) fn load_tts_config(model_dir: &Path) -> Result<SupertonicTtsConfig, SupertonicError> {
    let bytes = std::fs::read(model_dir.join("tts.json"))
        .map_err(|e| SupertonicError::OrtInit(format!("read tts.json: {e}")))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| SupertonicError::OrtInit(format!("parse tts.json: {e}")))?;

    let field = |section: &str, key: &str| -> Result<usize, SupertonicError> {
        v.get(section)
            .and_then(|s| s.get(key))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| SupertonicError::OrtInit(format!("tts.json missing {section}.{key}")))
    };

    Ok(SupertonicTtsConfig {
        sample_rate: field("ae", "sample_rate")? as u32,
        base_chunk_size: field("ae", "base_chunk_size")?,
        chunk_compress_factor: field("ttl", "chunk_compress_factor")?,
        latent_dim: field("ttl", "latent_dim")?,
    })
}

/// Load `unicode_indexer.bin` — raw little-endian int32 array mapping codepoint → token id.
pub(super) fn load_unicode_indexer(model_dir: &Path) -> Result<Vec<i32>, SupertonicError> {
    let bytes = std::fs::read(model_dir.join("unicode_indexer.bin"))
        .map_err(|e| SupertonicError::OrtInit(format!("read unicode_indexer.bin: {e}")))?;
    if bytes.len() % 4 != 0 {
        return Err(SupertonicError::OrtInit(format!(
            "unicode_indexer.bin size {} not a multiple of 4",
            bytes.len()
        )));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

/// Load `voice.bin` (6 × i64 header + TTL f32 block + DP f32 block).
///
/// Returns `(style_ttl, style_dp, num_speakers, ttl_dim1, ttl_dim2, dp_dim1, dp_dim2)`.
#[allow(clippy::type_complexity)]
pub(super) fn load_voice_bin(
    model_dir: &Path,
) -> Result<(Vec<f32>, Vec<f32>, usize, i64, i64, i64, i64), SupertonicError> {
    let bytes = std::fs::read(model_dir.join("voice.bin"))
        .map_err(|e| SupertonicError::OrtInit(format!("read voice.bin: {e}")))?;

    const HEADER: usize = 6 * 8;
    if bytes.len() < HEADER {
        return Err(SupertonicError::OrtInit(
            "voice.bin too short for 6-int64 header".to_string(),
        ));
    }

    let read_i64 = |i: usize| -> Result<i64, SupertonicError> {
        let slice = &bytes[i * 8..(i + 1) * 8];
        slice.try_into().map(i64::from_le_bytes).map_err(|_| {
            SupertonicError::OrtInit(format!("voice.bin: header field {i} slice error"))
        })
    };

    let (ttl_d0, ttl_d1, ttl_d2) = (read_i64(0)?, read_i64(1)?, read_i64(2)?);
    let (dp_d0, dp_d1, dp_d2) = (read_i64(3)?, read_i64(4)?, read_i64(5)?);

    let ttl_n = (ttl_d0 * ttl_d1 * ttl_d2) as usize;
    let dp_n = (dp_d0 * dp_d1 * dp_d2) as usize;
    let need = HEADER + (ttl_n + dp_n) * 4;
    if bytes.len() < need {
        return Err(SupertonicError::OrtInit(format!(
            "voice.bin: need {need} bytes, got {}",
            bytes.len()
        )));
    }

    let f32s = |off: usize, n: usize| -> Vec<f32> {
        bytes[off..off + n * 4]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    };

    Ok((
        f32s(HEADER, ttl_n),
        f32s(HEADER + ttl_n * 4, dp_n),
        ttl_d0 as usize,
        ttl_d1,
        ttl_d2,
        dp_d1,
        dp_d2,
    ))
}

// ─── Text preprocessing ───────────────────────────────────────────────────────

/// Preprocess text for Supertonic tokenization.
///
/// Pipeline (from `supertone-inc/supertonic:rust/src/helper.rs`):
/// 1. Character replacements (dashes, quotes, slashes, etc.)
/// 2. Emoji removal (U+1F000–U+1FFFF)
/// 3. Fix space before punctuation
/// 4. Collapse duplicate quotes and whitespace, trim
/// 5. Append `"."` if no trailing punctuation
/// 6. Wrap: `<lang>text</lang>` — MUST be last step
pub(super) fn preprocess_text(text: &str, lang: &str) -> String {
    let mut t = text.to_string();

    for (from, to) in &[
        ("\u{2013}", "-"),
        ("\u{2011}", "-"),
        ("\u{2014}", "-"),
        ("_", " "),
        ("\u{201C}", "\""),
        ("\u{201D}", "\""),
        ("\u{2018}", "'"),
        ("\u{2019}", "'"),
        ("\u{00B4}", "'"),
        ("`", "'"),
        ("[", " "),
        ("]", " "),
        ("|", " "),
        ("/", " "),
        ("#", " "),
        ("\u{2192}", " "),
        ("\u{2190}", " "),
    ] {
        t = t.replace(from, to);
    }
    for sym in &["\u{2665}", "\u{2606}", "\u{2661}", "\u{00A9}", "\\"] {
        t = t.replace(sym, "");
    }
    t = t.replace('@', " at ");
    t = t.replace("e.g.,", "for example,");
    t = t.replace("i.e.,", "that is,");

    // Remove emoji (U+1F000–U+1FFFF block).
    t = t
        .chars()
        .filter(|&c| !('\u{1F000}'..='\u{1FFFF}').contains(&c))
        .collect();

    for punct in &[",", ".", "!", "?", ";", ":", "'"] {
        let pat = format!(" {punct}");
        while t.contains(pat.as_str()) {
            t = t.replace(pat.as_str(), punct);
        }
    }
    while t.contains("\"\"") {
        t = t.replace("\"\"", "\"");
    }
    while t.contains("''") {
        t = t.replace("''", "'");
    }

    // Collapse whitespace and trim.
    let mut out = String::with_capacity(t.len());
    let mut sp = false;
    for ch in t.chars() {
        if ch.is_whitespace() {
            if !sp {
                out.push(' ');
            }
            sp = true;
        } else {
            out.push(ch);
            sp = false;
        }
    }
    t = out.trim().to_string();

    let trail = [
        '.', '!', '?', ';', ':', ',', '\'', '"', ')', ']', '}', '\u{2026}',
    ];
    if !t.is_empty() && !t.ends_with(|c| trail.contains(&c)) {
        t.push('.');
    }

    format!("<{lang}>{t}</{lang}>")
}

/// Convert preprocessed text to token IDs.
///
/// Maps each Unicode codepoint to its token via `indexer[codepoint]`.
/// Codepoints beyond the indexer length fall back to 0 (unknown token).
pub(super) fn text_to_token_ids(text: &str, indexer: &[i32]) -> Vec<i32> {
    text.chars()
        .map(|c| {
            let cp = c as usize;
            if cp < indexer.len() {
                indexer[cp]
            } else {
                0
            }
        })
        .collect()
}

/// Sample `n` values from N(0, 1) using the Box-Muller transform.
pub(super) fn sample_gaussian_vec(n: usize) -> Vec<f32> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let u1: f32 = rng.gen::<f32>().max(f32::EPSILON);
        let u2: f32 = rng.gen::<f32>();
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = 2.0 * std::f32::consts::PI * u2;
        out.push(r * theta.cos());
        if i + 1 < n {
            out.push(r * theta.sin());
        }
        i += 2;
    }
    out.truncate(n);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_text_wraps_with_language_tags() {
        let r = preprocess_text("Hello", "en");
        assert!(r.starts_with("<en>") && r.ends_with("</en>"));
        assert!(r.contains("Hello"));
    }

    #[test]
    fn preprocess_text_appends_period() {
        let r = preprocess_text("Hello", "en");
        assert!(r.contains("Hello."), "should append period; got: {r}");
    }

    #[test]
    fn preprocess_text_replaces_em_dash() {
        let r = preprocess_text("foo\u{2014}bar", "en");
        assert!(
            r.contains("foo-bar"),
            "em dash should become hyphen; got: {r}"
        );
    }

    #[test]
    fn text_to_token_ids_maps_known_codepoints() {
        let mut idx = vec![0i32; 128];
        idx[65] = 42;
        assert_eq!(text_to_token_ids("A", &idx), vec![42]);
    }

    #[test]
    fn text_to_token_ids_falls_back_for_out_of_range() {
        let idx = vec![99i32; 10];
        assert_eq!(text_to_token_ids("A", &idx), vec![0]);
    }

    #[test]
    fn sample_gaussian_vec_correct_length() {
        assert_eq!(sample_gaussian_vec(144).len(), 144);
    }
}
