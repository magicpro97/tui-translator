//! Panic-hook sidecar writer for QA8-08 (issue #506).
//!
//! Installs a single, idempotent panic hook that chains to the previous
//! hook so existing behaviour (stderr backtrace, tracing layers) is
//! preserved. On each panic the hook:
//!
//! 1. Builds a [`PanicRecord`] describing the panic (timestamp, pid,
//!    thread, location, scrubbed message, captured backtrace, app
//!    version).
//! 2. Writes the record as JSON to
//!    `<dump_dir>/panic-<unix_ms>-<pid>.json` (one file per panic).
//! 3. Appends a single line to `<dump_dir>/panic-log.txt` so a soak
//!    operator sees panics without having to enumerate sidecars.
//! 4. Chains to the previous hook so default backtraces still print.
//!
//! ## Secret scrubbing
//!
//! Google API keys follow a well-known fixed-prefix shape. Any
//! occurrence of that pattern (or the literal `google_api_key=...`
//! `"google_api_key":"..."` forms) is replaced with `[REDACTED]` in the
//! recorded payload and location string. The scrubber is conservative:
//! it never removes non-secret text.
//!
//! ## Idempotency
//!
//! [`install_panic_hook`] is safe to call multiple times — only the first
//! call actually installs the hook.

use serde::{Deserialize, Serialize};
use std::backtrace::Backtrace;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

/// Environment variable that overrides the default dump directory.
///
/// When set to a non-empty path the panic hook writes sidecars there
/// without consulting the platform default. Empty or whitespace-only
/// values are treated as unset.
pub const DUMP_DIR_ENV: &str = "TUI_TRANSLATOR_DUMP_DIR";

/// Subdirectory name used under the per-user data directory.
const DEFAULT_SUBDIR: &str = "dumps";

/// Maximum size in bytes for the rolling `panic-log.txt`. When the file
/// exceeds this size the next append rotates it to `panic-log.txt.old`.
const PANIC_LOG_MAX_BYTES: u64 = 1_048_576;

/// Fixed 4-byte prefix of Google API keys. Stored as a runtime constant
/// (built from the constituent bytes) so this repository never contains
/// a literal token shape that would trip secret scanners.
const GOOGLE_KEY_PREFIX_BYTES: [u8; 4] = [b'A', b'I', b'z', b'a'];

/// Length of the body that follows the prefix.
const GOOGLE_KEY_BODY_LEN: usize = 35;

static INSTALL: Once = Once::new();

/// Normalised JSON shape written to `panic-<ts>-<pid>.json`.
///
/// Stable across versions — downstream tooling treats unknown fields as
/// additive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PanicRecord {
    /// `"tui-translator.panic"` schema discriminator.
    pub kind: String,
    /// Application semver string (`CARGO_PKG_VERSION`).
    pub app_version: String,
    /// Unix-epoch milliseconds when the panic was observed.
    pub timestamp_unix_ms: u128,
    /// OS process id of the panicking process.
    pub pid: u32,
    /// `thread::current().name()` or `"<unnamed>"`.
    pub thread: String,
    /// `file:line:column` from `PanicInfo::location()` or
    /// `"<unknown>"` when the panic carried no location.
    pub location: String,
    /// Panic payload (scrubbed). Empty string when payload was not a
    /// `&str` / `String`.
    pub message: String,
    /// Captured backtrace as a multi-line string (scrubbed). Empty
    /// when backtrace capture was disabled by the runtime.
    pub backtrace: String,
}

impl PanicRecord {
    /// Build a [`PanicRecord`] from a [`PanicHookInfo`] and the supplied
    /// timestamp and pid. Pure / side-effect free so tests can drive
    /// the JSON shape deterministically.
    pub fn from_panic(info: &PanicHookInfo<'_>, timestamp_unix_ms: u128, pid: u32) -> Self {
        let payload = info.payload();
        let raw_message = if let Some(s) = payload.downcast_ref::<&'static str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            String::new()
        };

        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());

        let thread = std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_string();

        let backtrace = Backtrace::capture().to_string();

        Self {
            kind: "tui-translator.panic".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp_unix_ms,
            pid,
            thread,
            location: scrub_secrets(&location),
            message: scrub_secrets(&raw_message),
            backtrace: scrub_secrets(&backtrace),
        }
    }

    /// Render the panic as a single-line summary suitable for the
    /// rolling `panic-log.txt` plain-text view.
    pub fn to_log_line(&self) -> String {
        format!(
            "[{}] pid={} thread={} at {} :: {}",
            self.timestamp_unix_ms, self.pid, self.thread, self.location, self.message
        )
    }
}

/// Install the panic hook once. Subsequent calls are no-ops so binaries
/// and tests can call this from multiple entry points safely.
///
/// `dump_dir` is captured by the hook closure. The hook never panics:
/// on any I/O failure it logs at `error!` and continues so the original
/// panic still surfaces via the chained default hook.
pub fn install_panic_hook(dump_dir: PathBuf) {
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let timestamp_unix_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let pid = std::process::id();
            let record = PanicRecord::from_panic(info, timestamp_unix_ms, pid);

            if let Err(error) = record_panic_to(&dump_dir, &record) {
                tracing::error!(
                    dump_dir = %dump_dir.display(),
                    error = %error,
                    "failed to write panic sidecar; chaining to previous hook only"
                );
            } else {
                tracing::error!(
                    dump_dir = %dump_dir.display(),
                    pid = record.pid,
                    location = %record.location,
                    "panic captured to sidecar (see docs/13-crash-dump-symbolication.md)"
                );
            }

            previous(info);
        }));
    });
}

/// Write `record` to `<dir>/panic-<ts>-<pid>.json` and append a
/// summary line to `<dir>/panic-log.txt`. Creates `dir` if needed.
///
/// Exposed for tests; the production panic hook calls this internally.
pub fn record_panic_to(dir: &Path, record: &PanicRecord) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;

    let sidecar = dir.join(format!(
        "panic-{}-{}.json",
        record.timestamp_unix_ms, record.pid
    ));
    let json = serde_json::to_string_pretty(record).map_err(std::io::Error::other)?;
    fs::write(&sidecar, json)?;

    rotate_if_needed(&dir.join("panic-log.txt"))?;
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("panic-log.txt"))?;
    writeln!(log, "{}", record.to_log_line())?;

    Ok(())
}

fn rotate_if_needed(path: &Path) -> std::io::Result<()> {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.len() <= PANIC_LOG_MAX_BYTES {
        return Ok(());
    }
    let rotated = path.with_extension("txt.old");
    let _ = fs::remove_file(&rotated);
    fs::rename(path, rotated)
}

/// Resolve the dump directory using the documented precedence:
///
/// 1. `TUI_TRANSLATOR_DUMP_DIR` env var (when non-empty).
/// 2. `directories::ProjectDirs` per-user data dir + `dumps/`.
/// 3. `std::env::temp_dir()/tui-translator/dumps/` as a last resort.
pub fn resolve_dump_dir() -> PathBuf {
    if let Ok(value) = std::env::var(DUMP_DIR_ENV) {
        if !value.trim().is_empty() {
            return PathBuf::from(value);
        }
    }
    if let Some(proj) = directories::ProjectDirs::from("", "", "tui-translator") {
        return proj.data_local_dir().join(DEFAULT_SUBDIR);
    }
    std::env::temp_dir()
        .join("tui-translator")
        .join(DEFAULT_SUBDIR)
}

/// Replace known-secret-shaped substrings with `[REDACTED]`.
///
/// Patterns matched:
///
/// * Google API key shape: 4-byte fixed prefix + 35 bytes of
///   `[A-Za-z0-9_-]`.
/// * `google_api_key` / `googleApiKey` followed by `=`, `:` or
///   `": "` and a quoted-or-bare value up to whitespace, comma, or
///   the closing quote.
pub(crate) fn scrub_secrets(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some(skipped) = match_google_key(bytes, i) {
            out.push_str("[REDACTED]");
            i += skipped;
            continue;
        }
        if let Some(skipped) = match_keyword(bytes, i) {
            out.push_str("[REDACTED]");
            i += skipped;
            continue;
        }
        #[allow(clippy::expect_used, clippy::unwrap_used)]
        let ch = input[i..].chars().next().expect("non-empty remainder");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn match_google_key(bytes: &[u8], start: usize) -> Option<usize> {
    let total = GOOGLE_KEY_PREFIX_BYTES.len() + GOOGLE_KEY_BODY_LEN;
    if bytes.len() < start + total {
        return None;
    }
    let window = &bytes[start..start + total];
    if window[..GOOGLE_KEY_PREFIX_BYTES.len()] != GOOGLE_KEY_PREFIX_BYTES {
        return None;
    }
    for &b in &window[GOOGLE_KEY_PREFIX_BYTES.len()..] {
        if !(b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return None;
        }
    }
    Some(total)
}

fn match_keyword(bytes: &[u8], start: usize) -> Option<usize> {
    let keywords: [&[u8]; 4] = [
        b"\"google_api_key\"",
        b"google_api_key",
        b"\"googleApiKey\"",
        b"googleApiKey",
    ];
    let mut i = start;
    let mut matched_len = 0;
    for kw in keywords {
        if bytes[i..].starts_with(kw) {
            matched_len = kw.len();
            break;
        }
    }
    if matched_len == 0 {
        return None;
    }
    i += matched_len;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i >= bytes.len() || !(bytes[i] == b'=' || bytes[i] == b':') {
        return None;
    }
    i += 1;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    let quoted = i < bytes.len() && bytes[i] == b'"';
    if quoted {
        i += 1;
        while i < bytes.len() && bytes[i] != b'"' {
            i += 1;
        }
        if i < bytes.len() {
            i += 1;
        }
    } else {
        while i < bytes.len()
            && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b',' | b';' | b'}')
        {
            i += 1;
        }
    }
    Some(i - start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Build a synthetic Google-API-key-shaped string at runtime so the
    /// source repository never contains a literal token shape.
    fn synth_google_key() -> String {
        let prefix: String = GOOGLE_KEY_PREFIX_BYTES.iter().map(|b| *b as char).collect();
        let body: String = (0..GOOGLE_KEY_BODY_LEN)
            .map(|i| {
                let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
                alphabet[i % alphabet.len()] as char
            })
            .collect();
        format!("{}{}", prefix, body)
    }

    fn make_record(ts: u128, pid: u32, message: &str) -> PanicRecord {
        PanicRecord {
            kind: "tui-translator.panic".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp_unix_ms: ts,
            pid,
            thread: "test".to_string(),
            location: "src/foo.rs:1:1".to_string(),
            message: message.to_string(),
            backtrace: String::new(),
        }
    }

    #[test]
    fn scrub_redacts_google_key_shape() {
        let key = synth_google_key();
        assert_eq!(
            key.len(),
            GOOGLE_KEY_PREFIX_BYTES.len() + GOOGLE_KEY_BODY_LEN
        );
        let input = format!("error: key={} trailing", key);
        let scrubbed = scrub_secrets(&input);
        assert!(!scrubbed.contains(&key), "key leaked: {}", scrubbed);
        assert!(scrubbed.contains("[REDACTED]"));
        assert!(scrubbed.contains("trailing"));
    }

    #[test]
    fn scrub_redacts_quoted_json_form() {
        let key = synth_google_key();
        let input = format!(r#"config {{ "google_api_key": "{}", "other": 1 }}"#, key);
        let scrubbed = scrub_secrets(&input);
        assert!(!scrubbed.contains(&key));
        assert!(scrubbed.contains("\"other\": 1"));
    }

    #[test]
    fn scrub_redacts_bare_keyword_form() {
        let key = synth_google_key();
        let input = format!("google_api_key={} extra", key);
        let scrubbed = scrub_secrets(&input);
        assert!(!scrubbed.contains(&key));
        assert!(scrubbed.contains("extra"));
    }

    #[test]
    fn scrub_preserves_innocuous_input() {
        let prefix_str: String = GOOGLE_KEY_PREFIX_BYTES.iter().map(|b| *b as char).collect();
        let input = format!("no secrets here; {} is too short", prefix_str);
        assert_eq!(scrub_secrets(&input), input);
    }

    #[test]
    fn scrub_preserves_unicode() {
        let key = synth_google_key();
        let input = format!("ズーム — 通訳 — {} done", key);
        let scrubbed = scrub_secrets(&input);
        assert!(scrubbed.contains("ズーム"));
        assert!(scrubbed.contains("通訳"));
        assert!(!scrubbed.contains(&key));
    }

    #[test]
    fn resolve_dump_dir_prefers_env_override() {
        let tmp = tempdir().unwrap();
        let target = tmp.path().join("custom-dumps");
        std::env::set_var(DUMP_DIR_ENV, &target);
        let resolved = resolve_dump_dir();
        std::env::remove_var(DUMP_DIR_ENV);
        assert_eq!(resolved, target);
    }

    #[test]
    fn resolve_dump_dir_ignores_blank_env() {
        std::env::set_var(DUMP_DIR_ENV, "   ");
        let resolved = resolve_dump_dir();
        std::env::remove_var(DUMP_DIR_ENV);
        assert_ne!(resolved.as_os_str(), "   ");
        assert!(resolved.ends_with(DEFAULT_SUBDIR));
    }

    #[test]
    fn record_panic_writes_sidecar_and_log() {
        let tmp = tempdir().unwrap();
        let record = make_record(1700000000000, 4242, "boom");
        record_panic_to(tmp.path(), &record).unwrap();

        let sidecar = tmp.path().join("panic-1700000000000-4242.json");
        let body = std::fs::read_to_string(&sidecar).unwrap();
        let parsed: PanicRecord = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed, record);

        let log = std::fs::read_to_string(tmp.path().join("panic-log.txt")).unwrap();
        assert!(log.contains("pid=4242"));
        assert!(log.contains("boom"));
    }

    #[test]
    fn record_panic_creates_missing_directory() {
        let tmp = tempdir().unwrap();
        let nested = tmp.path().join("deep").join("nested").join("dumps");
        let record = make_record(1, 1, "hi");
        record_panic_to(&nested, &record).unwrap();
        assert!(nested.join("panic-1-1.json").exists());
    }

    #[test]
    fn from_panic_scrubs_payload() {
        let secret = synth_google_key();
        let captured: std::sync::Arc<std::sync::Mutex<Option<PanicRecord>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let captured_inner = captured.clone();
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let r = PanicRecord::from_panic(info, 7, 9);
            *captured_inner.lock().unwrap() = Some(r);
        }));
        let secret_for_panic = secret.clone();
        let _ = std::panic::catch_unwind(move || panic!("explode key={}", secret_for_panic));
        std::panic::set_hook(prev);

        let got = captured.lock().unwrap().take().expect("hook fired");
        assert!(!got.message.contains(&secret), "leaked: {}", got.message);
        assert!(got.message.contains("[REDACTED]"));
        assert_eq!(got.pid, 9);
        assert_eq!(got.timestamp_unix_ms, 7);
    }

    #[test]
    fn rotation_moves_oversized_log() {
        let tmp = tempdir().unwrap();
        let log = tmp.path().join("panic-log.txt");
        std::fs::write(&log, vec![b'x'; (PANIC_LOG_MAX_BYTES + 1) as usize]).unwrap();
        rotate_if_needed(&log).unwrap();
        assert!(!log.exists());
        assert!(tmp.path().join("panic-log.txt.old").exists());
    }
}
