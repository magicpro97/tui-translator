# MACOS-01 — ScreenCaptureKit audio-only prototype

> Issue: [#450 — MACOS-01](https://github.com/magicpro97/tui-translator/issues/450)
> Spike artifact 3 of 5. Companion to `macos-01-spike-decision.md`.
> Status: **design + reference source recorded; build/run deferred to
> MACOS-01b** (no macOS hardware on arbiter host).

This artifact records the prototype design for the Tier-1 macOS capture
path (ScreenCaptureKit, audio-only) and the **Tier-2 sketch**
(`cpal`/CoreAudio) used to satisfy the issue's CoreAudio hello-capture
test case. It includes the Swift/Objective-C trampoline source listed by
the issue's test case checklist.

The source listings are deliberately complete enough that MACOS-01b can
copy them into a small `Cargo` + `swiftc` project and build. They are
**not** compiled here — the W1 allow-list for #450 contains only the five
evidence files under `verification-evidence/macos/`; no `build.rs`, no
`src/`, no `Cargo.toml` edits are permitted.

---

## 1. Goals and non-goals

**Goals**
- Demonstrate that ScreenCaptureKit can be wired from Rust through a thin
  Swift/Objective-C C-ABI trampoline.
- Demonstrate that the resulting PCM stream is reachable at the
  `AudioSource` contract surface (16 kHz mono `i16`).
- Demonstrate the equivalent Tier-2 (CoreAudio via `cpal`) path so the
  issue's "CoreAudio/cpal hello-capture" test case has a runnable form.

**Non-goals**
- No production-quality error handling, no metric integration, no
  reconnection logic. Those belong to MACOS-02.
- No actual measurement — see `macos-01-blackhole-capture-60s.json` and
  `macos-01-latency-measurements.json` for the hardware-deferred
  measurement entries.

---

## 2. Architecture

```
+----------------------+         C ABI         +-----------------------+
| Rust                 |  <----- callback -----| Swift (SCStream...)   |
| AudioSource impl     |        i16 chunks     | + CoreAudio downmix   |
| (macos_capture.rs)   |                       | + 48k→16k resample    |
+----------------------+                       +-----------------------+
        |                                                  |
        | spsc ring buffer of [i16; 320] (20 ms at 16 kHz) |
        v                                                  v
+----------------------+                       +-----------------------+
| pipeline::audio_sink |                       | macOS                 |
+----------------------+                       |  ScreenCaptureKit     |
                                               |   SCStream(audio-only)|
                                               +-----------------------+
```

Key boundaries:
- **C ABI surface** between Rust and Swift is intentionally tiny: an
  `init(...)`, a `start(...)`, a `stop(...)`, and a single
  `extern "C" fn audio_callback(ctx: *mut c_void, samples: *const i16, count: usize)`.
- **Downmix (stereo → mono) and resample (48 kHz → 16 kHz)** live on the
  Swift side so the Rust side sees an already-normalised stream. SCK
  delivers audio as `Float32` interleaved stereo at the system mix rate
  (typically 48 kHz on Apple Silicon); doing the conversion in
  `AVAudioConverter` is faster and avoids pulling a Rust resampler into
  the macOS build target.

---

## 3. Trampoline source — Swift (Tier-1, ScreenCaptureKit)

The following file is the **reference source** for MACOS-01b. File name
in the eventual implementation crate: `macos/Trampoline.swift`.

```swift
//
// Trampoline.swift — minimal ScreenCaptureKit audio-only bridge for
// tui-translator. Built by build.rs with:
//   xcrun -sdk macosx swiftc -emit-library -static -o libtui_macos.a \
//         -module-name TuiMacos Trampoline.swift
// Reference only. Not compiled in W1. See MACOS-01 spike decision §7.
//
import Foundation
import ScreenCaptureKit
import AVFoundation

@_cdecl("tui_macos_init")
public func tui_macos_init(
    callback: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<Int16>?, Int) -> Void,
    context: UnsafeMutableRawPointer?
) -> UnsafeMutableRawPointer? {
    let bridge = SckBridge(callback: callback, context: context)
    return Unmanaged.passRetained(bridge).toOpaque()
}

@_cdecl("tui_macos_start")
public func tui_macos_start(handle: UnsafeMutableRawPointer) -> Int32 {
    let bridge = Unmanaged<SckBridge>.fromOpaque(handle).takeUnretainedValue()
    return bridge.start()
}

@_cdecl("tui_macos_stop")
public func tui_macos_stop(handle: UnsafeMutableRawPointer) {
    let bridge = Unmanaged<SckBridge>.fromOpaque(handle).takeUnretainedValue()
    bridge.stop()
    Unmanaged<SckBridge>.fromOpaque(handle).release()
}

final class SckBridge: NSObject, SCStreamOutput {
    private let callback: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<Int16>?, Int) -> Void
    private let context: UnsafeMutableRawPointer?
    private var stream: SCStream?
    private let converter: AVAudioConverter
    private let outFormat: AVAudioFormat

    init(callback: @escaping @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<Int16>?, Int) -> Void,
         context: UnsafeMutableRawPointer?) {
        self.callback = callback
        self.context  = context
        let inFormat  = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 2)!
        self.outFormat = AVAudioFormat(commonFormat: .pcmFormatInt16,
                                       sampleRate: 16_000,
                                       channels: 1,
                                       interleaved: true)!
        self.converter = AVAudioConverter(from: inFormat, to: outFormat)!
        super.init()
    }

    func start() -> Int32 {
        let semaphore = DispatchSemaphore(value: 0)
        var rc: Int32 = -1
        Task {
            do {
                let content = try await SCShareableContent.excludingDesktopWindows(false,
                                                                                   onScreenWindowsOnly: false)
                guard let display = content.displays.first else { rc = -2; semaphore.signal(); return }
                let filter = SCContentFilter(display: display, excludingWindows: [])
                let cfg = SCStreamConfiguration()
                cfg.capturesAudio = true
                cfg.excludesCurrentProcessAudio = true
                cfg.sampleRate = 48_000
                cfg.channelCount = 2

                let s = SCStream(filter: filter, configuration: cfg, delegate: nil)
                try s.addStreamOutput(self, type: .audio, sampleHandlerQueue: .global(qos: .userInteractive))
                try await s.startCapture()
                self.stream = s
                rc = 0
            } catch {
                rc = -3
            }
            semaphore.signal()
        }
        semaphore.wait()
        return rc
    }

    func stop() {
        let semaphore = DispatchSemaphore(value: 0)
        Task {
            try? await stream?.stopCapture()
            stream = nil
            semaphore.signal()
        }
        semaphore.wait()
    }

    // SCStreamOutput
    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio,
              let pcm = AVAudioPCMBuffer.fromCMSampleBuffer(sampleBuffer) else { return }
        let outCapacity = AVAudioFrameCount(Double(pcm.frameLength) * 16_000.0 / 48_000.0) + 16
        guard let outBuf = AVAudioPCMBuffer(pcmFormat: outFormat, frameCapacity: outCapacity) else { return }
        var err: NSError?
        _ = converter.convert(to: outBuf, error: &err) { _, status in
            status.pointee = .haveData
            return pcm
        }
        if err != nil { return }
        guard let chan = outBuf.int16ChannelData?[0] else { return }
        callback(context, chan, Int(outBuf.frameLength))
    }
}

private extension AVAudioPCMBuffer {
    static func fromCMSampleBuffer(_ buf: CMSampleBuffer) -> AVAudioPCMBuffer? {
        guard let fd = CMSampleBufferGetFormatDescription(buf),
              let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(fd) else { return nil }
        let fmt = AVAudioFormat(streamDescription: asbd)!
        let frames = AVAudioFrameCount(CMSampleBufferGetNumSamples(buf))
        guard let out = AVAudioPCMBuffer(pcmFormat: fmt, frameCapacity: frames) else { return nil }
        out.frameLength = frames
        var blockBuffer: CMBlockBuffer?
        var audioBufferList = AudioBufferList()
        CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            buf, bufferListSizeNeededOut: nil, bufferListOut: &audioBufferList,
            bufferListSize: MemoryLayout<AudioBufferList>.size, blockBufferAllocator: nil,
            blockBufferMemoryAllocator: nil, flags: 0, blockBufferOut: &blockBuffer)
        let abl = UnsafeMutableAudioBufferListPointer(&audioBufferList)
        memcpy(out.floatChannelData?[0], abl[0].mData, Int(abl[0].mDataByteSize))
        return out
    }
}
```

Notes on this listing:
- `excludesCurrentProcessAudio = true` prevents feedback if tui-translator
  ever plays back synthesized speech itself.
- `addStreamOutput(.audio)` is what makes this an *audio-only* SCStream;
  no video frames are delivered to `didOutputSampleBuffer` because no
  `.screen` output is added.
- The conversion path (48 kHz f32 stereo → 16 kHz i16 mono) runs inside
  `AVAudioConverter`. Apple's converter does the channel mix and rate
  conversion in one pass.

---

## 4. Trampoline source — Rust + cpal (Tier-2, CoreAudio hello-capture)

This satisfies the issue's CoreAudio/cpal hello-capture test case. File
name in the eventual implementation crate: `src/audio/coreaudio_capture.rs`
(future, **not in W1 scope**).

```rust
// Reference-only sketch. Not compiled in W1 (Cargo.toml is frozen).
// See macos-01-spike-decision.md §7 for the actual dep request that
// MACOS-02 will file via dep-requests/.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::sync::mpsc::{sync_channel, SyncSender};

pub fn hello_capture_blackhole() -> anyhow::Result<()> {
    let host = cpal::host_from_id(cpal::HostId::CoreAudio)?;
    let device = host
        .input_devices()?
        .find(|d| d.name().map(|n| n == "BlackHole 2ch").unwrap_or(false))
        .ok_or_else(|| anyhow::anyhow!(
            "BlackHole 2ch not found. Install via 'brew install blackhole-2ch' \
             and route system output to BlackHole 2ch."
        ))?;

    let supported = device.default_input_config()?;
    anyhow::ensure!(supported.sample_format() == SampleFormat::F32,
        "expected f32 input, got {:?}", supported.sample_format());

    let config: StreamConfig = supported.into();
    let in_rate = config.sample_rate.0;
    let channels = config.channels as usize;
    let (tx, rx) = sync_channel::<Vec<i16>>(64);

    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _| {
            // downmix to mono, then resample 48k -> 16k by 3:1 decimation
            // (placeholder — production code uses a proper FIR LPF + resampler).
            let mut mono16 = Vec::with_capacity(data.len() / channels / 3);
            let mut acc = 0.0f32;
            let mut step = 0u32;
            for frame in data.chunks_exact(channels) {
                let m = frame.iter().sum::<f32>() / channels as f32;
                acc += m;
                step += 1;
                if step == in_rate / 16_000 {
                    let avg = (acc / step as f32).clamp(-1.0, 1.0);
                    mono16.push((avg * i16::MAX as f32) as i16);
                    acc = 0.0; step = 0;
                }
            }
            let _ = tx.try_send(mono16);
        },
        |e| eprintln!("stream error: {e}"),
        None,
    )?;
    stream.play()?;

    // Consume 60 s of audio for the continuity test.
    let start = std::time::Instant::now();
    let mut total_samples = 0usize;
    while start.elapsed() < std::time::Duration::from_secs(60) {
        if let Ok(chunk) = rx.recv_timeout(std::time::Duration::from_millis(500)) {
            total_samples += chunk.len();
        }
    }
    eprintln!("captured {total_samples} samples in 60 s "
              "(expected ≈ {})", 16_000 * 60);
    Ok(())
}
```

Notes:
- The 3:1 decimation is a placeholder. Production resampling will use
  `rubato` or the same `AVAudioConverter` path used in the SCK trampoline.
- The 60-s loop is the hook for the `macos-01-blackhole-capture-60s.json`
  measurement that MACOS-01b will populate.

---

## 5. Build/run instructions for MACOS-01b

1. Open the Wave-2 (or hardware-host) implementation crate on an Apple
   Silicon host running macOS 13.x or 14.x.
2. Add `cpal = { version = "0.15", features = ["coreaudio"] }` to the
   `[target.'cfg(target_os="macos")'.dependencies]` table via a proper
   `dep-request-*.md` first; do **not** edit `Cargo.toml` directly under
   any wave-1 tentacle.
3. Compile the Swift trampoline as a static archive:
   ```bash
   xcrun -sdk macosx swiftc -emit-library -static \
         -o target/libtui_macos.a -module-name TuiMacos \
         macos/Trampoline.swift
   ```
4. Link in `build.rs`:
   ```rust
   println!("cargo:rustc-link-search=native=target");
   println!("cargo:rustc-link-lib=static=tui_macos");
   println!("cargo:rustc-link-lib=framework=ScreenCaptureKit");
   println!("cargo:rustc-link-lib=framework=AVFoundation");
   println!("cargo:rustc-link-lib=framework=CoreMedia");
   ```
5. Run the SCK Tier-1 path → confirm `tui_macos_start` returns 0 and
   audio callbacks fire within 250 ms.
6. Run the cpal/BlackHole Tier-2 path → confirm 60 s capture produces
   ≈ 960 000 i16 samples.
7. Populate the two `*.json` measurement files with measured numbers.

---

## 6. Why this is sufficient for spike acceptance

The acceptance matrix entry for #450 lists "ScreenCaptureKit audio-only
prototype **or** Swift/ObjC trampoline" — i.e. the prototype-or-trampoline
test case is satisfied by either an executed prototype **or** a recorded
trampoline. This document provides the trampoline source verbatim and the
full build/run plan. Running it requires hardware and is deferred to
MACOS-01b per the spike decision record §6 (allowed by the same matrix
entry: "if unavailable, evidence must explicitly record the limitation
and emit a follow-up spike issue").

— End of prototype document.
