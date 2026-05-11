# Business Requirements — TUI Translator

**Document status:** Draft v1  
**Audience:** Non-technical stakeholders, product owners, and operators  
**Related research:** `docs/00-research-findings.md`

---

## 1. The Problem We Are Solving

Zoom meetings increasingly cross language boundaries. A participant who does not speak the meeting language has very few good options today:

- **Zoom's built-in captions and translated captions** may exist, but they depend on the host's account, settings, and whether the host enabled them. A guest cannot rely on those features being available in a given meeting.
- **Zoom's Interpretation feature** (which lets a human interpreter speak a second audio channel) is a paid add-on that must be set up by the meeting host before the call starts. A guest cannot activate it themselves.
- **Real-time translation services outside Zoom** require the user to run a separate screen-share or browser window alongside the call, breaking focus and adding manual steps.

The result: **a meeting guest who needs real-time translation is completely dependent on the host**. If the host has not configured anything, the guest gets nothing.

This product removes that dependency. It runs entirely on the guest's own computer, captures the Zoom audio without touching the Zoom application, and delivers real-time bilingual subtitles — keeping the original line and the translated line together for comparison — plus optional translated audio in a simple terminal window, regardless of what the host has or has not configured.

---

## 2. Product Summary

**TUI Translator** is a lightweight Windows desktop program for real-time meeting translation in a terminal (command-line) window. Version 1 runs alongside Zoom, listens to the audio coming from the Zoom meeting on the user's machine, converts that speech to text, translates the text into the user's chosen language, and displays the result as scrolling bilingual subtitles (source line plus translated line).

If the user enables it, the program can also read the translated line aloud as optional synthesized audio.

It does not modify Zoom. It does not connect to the Zoom API. It does not require the meeting host to do anything. From Zoom's point of view, it is invisible.

---

## 3. User Personas

### 3.1 The Non-Native-Language Meeting Participant

**Who they are:** A professional or student who attends Zoom meetings conducted in a language they do not speak fluently — for example, a Vietnamese software developer attending a Japanese-language all-hands, or a Spanish-speaking manager sitting in on an English board update.

**What they need:**
- Understand what is being said, in real time, without interrupting the meeting.
- Not be dependent on the meeting organiser to set anything up.
- Receive subtitles that are good enough to follow the main points, even if phrasing is occasionally imperfect.

**What they do not need:**
- High-fidelity translation of technical nuance (that can be verified after the meeting).
- A graphical or visual interface beyond readable text in a window.

### 3.2 The Solo Operator (Developer or Power User)

**Who they are:** A technically comfortable user — not necessarily a developer — who is comfortable opening a terminal, typing a start command, and reading basic status messages.

**What they need:**
- To be able to start and stop translation from the keyboard, without touching the mouse.
- To see whether the system is working (connected, listening, translating) at a glance.
- To see live operational signals such as CPU, memory, network traffic, latency, loss, and cost without opening a second tool.
- To know approximately how much cloud-API cost has accumulated in the current session.
- To be able to switch target language without restarting the program.

**What they do not need:**
- A visual dashboard, charts, or icons.
- Integration with a billing or account management system.

---

## 4. The Guest-Only Constraint

A key design principle for this product is that it **must work when the meeting guest has no cooperation from the host**.

Native Zoom features that require host action — such as enabling captions, enabling interpretation, or granting interpreter roles — are explicitly out of scope for this product. They are unreliable for guest-only use because:

1. They must be configured before the meeting starts.
2. They depend on the host's account plan (some are paid features).
3. The guest has no way to verify or change these settings.

This product therefore works by capturing audio that is already playing on the guest's computer through the Windows audio system, not through any Zoom integration. No Zoom account, no Zoom API key, and no host permission is required.

*Source: `docs/00-research-findings.md`, "The product must work without Zoom host cooperation."*

---

## 5. Operator Workflow

Below is the typical session from the user's point of view, in plain language.

### Before the Meeting

1. The user opens a terminal window on their Windows computer.
2. The user runs the translator program with a simple command, specifying the language pair they want (for example, "Japanese to Vietnamese").
3. The program displays a status screen confirming it is ready and showing the selected languages.

### During the Meeting

4. The user joins the Zoom meeting as normal in the Zoom application.
5. The translator program is now active in the terminal window alongside Zoom.
6. As participants speak, paired subtitle lines appear in the terminal window within a few seconds: the original transcript and the translation are kept together so the user can compare them.
7. The user can at any time:
   - **Pause** translation (to focus on reading a document, for instance) and **resume** it.
   - **Change the target language** if the conversation switches language.
   - **Toggle the optional translated audio channel** on or off (a synthesised voice reading translations aloud through the speakers).
   - **Expand or collapse the live metrics view** to inspect CPU, memory, network, latency, loss, and cost in more detail.
   - **Reload the configuration** after editing `config.json`.
8. The terminal window never freezes, never blinks in a distracting way, and adapts if the window is resized.

### After the Meeting

9. The user presses a quit key to stop the program.
10. The program shows a brief session summary: duration, estimated cloud API cost, and number of lines translated.
11. All session data stays on the user's machine. Nothing is sent anywhere other than the configured cloud translation provider.

---

## 6. Inputs and Outputs

### Inputs

| Input | Description |
|---|---|
| Meeting audio | The audio currently playing through the Windows sound system from the Zoom meeting. The program captures this automatically. No microphone is used to pick up meeting audio. |
| User commands | Keystrokes entered by the user in the terminal to pause, resume, change language, or quit. |
| Configuration | A simple `config.json` file that specifies language pair, API key, feature toggles, and provider settings before the session starts. |

### Outputs

| Output | Description |
|---|---|
| Bilingual subtitles | The original transcript and translated line are displayed together as timestamped paired lines in the terminal window. |
| Status and metrics area | A persistent area of the terminal showing connection state, active language pair, CPU, RAM, network up/down, latency, loss, and session cost. |
| Optional translated audio | A synthesised voice reading the translation aloud through the computer speakers. The user can turn this on or off at any time. |
| Session summary | A brief end-of-session report shown when the program exits. |

---

## 7. Runtime Controls (User Perspective)

The user controls the program entirely through the keyboard while it is running. No mouse is needed.

| Key | Action |
|---|---|
| **Space** | Pause / resume translation |
| **L** | Change the target language |
| **T** | Toggle translated audio on or off |
| **M** | Expand / collapse the detailed metrics view |
| **R** | Reload `config.json` from disk |
| **Q** or **Ctrl+C** | Quit and show session summary |

All controls take effect immediately. There is no confirmation dialog. The status bar updates to reflect the new state within one second.

---

## 8. Success Criteria

The product is considered successful when the following conditions are met:

1. **A guest-only user can receive translated subtitles in a Zoom meeting without the host enabling any Zoom features.** There is no setup required on the Zoom side.

2. **Bilingual subtitle pairs appear within five seconds of the speaker finishing a sentence.** Users report that they can follow the meeting in real time and compare the source line with the translation.

3. **If translated audio is enabled, it can be turned on or off without restarting the program.** Disabling it stops new spoken output immediately.

4. **The program runs for the full duration of a one-hour meeting without crashing, freezing the terminal, or losing the live metrics display.**

5. **The on-screen cost counter is accurate within 10% of the actual bill from the cloud provider at the end of the session.**

6. **A user who is comfortable with a terminal but is not a developer can start and use the program by following a one-page setup guide** — no compilation, no dependency installation.

7. **The program works on a standard Windows 10 or Windows 11 machine using the normal Windows loopback path.** If a particular older setup needs a documented fallback step, that fallback is clearly explained in the setup guide.

---

## 9. Non-Goals

The following are explicitly out of scope for the first version of this product:

| Not in scope | Reason |
|---|---|
| macOS or Linux support | Audio capture on those platforms requires a different technical approach. Windows is the first target. |
| In-meeting transcription sharing | Subtitles are shown only to the local user. Sharing a transcript with other participants is not in scope. |
| Meeting recording or archiving | The product does not save audio or transcripts to disk by default. |
| Replacing Zoom's own caption feature | If Zoom captions are available and sufficient, the user can use them. This product is for when they are not. |
| Translating the user's own speech | The product translates what the user hears, not what they say. |
| Mobile (phone/tablet) use | The product is a desktop terminal application only. |
| Automatic language detection for the speaker | In v1, the user must specify the source language. Automatic detection is a future enhancement. |
| Billing integration or payment management | The product shows a cost estimate; it does not manage or pay cloud provider bills. |
| Graphical user interface | The product is intentionally terminal-based. A GUI version is not planned. |
| Support for platforms other than Zoom | The audio capture approach is general, but the product is designed and tested for Zoom on Windows. |

---

## 10. Assumptions

- The user's Windows machine can play Zoom meeting audio through its speakers or headphones. The program captures audio from the Windows audio system.
- The user has a valid API key for the chosen cloud provider (Google Cloud in v1).
- The user has an internet connection for the duration of the meeting (cloud APIs require connectivity).
- The user edits a plain `config.json` file before first use.
- The terminal window is kept visible on screen alongside the Zoom window. The subtitles are not overlaid on the Zoom window.

---

## 11. Glossary

| Term | Plain-language meaning |
|---|---|
| **Terminal / Command line** | A text-only window on the computer where the user types commands and reads output. No buttons or icons. |
| **Subtitle** | A line of translated text displayed on screen shortly after someone speaks. |
| **Speech-to-text (STT)** | Automated conversion of spoken words in audio into written text. |
| **Translation** | Automated conversion of written text from one language to another. |
| **Text-to-speech (TTS)** | Automated conversion of written text into a spoken audio voice. Used for the optional translated audio output. |
| **Loopback audio** | Audio that the computer is already playing back (e.g. the Zoom meeting sound) captured and fed into another program. No physical microphone is involved. |
| **Cloud API** | A remote internet service (such as Google Cloud) that the program sends audio or text to and receives translations from. The provider charges per unit of usage. |
| **Session cost** | The estimated dollar amount charged by the cloud provider for the current session, calculated from the amount of audio and text processed. |
| **Guest** | A Zoom meeting participant who does not own or host the meeting. Guests have limited control over meeting settings. |
| **Host** | The person who created and started a Zoom meeting. The host controls most meeting settings, including caption and interpretation features. |
| **v1** | The first released version of the product. It targets the most important use case with the most practical provider available for testing (Google Cloud). |

---

## 12. Relationship to Other Documents

| Document | What it covers |
|---|---|
| `docs/00-research-findings.md` | Verified technical constraints and source references that back up the decisions in this document. |
| `docs/02-google-first-provider.md` | Why Google Cloud is chosen as the first provider, what its current limitations are, and how the multi-provider path is kept open. |
| `docs/03-system-design.md` | How the components fit together technically (audio capture, translation pipeline, terminal display). Written for a technical audience. |
| `docs/04-verification-plan.md` | How the team will verify that the product actually works before calling it done. |
| `docs/05-implementation-roadmap.md` | The step-by-step plan for building and delivering the first version. |
