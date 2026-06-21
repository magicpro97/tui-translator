//! I18N-01 (issue #481): cross-platform i18n architecture.
//!
//! Lightweight Project Fluent runtime that the TUI uses to look up
//! user-facing strings.  The module exposes three responsibilities:
//!
//! 1. **Catalog loading.**  English (`en-US`) and Vietnamese (`vi-VN`)
//!    catalogs are embedded at build time via [`include_str!`] so the
//!    shipped `.exe` does not depend on disk layout.  Each locale is parsed
//!    into a [`FluentBundle`] when first used.
//! 2. **Lookup with explicit fallback.**  [`t`] and [`t_args`] resolve a
//!    message in the active locale; if the key is absent or fails to
//!    format, the loader falls back to `en-US` and logs a
//!    [`tracing::warn!`] with the locale + key.  A truly missing key (also
//!    absent from `en-US`) returns the key itself wrapped in `??` so it is
//!    visible in the UI rather than silently empty — the missing-key CI
//!    check (`scripts/i18n-check.ps1`) hard-fails the build before that
//!    can happen in release.
//! 3. **Live locale switching.**  [`set_locale`] is called from
//!    `apply_runtime_config` after a config hot-reload (`R` key or
//!    file-system watcher) so changes to `AppConfig::locale` take effect
//!    on the next frame without restarting the TUI.
//!
//! Pseudo-locale (`x-pseudo`) wraps every translation in `⟦…⟧` and pads
//! the body with mid-dot characters so adaptive-layout truncation is
//! visible in tests and screenshots; see [`Locale::Pseudo`].

use std::collections::HashMap;
use std::sync::RwLock;

use fluent_bundle::{bundle::FluentBundle, FluentArgs, FluentResource, FluentValue};
use unic_langid::{langid, LanguageIdentifier};

/// Default locale used at startup and whenever fallback is required.
#[allow(dead_code)]
pub const DEFAULT_LOCALE: &str = "en-US";

/// Pseudo-locale tag.  `x-pseudo` is a valid BCP-47 private-use tag.
/// The config validator currently accepts it unconditionally so QA and
/// developers can opt in via `config.json`; tightening for end-user
/// release builds is left to a follow-up (see `docs/adr/i18n.md`).
pub const PSEUDO_LOCALE: &str = "x-pseudo";

const EN_US_FTL: &str = include_str!("../../locales/en-US.ftl");
const VI_VN_FTL: &str = include_str!("../../locales/vi-VN.ftl");

/// Concrete locale variants the binary knows how to load.
///
/// New translations are added by extending this enum, dropping a `.ftl`
/// file into `locales/`, and updating the CI check.  Keeping the set
/// closed makes missing-key detection static rather than file-system
/// dependent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locale {
    EnUS,
    ViVN,
    /// Pseudo-locale that wraps every English message in `⟦…⟧` plus
    /// padding so adaptive-layout truncation is visible during tests.
    Pseudo,
}

impl Locale {
    /// Parse a canonical locale tag.  Returns [`None`] for unknown tags.
    ///
    /// Accepts the same closed set as `AppConfig::validate` so the
    /// runtime and the config layer agree on what a valid locale is;
    /// short forms like `"en"` or `"vi"` are intentionally rejected.
    pub fn parse(tag: &str) -> Option<Self> {
        match tag.to_ascii_lowercase().as_str() {
            "en-us" => Some(Self::EnUS),
            "vi-vn" => Some(Self::ViVN),
            "x-pseudo" => Some(Self::Pseudo),
            _ => None,
        }
    }

    /// Canonical BCP-47 tag for this locale.
    pub fn as_tag(self) -> &'static str {
        match self {
            Self::EnUS => "en-US",
            Self::ViVN => "vi-VN",
            Self::Pseudo => PSEUDO_LOCALE,
        }
    }

    fn lang_id(self) -> LanguageIdentifier {
        match self {
            Self::EnUS | Self::Pseudo => langid!("en-US"),
            Self::ViVN => langid!("vi-VN"),
        }
    }

    fn ftl_source(self) -> &'static str {
        match self {
            Self::EnUS | Self::Pseudo => EN_US_FTL,
            Self::ViVN => VI_VN_FTL,
        }
    }
}

/// Concurrent bundle alias — [`fluent_bundle`] requires us to pin the
/// generics; this keeps the type tractable elsewhere in the module.
type Bundle = FluentBundle<FluentResource, intl_memoizer::concurrent::IntlLangMemoizer>;

fn build_bundle(locale: Locale) -> Bundle {
    let mut bundle = FluentBundle::new_concurrent(vec![locale.lang_id()]);
    // Strip Fluent's default U+2068/U+2069 bidi isolation marks so plain
    // ASCII rendering in ratatui buffers does not see invisible chars
    // around interpolated arguments; tests assert on raw substrings.
    bundle.set_use_isolating(false);
    // The FTL sources are embedded via include_str! and exercised by the
    // module unit tests, so a parse or insert failure here is a build-time
    // programmer error rather than a runtime condition we can recover
    // from.  Keep the panic explicit and gate-acknowledged.
    #[allow(clippy::expect_used, clippy::unwrap_used)]
    let resource = FluentResource::try_new(locale.ftl_source().to_string())
        .expect("locale catalog must be valid Fluent FTL"); // allow-unwrap: #481
    #[allow(clippy::expect_used, clippy::unwrap_used)]
    bundle
        .add_resource(resource)
        .expect("locale catalog must add cleanly to its bundle"); // allow-unwrap: #481
    bundle
}

struct Catalog {
    active: Locale,
    bundles: HashMap<Locale, Bundle>,
}

impl Catalog {
    fn new() -> Self {
        let mut bundles = HashMap::new();
        bundles.insert(Locale::EnUS, build_bundle(Locale::EnUS));
        Self {
            active: Locale::EnUS,
            bundles,
        }
    }

    fn ensure(&mut self, locale: Locale) {
        self.bundles
            .entry(locale)
            .or_insert_with(|| build_bundle(locale));
    }

    fn format(&self, locale: Locale, key: &str, args: Option<&FluentArgs<'_>>) -> Option<String> {
        let bundle = self.bundles.get(&locale)?;
        let msg = bundle.get_message(key)?;
        let pattern = msg.value()?;
        let mut errors = vec![];
        let formatted = bundle.format_pattern(pattern, args, &mut errors);
        if !errors.is_empty() {
            tracing::warn!(
                locale = locale.as_tag(),
                key,
                errors = ?errors,
                "i18n: fluent formatting reported errors; using rendered output anyway"
            );
        }
        Some(formatted.into_owned())
    }
}

static CATALOG: RwLock<Option<Catalog>> = RwLock::new(None);

fn with_catalog<R>(f: impl FnOnce(&mut Catalog) -> R) -> R {
    // RwLock poisoning here means a previous holder panicked mid-update;
    // the catalog state is still consistent (we only mutate inside `f`
    // and the bundle builder is infallible after parse), so recover by
    // taking the inner guard rather than propagating the panic.
    let mut guard = CATALOG
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        *guard = Some(Catalog::new());
    }
    // Safety: just initialised above when `None`; the `Some` branch is
    // also covered.  The expect documents the invariant for readers.
    let cat = match guard.as_mut() {
        Some(cat) => cat,
        // Unreachable: we just set the slot to Some above.  Keep the
        // explicit panic so a future refactor that breaks the invariant
        // surfaces loudly instead of silently corrupting catalog state.
        None => unreachable!("catalog was just initialised in with_catalog"),
    };
    f(cat)
}

/// Initialise the global catalog if it has not been built yet.
///
/// Safe to call multiple times; only the first call performs work.  The
/// TUI calls this on startup so the first frame can resolve strings
/// without waiting on lazy initialisation inside a hot render path.
#[allow(dead_code)]
pub fn init() {
    with_catalog(|_| ());
}

/// Switch the active locale.  Unknown tags fall back to `en-US` and log
/// a single tracing warning.
///
/// Called from `apply_runtime_config` after a config hot-reload so the
/// `R` key and the file-system watcher both pick up locale changes.
pub fn set_locale(tag: &str) {
    let locale = Locale::parse(tag).unwrap_or_else(|| {
        tracing::warn!(
            requested = tag,
            "i18n: unknown locale tag — falling back to en-US"
        );
        Locale::EnUS
    });
    with_catalog(|cat| {
        cat.ensure(locale);
        cat.active = locale;
    });
}

/// Return the currently active locale.  Mostly used by tests; production
/// code should prefer [`t`] / [`t_args`].
#[allow(dead_code)]
pub fn active_locale() -> Locale {
    with_catalog(|cat| cat.active)
}

/// Look up a key in the active locale, falling back to `en-US` with an
/// explicit tracing warning when the active locale lacks the key.
///
/// If the key is also missing from `en-US`, returns `??<key>??` so the
/// regression is obvious in the UI rather than silently empty.  The
/// i18n CI check (`scripts/i18n-check.ps1`) is the gate that prevents
/// this state in shipped releases.
pub fn t(key: &str) -> String {
    t_args(key, None)
}

/// As [`t`] but with named Fluent arguments.
pub fn t_args(key: &str, args: Option<&FluentArgs<'_>>) -> String {
    with_catalog(|cat| {
        let active = cat.active;
        cat.ensure(active);
        if active != Locale::EnUS {
            cat.ensure(Locale::EnUS);
        }
        if let Some(rendered) = cat.format(active, key, args) {
            return maybe_pseudo(active, rendered);
        }
        if active != Locale::EnUS {
            tracing::warn!(
                locale = active.as_tag(),
                key,
                "i18n: missing translation — falling back to en-US"
            );
            if let Some(rendered) = cat.format(Locale::EnUS, key, args) {
                return maybe_pseudo(active, rendered);
            }
        }
        tracing::error!(
            locale = active.as_tag(),
            key,
            "i18n: missing key in en-US — CI i18n check should have blocked this build"
        );
        format!("??{key}??")
    })
}

/// Convenience for callers with a single `name = value` pair.
pub fn t_arg(key: &str, name: &str, value: impl Into<FluentValue<'static>>) -> String {
    let mut args = FluentArgs::new();
    args.set(name.to_string(), value);
    t_args(key, Some(&args))
}

fn maybe_pseudo(locale: Locale, rendered: String) -> String {
    if locale != Locale::Pseudo {
        return rendered;
    }
    // Wrap in U+27E6/U+27E7 brackets and append mid-dot padding equal to
    // ~30% of the visible length so adaptive layouts visibly expand and
    // truncation tests can detect it.
    let pad = (rendered.chars().count() / 3).max(2);
    let padding: String = "·".repeat(pad);
    format!("⟦{rendered}{padding}⟧")
}

/// Reset the catalog to a freshly initialised state.  Test-only escape
/// hatch so unit tests can hop between locales without leaking state.
#[cfg(test)]
pub fn reset_for_test() {
    let mut guard = CATALOG.write().expect("i18n catalog rwlock poisoned");
    *guard = Some(Catalog::new());
}

/// Shared test mutex.  All tests that read or write the global i18n
/// catalog must acquire this guard so `cargo test`'s thread pool does
/// not interleave locale switches.  The same lock is used by the TUI
/// help-overlay tests in `src/tui/mod.rs` because they render strings
/// that the catalog can change underneath them.
#[cfg(test)]
pub fn lock_for_test() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::Mutex;
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    let guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    reset_for_test();
    set_locale("en-US");
    guard
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catalog round-trip: known key in `en-US` resolves to its English
    /// message.  This is the baseline contract that all other behaviours
    /// build on.
    #[test]
    fn en_us_resolves_known_key() {
        let _guard = super::lock_for_test();
        reset_for_test();
        set_locale("en-US");
        let rendered = t("help-title");
        assert_eq!(rendered, "Keyboard Shortcuts");
    }

    /// Vietnamese catalog round-trip.  A locale switch must not require
    /// reloading the catalog from disk; both bundles live in memory once
    /// initialised.
    #[test]
    fn vi_vn_resolves_known_key() {
        let _guard = super::lock_for_test();
        reset_for_test();
        set_locale("vi-VN");
        let rendered = t("help-title");
        assert_eq!(rendered, "Phím tắt");
    }

    /// Missing-key fallback: an entry absent from `vi-VN` but present in
    /// `en-US` must surface the English message rather than an empty
    /// string.  The fallback also emits a `tracing::warn!`; we cannot
    /// assert on tracing here without extra plumbing, but the explicit
    /// non-empty return is the user-visible contract.
    #[test]
    fn missing_key_falls_back_to_en_us() {
        let _guard = super::lock_for_test();
        reset_for_test();
        // Inject a synthetic missing entry by reaching into the catalog:
        // remove `help-title` from the vi-VN bundle and confirm the
        // English value is returned for the active vi-VN locale.
        set_locale("vi-VN");
        with_catalog(|cat| {
            // Replace the vi-VN bundle with a stub that lacks help-title.
            let mut stub: Bundle = FluentBundle::new_concurrent(vec![Locale::ViVN.lang_id()]);
            stub.set_use_isolating(false);
            let res = FluentResource::try_new(String::new()).unwrap();
            stub.add_resource(res).unwrap();
            cat.bundles.insert(Locale::ViVN, stub);
        });
        let rendered = t("help-title");
        assert_eq!(
            rendered, "Keyboard Shortcuts",
            "missing vi-VN key must fall back to en-US message"
        );
    }

    /// A key missing from every catalog must surface the sentinel
    /// `??key??` form so QA spots the regression even if the CI gate
    /// failed.  Silent empty strings are explicitly prohibited.
    #[test]
    fn missing_everywhere_returns_sentinel() {
        let _guard = super::lock_for_test();
        reset_for_test();
        set_locale("en-US");
        let rendered = t("this-key-does-not-exist");
        assert_eq!(rendered, "??this-key-does-not-exist??");
    }

    /// Pseudo-locale wraps English strings in ⟦…⟧ and adds padding so
    /// adaptive layouts that fit `Keyboard Shortcuts` (18 cols) but
    /// truncate the pseudo form expose the bug.  Verifying the prefix
    /// and that the rendered length grew is enough — exact padding
    /// width is an implementation detail.
    #[test]
    fn pseudo_locale_wraps_and_pads() {
        let _guard = super::lock_for_test();
        reset_for_test();
        set_locale("x-pseudo");
        let rendered = t("help-title");
        assert!(
            rendered.starts_with('⟦') && rendered.ends_with('⟧'),
            "pseudo-locale must wrap with ⟦…⟧; got {rendered:?}"
        );
        assert!(
            rendered.chars().count() > "Keyboard Shortcuts".chars().count() + 2,
            "pseudo-locale must add padding to expose truncation; got {rendered:?}"
        );
    }

    /// Unknown locale tags fall back to `en-US` (logged), and the
    /// active locale reflects that fallback so the rest of the system
    /// renders predictable English.
    #[test]
    fn unknown_locale_falls_back_to_en_us() {
        let _guard = super::lock_for_test();
        reset_for_test();
        set_locale("zz-ZZ");
        assert_eq!(active_locale(), Locale::EnUS);
        assert_eq!(t("help-title"), "Keyboard Shortcuts");
    }

    /// Fluent argument interpolation works for both locales.
    #[test]
    fn settings_line_interpolates_cycle_arg() {
        let _guard = super::lock_for_test();
        reset_for_test();
        set_locale("en-US");
        let rendered = t_arg("help-settings", "cycle", "F2/Ctrl+D");
        assert!(
            rendered.contains("F2/Ctrl+D"),
            "interpolation must keep the cycle arg verbatim; got {rendered:?}"
        );
        assert!(rendered.contains("Settings"));
    }
}
