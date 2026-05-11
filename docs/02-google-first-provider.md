# Decision Memo: Google-First Provider for v1

**Audience:** Anyone involved in the project, including non-engineers.
**Status:** Decided — no remaining open questions for v1.
**Date:** 2026-05-11

---

## What This Document Decides

This memo resolves which translation and speech service the app will use for its first real, working version (v1), and explains how the architecture is designed so that switching to or adding more services later does not require rebuilding everything.

The short answer is: **Google is the only provider for v1**, and the app is structured from the start so that Azure and Ollama can be added later without redesigning the core.

---

## Why Google-First

The user currently has exactly one set of real API credentials: a Google Cloud API key. No Azure subscription, no other paid account, is available for testing right now.

That single fact drives the v1 decision. Building or testing against Azure or any other provider would require placeholder code that can never be verified on real hardware. Unverified code is a liability, not an asset. The v1 plan therefore commits to building and verifying only what can be tested today — and today that means Google.

---

## What Google Can and Cannot Do in Rust Today

This is the most important part of the memo to read carefully, because Google's support in Rust is stronger in some areas and weaker in others.

### What works well right now

Google provides official Rust client libraries for its cloud services. Two of them are solid and production-ready:

- **Translation** (converting text from one language to another) works fully and reliably using the official Rust library.
- **Text-to-Speech** (converting translated text into spoken audio) also works fully for the standard use case, where you send a piece of text and receive back an audio file.

These two capabilities cover two of the user-visible outputs of the app: translated text on screen and the optional spoken translation channel.

### Where the story is more complicated: real-time transcription

The third capability — **Speech-to-Text**, which converts live audio into text as you speak — is where Google's Rust situation is less straightforward.

Google does offer a Speech-to-Text service that supports real-time transcription. But the official Rust client library does not yet expose the clean streaming interface that would make this simple in Rust.

For v1, the honest and testable path is to send short rolling audio chunks to Google's Speech-to-Text service and treat each response as a near-real-time subtitle update. This is slightly less elegant than true streaming, but it is compatible with the credentials available today and can be verified on real hardware.

**To be clear about the tradeoff:** This approach is technically solid, but it may add a little more delay than a fully streaming implementation. A later validation phase can test lower-latency community gRPC streaming on real hardware. That future improvement does not change the v1 provider decision.

---

## Why the App Still Needs a "Stage-Based Provider" Design

Even though v1 only uses Google, the app must be designed from the start with a layer that separates "what the app does" from "which service does it."

Here is why this matters, in plain terms.

The app has three distinct jobs:
1. Listen to audio and convert speech to text (Speech-to-Text).
2. Translate that text into another language (Translation).
3. Optionally speak the translated text aloud (Text-to-Speech).

Each of those three jobs is handled by a provider — a cloud service or local program that knows how to do that specific thing. A "stage-based" design means the app treats each of these three stages as a slot, and any compatible service can fill that slot.

If the app is not designed this way — if Google's specific way of doing things is baked directly into the app's core — then adding Azure later would mean rewriting large parts of the application. That is expensive and risky.

Doing the design correctly now costs very little extra effort. It means writing a small "contract" for what a Speech-to-Text provider must do, what a Translation provider must do, and what a Text-to-Speech provider must do. Google fills all three slots in v1. Azure, Ollama, or other services can fill one or more slots later without touching the app's core.

This is the standard way professional software is structured when multi-service support is planned. It is not over-engineering; it is the minimum architecture that avoids expensive rewrites later.

The same idea appears in the user-facing `config.json` file. The file is stage-based from day one (`stt`, `translate`, `tts`), even though only Google values are valid and tested in v1.

---

## Where Azure Fits Later

Azure is Microsoft's cloud AI platform, and it has strong speech services. It is a credible second provider for this application, particularly for enterprise users who already have Azure subscriptions.

**The honest current situation with Azure in Rust:**
Azure does not publish an official Rust library for its speech services. A community-built library exists and is reasonably capable, but it covers transcription and text-to-speech, not the full speech translation path. It has not been tested at the same confidence level as the Google path.

**The plan:** Azure is not in scope for v1. After v1 is working and verified on Google, Azure can be evaluated as an alternative for Speech-to-Text and Text-to-Speech. The stage-based design means that adding Azure requires only filling in the Azure-specific implementation for one or more of the three slots — it does not require redesigning the application.

Azure is also a natural choice for cost optimization later: depending on the provider's pricing at the time and the volume of usage, routing some requests to Azure could reduce costs without changing the user's experience.

---

## Where Ollama Fits Later

Ollama is a tool for running large language models locally on the user's own computer, without sending data to a cloud service. It is not a speech service — it cannot listen to audio or speak text aloud.

**What Ollama can contribute:** Ollama is useful for the Translation stage. A large language model running locally can translate text, and in some cases the quality is comparable to cloud services, with the advantage that data stays on the machine.

Ollama is particularly interesting for:
- Privacy-sensitive use cases where users do not want text leaving their machine.
- Offline or low-connectivity scenarios.
- Cost reduction, since local processing has no per-character fees.

**The plan:** Ollama is not in scope for v1. After v1 is working with Google, Ollama can be evaluated as an alternative or hybrid for the Translation slot. Because the architecture has a clean Translation slot, slotting in Ollama later requires no changes to the audio or output parts of the application.

---

## What Multi-Provider Support Is Not Blocking v1

The design principle throughout is that multi-provider support is an architectural property — something built into the structure from the start — but it is not a feature that has to be implemented, tested, or shipped in v1.

In v1, Google fills all three slots. The slots exist and are defined, but they have exactly one implementation each. That is sufficient for a first working version.

Multi-provider routing, automatic failover, cost-based provider selection, and user-configurable provider preferences are all valid future features. They belong in a later phase after the core application is verified on real hardware with real credentials.

---

## Summary of Decisions

| Question | Decision |
|---|---|
| Which provider is used for v1? | Google only |
| Can the app be tested with current credentials? | Yes — the user has a Google key |
| Is Google's Rust support complete? | Partially — Translation and TTS are solid; STT works in v1 through short rolling requests, while lower-latency streaming remains a later validation path |
| Does v1 need a provider abstraction layer? | Yes — otherwise adding Azure or Ollama later requires a rewrite |
| When does Azure enter the picture? | After v1 is verified on Google |
| When does Ollama enter the picture? | After v1 is verified; useful for Translation cost reduction and privacy |
| Does multi-provider block v1? | No |

---

## Source Evidence

All technical claims in this document are based on verified findings documented in `docs/00-research-findings.md`, which includes source references for:
- Google Cloud Rust clients (googleapis/google-cloud-rust)
- Azure community Rust crate (jBernavaPrah/azure-speech-sdk-rs)
- Ollama Rust client (pepperoni21/ollama-rs)
- The practical limits of Google's official streaming Rust support
