# Model Provenance and Licensing

This document records the known provenance of every asset shipped with Fluent
and the restrictions that apply to distribution. It is a release-blocking
document: **Fluent source code is MIT-licensed, but the NLLB-200 assets are
not.**

No model weights were downloaded, copied, packaged, or committed during
Cycle 6. The real 1.3B weights used for validation remain at their original
read-only location outside the repository.

## 1. Asset table

| Asset | Known source | Known license | Distribution status |
|---|---|---|---|
| Rust source (`crates/`, `install.sh`, docs) | This repository | MIT (`LICENSE`) | Redistributable under MIT |
| NLLB-200 1.3B weights | Meta AI, `facebook/nllb-200-1.3B` (Hugging Face) | **CC-BY-NC-4.0** (model card: `license: "cc-by-nc-4.0"`, “License: CC-BY-NC”) | **Not redistributed.** 1-byte placeholder only; user supplies the real file |
| NLLB-200 3.3B weights | Meta AI, `facebook/nllb-200-3.3B` (Hugging Face) | **CC-BY-NC-4.0** | **Not redistributed.** 1-byte placeholders only |
| Tokenizer: `tokenizer.json`, `sentencepiece.bpe.model`, `special_tokens_map.json`, `tokenizer_config.json` | NLLB-200 release (the model card states “SentencePiece model is released along with NLLB-200”) | **Believed CC-BY-NC-4.0**, inherited from the NLLB release; not independently reviewed | **Currently committed under `models/`.** Public redistribution requires legal confirmation or removal with a compatible replacement |
| Model `config.json`, `generation_config.json`, `pytorch_model.bin.index.json` | NLLB-200 release | Believed CC-BY-NC-4.0 | Committed with the tokenizer; same blocker |
| NLLB model cards (`models/*/README.md`) | NLLB-200 release | Believed CC-BY-NC-4.0 (contains the license statement) | Committed; same blocker |
| Screenshots (`src/image1.png`, `src/image2.png`) | This project | MIT with the source (project asset) | Redistributable |
| `candle-core`, `candle-nn` | Candle project | MIT OR Apache-2.0 | Redistributable per crate license |
| `eframe`, `egui` | egui project | MIT OR Apache-2.0 | Redistributable per crate license |
| `serde`, `serde_json`, `thiserror`, `rayon`, `directories` | crates.io | MIT OR Apache-2.0 | Redistributable per crate license |
| `tokenizers` | Hugging Face | Apache-2.0 | Redistributable per crate license |
| `whatlang` | crates.io | MIT | Redistributable per crate license |
| CUDA toolkit / NVIDIA runtime (optional feature) | NVIDIA | NVIDIA license | Not redistributed; user installs it |
| Metal (optional feature) | Apple SDK / OS | Apple SDK terms | Not redistributed; built by the user on macOS |

The table covers direct dependencies only. Transitive crates keep their own
licenses; a full `cargo-about`/`cargo-deny` audit is a packaging task.

## 2. Consequences

- **Fluent cannot ship NLLB weights in an installer or archive.** The license is
  non-commercial and requires attribution; bundling would also make installers
  multi-gigabyte.
- **The non-commercial restriction applies to the model assets, not the MIT
  source.** Using Fluent with locally supplied NLLB weights for research or
  other non-commercial purposes is consistent with the model card; commercial
  use requires a separate license from Meta.
- **The committed tokenizer files are the unresolved part.** They are small
  enough to be committed, but they originate from the NLLB release. Until the
  license application is confirmed, publishing the repository or a source
  archive may redistribute CC-BY-NC material. Options for a future cycle:
  1. obtain written confirmation that the tokenizer files are covered by the
     repository's MIT license (unlikely), or
  2. remove the tokenizer files from the repository and have the installer
     validate/copy them from a user-supplied model directory, or
  3. keep the repository private until the model-asset question is resolved.
  Do not remove the tokenizer files without implementing and testing the
  user-supplied path; the application must keep working.
- **The model card states the model is for research** and is not released for
  production deployment. That is a product-risk statement independent of the
  license.

## 3. Model acquisition design (not implemented)

A future downloader is only safe when all of the following are true:

1. The official source URL and expected file list are documented.
2. The license text is displayed **before** any download.
3. The user explicitly accepts the license and the network access.
4. A trusted checksum manifest (SHA-256) is published separately from the
   download; TLS alone is not treated as integrity.
5. Downloads resume safely and are written to a temporary directory.
6. The complete file set and directory layout are validated before the model is
   made visible to the engine.
7. Nothing downloaded is executed.
8. An existing working model is never overwritten without explicit consent.
9. Models are stored in the model directory, never inside the application
   payload.
10. The UI explains that translation itself remains offline after acquisition.

**Status: not implemented.** The source, file list, and checksum manifest are
not established to the standard above, so adding a downloader would create an
unsafe or legally ambiguous workflow. Users must continue to supply model files
manually.

## 4. What the installer does today

- Looks for `nllb-200-1.3b/` and `nllb-200-3.3b/` under `--models-src`
  (default: the repository's `models/` directory) and under a legacy
  `~/.fluent/models/`.
- Rejects directories that contain only placeholder or incomplete files: a
  checkpoint larger than 1 MB (or a sharded checkpoint with an index) is
  required.
- Copies or hard-links the files into the user model directory; it never
  downloads, modifies, or deletes the source.
- Leaves model directories untouched on reinstall and uninstall.

## 5. Downloads and copies during Cycle 6

- Model weights downloaded: **NO**
- Model weights copied into the repository: **NO**
- Model weights committed: **NO**
- Model weights packaged: **NO**
- Tokenizer assets added or modified: **NO**
- Real weights used: **read-only**, at
  `/home/neon/Documents/CODE LOCAL/Fluent/models/nllb-200-1.3b`, for existing
  reliability and benchmark checks only.
