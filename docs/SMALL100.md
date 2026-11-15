# SMaLL-100

SMaLL-100 is Fluent's lightweight multilingual backend: a 330M-parameter
M2M-100 family model with a 12-layer encoder and a shallow 3-layer decoder.

- Upstream: [`alirezamsh/small100`](https://huggingface.co/alirezamsh/small100)
- Pinned revision: `8ab680e26a596d2e3d2d2d17ae0f68df1037328c`
- Model licence: **MIT** (the model card's licence). Fluent's own source code
  is also MIT, but the two licences are independent.
- Paper: Mohammadshahi et al., *SMaLL-100: Introducing Shallow Multilingual
  Machine Translation Model for Low-Resource Languages* (EMNLP 2022).

SMaLL-100 is **not** a replacement for NLLB or MADLAD. It is a fast option for
the languages below, while NLLB keeps the broadest coverage and MADLAD offers
T5-style translation.

## Why it needs a dedicated backend path

SMaLL-100 reuses the NLLB model, checkpoint reader, Q8 quantization and beam
search, but its tokenizer and language conditioning differ:

| | NLLB-200 | SMaLL-100 |
|---|---|---|
| Encoder prefix | **Source** language token | **Target** language token |
| Decoder start | `</s>` + forced target token | `</s>`, no forced token |
| Tokenizer file | `tokenizer.json` | `sentencepiece.bpe.model` + `vocab.json` |
| Language tokens | NLLB `xxx_Latn` tokens | 100 `__xx__` tokens after the text vocabulary |

Fluent implements this exactly:

- `Small100Tokenizer` segments text with the upstream SentencePiece BPE model
  using the pure-Rust `sentencepiece-rust` crate, then maps pieces through the
  official `vocab.json`. Encoding was differentially checked against Python
  `sentencepiece` on 13 cases covering Latin, CJK, Cyrillic, emoji, mixed
  scripts, leading/trailing whitespace and newlines; all produced identical
  token ids.
- `GenerateOptions::force_target_token` is `false` for SMaLL-100, so the decoder
  starts from `</s>` and the target language is carried by the encoder prefix.
- The NLLB backend variant and its tests are unchanged.

No Python runtime is required.

## Supported languages

Fluent maps **98** of SMaLL-100's 100 language tokens to NLLB/Fluent language
codes. Breton (`br`) and Western Frisian (`fy`) are absent from NLLB's language
table and are therefore not offered.

The seven required languages are all mapped:

| Language | Fluent code | SMaLL token |
|---|---|---|
| English | `eng_Latn` | `__en__` |
| French | `fra_Latn` | `__fr__` |
| German | `deu_Latn` | `__de__` |
| Spanish | `spa_Latn` | `__es__` |
| Portuguese | `por_Latn` | `__pt__` |
| Italian | `ita_Latn` | `__it__` |
| Chinese (Simplified) | `zho_Hans` | `__zh__` |

The upstream `__zh__` token does not distinguish scripts. Fluent maps it to
Simplified Chinese (`zho_Hans`) and does not claim Traditional Chinese
(`zho_Hant`) quality. The full mapping is `LANGUAGES` in
`crates/fluent-core/src/model/small100.rs`; a unit test checks every entry
against the language table.

## Installation

The native Model Manager downloads and verifies the pinned files:

| File | Size |
|---|---:|
| `config.json` | 890 B |
| `tokenizer_config.json` | 1,867 B |
| `special_tokens_map.json` | 1,559 B |
| `vocab.json` | 3.7 MB |
| `sentencepiece.bpe.model` | 2.4 MB |
| `model.safetensors` | 1.33 GB |

Open **Models** in the header, choose **SMaLL-100**, review the MIT licence and
the ~1.4 GB download, then confirm. The default directory is:

```text
~/.local/share/fluent/models/small100/        Linux
```

Every file is checked against its exact size and SHA-256 before the model is
reported as installed.

## Runtime behaviour

- **CPU first.** Automatic precision selects `f32` weights with Q8 blocks, the
  same path as NLLB. On the reference machine the model loads in about one
  second and uses roughly 0.4 GB resident after load (peak during conversion is
  higher; see [BENCHMARKS.md](BENCHMARKS.md)).
- **Beam search** is supported. The upstream recommendation is 5 beams and a
  256-token target limit; Fluent uses its normal manual settings.
- **Deterministic** for a fixed beam count, prompt and build.
- **Cancellation and output limits** use the shared engine lifecycle, including
  `Tokens(n)` and `UntilEos`.
- **CUDA/Metal** are compiled through the same backend but were not validated
  in this cycle. CPU is the validated path.

## Known limitations

- 98 of 100 languages are mapped; Breton and Western Frisian are not offered.
- `zho_Hans` is the only Chinese variant; Traditional Chinese is not claimed.
- The upstream release is distributed as `model.safetensors` and
  `pytorch_model.bin`; Fluent downloads only the safetensors file.
- Floating-point results can differ slightly across CPU architectures, as with
  NLLB.
- The model was trained for a 256-token target length. Fluent permits longer
  output, but quality beyond the training length is not guaranteed.

## Licence and attribution

The SMaLL-100 model card declares **MIT**. If you redistribute the model files,
keep the upstream attribution and licence notice. Fluent does not bundle or
redistribute the weights.

```bibtex
@inproceedings{mohammadshahi-etal-2022-small,
  title     = "{SM}a{LL}-100: Introducing Shallow Multilingual Machine Translation Model for Low-Resource Languages",
  author    = "Mohammadshahi, Alireza and Nikoulina, Vassilina and Berard, Alexandre and
               Gagnon, Michel and Ollagnier, Jean-Philippe and Bosc, Thomas",
  booktitle = "Proceedings of the 2022 Conference on Empirical Methods in Natural Language Processing",
  year      = "2022"
}
```
