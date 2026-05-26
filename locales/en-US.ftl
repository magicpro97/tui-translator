# I18N-01 (issue #481): English (United States) catalog.
#
# This is the canonical source catalog.  Every key referenced from Rust code
# MUST exist here; the i18n CI check fails the build if any key is missing.
# Other catalogs (vi-VN, future locales) fall back to this catalog when they
# are missing a key, and the i18n loader logs an explicit tracing warning so
# missing translations never silently succeed.
#
# Scope for I18N-01: keyboard-shortcut help overlay (see render_help_overlay
# in src/tui/mod.rs).  Wider migration of the status strip and settings UI
# is intentionally deferred — see docs/adr/i18n.md for the allowlist.

# ── Help overlay ──────────────────────────────────────────────────────────
help-title = Keyboard Shortcuts
help-scroll = Scroll subtitles/help
help-home = Scroll to top
help-end = Scroll to bottom / auto-follow
help-pause = Pause / resume translation
help-tts = Toggle TTS audio output
help-voice = Cycle TTS voice (CTRL-02)
help-metrics = Toggle metrics panel (compact/expanded)
help-language = Change target language
help-settings = Settings ({ $cycle } cycles field values)
help-reload = Reload config from disk
help-help = Show / hide this help
help-esc = Dismiss this overlay
help-tab = Switch A/B pane focus (dual-slot mode)
help-gain = Mic gain  -1/+1 dB    { "{" } / { "}" } TTS vol  -1/+1 dB
help-reset = Reset mic gain and TTS volume to 0 dB (CTRL-01)
help-quit = Quit — shows session summary

# Title bar variants for the help overlay.  `position` carries `{current}/{max}`
# only when the panel scrolls; otherwise the static hint is rendered.
help-bar-scrollable = Help [{ $position }] — ↑↓ scroll · Esc close
help-bar-static = Help — press ? or Esc to close
