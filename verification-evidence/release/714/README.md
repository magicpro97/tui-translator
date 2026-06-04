# Release Evidence — Issue #714 (validator: `mt_provider=llm` rejected without `local-llm-mt`)

- **PR:** _placeholder — orchestrator to fill after push_
- **Branch:** `fix/v0114-validator-multiline-readiness`
- **Commit (fix):** see tentacle handoff `.octogent/tentacles/fix-v0114-loadguard-errors/handoff.md`
- **Council:** dev/test/qa leaders signed off at confidence 1.0 before implementation.

## Binding tests (in `src/main.rs::tests`)

- `runtime_provider_error_rejects_llm_mt_without_feature` — RED-first
- `runtime_provider_error_accepts_llm_mt_with_feature` (cfg-gated)
- `runtime_provider_error_rejects_truly_unknown_mt_provider`
- `runtime_provider_error_rejects_unknown_stt_provider`
- `mt_provider_choices_all_validate` (enumerates `tui::MT_PROVIDER_CHOICES`)
- `stt_provider_choices_all_validate`

## Snapshot diffs

None for #714 (validator is pre-TUI; surfaces via `eprintln!` startup error).

## Functional summary

When `mt_provider="llm"` and the `local-llm-mt` cargo feature is *not*
compiled in, the validator now returns a fatal error mentioning both
`mt_provider="llm"` and the literal feature name `local-llm-mt`, instead
of falling through to the wildcard "unknown provider" arm. Slot A and
slot B branches are symmetric; `MT_PROVIDER_CHOICES` enumeration guards
against future drift between picker entries and validator arms.
