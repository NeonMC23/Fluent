# Model Package Specification

This is the contract between a user-supplied NLLB-200 model directory and the
Fluent engine. It describes what is implemented today; it is not a download
specification.

MADLAD-400 uses a different GGUF/tokenizer contract documented in
[MADLAD.md](MADLAD.md). SMaLL-100 uses `vocab.json` plus
`sentencepiece.bpe.model` and `model.safetensors`; see [SMALL100.md](SMALL100.md).
This page covers NLLB-200 only.

## 1. Directory layout

Fluent never bundles or downloads a model. Each model directory must contain:

```text
<model-dir>/
├── config.json                  required  NLLB architecture metadata
├── tokenizer.json               required  Hugging Face tokenizer, including NLLB language tokens
├── pytorch_model.bin            required  single-file PyTorch checkpoint
│   or model.safetensors                     single-file safetensors checkpoint
│   or pytorch_model.bin.index.json          sharded PyTorch checkpoint
│      + pytorch_model-*-of-*.bin
│   or model.safetensors.index.json          sharded safetensors checkpoint
│      + *.safetensors
└── (optional extras)            ignored   sentencepiece.bpe.model, tokenizer_config.json,
                                            special_tokens_map.json, generation_config.json,
                                            model card, README, gitattributes, ...
```

Only `config.json`, `tokenizer.json`, and one complete checkpoint form are
required by the current code. Files copied from a Hugging Face model repository
can remain in place; unused files are ignored.

The default install layout is:

```text
~/.local/share/fluent/models/nllb-200-1.3b/
~/.local/share/fluent/models/nllb-200-3.3b/
```

`config.json` (the Fluent configuration, not the model config) maps the
NLLB-200 1.3B and 3.3B models to directories. `--models-src DIR` tells the installer where to
find `<DIR>/nllb-200-1.3b/` and `<DIR>/nllb-200-3.3b/`.

## 2. `config.json`

The model configuration is parsed by `NllbConfig`. Required fields:

| Field | Rules |
|---|---|
| `d_model` | > 0; divisible by both attention-head counts |
| `encoder_attention_heads`, `decoder_attention_heads` | > 0 |
| `encoder_ffn_dim`, `decoder_ffn_dim` | > 0 |
| `encoder_layers`, `decoder_layers` | > 0 |
| `vocab_size` | > 0; must match the checkpoint and tokenizer |

Optional fields and defaults:

| Field | Default | Notes |
|---|---|---|
| `max_position_embeddings` | 1024 | Bounds source positions and generated output; `No limit` stops here if EOS never arrives |
| `pad_token_id` | 1 | Used for position-table construction |
| `eos_token_id` | 2 | Must match the tokenizer’s `</s>` for generation |
| `decoder_start_token_id` | 2 | Read for compatibility; the engine follows the NLLB prefix convention |
| `scale_embedding` | `true` | NLLB default |
| `activation_function` | `relu` | `relu`, `gelu`, or `silu` |

Unknown fields (for example `architectures`, `dropout`, `is_encoder_decoder`)
are accepted and ignored by Serde.

## 3. `tokenizer.json`

`NllbTokenizer::from_dir` loads only `tokenizer.json` through the Hugging Face
`tokenizers` crate. It must:

- contain the NLLB language tokens (for example `eng_Latn`, `fra_Latn`);
- contain `</s>`; loading fails if it is absent;
- preferably contain `<pad>`, `<s>`, and `<unk>`, which are used when present;
- support the normal tokenizer encode/decode API.

The SentencePiece file (`sentencepiece.bpe.model`) is **not** read at runtime.
It is optional. `tokenizer_config.json` and `special_tokens_map.json` are also
not read by Fluent.

## 4. Checkpoints

Discovery order in `Checkpoint::discover`:

1. `model.safetensors`
2. `model.safetensors.index.json` → listed shards
3. `pytorch_model.bin`
4. `pytorch_model.bin.index.json` → listed shards
5. bare `pytorch_model-*.bin` shards, sorted

The weight names must match the NLLB/M2M100 layout used by Fluent
(`model.shared.weight`, `model.encoder.*`, `model.decoder.*`, `lm_head.*` as
present in the checkpoint). Unsupported architectures or renamed tensors fail
during model construction with a Candle error.

Sharded checkpoints need all shards referenced by the index. A missing shard
fails at load time; an empty index produces
`MissingModelFile { hint: "empty weight index" }`.

## 5. Validation and errors

| Condition | Behaviour |
|---|---|
| Model directory absent | `Error::ModelNotFound(dir)` |
| `config.json` missing/unreadable | `Error::Config("could not read …")` |
| Invalid dimensions / divisibility | `Error::Config(...)` |
| `tokenizer.json` missing | `Error::MissingModelFile { hint: "missing tokenizer.json" }` |
| `</s>` missing | `Error::Tokenizer("the </s> token is missing…")` |
| No recognised checkpoint | `Error::MissingModelFile { hint: "expected pytorch_model.bin, …" }` |
| Unknown language token | `Error::UnknownLanguage(...)` |
| Insufficient memory | `Error::OutOfMemory { needed_mb, available_mb }` |

The **installer** applies a pre-flight rule before copying: a non-empty
`config.json`, a non-empty `tokenizer.json`, and a checkpoint larger than 1 MB
(or a sharded checkpoint with an index) must exist. Placeholder-only
directories are rejected with a warning. The installer never deletes or
overwrites an existing non-empty model directory.

Fluent then validates the real files at load time. The 1 MB installer check is a
placeholder guard, not a substitute for checkpoint validation.

## 6. Constraints and unsupported cases

- This contract covers NLLB-200/M2M100. MADLAD uses a separate T5/GGUF layout
  documented in [MADLAD.md](MADLAD.md); there is no generic
  Hugging Face model loader.
- Source sequences are limited to 1024 positions by the model’s position table
  and Fluent’s clamp. Longer text is truncated explicitly and reported.
- The 3.3B model requires substantially more RAM; the engine estimates memory
  before loading, but does not guarantee that the host has enough swap.
- No model is downloaded, executed, or modified by Fluent.

## 7. Future acquisition contract (not implemented)

If a downloader is added later, it must:

1. use a documented official source;
2. show the license before download;
3. require explicit user acceptance;
4. verify a trusted SHA-256 manifest obtained through the same trusted channel;
5. support resumable/interrupted downloads in a staging directory;
6. validate the exact file list and this package layout;
7. never execute downloaded content;
8. never overwrite a working model without consent;
9. keep models outside the application payload;
10. preserve offline translation after installation.

None of this is implemented in Cycle 7. Users must supply the model files
themselves, subject to the CC-BY-NC-4.0 terms described in
[MODEL_PROVENANCE.md](MODEL_PROVENANCE.md).
