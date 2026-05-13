# Verification and Acceptance Plan

> **Audience:** Product owners, project managers, QA leads, and anyone who needs to understand how this product is proven safe, correct, and release-ready — without needing a software engineering background.

---

## 1. Why Unit Tests Are Not Enough

A unit test checks a small, isolated piece of code in a controlled environment. It is a useful baseline, but it cannot answer the questions that matter most before shipping this product:

- Does Zoom audio actually arrive at the translation engine, on a real Windows machine, in a real meeting?
- Does the terminal display stay readable in every terminal emulator a user might open?
- Does the Google Speech API respond correctly when network conditions are imperfect?
- Does the application remain stable after running for four hours in a real meeting?
- Is the cost counter accurate enough to trust before a user's API bill arrives?
- If optional translated audio is enabled, can it be toggled safely without making the meeting harder to follow?

These questions can only be answered by running the real product, on real hardware, against real external services, with real humans watching the result. This document defines exactly how each question will be answered and what evidence must exist before a release is approved.

---

## 2. Verification Layers

This plan organizes verification into five layers, ordered from most automated to most human-driven. Every layer must pass before a release is approved. No layer can substitute for another.

| Layer | Name | Who or what runs it | When |
|-------|------|---------------------|------|
| L1 | Automated build and unit checks | CI pipeline | Every code push |
| L2 | Integration and contract tests | CI pipeline | Every code push |
| L3 | Terminal behavior tests | CI pipeline | Every code push |
| L4 | Soak and stability tests | Scheduled overnight | Before each release candidate |
| L5 | Human acceptance on real hardware | Named human reviewers | Before each release |

---

## 3. Layer 1 — Automated Build and Unit Checks

### What this layer does

Every time a developer pushes code, an automated pipeline compiles the application and runs fast automated checks. These checks confirm that the code is internally consistent, that basic logic rules are followed, and that the build itself succeeds on the target platform (Windows).

### What this layer proves

- The code compiles without errors on Windows.
- Core data transformations (audio-to-text handoff format, subtitle chunking, cost calculation arithmetic) produce the expected output for known inputs.
- Configuration loading correctly rejects invalid values and accepts valid ones.
- No obvious memory safety violations exist in Rust code.

### What this layer does NOT prove

- Nothing about real audio, real APIs, real terminals, or real user experience.
- Nothing about behavior over time.
- Nothing about how the application responds to network failures or API rate limits.

### Evidence required to pass

- A green CI badge on the main branch.
- Zero compilation warnings treated as errors.
- All unit tests pass with no skips.

### Release blocker

If the build fails or any unit test fails, the release cannot proceed. This is a hard gate.

---

## 4. Layer 2 — Integration and Contract Tests

### What this layer does

Integration tests run the full pipeline — audio input, speech-to-text, translation, optional text-to-speech, and display — against controlled test doubles or live sandboxed API accounts. Contract tests specifically verify that the application speaks the correct language to each external provider (Google, and later Azure), so that a future API change by the provider is detected immediately rather than discovered in a real meeting.

### 4.1 Audio-to-Transcript Integration

An audio file containing recorded Japanese speech is fed into the audio capture module. The output transcript is compared against a known reference after standard text normalization. The test passes only if the transcript matches the reference within an acceptable accuracy boundary.

**Why this matters:** Proves that the audio handoff, sample-rate conversion, and speech recognition work together in sequence — not just individually.

**Evidence required:** At least three different Japanese speech samples (clear speech, accented speech, overlapping background noise) produce transcripts that match reference transcripts at 90% normalized-text accuracy or better when using the Google Speech API in sandbox mode.

**Release blocker:** Any sample falling below 85% normalized-text accuracy blocks the release.

### 4.2 Translation Round-Trip Integration

A known Japanese sentence is sent through the translation engine and the result is checked against a known reference in Vietnamese. A second run uses a sentence with technical terms. A third run introduces a deliberate truncation mid-sentence to confirm the application handles incomplete input gracefully rather than crashing.

**Evidence required:** All reference sentences translate within ±5% of the expected character length and match semantic meaning as confirmed by a bilingual reviewer (human check documented in the evidence log).

**Release blocker:** Any crash on incomplete input blocks the release.

### 4.3 Google Provider Contract Test

A dedicated test account sends a minimal valid request to each Google API endpoint used by the application: Speech-to-Text, Cloud Translation, and Text-to-Speech. The test checks that the response structure matches what the application code expects. This test is run against the live Google API, not a fake, because the goal is to detect real API changes.

**Why this matters:** Google occasionally updates its API response format or deprecates a field. Without a contract test, this would only be discovered by a user seeing a crash. With a contract test, it is caught in CI within 24 hours of the change.

**Evidence required:** Each endpoint returns a response that passes the application's own parsing logic. Response time is logged. If response time exceeds 3 seconds for a simple request, this is flagged as a performance concern even if it is not a blocker.

**Release blocker:** Any parsing failure against the live Google API blocks the release.

### 4.4 Error and Retry Behavior

The application is tested against a network condition simulator that drops connections, returns HTTP 429 rate-limit errors, and returns HTTP 503 service-unavailable errors. The test confirms that the application retries correctly, displays a meaningful status message in the terminal, and does not crash or corrupt its internal state.

**Evidence required:** For each simulated error type, the application recovers or exits gracefully within 10 seconds. No crash dump is produced. The terminal status area displays a readable message during the error state.

**Release blocker:** Any crash during simulated network failure blocks the release.

---

## 5. Layer 3 — Terminal Behavior Tests

### Why the terminal needs its own layer

The product lives inside a terminal window. Most testing tools assume a graphical window with a mouse cursor. This product's entire user interface is rendered as characters on a text screen. Special tools (called PTY test harnesses) simulate a real terminal session — they can start the application, read what it displays as a grid of characters, and check whether the correct text appears in the correct position.

Terminal behavior tests answer questions that no unit test can answer:

- Does the subtitle panel appear where it is supposed to be?
- When the terminal window is resized, does the layout reflow without garbled characters?
- Does the application clean up the terminal correctly when it exits, so the user's shell prompt returns in the right place?
- Are colors and emphasis used correctly on terminals that support them, and omitted gracefully on terminals that do not?

### 5.1 Layout and Rendering Correctness

A PTY test starts the application in a simulated terminal of a fixed size (for example, 80 columns by 24 rows). After a moment, the test reads the screen contents and verifies that the bilingual subtitle area, the status bar, the metrics area, the cost display, and the provider indicator all appear in their documented positions.

The test then resizes the simulated terminal to a smaller size (for example, 40 columns by 12 rows) and verifies that the layout either reflows correctly or truncates gracefully — never producing overlapping text or a crash.

**Evidence required:** Layout matches documented positions at three standard sizes: 80×24, 120×40, and 200×50. No crash on resize from any of those sizes.

**Release blocker:** Any crash on resize, or any layout that puts text in the wrong panel, blocks the release.

### 5.2 Terminal Cleanup on Exit

A PTY test starts the application and then sends an exit signal (the equivalent of pressing Ctrl+C). The test reads the terminal state after the application exits and verifies that the original terminal settings are restored. The user's cursor is in the correct position. No leftover rendering artifacts remain on the screen.

**Evidence required:** After an exit signal, the terminal state is indistinguishable from a clean shell session. Verified for three exit scenarios: normal quit command, Ctrl+C interrupt, and forced process termination.

**Release blocker:** Any scenario that leaves the terminal in an unusable state (wrong cursor, color codes not reset, scroll buffer broken) blocks the release.

### 5.3 Graceful Degradation on Limited Terminals

Many production terminals do not support color or mouse input. The PTY test runs the application in a mode that declares no color support and verifies that the application still renders readable text without any control characters appearing visibly on screen.

**Evidence required:** The application runs to a usable state in a simulated monochrome, no-color terminal. All critical information (current transcript, status, and cost) is readable.

**Release blocker:** Any crash or unreadable output in monochrome mode blocks the release.

---

## 6. Layer 4 — Soak and Stability Tests

### What soak testing means

A soak test runs the application continuously for an extended period — typically four to eight hours — under realistic load. It measures whether the application stays stable, whether memory usage grows unexpectedly, whether error rates increase over time, and whether audio quality degrades.

Soak tests are run on a dedicated test machine overnight before each release candidate is declared.

### 6.1 Four-Hour Audio Soak

The application runs for four hours with a continuous audio stream. The audio stream is a pre-recorded mix of speech, silence, background noise, and occasional loud events (a door slamming, a keyboard click). The following measurements are taken automatically every five minutes throughout the run:

- Memory used by the application process.
- CPU usage percentage.
- Network upload and download throughput during active speech.
- Number of audio chunks dropped or retried.
- Number of failed API calls.
- Average time from speech end to subtitle appearance.

**Evidence required:**
- Memory usage must not grow by more than 50 megabytes over the four-hour run. Growth larger than this indicates a memory leak.
- CPU usage must stay below 40% on a mid-range development laptop.
- Upload and download throughput must be visible in the evidence log whenever speech is active.
- No more than 2% of audio chunks may be dropped or require retry after the retry policy is exhausted.
- No more than 2% of API calls may fail.
- Average subtitle latency must stay below 3 seconds throughout the run.

**Release blocker:** Memory growth exceeding 50 megabytes, CPU exceeding 60% at any sample, chunk loss exceeding 5% in any 15-minute sample, or subtitle latency exceeding 5 seconds at any 15-minute sample blocks the release.

### 6.2 Cost Accuracy Soak

During the same four-hour soak run, the application's built-in rolling cost counter is compared against the actual charges on the Google Cloud billing console at the end of the run. The two numbers are compared.

**Evidence required:** The application's displayed cost is within 10% of the actual Google billing amount for the same period.

**Release blocker:** A discrepancy of more than 15% between displayed cost and actual billed cost blocks the release. This matters because users make decisions about API usage based on the displayed cost counter.

### 6.3 Recovery After Network Interruption During Soak

Partway through the soak run (at the two-hour mark), the network is disconnected for 30 seconds and then reconnected. The test verifies that the application reconnects automatically, resumes transcription, and does not require a restart.

**Evidence required:** The application displays a meaningful status message during the outage. Within 60 seconds of network restoration, the application is processing audio again. No manual intervention is needed.

**Release blocker:** Any scenario where the user must restart the application after a transient network interruption blocks the release.

---

## 7. Layer 5 — Human Acceptance on Real Hardware

### Why human verification cannot be automated

No automated test can verify that a translated subtitle feels correct to a human reader, that the audio timing creates a natural listening experience, or that a real Zoom meeting produces the same audio stream as a simulated one. These verifications require a real person, in a real environment, making a judgment call and recording their finding.

Human acceptance tests are performed by at least two named reviewers. Each reviewer signs their initials and the date next to each finding in the acceptance log. The acceptance log is a required artifact for every release.

### 7.1 Real Zoom Meeting — Audio Capture Verification

**Setup:** Two machines are used. Machine A hosts a Zoom meeting. Machine B joins the meeting and runs the application.

**Task:** A speaker on Machine A speaks ten predetermined Japanese sentences at a natural conversational pace. The reviewer on Machine B watches the subtitle panel and compares what appears on screen against the sentences spoken.

**What is being verified:**
- Audio from Zoom is being captured correctly by the application.
- Subtitles appear within a noticeable but acceptable delay (under 3 seconds for most sentences).
- No sentences are silently dropped (every sentence produces at least a partial subtitle).

**Evidence required:** The reviewer records, for each of the ten sentences:
- Subtitle appeared: yes or no.
- Approximate delay from end of speech to subtitle appearance: fast (under 2s), acceptable (2–4s), or slow (over 4s).
- Accuracy rating: exact, mostly correct, or garbled.

**Pass criteria:** At least 9 of 10 sentences produce a subtitle. At least 8 of those are rated mostly correct or exact. No sentence is rated slow more than once consecutively.

**Release blocker:** Three or more sentences producing no subtitle, or two or more sentences rated garbled, blocks the release.

### 7.2 Real Zoom Meeting — Translation Quality Verification

**Setup:** Same two-machine setup as above, but this time the human reviewer is a fluent reader of both the source language (Japanese) and the target language (Vietnamese).

**Task:** The speaker on Machine A speaks ten Japanese sentences. The reviewer reads the Vietnamese translation shown in the subtitle panel and rates it.

**Evidence required:** For each sentence, the reviewer records whether the translation conveys the intended meaning, is partially correct, or is misleading. A misleading translation is one where the meaning is reversed or lost entirely.

**Pass criteria:** At least 8 of 10 translations convey the intended meaning. No more than 1 translation rated misleading.

**Release blocker:** Two or more misleading translations in a single session blocks the release. This is treated as a critical quality issue because a misleading translation in a real meeting could cause misunderstanding.

### 7.3 Real Zoom Meeting — Optional Translated Audio Verification

**Setup:** Same two-machine setup as above, with translated audio enabled on Machine B.

**Task:** The speaker on Machine A speaks ten Japanese sentences. The reviewer on Machine B listens to the Vietnamese spoken output while also hearing the original Zoom meeting audio.

**What is being verified:**
- Spoken translated audio is produced for completed subtitle lines.
- The `T` toggle can turn translated audio off and on immediately without restarting the application.
- The optional audio channel does not make the underlying Zoom audio unusable.

**Evidence required:** For each of the ten sentences, the reviewer records:
- Spoken output played: yes or no.
- Understandable in Vietnamese: yes or no.
- Toggle behaved correctly before and after the sentence: yes or no.
- Did the translated audio interfere with the meeting: acceptable or unacceptable.

**Pass criteria:** At least 8 of 10 sentences produce understandable spoken Vietnamese output. The toggle works in both directions without restarting. No sentence is rated unacceptable because of audio interference.

**Release blocker:** If translated audio cannot be disabled immediately, repeatedly speaks the wrong sentence, or makes the meeting audio unusable, the release is blocked.

### 7.4 Terminal Emulator Compatibility on Real Machines

**Task:** A reviewer opens the application in each of the following real terminal environments on a real Windows machine and confirms it starts, displays correctly, and exits cleanly:

1. Windows Terminal (the default modern terminal on Windows 11)
2. ConEmu (a widely used third-party terminal)
3. The built-in Windows Console Host (cmd.exe / conhost.exe)
4. VS Code integrated terminal
5. Git Bash terminal

**Evidence required:** For each terminal, the reviewer records whether the application started without errors, whether the layout appeared correct, whether the subtitle text was readable, and whether the terminal returned to a clean state after exit.

**Pass criteria:** All five environments must start the application and display a usable interface. All five must exit cleanly.

**Release blocker:** Any environment that crashes on start, renders an unreadable layout, or leaves a broken terminal state after exit blocks the release.

### 7.5 Real-World Provider Key Verification

**Task:** A reviewer obtains a fresh Google Cloud API key (using the standard Cloud Console flow, not a test account), enters it into the application configuration, and runs a short five-minute translation session in a real Zoom meeting.

**What is being verified:** That the instructions provided to end users for obtaining and configuring an API key actually work. This test specifically validates the onboarding experience, not just the technical pipeline.

**Evidence required:** The reviewer documents each step they took (number of clicks, pages visited, configuration file edited) and rates the experience: straightforward, confusing, or broken.

**Pass criteria:** The reviewer rates the experience as straightforward or confusing. A rating of broken (for example, the key is entered but no translation appears and no error message explains why) blocks the release.

**Release blocker:** A broken rating on the onboarding experience blocks the release. A confusing rating requires that the documentation be updated before the release is approved.

### 7.6 Accessibility and Readability Review

**Task:** A reviewer who is not a software developer reads the subtitle output during a 10-minute Zoom meeting and answers the following questions:

- Could you read the subtitles comfortably without squinting or leaning in?
- Did the subtitles stay on screen long enough to finish reading each one?
- Was the cost display useful, or was it distracting?
- Did anything confuse you during the session?

**Evidence required:** Written answers to each question, recorded in the acceptance log.

**Pass criteria:** No answer describes subtitles that were unreadable, disappearing too fast, or actively confusing. Reviewer may note suggestions for improvement without blocking the release, unless the issue is rated critical.

**Release blocker:** Any reviewer statement that subtitles were unreadable or disappeared before they could finish reading blocks the release.

---

## 8. Release Blocker Summary

The following is a complete list of conditions that, if present, prevent a release from proceeding. This list is definitive. A release may not be marked as approved unless every item in this list is in a passing state.

| ID | Category | Condition |
|----|----------|-----------|
| B-01 | Build | CI pipeline fails or any unit test fails |
| B-02 | Contract | Google API contract test fails to parse live API response |
| B-03 | Integration | Any audio sample falls below 85% normalized-text accuracy |
| B-04 | Integration | Application crashes on incomplete or malformed audio input |
| B-05 | Integration | Application crashes during simulated network failure |
| B-06 | Terminal | Application crashes on terminal resize |
| B-07 | Terminal | Application leaves terminal in broken state after exit |
| B-08 | Terminal | Application crashes or produces unreadable output in monochrome mode |
| B-09 | Soak | Memory growth exceeds 50 MB over four-hour run |
| B-10 | Soak | CPU usage exceeds 60% at any sample during soak |
| B-11 | Soak | Chunk loss exceeds 5% in any 15-minute block during soak |
| B-12 | Soak | Subtitle latency exceeds 5 seconds in any 15-minute block during soak |
| B-13 | Soak | Cost counter differs from actual billing by more than 15% |
| B-14 | Soak | Application requires manual restart after 30-second network interruption |
| B-15 | Human | Three or more sentences produce no subtitle in Zoom meeting test |
| B-16 | Human | Two or more translated sentences rated misleading by bilingual reviewer |
| B-17 | Human | Any real terminal emulator crashes, renders unreadable layout, or leaves broken state |
| B-18 | Human | Onboarding experience with a fresh API key rated broken |
| B-19 | Human | Any reviewer finds subtitles unreadable or disappearing too fast |
| B-20 | Human | Translated audio cannot be toggled reliably or makes the meeting audio unusable |

---

## 9. What Each Gate Proves and Does Not Prove

Understanding what each gate does and does not prove prevents overconfidence and helps prioritize investigation when something goes wrong.

### Automated build and unit checks (L1)

**Proves:** The code compiles. Basic logic is correct for known inputs. The build is repeatable.

**Does not prove:** Anything about real audio, real users, real providers, or real terminals.

### Integration and contract tests (L2)

**Proves:** The pieces work together in sequence. External providers respond in the expected format. The error-handling logic functions.

**Does not prove:** That real Zoom audio produces better or worse results than pre-recorded samples. That provider APIs behave the same under high load or regional latency.

### Terminal behavior tests (L3)

**Proves:** The terminal UI renders correctly in simulated environments. Cleanup and resize work.

**Does not prove:** That the application looks and feels correct to a human in every real terminal. That font rendering, color accuracy, and scrollback behavior in real terminals match the simulation.

### Soak and stability tests (L4)

**Proves:** The application can run for extended periods without degrading. Memory and CPU behavior are under control. Cost tracking is approximately accurate.

**Does not prove:** That real meeting audio has the same statistical properties as the test audio stream. That a real Zoom meeting over a real network produces the same latency as a local audio feed.

### Human acceptance (L5)

**Proves:** Real users in real environments can use the product for its intended purpose. Real Zoom audio is captured and translated. Real terminal emulators display the interface correctly. The onboarding experience is functional.

**Does not prove:** That every possible user, terminal, or network condition has been tested. That translation quality will be equally good for all speakers, accents, and vocabulary domains.

---

## 10. Evidence Collection and Storage

All test results, reviewer logs, and soak reports are stored in the `verification-evidence/` directory at the root of the project repository. Each release candidate receives its own dated subdirectory.

The minimum evidence set required for a release approval consists of:

1. A CI run link showing all L1 and L2 tests passing.
2. A terminal test report showing all L3 tests passing.
3. A soak test report with the measurements defined in Section 6.
4. A signed acceptance log from at least two human reviewers, covering all L5 tests.

A release is not approved until all four artifacts exist and no release blockers are open.

---

## 11. Recurring Verification After Release

Verification does not stop at release. The following checks run on a recurring schedule after the product is deployed.

### Weekly contract test

The Google API contract test (Section 4.3) runs every week against the live API, even when no code changes have been made. This detects provider-side changes that could break existing deployments without any action by the development team.

**If this fails:** Users are notified within 24 hours. A patch is issued within 72 hours if the provider has changed a response format.

### Monthly human spot-check

Once a month, a reviewer repeats the Zoom meeting test (Section 7.1 and 7.2) on the current production version. This catches quality degradation from provider model updates.

**If this fails:** The finding is logged, investigated, and either a patch is issued or users are notified of a known quality degradation with an expected resolution timeline.

---

## 12. Glossary for Non-Technical Readers

**API (Application Programming Interface):** A standardized way for one software system to talk to another. In this product, the application uses Google's APIs to convert speech to text and translate it.

**Contract test:** A check that two systems are still speaking the same language to each other. If Google changes its API format, a contract test catches it immediately.

**CPU:** The main processing chip in a computer. High CPU usage means the application is working hard and may slow down other things running on the machine.

**Memory leak:** When software slowly uses more and more computer memory over time without releasing it. Eventually this can slow down or crash the machine.

**PTY (Pseudo-Terminal):** A software tool that simulates a real terminal window. It is used by automated tests to check how the application behaves in a terminal without needing a human to watch a screen.

**Soak test:** Running a program for a long time (hours) under realistic conditions to see if it stays stable. Named after the idea of "soaking" something to see if it leaks.

**STT (Speech-to-Text):** Converting spoken audio into written words.

**TTS (Text-to-Speech):** Converting written words into spoken audio.

**TUI (Text User Interface):** An application whose entire visual interface is made of characters displayed in a terminal window, rather than graphical buttons and images.

**Word error rate:** A measure of transcription accuracy. A word error rate of 10% means 1 in 10 words was incorrect. Lower is better.
