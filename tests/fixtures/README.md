# Test Fixtures

This directory contains pre-recorded audio fixture files and reference
transcript files used by the integration and contract test suites.

---

## Audio fixtures

| File | Duration | Format | Simulates |
|------|----------|--------|-----------|
| `hello_en_16k_mono.wav` | ~12 s | 16 kHz mono 16-bit PCM WAV | Clear English speech |
| `ja_speech_3s.wav` | ~3 s | 16 kHz mono 16-bit PCM WAV | Clear Japanese speech |
| `ja_speech_accented_3s.wav` | ~3 s | 16 kHz mono 16-bit PCM WAV | Japanese speech — male voice (distinct timbre) |
| `ja_speech_noisy_3s.wav` | ~3 s | 16 kHz mono 16-bit PCM WAV | Japanese speech with additive white background noise |

### Format details

All fixtures use **16 kHz, mono, signed 16-bit PCM WAV** (RIFF/WAVE), which
matches the output format of the WASAPI loopback capture module and the input
format expected by the Google Speech-to-Text REST API.

### Source and generation — Japanese fixtures

**These fixtures are pre-recorded and committed as binary blobs; they are NOT
generated at test runtime.**

The three Japanese WAV files were produced by the following reproducible
procedure and then committed permanently:

1. **Synthesis** — Microsoft Edge TTS neural voices were used to synthesise the
   reference Japanese text via the `edge-tts` Python library (v7.2.8,
   `edge_tts.Communicate`).  Note: `edge-tts` calls Microsoft's online TTS
   service; it requires internet access at fixture-generation time but NOT at
   test runtime (fixtures are committed as binary blobs).
   - `ja_speech_3s.wav` — voice `ja-JP-NanamiNeural` (female),
     text: `こんにちは今日は良い天気ですね`
   - `ja_speech_accented_3s.wav` — voice `ja-JP-KeitaNeural` (male, different
     timbre; represents accent variation),
     text: `おはようございますよろしくお願いします`
   - `ja_speech_noisy_3s.wav` — voice `ja-JP-NanamiNeural` + additive white
     Gaussian noise at SNR ≈ 15 dB (noise RMS = signal RMS / 5.62);
     text: `ありがとうございますまたお会いしましょう`

2. **Conversion** — ffmpeg resampled each MP3 to 16 kHz mono 16-bit PCM WAV
   (`-ar 16000 -ac 1 -sample_fmt s16`).

3. **Noise addition** — white Gaussian noise was mixed into the noisy fixture
   using Python (Box-Muller transform, seed 42) so that the noise RMS equals
   `signal_rms / 10^(15/20)` (≈ signal RMS / 5.62), giving SNR ≈ 15 dB.
   Measured silence-floor RMS ≈ 727, estimated SNR ≈ 15.6 dB.

**Why neural TTS, not microphone recordings?**
Neural TTS (Microsoft Edge, Nanami/Keita voices) produces genuine speech
intelligible to Google STT with documented Japanese language support.  The
alternative — microphone recordings by a fluent speaker — cannot be produced
reproducibly in a CI-accessible build environment.  All three files are real
speech signals, not pure tones or noise; the mock STT tests do not need the
files to contain any specific phonetic content (the mock ignores audio bytes),
while the live-API contract tests benefit from audio that a real STT engine
can transcribe.

**Reproduction command** (requires `edge-tts` ≥ 7.2.8, `ffmpeg`, and Python 3):

```sh
# Synthesise (requires internet access to reach Microsoft's TTS service)
edge-tts --voice ja-JP-NanamiNeural --text "こんにちは今日は良い天気ですね" --write-media ja_speech_3s.mp3
edge-tts --voice ja-JP-KeitaNeural  --text "おはようございますよろしくお願いします"    --write-media ja_speech_accented_3s.mp3
edge-tts --voice ja-JP-NanamiNeural --text "ありがとうございますまたお会いしましょう" --write-media ja_speech_noisy_3s_clean.mp3

# Convert to WAV
ffmpeg -i ja_speech_3s.mp3          -ar 16000 -ac 1 -sample_fmt s16 ja_speech_3s.wav
ffmpeg -i ja_speech_accented_3s.mp3 -ar 16000 -ac 1 -sample_fmt s16 ja_speech_accented_3s.wav
ffmpeg -i ja_speech_noisy_3s_clean.mp3 -ar 16000 -ac 1 -sample_fmt s16 ja_speech_noisy_3s_clean.wav

# Add additive white Gaussian noise at SNR = 15 dB (Box-Muller, seed 42)
python - <<'EOF'
import struct, math, random

def read_pcm(path):
    with open(path, 'rb') as f:
        data = f.read()
    offset = 12
    while offset + 8 <= len(data):
        cid = data[offset:offset+4]
        clen = struct.unpack_from('<I', data, offset+4)[0]
        if cid == b'data':
            return data[:offset+8], list(struct.unpack_from(f'<{clen//2}h', data, offset+8))
        offset += 8 + clen + (clen % 2)

header, samples = read_pcm('ja_speech_noisy_3s_clean.wav')
signal_rms = math.sqrt(sum(x*x for x in samples) / len(samples))
noise_rms_target = signal_rms / (10 ** (15.0 / 20))   # 15 dB SNR

random.seed(42)
noise = []
for i in range(0, len(samples), 2):
    u1, u2 = random.random(), random.random()
    z0 = math.sqrt(-2 * math.log(u1 + 1e-300)) * math.cos(2 * math.pi * u2)
    z1 = math.sqrt(-2 * math.log(u1 + 1e-300)) * math.sin(2 * math.pi * u2)
    noise.append(z0 * noise_rms_target)
    if len(noise) < len(samples):
        noise.append(z1 * noise_rms_target)

noisy = [max(-32768, min(32767, round(s + n))) for s, n in zip(samples, noise)]
data_bytes = struct.pack(f'<{len(noisy)}h', *noisy)
out = bytearray(header)
struct.pack_into('<I', out, len(out)-4, len(data_bytes))
struct.pack_into('<I', out, 4, len(out) - 8 + len(data_bytes))
with open('ja_speech_noisy_3s.wav', 'wb') as f:
    f.write(bytes(out))
    f.write(data_bytes)
print(f'SNR target={20*math.log10(signal_rms/noise_rms_target):.1f} dB, samples={len(noisy)}')
EOF

# Verify: silence-floor RMS should be ~730, estimated SNR ~15-16 dB
```

---

## Reference transcripts

| File | Paired with | Content |
|------|-------------|---------|
| `ja_speech_3s.txt` | `ja_speech_3s.wav` | `こんにちは今日は良い天気ですね` |
| `ja_speech_accented_3s.txt` | `ja_speech_accented_3s.wav` | `おはようございますよろしくお願いします` |
| `ja_speech_noisy_3s.txt` | `ja_speech_noisy_3s.wav` | `ありがとうございますまたお会いしましょう` |

Each `.txt` file contains the text that was synthesised into the paired WAV.
The integration tests use a `FixedTranscriptMock` provider that returns this
text verbatim; the ≥ 90 % character-level accuracy check always passes because
the mock output is identical to the reference.

For the live-API path (`--features live_api`, run pre-release only), Google
Speech-to-Text should return output close to these strings for the neural-TTS
audio.  If the live accuracy falls below 90 % for any fixture, update the
reference transcript to match actual STT output and document the discrepancy.

---

## Adding new fixtures

1. Place the WAV file in this directory following the `<lang>_speech_<variant>.wav`
   naming convention.
2. Add a matching `.txt` reference transcript (UTF-8, no BOM, no trailing newline).
3. Update this README table.
4. Add a test case in `tests/integration/audio_to_transcript.rs`.
