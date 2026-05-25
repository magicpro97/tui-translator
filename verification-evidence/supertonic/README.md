# Supertonic evidence index

This folder collects the **non-code** evidence and policy substrate for
the Supertonic TTS family of issues (parent: [#485](https://github.com/magicpro97/tui-translator/issues/485)).

No code under `src/providers/supertonic/` exists yet. The artifacts
here pin the contract that any future implementation PR must honour.

## Inventory

| File | Issue | Status |
|------|-------|--------|
| `SUPERTONIC-01-spike.md` | [#486](https://github.com/magicpro97/tui-translator/issues/486) — feasibility & integration shape | Decision confidence **0.85**; empirical measurements deferred (see file §0, §8). |
| `SUPERTONIC-02-license-privacy.md` | [#487](https://github.com/magicpro97/tui-translator/issues/487) — license, distribution, consent, privacy | **DRAFT**; policy committed, runtime evidence deferred (file §8). |

## Related (outside this folder)

- `docs/adr/supertonic-11-default-readiness.md` — issue [#496](https://github.com/magicpro97/tui-translator/issues/496),
  default-readiness ADR. Status: **DEFERRED** — Google remains the
  default TTS provider.
- `docs/supertonic-user-guide.md` — issue [#497](https://github.com/magicpro97/tui-translator/issues/497),
  user-facing setup / consent / troubleshooting. Status: **DRAFT**.
- `docs/supertonic-release-checklist.md` — issue [#497](https://github.com/magicpro97/tui-translator/issues/497),
  release packaging checklist. Status: **DRAFT**.

## What this folder does **not** contain

- A Supertonic provider implementation (tracked by #493).
- A TTS trait or new trait wiring (tracked by #490, #491).
- A model cache implementation (tracked by #492).
- A voice catalog (tracked by #494).
- A soak gate run (tracked by #495).

Implementation PRs for the above must reference both
`SUPERTONIC-01-spike.md` and `SUPERTONIC-02-license-privacy.md` and
must move the corresponding deferred blockers to "closed" with
committed evidence.
