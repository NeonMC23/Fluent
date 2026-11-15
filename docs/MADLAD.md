# MADLAD-400-3B-MT

Fluent supports **MADLAD-400-3B-MT** as a second model family alongside
NLLB-200. MADLAD is a T5 encoder-decoder model. It runs fully offline on the
CPU and does not use Python, a server or a network connection at runtime.

## Model and license

- Repository: <https://huggingface.co/google/madlad400-3b-mt>
- Architecture: `T5ForConditionalGeneration`
- Parameters: 3B
- Pinned revision: `fa184c675da0b5c9e1c8694fccd4e12e2d422094`
- Model license: **Apache-2.0** (from the model card)
- Fluent source license: **MIT** (unchanged)
- The model weights, tokenizer and GGUF file are **not** distributed with
  Fluent and are not included in installers, portable archives or CI
  artifacts.

The official f32 `model.safetensors` is 11.8 GB. For CPU workstations, Fluent
uses the same repository's Apache-2.0 `model-q4k.gguf` (1.65 GB). Candle's
quantized T5 implementation loads it directly; no conversion script and no
Python are required.

## Download and storage

The preferred route is the native **Model Manager** (Models in the header).
It uses the same pinned manifest, resumes downloads, verifies SHA-256 and
shows progress. See [MODEL_MANAGER.md](MODEL_MANAGER.md).

The command-line fallback is:

```bash
python3 scripts/fetch-madlad.py
```

The default destination is:

```text
~/.local/share/fluent/models/madlad400-3b-mt/
```

Use `--dest DIR` to choose another directory. The script:

- checks free disk space before downloading;
- resumes `.part` files with `curl --continue-at -`;
- verifies every file against the pinned revision's size and SHA-256;
- leaves an existing unexpected file as `.invalid` rather than deleting it;
- verifies the complete directory again on completion.

Check an existing installation without downloading:

```bash
python3 scripts/fetch-madlad.py --check
```

Expected files include `config.json`, `model-q4k.gguf`, `tokenizer.json`,
`spiece.model` and the tokenizer metadata. The model directory is outside Git
and is ignored by the repository.

Add it to `config.json` if it is not in a conventional location:

```json
{
  "models": {
    "fast": "/path/to/nllb-200-1.3b",
    "accurate": "/path/to/nllb-200-3.3b",
    "madlad": "/path/to/madlad400-3b-mt"
  }
}
```

## Select and use it

1. Open **Settings**.
2. Select **MADLAD-400 3B MT** under Translation model.
3. Click **Load now**, or translate directly; the model loads on first use.
4. Choose a target language from the language picker. The picker shows only
   the verified MADLAD target languages.
5. Enter text and translate as usual.

MADLAD uses deterministic greedy decoding. Beam search is disabled for this
backend and the Settings page says so. The normal output-limit choices apply:
Automatic, Custom limit and No limit. Unlimited mode runs until EOS,
cancellation or an inference/memory error; it has no artificial token ceiling.

Cancellation remains available while the model is loading and during every
generated token.

## Verified language coverage

The GUI exposes a curated **72-language** MADLAD subset whose prefixes were
checked against the official tokenizer. It does not claim all 450+ MADLAD
languages. The list includes:

- English, French, German, Spanish, Portuguese, Italian, Dutch, Polish,
  Czech, Slovak, Slovenian, Croatian, Serbian, Bulgarian, Romanian, Hungarian,
  Finnish, Swedish, Danish, Norwegian, Estonian, Latvian, Lithuanian, Greek,
  Turkish, Ukrainian, Russian, Catalan, Galician, Basque, Icelandic, Irish,
  Welsh, Maltese, Luxembourgish, Bosnian, Albanian;
- Arabic, Hebrew, Persian, Urdu, Hindi, Bengali, Tamil, Nepali, Sinhala;
- Japanese, Korean, Chinese (Simplified and Traditional), Thai, Vietnamese,
  Indonesian, Malay, Tagalog, Khmer, Lao, Burmese;
- Swahili, Amharic, Zulu, Afrikaans, Hausa, Yoruba, Igbo, Somali;
- Georgian, Armenian, Kazakh, Uzbek, Azerbaijani.

Unsupported languages produce an explicit error; Fluent does not silently fall
back to NLLB.

## Measured CPU performance

Measured on the canonical development host:

| Component | Value |
|---|---|
| CPU | Intel Core i5-11400H, 6 cores / 12 threads |
| RAM | 15 GiB total, about 9 GiB available during testing |
| GPU | NVIDIA RTX 3050 Laptop, 4 GiB — **not used by MADLAD** |
| Model storage | 1.70 GB (q4_k GGUF + tokenizer) |
| Load time | 1.89 s |
| Peak process RAM | 2.72 GB |
| RSS after unload | ~1.54 GB retained by the allocator; engine state is released |
| First output | 1.4–2.2 s |
| Throughput | roughly 1.8–2.4 generated tokens/s depending on length |
| Typical sentence | 21–33 tokens in 8–13 s |
| Long generation | 520 tokens in 124 s; 2.98 GB peak |
| Unlimited cancellation | cancelled after 515 tokens; 3.22 GB peak |

These are CPU measurements only. GPU acceleration is not implemented for
MADLAD and is not advertised.

## Offline behavior

After the files above are present, no network access is needed. The model is
loaded from local storage and the application never downloads weights
implicitly.

## Known limitations

- CPU-only; CUDA/Metal are rejected for this backend.
- Greedy decoding only; no beam search.
- Curated 72-language mapping rather than the model's full advertised set.
- q4_k GGUF is a quantized route; quality and speed differ from the official
  f32 checkpoint.
- Allocation is not unlimited: very long generations can exhaust memory, and
  the error is reported rather than treated as success.
- A 520-token finite run, unlimited cancellation after 515 tokens, a
  post-cancellation translation and a drop/reload were tested successfully.
  Allocation failure itself was not deliberately induced.
- Physical Windows/macOS runtime validation is separate from the Linux
  measurements above.

## Tests

The real-model tests are opt-in because the weights are not in the repository:

```bash
FLUENT_MADLAD_MODEL="$HOME/.local/share/fluent/models/madlad400-3b-mt" \
    cargo test --release -p fluent-core --test madlad -- --nocapture
```

```bash
FLUENT_MADLAD_MODEL="$HOME/.local/share/fluent/models/madlad400-3b-mt" \
    cargo test --release -p fluent-core --lib real_model_translates_required_pairs -- --nocapture
```

The opt-in long-generation test generates 520 tokens with EOS suppressed,
cancels unlimited generation after 515 tokens, and then reloads the model:

```bash
FLUENT_MADLAD_MODEL="$HOME/.local/share/fluent/models/madlad400-3b-mt" \
    cargo test --release -p fluent-core --lib \
    real_model_exceeds_512_tokens_and_cancels_unlimited -- --nocapture
```

The normal workspace test suite skips these tests and remains fully offline.

## Architecture note

See [ADR-001](ADR-001-model-backends.md) for the shared engine interface and
the separate NLLB/T5 backends.
