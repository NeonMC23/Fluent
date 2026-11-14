# Model Provenance and Licensing — Cycle 7 Audit

This document records the evidence available for every NLLB-derived asset, what
was removed from the working tree, and what remains unresolved. It is an
engineering audit, not legal advice.

No model weights were downloaded, copied, packaged, or committed during Cycle 7.
The real 1.3B weights used for validation remained at their original read-only
location outside the repository.

## 1. What was committed at the Cycle 6 baseline

`git ls-files models` listed:

| Group | Files |
|---|---|
| NLLB 1.3B / 3.3B tokenizer | `tokenizer.json` (~17 MB each), `sentencepiece.bpe.model` (~4.9 MB each), `special_tokens_map.json`, `tokenizer_config.json` |
| NLLB model configuration | `config.json`, `generation_config.json` (1.3B), `pytorch_model.bin.index.json` (3.3B) |
| NLLB model cards / metadata | `README.md`, `gitattributes` |
| Weight placeholders | 1-byte `pytorch_model*.bin` files (not weights) |

Git history shows the files were uploaded in commits dated 2026-10-23
(`59e7874`, `04fd9ac`, and related “Add files via upload” commits), with no
per-file license notice.

## 2. Evidence table

| Asset | Source | License evidence | Confidence | Repository status after Cycle 7 | Distribution implication |
|---|---|---|---|---|---|
| Fluent Rust source | This repository | Root `LICENSE` is MIT | Confirmed by file | Present | MIT distribution permitted |
| NLLB-200 1.3B weights | Meta AI, `facebook/nllb-200-1.3B` | Model card: `license: "cc-by-nc-4.0"`, “License: CC-BY-NC” | High | **Never committed**; user-supplied | Non-commercial restriction; Fluent does not redistribute or bundle them |
| NLLB-200 3.3B weights | Meta AI, `facebook/nllb-200-3.3B` | Same model-card license | High | **Never committed**; user-supplied | Non-commercial restriction |
| `config.json`, `generation_config.json`, `pytorch_model.bin.index.json` | NLLB-200 release on Hugging Face | No separate license; model card is the only statement. Propagation to individual JSON files is an assumption | Medium | **Removed in Cycle 7** | No longer in the working tree; history still contains them |
| `tokenizer.json` | NLLB-200 release on Hugging Face | Model card states the SentencePiece model is released with NLLB-200; license line is CC-BY-NC-4.0. No per-file notice | Medium; not independently reviewed | **Removed in Cycle 7** | Same as above |
| `sentencepiece.bpe.model`, `special_tokens_map.json`, `tokenizer_config.json` | NLLB-200 release | Same assumption | Medium | **Removed in Cycle 7** | Same |
| NLLB model cards (`README.md`) and `gitattributes` | NLLB-200 release | Cards carry the CC-BY-NC-4.0 metadata | Medium | **Removed in Cycle 7** | Same |
| Weight placeholder files (1 byte) | Created for this repository | Inert placeholder, no model content | High | **Removed in Cycle 7** | None |
| Screenshot (`src/image1.png`) | This project (captured from the running app) | Project asset; treated as MIT with the source | High | Present | Redistributable |
| Runtime crates | crates.io / projects | See table below | High | Not bundled | Per-crate licenses apply |
| CUDA toolkit / Metal | NVIDIA / Apple | Vendor terms | High | Not redistributed | Users install/build their own |

The tokenizer/model-file license conclusion is **not** a legal determination.
The model card is evidence, not a per-file grant. Public redistribution of the
repository or a source archive should be reviewed before release.

## 3. What Cycle 7 changed

- The entire contents of `models/nllb-200-1.3b/` and `models/nllb-200-3.3b/`
  were removed from the working tree (43 MB of NLLB assets and placeholders).
- `models/README.md` (original text, MIT) now explains the user-supplied layout.
- `.gitignore` ignores `/models/*` except that README, so future model files
  cannot be committed accidentally.
- The engine was **not** changed: it already loaded `config.json`,
  `tokenizer.json`, and a checkpoint from the user-selected model directory.
  A unit test now pins the actionable error for a missing `tokenizer.json`.
- No tokenizer asset was replaced with a fake placeholder; the user supplies the
  real files.

### Important limitation: Git history

Removing files from HEAD does **not** remove them from earlier commits. A clone
of the full repository history still downloads the NLLB assets. Resolving that
requires a deliberate history rewrite for a release repository, or distributing
a source archive/tarball produced from the current tree rather than the full
history. History rewriting was not performed in this cycle. This remains a
release blocker for publishing the Git history publicly.

## 4. Why removal is technically safe

- `NllbTokenizer::from_dir` reads only `<model-dir>/tokenizer.json`; the
  SentencePiece file and tokenizer metadata are not read.
- `NllbConfig::from_dir` reads `<model-dir>/config.json`.
- `Checkpoint::discover` reads a single checkpoint or index + shards from
  `<model-dir>`.
- The installer copies a complete user-selected model directory.
- Workspace tests and benchmarks use an explicit model path or no model at all;
  none require the committed assets.
- Missing assets produce `ModelNotFound`, `MissingModelFile`,
  `Error::Config`, or `Tokenizer` errors with the path and a hint.
- The real 1.3B checkpoint continues to be used read-only for reliability and
  benchmark smoke tests; it is unaffected by the repository cleanup.

## 5. Runtime dependency licenses

| Dependency | License |
|---|---|
| `candle-core`, `candle-nn` | MIT OR Apache-2.0 |
| `eframe`, `egui` | MIT OR Apache-2.0 |
| `serde`, `serde_json`, `thiserror`, `rayon`, `directories` | MIT OR Apache-2.0 |
| `tokenizers` | Apache-2.0 |
| `whatlang` | MIT |

Transitive dependencies keep their own licenses; a full `cargo-deny` /
`cargo-about` audit is still a packaging task.

## 6. Consequences

- **Fluent source at HEAD no longer redistributes NLLB tokenizer or model
  assets.** This is a major improvement over Cycle 6.
- **Git history still contains them.** Public repository publication requires
  a history decision (rewrite or archive-only release), which is outside this
  cycle’s authorization.
- **The weights remain CC-BY-NC-4.0 and are never bundled.** Users obtain them
  separately and must comply with the non-commercial terms. Commercial use
  requires a separate license from Meta.
- **No downloader exists.** The safe acquisition contract is documented in
  [MODEL_PACKAGE.md](MODEL_PACKAGE.md); it is not implemented.

## 7. Downloads and copies during Cycle 7

- Model weights downloaded: **NO**
- Model weights copied into the repository: **NO**
- Model weights packaged: **NO**
- Tokenizer assets added: **NO**
- NLLB-derived assets removed from HEAD: **YES** (43 MB)
- Real weights used: **read-only**, external path, reliability/benchmark smoke
  tests only
