# Third-Party Notices

The tui-translator application (MIT License, see `LICENSE`) bundles or
links against the following third-party open-source components. Their
licenses and copyright notices are reproduced below as required by
the upstream projects.

## Apache 2.0

```
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION
   ...
```

The full Apache 2.0 license text is also available at
<https://www.apache.org/licenses/LICENSE-2.0.txt> and bundled at
`assets/licenses/opus-mt-apache.txt`.

---

## k2-fsa / sherpa-onnx

- **Component**: `sherpa-onnx` Rust crate (v1.13.x) wrapping the
  `k2-fsa/sherpa-onnx` C++ library.
- **Purpose**: On-device streaming speech recognition (used by
  the FunASR STT backend introduced in v3, see issues #807–#811
  and `docs/adr/stt-01-funasr-local-stt-replacement.md`).
- **Source**: <https://github.com/k2-fsa/sherpa-onnx>
- **License**: Apache 2.0
- **Copyright**: 2022-2025, The k2-fsa developers.
- **License file**: <https://github.com/k2-fsa/sherpa-onnx/blob/main/LICENSE>

## FunASR (ModelScope, Alibaba DAMO Academy)

- **Component**: `funasr` Paraformer model weights (small, medium,
  large variants) consumed via `sherpa-onnx`.
- **Purpose**: Multilingual automatic speech recognition, with strong
  Vietnamese support.
- **Source**: <https://github.com/alibaba-damo-academy/FunASR>
- **Model hub**: <https://www.modelscope.cn/models/damo/speech_paraformer-large_asr_nat-zh-cn-16k-common-vocab8404-pytorch>
- **License**: MIT (model weights); Apache 2.0 (the k2-fsa sherpa-onnx
  re-export packaging).
- **Copyright**: 2022-2025, Alibaba DAMO Academy.

## Whisper (OpenAI)

- **Component**: GGML-format `ggml-*.bin` model weights (tiny, base,
  small, medium variants, with English-only `*.en` siblings).
- **Purpose**: Local speech recognition, pre-dates the FunASR backend.
- **Source**: <https://github.com/openai/whisper> and
  <https://huggingface.co/ggerganov/whisper.cpp>.
- **License**: MIT
- **Copyright**: 2022, OpenAI.
- **License file**: `assets/licenses/whisper-mit.txt`.

## OPUS-MT (Helsinki-NLP)

- **Component**: `Helsinki-NLP/opus-mt-ja-vi` model bundle
  (7 ONNX + SPM + config files).
- **Purpose**: Local Japanese→Vietnamese machine translation.
- **Source**: <https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi>
- **License**: Apache 2.0
- **Copyright**: 2020, The OPUS-MT developers.
- **License file**: `assets/licenses/opus-mt-apache.txt`.

## Supertonic (TierOneMute)

- **Component**: `supertonic` TTS model weights and preprocessor.
- **Purpose**: Local text-to-speech, pre-dates v3.
- **Source**: <https://github.com/SupertoneInc/supertonic> (formerly
  TierOneMute).
- **License**: As specified in `assets/licenses/supertonic-notice.txt`.

---

## Verification

Run `python3 scripts/ci/test_third_party_notices.py` to verify this
file exists, names every bundled third-party component, and cites the
correct upstream license URL for each.
