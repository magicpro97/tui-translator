#!/usr/bin/env python3
"""
Generate tests/soak/soak_audio.wav — deterministic soak fixture.

Assembles a 30-second, 16 kHz mono 16-bit PCM WAV from REAL speech fixtures
already committed to tests/fixtures/, plus synthetic silence, Gaussian
background noise, and a loud transient event.  No tone envelopes or AM
signals are used for the speech segments.

Segment map (sample-exact):
  0.000–2.000 s  : Silence (32 000 samples)
  2.000–5.240 s  : ja_speech_3s.wav            — real Japanese speech (female TTS)
  5.240–6.000 s  : Silence gap (12 160 samples)
  6.000–9.264 s  : ja_speech_accented_3s.wav   — real Japanese speech (male TTS)
  9.264–10.000 s : Silence gap (11 776 samples)
  10.000–13.360 s: ja_speech_noisy_3s.wav      — real Japanese speech + background noise
  13.360–15.000 s: Gaussian background noise   — office ambient room tone (RMS ≈ 300)
  15.000–21.425 s: hello_en_16k_mono.wav       — real English speech (TTS)
  21.425–23.425 s: Loud transient event        — decaying 880 Hz burst (door slam)
  23.425–28.000 s: Gaussian background noise   — office ambient room tone (RMS ≈ 300)
  28.000–30.000 s: Silence (32 000 samples)    — trailing gap for seamless loop

The file is designed to be looped end-to-end by the soak-test runner to
simulate a continuous 4-hour audio stream without committing a ~460 MB binary.

All randomness uses seed 42 (Box–Muller transform); output is bit-for-bit
reproducible on any platform with Python ≥ 3.8 stdlib only (no external deps).

Usage:
    python tests/soak/gen_fixture.py        # writes tests/soak/soak_audio.wav
"""

from __future__ import annotations

import math
import os
import random
import struct

SAMPLE_RATE: int = 16_000   # Hz — matches WASAPI capture and Google STT input
NUM_CHANNELS: int = 1       # mono
BIT_DEPTH: int = 16         # signed 16-bit PCM
DURATION_S: int = 30        # seconds

_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
_FIXTURES_DIR = os.path.join(_SCRIPT_DIR, "..", "fixtures")


def _read_wav_pcm(rel_path: str) -> list[int]:
    """Read all PCM samples from a 16 kHz mono 16-bit WAV file in tests/fixtures/."""
    path = os.path.join(_FIXTURES_DIR, rel_path)
    with open(path, "rb") as f:
        data = f.read()
    if data[0:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError(f"Not a valid RIFF/WAVE file: {path}")
    fmt_size = struct.unpack_from("<I", data, 16)[0]
    channels = struct.unpack_from("<H", data, 22)[0]
    sr = struct.unpack_from("<I", data, 24)[0]
    bps = struct.unpack_from("<H", data, 34)[0]
    if sr != SAMPLE_RATE or channels != NUM_CHANNELS or bps != BIT_DEPTH:
        raise ValueError(
            f"Unexpected format in {path}: {sr} Hz, {channels} ch, {bps} bit; "
            f"expected {SAMPLE_RATE} Hz, {NUM_CHANNELS} ch, {BIT_DEPTH} bit"
        )
    offset = 12 + 8 + fmt_size
    if fmt_size % 2 != 0:
        offset += 1
    while offset + 8 <= len(data):
        cid = data[offset : offset + 4]
        clen = struct.unpack_from("<I", data, offset + 4)[0]
        if cid == b"data":
            n = clen // 2
            return list(struct.unpack_from(f"<{n}h", data, offset + 8))
        offset += 8 + clen
        if clen % 2 != 0:
            offset += 1
    raise ValueError(f"No data chunk found in {path}")


def _silence(n: int) -> list[int]:
    return [0] * n


def _background_noise(n: int, rng: random.Random, rms: float = 300.0) -> list[int]:
    """Return *n* Gaussian-distributed int16 samples via Box–Muller (deterministic seed)."""
    out: list[float] = []
    while len(out) < n:
        u1 = rng.random() + 1e-300
        u2 = rng.random()
        z0 = math.sqrt(-2.0 * math.log(u1)) * math.cos(2.0 * math.pi * u2)
        z1 = math.sqrt(-2.0 * math.log(u1)) * math.sin(2.0 * math.pi * u2)
        out.append(z0 * rms)
        if len(out) < n:
            out.append(z1 * rms)
    return [max(-32_768, min(32_767, round(x))) for x in out[:n]]


def _loud_transient(
    n: int,
    freq_hz: float = 880.0,
    peak: float = 20_000.0,
    decay_rate: float = 5.0,
) -> list[int]:
    """Exponentially decaying sine burst — simulates a door slam or loud knock."""
    samples: list[int] = []
    for i in range(n):
        t = i / SAMPLE_RATE
        env = math.exp(-decay_rate * t)
        samples.append(
            max(-32_768, min(32_767, round(env * peak * math.sin(2.0 * math.pi * freq_hz * t))))
        )
    return samples


def build_samples() -> list[int]:
    sr = SAMPLE_RATE
    rng = random.Random(42)  # deterministic seed

    # --- Real speech blocks (loaded from committed fixtures) ---
    ja1 = _read_wav_pcm("ja_speech_3s.wav")           # 51 840 samples (3.240 s)
    ja2 = _read_wav_pcm("ja_speech_accented_3s.wav")  # 52 224 samples (3.264 s)
    ja3 = _read_wav_pcm("ja_speech_noisy_3s.wav")     # 53 760 samples (3.360 s)
    en1 = _read_wav_pcm("hello_en_16k_mono.wav")      # 102 800 samples (6.425 s)

    # --- Derived gap / filler lengths (sample-exact arithmetic) ---
    # Each gap is chosen so that each speech run begins on a round-second boundary.
    gap1 = 6 * sr - 2 * sr - len(ja1)                  # 12 160
    gap2 = 10 * sr - 6 * sr - len(ja2)                 # 11 776
    noise1_count = 15 * sr - 10 * sr - len(ja3)        # 26 240
    transient_count = 2 * sr                            # 32 000
    noise2_count = 28 * sr - 15 * sr - len(en1) - transient_count  # 73 200

    if gap1 <= 0:
        raise ValueError(f"gap1={gap1} must be positive; ja1 too long?")
    if gap2 <= 0:
        raise ValueError(f"gap2={gap2} must be positive; ja2 too long?")
    if noise1_count <= 0:
        raise ValueError(f"noise1_count={noise1_count} must be positive; ja3 too long?")
    if noise2_count <= 0:
        raise ValueError(f"noise2_count={noise2_count} must be positive; en1 too long?")

    parts: list[int] = []
    parts += _silence(2 * sr)                      # 0.000–2.000 s  : silence
    parts += ja1                                   # 2.000–5.240 s  : Japanese speech (female)
    parts += _silence(gap1)                        # 5.240–6.000 s  : inter-speaker gap
    parts += ja2                                   # 6.000–9.264 s  : Japanese speech (male)
    parts += _silence(gap2)                        # 9.264–10.000 s : inter-speaker gap
    parts += ja3                                   # 10.000–13.360 s: Japanese noisy speech
    parts += _background_noise(noise1_count, rng)  # 13.360–15.000 s: ambient noise
    parts += en1                                   # 15.000–21.425 s: English speech
    parts += _loud_transient(transient_count)      # 21.425–23.425 s: loud transient
    parts += _background_noise(noise2_count, rng)  # 23.425–28.000 s: ambient noise
    parts += _silence(2 * sr)                      # 28.000–30.000 s: trailing silence

    total = len(parts)
    if total != DURATION_S * sr:
        raise ValueError(f"Expected {DURATION_S * sr} samples, got {total}")
    return parts


def write_wav(path: str, samples: list[int]) -> None:
    n = len(samples)
    data_bytes = struct.pack(f"<{n}h", *samples)
    byte_rate = SAMPLE_RATE * NUM_CHANNELS * (BIT_DEPTH // 8)
    block_align = NUM_CHANNELS * (BIT_DEPTH // 8)
    data_size = len(data_bytes)
    riff_size = 36 + data_size

    with open(path, "wb") as f:
        f.write(b"RIFF")
        f.write(struct.pack("<I", riff_size))
        f.write(b"WAVE")
        # fmt  chunk
        f.write(b"fmt ")
        f.write(struct.pack("<I", 16))            # PCM fmt chunk is always 16 bytes
        f.write(struct.pack("<H", 1))             # AudioFormat = PCM
        f.write(struct.pack("<H", NUM_CHANNELS))
        f.write(struct.pack("<I", SAMPLE_RATE))
        f.write(struct.pack("<I", byte_rate))
        f.write(struct.pack("<H", block_align))
        f.write(struct.pack("<H", BIT_DEPTH))
        # data chunk
        f.write(b"data")
        f.write(struct.pack("<I", data_size))
        f.write(data_bytes)


def main() -> None:
    out_path = os.path.join(_SCRIPT_DIR, "soak_audio.wav")
    samples = build_samples()
    write_wav(out_path, samples)

    rms = math.sqrt(sum(x * x for x in samples) / len(samples))
    size = os.path.getsize(out_path)
    print(
        f"Written: {out_path}\n"
        f"  Samples : {len(samples)}\n"
        f"  Duration: {len(samples) / SAMPLE_RATE:.1f} s\n"
        f"  RMS     : {rms:.0f}\n"
        f"  Size    : {size:,} bytes ({size / 1024:.1f} KiB)\n"
        f"  Format  : {SAMPLE_RATE} Hz, mono, {BIT_DEPTH}-bit PCM WAV"
    )


if __name__ == "__main__":
    main()
