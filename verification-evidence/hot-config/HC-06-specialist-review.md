# HC-06 specialist review sign-off

Issue: #391
Worktree: `C:\Users\linhnt102\tui-translator-hc-06`
Branch: `feat/hc-06-hot-config-regression-gate`

## Reviewer

- Agent: `tui-rust-code-reviewer`
- Model: `claude-opus-4.7`
- Review target: staged HC-06 diff covering `tests/hot_config.rs`, `tests/hot_config/matrix.rs`, `.github/workflows/ci.yml`, and `.gitignore`
- Verdict: **CLEAN**

## Evidence checked

- Matrix test asserts the real classifier APIs:
  - `AppConfig::requires_restart`
  - `AppConfig::requires_restart_ignoring_capture`
  - `AppConfig::requires_capture_hot_swap`
  - `classify_capture_change`
  - `classify_recorder_change`
  - `ProviderBundle::evaluate_change`
  - `AppConfig::validate`
- Matrix evidence is written before assertion so CI can upload failure artifacts.
- CI runs `cargo test --test hot_config -- --nocapture`, validates the JSON evidence, uploads it with `if: always()`, and gates `vmic-production-readiness`.
- The generated report records `schema_version = 1`, `issue = "#391"`, `status = "pass"`, `case_count = 50`, and `missing_fields = []`.
- The known `audio_file_path` / `wasapi` classifier divergence is pinned explicitly as a regression guard.
- The synthetic `SECRET_KEY` is used only to assert redaction and is not a real credential.

## Local verification evidence

- `cargo fmt --check`: PASS
- `cargo test --test hot_config -- --nocapture`: PASS
- `cargo clippy --test hot_config -- -D warnings`: PASS
- `cargo test --all`: PASS in CI-shaped local target layout
- `cargo clippy --all-targets -- -D warnings`: PASS in CI-shaped local target layout

## Notes

One non-blocking future-drift risk remains: `REQUIRED_FIELDS` is hard-coded in the matrix. It matches the current `AppConfig` classified field set and is guarded by the matrix report, but a future config field addition must update the list or add a shared source of truth.
