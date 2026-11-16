# ADR-001: Coexisting NLLB and T5 (MADLAD) backends

## Status

Accepted for Cycle 13.

## Context

Fluent's engine is built around NLLB-200: a Python-free checkpoint reader,
tokenizer wrapper, quantized CPU kernels and beam search. MADLAD-400-3B-MT is
a T5 encoder-decoder model with a SentencePiece vocabulary, relative-position
attention and a target-language prefix. It does not share NLLB's tokenizer,
weight layout, language identifiers or generation loop.

The official MADLAD `model.safetensors` is 11.8 GB and does not fit this
development machine's available RAM. The same Apache-2.0 model repository
publishes a 1.65 GB `model-q4k.gguf`, which Candle's quantized T5 backend can
load directly without conversion.

## Decision

Use a small closed enum of backends inside the existing engine rather than a
plugin registry:

```text
LoadedBackend
├── Nllb  { Nllb, NllbTokenizer, existing beam search }
└── Madlad { QuantizedT5, MadladTokenizer, deterministic greedy search }
```

The engine owns the shared translation lifecycle: source detection, progress,
cancellation, output-limit policy, result construction, memory checks and
resource release. Each backend owns only model-specific loading, tokenization
and token generation.

A shared `ModelSpec` registry supplies GUI metadata (name, architecture,
parameter count, storage, license, default directory, decoding capabilities).
The GUI consumes that registry and does not branch on internal weight formats.

## Consequences

- NLLB code paths, quantization, beam search and tests remain intact.
- MADLAD is CPU-only through the GGUF q4k path; CUDA/Metal are not advertised.
- MADLAD uses deterministic greedy decoding. Beam search remains an NLLB
  capability and the GUI disables it for MADLAD instead of silently ignoring it.
- NLLB keeps its full 202-language list. MADLAD exposes only a curated,
  tokenizer-verified language subset; unsupported selections fail explicitly.
- The model acquisition script stores GGUF and tokenizer files outside Git.
- Adding `candle-transformers` is accepted for its tested quantized T5
  implementation; it does not add Python or a runtime service.

## Cycle 16 addition: M2M-100-style SMaLL-100

SMaLL-100 is an M2M-100 family checkpoint (12 encoder layers, 3 decoder layers,
`d_model=1024`, shared 128,112-entry vocabulary). It reuses the existing NLLB
model, checkpoint reader, quantization and beam search. Two parts differ and
are implemented explicitly rather than approximated:

1. **Tokenizer.** SMaLL-100 ships a SentencePiece BPE model, a separate
   `vocab.json` for token ids, and 100 `__xx__` language tokens appended after
   the 128,004 text tokens. There is no `tokenizer.json`. `Small100Tokenizer`
   uses the pure-Rust `sentencepiece-rust` crate for segmentation and applies
   the official `vocab.json` id mapping. Encoding was differentially checked
   against Python `sentencepiece` for Latin, CJK and mixed input.
2. **Language conditioning.** SMaLL-100 prepends the **target** language token
   to the encoder input (`[__tgt__] source </s>`) and starts the decoder at
   `</s>` without a forced language token. NLLB/M2M-100 prepend the source
   language and force the target token on the decoder. `GenerateOptions` now
   carries `force_target_token`, which is `false` for SMaLL-100.

The backend is exposed as `LoadedBackend::Small100 { Nllb, Small100Tokenizer }`
so the NLLB variant and its tests remain byte-for-byte unchanged. Language
coverage is a curated 98-language mapping between Fluent/NLLB codes and the
M2M-100 short codes; Breton and Western Frisian are absent from NLLB's table.

## Cycle 17 addition: capability registry and display identity

Three product-level decisions were made in Cycle 17:

1. **One capability API.** `registry::supports_language`, `language_list` and
   `resolve_language_selection` are the only language-availability checks used
   by the GUI, engine and catalogue. The NLLB table was corrected from 204 to
   the 202 language tokens actually present in the tokenizer; load-time checks
   reject an installed tokenizer that is missing an advertised code.
2. **Explicit display identity.** The main selector, Model Manager, settings,
   status bar and CLI use the real model names (NLLB-200 1.3B, NLLB-200 3.3B,
   MADLAD-400 3B, SMaLL-100). Internal `fast`/`accurate`/`madlad`/`small`
   identifiers, config keys and directories are unchanged for compatibility.
3. **One brand symbol.** The turquoise F mark already used in the header is the
   canonical application symbol. Window, Linux, Windows and macOS icons are
   derived from that exact crop. The full logo and banner masters remain
   untouched but are no longer used as icons.

## Cycle 18 addition: separate chat engine

Chat is not a translation backend. It uses a separate `ChatEngine`, Qwen2 GGUF
weights, a chat template and a dedicated worker and GUI view. Translation
models are never exposed as chat models, and chat model selection is independent
of the translation mode. See [CHAT.md](CHAT.md).
