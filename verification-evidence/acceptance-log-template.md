# Layer 5 Human Acceptance Log — tui-translator

> **This is a template.** Fill in the tables and fields below when performing
> the Layer 5 human acceptance tests described in `docs/04-verification-plan.md`
> Section 7. Do not mark this document complete until all six tests have been
> run by at least two named human reviewers and every field has been filled in.
>
> The completed version of this file, saved under a dated subdirectory of
> `verification-evidence/`, is a required artifact before any release is approved.

---

## How to use this template

1. Copy this file to a dated subdirectory, for example
   `verification-evidence/rc-2025-01-15/acceptance-log.md`.
2. Fill in the **Release Candidate** and **Test Environment** fields below.
3. Work through L5-1 through L5-6 in order. Two independent reviewers must
   complete the tests; each reviewer signs their own row in each table.
4. After all six sections are complete, both reviewers sign the
   **Overall Sign-Off** area at the bottom.
5. Commit the completed file. The release may not proceed until it is committed
   and both signatures are present.

---

## Document header

| Field | Value |
|-------|-------|
| Release candidate tag | _(e.g. `v0.1.0-rc1`)_ |
| Date tests were run | YYYY-MM-DD |
| Machine B (test machine) OS | _(e.g. Windows 11 22H2)_ |
| Machine A (speaker machine) OS | _(e.g. Windows 10 21H2)_ |
| Zoom version (Machine B) | _(e.g. 5.17.x)_ |
| Application binary SHA-256 | _(run `certutil -hashfile tui-translator.exe SHA256`)_ |
| Google Cloud project used | _(project ID, not API key value)_ |

---

## L5-1 — Real Zoom Meeting: Audio Capture Verification

**Source:** `docs/04-verification-plan.md` Section 7.1  
**Release blocker:** B-15

### Description

Two machines are used. Machine A hosts a Zoom meeting. Machine B joins the
meeting and runs the application. A speaker on Machine A speaks ten
predetermined Japanese sentences at a natural conversational pace. The reviewer
on Machine B watches the subtitle panel and records what appears on screen.

### What is being verified

- Audio from Zoom is captured correctly by the application.
- Subtitles appear within a noticeable but acceptable delay (under 3 seconds for
  most sentences).
- No sentences are silently dropped — every sentence produces at least a partial
  subtitle.

### Exact steps

1. Start a Zoom meeting on Machine A with at least the speaker and one reviewer
   as participants.
2. On Machine B, launch `tui-translator.exe` and confirm the status bar shows
   "Listening".
3. The speaker reads each of the ten Japanese test sentences below, at a
   natural conversational pace, one at a time. Wait for the subtitle to appear
   (or for 6 seconds to elapse with no subtitle) before reading the next.
4. For each sentence, the reviewer records the three measurements in the table
   below immediately after the subtitle appears (or after the 6-second timeout).
5. After all ten sentences, the reviewer calculates the totals row.

> **Ten test sentences (copy or translate as appropriate for your test session):**
> Record them here before starting the test so the speaker can read from this list.
>
> 1. _(Enter Japanese test sentence 1)_
> 2. _(Enter Japanese test sentence 2)_
> 3. _(Enter Japanese test sentence 3)_
> 4. _(Enter Japanese test sentence 4)_
> 5. _(Enter Japanese test sentence 5)_
> 6. _(Enter Japanese test sentence 6)_
> 7. _(Enter Japanese test sentence 7)_
> 8. _(Enter Japanese test sentence 8)_
> 9. _(Enter Japanese test sentence 9)_
> 10. _(Enter Japanese test sentence 10)_

### Recording table

Fill in one row per sentence. Use the exact values in the column headers.

| # | Subtitle appeared | Delay | Accuracy |
|---|-------------------|-------|----------|
| 1 | Yes / No | fast (< 2 s) / acceptable (2–4 s) / slow (> 4 s) | exact / mostly correct / garbled |
| 2 | Yes / No | fast / acceptable / slow | exact / mostly correct / garbled |
| 3 | Yes / No | fast / acceptable / slow | exact / mostly correct / garbled |
| 4 | Yes / No | fast / acceptable / slow | exact / mostly correct / garbled |
| 5 | Yes / No | fast / acceptable / slow | exact / mostly correct / garbled |
| 6 | Yes / No | fast / acceptable / slow | exact / mostly correct / garbled |
| 7 | Yes / No | fast / acceptable / slow | exact / mostly correct / garbled |
| 8 | Yes / No | fast / acceptable / slow | exact / mostly correct / garbled |
| 9 | Yes / No | fast / acceptable / slow | exact / mostly correct / garbled |
| 10 | Yes / No | fast / acceptable / slow | exact / mostly correct / garbled |
| **Totals** | _x / 10 appeared_ | _consecutive slow count:_ | _garbled count:_ |

### Pass criteria (copied from Section 7.1)

- At least 9 of 10 sentences produce a subtitle.
- At least 8 of those are rated mostly correct or exact.
- No sentence is rated slow more than once consecutively.

### Release blocker condition (B-15)

Three or more sentences producing no subtitle, or two or more sentences rated
garbled, **blocks the release.**

### L5-1 result

| Field | Value |
|-------|-------|
| Overall result | **PASS** / **FAIL** / _(pending)_ |
| If FAIL, which blocker condition was triggered | |
| Reviewer 1 name | |
| Reviewer 1 signature | |
| Reviewer 1 date | YYYY-MM-DD |
| Reviewer 2 name | |
| Reviewer 2 signature | |
| Reviewer 2 date | YYYY-MM-DD |

---

## L5-2 — Real Zoom Meeting: Translation Quality Verification

**Source:** `docs/04-verification-plan.md` Section 7.2  
**Release blocker:** B-16

### Description

Same two-machine setup as L5-1. The reviewer for this test must be a fluent
reader of both the source language (Japanese) and the target language
(Vietnamese). The reviewer reads the Vietnamese translation shown in the subtitle
panel and rates whether it conveys the meaning of the original Japanese.

### What is being verified

- The translation conveys the intended meaning for most sentences.
- Misleading translations (meaning reversed or lost entirely) are rare.

### Exact steps

1. Use the same two-machine Zoom setup and the same ten test sentences as L5-1
   (or run L5-1 and L5-2 in the same session).
2. The reviewer for this test must be fluent in both Japanese and Vietnamese.
   They may be the same person as L5-1 reviewer only if they meet this
   language criterion.
3. For each Japanese sentence spoken on Machine A, the reviewer reads the
   Vietnamese subtitle on Machine B and rates the translation using the table below.
4. A "misleading" rating means the meaning is reversed or lost entirely — not
   merely awkward or imprecise.

### Recording table

| # | Original sentence spoken | Vietnamese subtitle shown | Rating |
|---|--------------------------|--------------------------|--------|
| 1 | | | conveys intended meaning / partially correct / misleading |
| 2 | | | conveys intended meaning / partially correct / misleading |
| 3 | | | conveys intended meaning / partially correct / misleading |
| 4 | | | conveys intended meaning / partially correct / misleading |
| 5 | | | conveys intended meaning / partially correct / misleading |
| 6 | | | conveys intended meaning / partially correct / misleading |
| 7 | | | conveys intended meaning / partially correct / misleading |
| 8 | | | conveys intended meaning / partially correct / misleading |
| 9 | | | conveys intended meaning / partially correct / misleading |
| 10 | | | conveys intended meaning / partially correct / misleading |
| **Totals** | | | _conveys: x / partially: x / misleading: x_ |

### Pass criteria (copied from Section 7.2)

- At least 8 of 10 translations convey the intended meaning.
- No more than 1 translation rated misleading.

### Release blocker condition (B-16)

Two or more misleading translations in a single session **blocks the release.**

### L5-2 result

| Field | Value |
|-------|-------|
| Overall result | **PASS** / **FAIL** / _(pending)_ |
| If FAIL, which blocker condition was triggered | |
| Reviewer name (must be fluent in Japanese and Vietnamese) | |
| Reviewer signature | |
| Reviewer date | YYYY-MM-DD |
| Second reviewer name | |
| Second reviewer signature | |
| Second reviewer date | YYYY-MM-DD |

---

## L5-3 — Real Zoom Meeting: Optional Translated Audio Verification

**Source:** `docs/04-verification-plan.md` Section 7.3  
**Release blocker:** B-20

### Description

Same two-machine setup, with translated audio (TTS) enabled on Machine B. The
reviewer listens to the Vietnamese spoken output while also hearing the original
Zoom meeting audio, and tests the `T` toggle.

### What is being verified

- Spoken translated audio is produced for completed subtitle lines.
- The `T` key toggles translated audio off and on immediately without restarting
  the application.
- The optional audio channel does not make the underlying Zoom audio unusable.

### Exact steps

1. Before starting the session, confirm translated audio is enabled in
   `config.json` on Machine B (`"tts": { "enabled": true }`).
2. Start the Zoom meeting and the application on Machine B.
3. Press `T` once to confirm translated audio is on; record the status bar state.
4. The speaker reads all ten Japanese test sentences at a natural pace.
5. After sentence 5, press `T` to turn translated audio off; confirm it stops
   immediately; record the result.
6. After sentence 7, press `T` to turn translated audio on again; confirm it
   resumes; record the result.
7. For each sentence, fill in all four columns of the table below.

### Recording table

| # | Spoken output played | Understandable in Vietnamese | Toggle behaved correctly | Audio interference |
|---|----------------------|-----------------------------|--------------------------|--------------------|
| 1 | Yes / No | Yes / No | Yes / No / N/A | acceptable / unacceptable |
| 2 | Yes / No | Yes / No | Yes / No / N/A | acceptable / unacceptable |
| 3 | Yes / No | Yes / No | Yes / No / N/A | acceptable / unacceptable |
| 4 | Yes / No | Yes / No | Yes / No / N/A | acceptable / unacceptable |
| 5 | Yes / No | Yes / No | Yes / No / N/A | acceptable / unacceptable |
| 6 (audio OFF from here) | Yes / No | Yes / No | **Toggle OFF worked: Yes / No** | acceptable / unacceptable |
| 7 (audio OFF) | Yes / No | Yes / No | Yes / No / N/A | acceptable / unacceptable |
| 8 (audio ON from here) | Yes / No | Yes / No | **Toggle ON worked: Yes / No** | acceptable / unacceptable |
| 9 | Yes / No | Yes / No | Yes / No / N/A | acceptable / unacceptable |
| 10 | Yes / No | Yes / No | Yes / No / N/A | acceptable / unacceptable |
| **Totals** | _played: x / 10_ | _understandable: x / 10_ | _toggle failures:_ | _unacceptable count:_ |

### Pass criteria (copied from Section 7.3)

- At least 8 of 10 sentences produce understandable spoken Vietnamese output.
- The toggle works in both directions (off and on) without restarting.
- No sentence is rated unacceptable because of audio interference.

### Release blocker condition (B-20)

If translated audio cannot be disabled immediately, repeatedly speaks the wrong
sentence, or makes the meeting audio unusable, **the release is blocked.**

### L5-3 result

| Field | Value |
|-------|-------|
| Overall result | **PASS** / **FAIL** / _(pending)_ |
| If FAIL, which blocker condition was triggered | |
| Reviewer 1 name | |
| Reviewer 1 signature | |
| Reviewer 1 date | YYYY-MM-DD |
| Reviewer 2 name | |
| Reviewer 2 signature | |
| Reviewer 2 date | YYYY-MM-DD |

---

## L5-4 — Terminal Emulator Compatibility on Real Machines

**Source:** `docs/04-verification-plan.md` Section 7.4  
**Release blocker:** B-17

### Description

A reviewer opens the application in each of five real terminal environments on a
real Windows machine and confirms it starts, displays correctly, and exits
cleanly.

### What is being verified

- The application starts without errors in each terminal.
- The layout appears correct and subtitle text is readable.
- The terminal returns to a clean state after the application exits.

### Exact steps

For each terminal listed in the table:

1. Open the terminal from its standard launch method (Start Menu, right-click,
   etc.).
2. Navigate to the directory containing `tui-translator.exe`.
3. Launch the application: `.\tui-translator.exe`.
4. Observe: does it start without error messages?
5. Observe the layout: is the subtitle panel visible, is text readable, are
   borders intact?
6. Press `Q` to exit cleanly.
7. Observe: does the terminal return to its normal prompt with no leftover
   artefacts (stray escape codes, broken cursor position, residual TUI
   elements)?
8. Record findings in the table. If any step fails, note the failure in the
   "Notes" column.

### Recording table

| # | Terminal | Started without errors | Layout correct | Text readable | Exited cleanly | Notes |
|---|----------|----------------------|----------------|---------------|----------------|-------|
| 1 | Windows Terminal (Windows Terminal app) | Yes / No | Yes / No | Yes / No | Yes / No | |
| 2 | ConEmu | Yes / No | Yes / No | Yes / No | Yes / No | |
| 3 | Windows Console Host (cmd.exe / conhost.exe) | Yes / No | Yes / No | Yes / No | Yes / No | |
| 4 | VS Code integrated terminal | Yes / No | Yes / No | Yes / No | Yes / No | |
| 5 | Git Bash terminal | Yes / No | Yes / No | Yes / No | Yes / No | |

### Pass criteria (copied from Section 7.4)

- All five environments must start the application and display a usable
  interface.
- All five must exit cleanly.

### Release blocker condition (B-17)

Any environment that crashes on start, renders an unreadable layout, or leaves a
broken terminal state after exit **blocks the release.**

### L5-4 result

| Field | Value |
|-------|-------|
| Overall result | **PASS** / **FAIL** / _(pending)_ |
| If FAIL, which terminal(s) failed and which step | |
| Reviewer 1 name | |
| Reviewer 1 signature | |
| Reviewer 1 date | YYYY-MM-DD |
| Reviewer 2 name | |
| Reviewer 2 signature | |
| Reviewer 2 date | YYYY-MM-DD |

---

## L5-5 — Real-World Provider Key Verification

**Source:** `docs/04-verification-plan.md` Section 7.5  
**Release blocker:** B-18

### Description

A reviewer obtains a fresh Google Cloud API key using the standard Cloud Console
flow (not a test account), enters it into the application configuration, and
runs a short five-minute translation session in a real Zoom meeting. This test
validates the **onboarding experience for a new user**, not just the technical
pipeline.

### What is being verified

- The end-user instructions for obtaining and configuring a Google Cloud API key
  actually work.
- A user following the documented steps can reach a working translation session
  without needing developer support.

### Exact steps

1. Start from a browser with no existing Google Cloud session logged in to the
   target project. (Use a private/incognito window or a separate browser
   profile.)
2. Follow the API key setup instructions in `USAGE.md` (or whatever end-user
   documentation is current) exactly, without referring to any internal notes.
3. Record each step in the table below: the action taken, the page or screen
   visited, and any confusion or error encountered.
4. Once the key is obtained and configured in `config.json`, launch the
   application, join a real Zoom meeting, and run a five-minute translation
   session.
5. Rate the overall onboarding experience at the bottom of this section.

### Recording table — onboarding steps

| Step | Action taken | Screen / page | Issues or confusion encountered |
|------|--------------|--------------|--------------------------------|
| 1 | | | |
| 2 | | | |
| 3 | | | |
| 4 | | | |
| 5 | | | |
| 6 | | | |
| _(add rows as needed)_ | | | |

### Five-minute session result

| Field | Value |
|-------|-------|
| Translation appeared within the session | Yes / No |
| Error messages seen (if any) | |
| Overall onboarding experience rating | **straightforward** / **confusing** / **broken** |
| If confusing: which step(s) need documentation updates | |

### Pass criteria (copied from Section 7.5)

- The reviewer rates the experience as straightforward or confusing.
- A rating of broken (for example, the key is entered but no translation appears
  and no error message explains why) blocks the release.
- A confusing rating requires that the documentation be updated before the
  release is approved.

### Release blocker condition (B-18)

A **broken** rating on the onboarding experience **blocks the release.**  
A **confusing** rating requires a documentation update before approval.

### L5-5 result

| Field | Value |
|-------|-------|
| Overall result | **PASS** / **FAIL** / **PASS (docs update required)** / _(pending)_ |
| If docs update required, describe the gap | |
| Reviewer 1 name | |
| Reviewer 1 signature | |
| Reviewer 1 date | YYYY-MM-DD |
| Reviewer 2 name | |
| Reviewer 2 signature | |
| Reviewer 2 date | YYYY-MM-DD |

---

## L5-6 — Accessibility and Readability Review

**Source:** `docs/04-verification-plan.md` Section 7.6  
**Release blocker:** B-19

### Description

A reviewer who is **not a software developer** reads the subtitle output during
a 10-minute Zoom meeting and answers four plain-language questions about the
experience. This test is deliberately not technical — it captures the genuine
first-impression experience of the intended end user.

### What is being verified

- Subtitles are readable without physical strain.
- Subtitles stay on screen long enough to finish reading.
- The cost display is useful rather than distracting.
- Nothing in the UI causes confusion.

### Exact steps

1. The reviewer must not be a software developer and should not have seen the
   application before this session.
2. Launch the application on Machine B and join a live Zoom meeting with at
   least one active speaker.
3. Let the reviewer observe the subtitle panel for 10 minutes without coaching.
4. After the session ends (or after 10 minutes), ask the reviewer to answer the
   four questions below in their own words. Do not suggest answers.
5. Record the answers verbatim in the table below.

### Reviewer written answers

| Question | Answer (record verbatim) |
|----------|--------------------------|
| Could you read the subtitles comfortably without squinting or leaning in? | |
| Did the subtitles stay on screen long enough to finish reading each one? | |
| Was the cost display useful, or was it distracting? | |
| Did anything confuse you during the session? | |

### Additional notes (optional)

_(The reviewer may suggest improvements here. Suggestions do not block the
release unless the reviewer rates the issue as critical.)_

### Pass criteria (copied from Section 7.6)

- No answer describes subtitles that were unreadable, disappearing too fast, or
  actively confusing.
- Reviewer may note suggestions for improvement without blocking the release,
  unless the issue is rated critical.

### Release blocker condition (B-19)

Any reviewer statement that subtitles were unreadable or disappeared before they
could finish reading **blocks the release.**

### L5-6 result

| Field | Value |
|-------|-------|
| Overall result | **PASS** / **FAIL** / _(pending)_ |
| If FAIL, which answer triggered the blocker | |
| Any improvement suggestions noted (non-blocking) | |
| Reviewer 1 name | |
| Reviewer 1 signature | |
| Reviewer 1 date | YYYY-MM-DD |
| Reviewer 2 name | |
| Reviewer 2 signature | |
| Reviewer 2 date | YYYY-MM-DD |

---

## Overall Sign-Off

This section must be completed **after** all six L5 test sections above have
been filled in with passing results and signed by both reviewers. It is required
before a release can be approved.

> **Requirements (from `docs/04-verification-plan.md` Section 10):**
> - All L5 tests must pass.
> - At least two named human reviewers must sign.
> - This completed file must be committed to `verification-evidence/<rc-date>/`
>   before the release is marked approved.

### Release blocker check

Confirm that none of the following blockers are active:

| Blocker | Condition | Status |
|---------|-----------|--------|
| B-15 | Three or more sentences produced no subtitle (L5-1) | _(CLEAR / ACTIVE)_ |
| B-16 | Two or more translations rated misleading (L5-2) | _(CLEAR / ACTIVE)_ |
| B-17 | Any terminal crashed, rendered unreadable layout, or left broken state (L5-4) | _(CLEAR / ACTIVE)_ |
| B-18 | Onboarding experience rated broken (L5-5) | _(CLEAR / ACTIVE)_ |
| B-19 | Subtitles rated unreadable or disappearing too fast (L5-6) | _(CLEAR / ACTIVE)_ |
| B-20 | Translated audio could not be toggled reliably or made meeting unusable (L5-3) | _(CLEAR / ACTIVE)_ |

### Overall acceptance decision

| Field | Value |
|-------|-------|
| All six L5 sections complete | Yes / No |
| All release blockers CLEAR | Yes / No |
| Documentation updates required before approval (L5-5) | Yes / No |
| **Release approved** | **YES** / **NO** / _(pending)_ |

### Reviewer 1 sign-off

| Field | Value |
|-------|-------|
| Full name | |
| Role | |
| Signature | |
| Date | YYYY-MM-DD |
| Statement | _"I have reviewed the evidence recorded in this log. To the best of my knowledge, the Layer 5 acceptance tests were performed honestly, on real hardware, with the release candidate binary identified in the document header."_ |

### Reviewer 2 sign-off

| Field | Value |
|-------|-------|
| Full name | |
| Role | |
| Signature | |
| Date | YYYY-MM-DD |
| Statement | _"I have reviewed the evidence recorded in this log. To the best of my knowledge, the Layer 5 acceptance tests were performed honestly, on real hardware, with the release candidate binary identified in the document header."_ |

---

_Template source: `verification-evidence/acceptance-log-template.md`  
Criteria source: `docs/04-verification-plan.md` Sections 7.1–7.6 and 8  
Issue: [#114](https://github.com/magicpro97/tui-translator/issues/114)_
