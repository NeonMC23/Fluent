# Changelog

All notable changes to Fluent are documented here. Versions are development
versions until a release is explicitly published.

## 0.2.0 — unreleased

### Added

- Local chat view with a separate Qwen2.5 Instruct engine, worker and model
  directory. Two chat models are supported: Qwen2.5 1.5B Instruct (Q4_K_M,
  1.1 GB) and Qwen2.5 0.5B Instruct (Q4_0, 0.4 GB), both Apache-2.0.
- Streaming assistant replies, cancellation, deterministic greedy decoding
  with a repetition penalty, conversation context, context-limit notices and
  session-only history.
- Chat models in the native Model Manager with the same pinned manifests,
  resumable downloads, SHA-256 verification, safe removal and licence display
  as translation models.
- Translate/Chat navigation. The two capabilities have independent model
  selection, and switching views unloads the other heavyweight model.
- First-run guidance when a translation or chat model is not installed.
- `chat_eval` reproducible benchmark harness and gated real-model chat tests.
- Canonical turquoise F mark used for the GUI header, window icon, Linux
  launcher icon, Windows executable/window icon and macOS bundle/dock icon.
  All platform variants derive reproducibly from the immutable logo master.
- Explicit model identities in the main selector, Model Manager, settings,
  status bar and CLI: NLLB-200 1.3B, NLLB-200 3.3B, MADLAD-400 3B and
  SMaLL-100, each with a short positioning description.
- Central language-capability reconciliation (`resolve_language_selection`)
  that filters both selectors, clamps saved settings on startup and switches,
  and rejects unsupported codes in the engine before a model load.
- NLLB-200 language table corrected from 204 advertised codes to the 202
  language tokens actually present in the tokenizer. `sat_Olck`, `arb_Latn`
  and `min_Arab` are not supported by NLLB-200; `sat_Beng` is.
- Load-time validation that installed NLLB, MADLAD and SMaLL-100 tokenizers
  cover every advertised language.
- Language-capability regression tests, including a gated real-tokenizer
  coverage test for all four models.

### Changed

- Language buttons and picker rows show human-readable names and native names
  instead of internal `xxx_Yyyy` codes.
- The main model control is a compact selector with a descriptive list; the
  old generic Fast/Accurate/MADLAD buttons are gone.
- Branding generation now supports `--check` for byte-for-byte reproducibility.

### Added

- Native Model Manager with a four-model catalogue, download/repair,
  SHA-256 verification, progress/cancellation, safe removal, storage usage and
  an optional custom storage root.
- Central registry manifests with pinned repositories, revisions, file sizes
  and SHA-256 values for NLLB-200, MADLAD-400 and SMaLL-100.
- SMaLL-100 (330M, MIT) backend with a pure-Rust SentencePiece tokenizer,
  target-language encoder conditioning, shallow-decoder generation, Q8 CPU
  inference, cancellation and output limits.
- Versioned remote model catalogue (`catalogue/models-v1.json`) with strict
  schema validation, HTTPS-only URLs, a body limit, fail-closed behaviour and a
  Model Manager refresh action. A committed catalogue test prevents drift from
  the compiled registry.
- 42 directed-pair FLORES-200 corpus for the seven required languages and a
  measured comparison against NLLB-200 1.3B/3.3B and MADLAD-400.
- Reproducible product-level benchmark harness (`examples/eval`), FLORES-200
  sample and Fluent-oriented review corpus, and chrF++/BLEU summarizer.
- Cycle 14 long-generation tests: MADLAD 520 tokens, cancellation after 515,
  post-cancellation translation and drop/reload.
- MADLAD-400-3B-MT (T5) backend with the pinned Apache-2.0 q4_k GGUF,
  SentencePiece-compatible tokenizer, target-language prefixes, curated
  language mapping, deterministic greedy decoding, output limits and
  cancellation.
- Central model registry used by the GUI for NLLB, MADLAD and SMaLL-100 names,
  architecture, storage, license and decoding capabilities.
- Reproducible `scripts/fetch-madlad.py` acquisition with resumable downloads,
  disk checks and pinned SHA-256 verification.
- Official Fluent branding in the application header and platform icons.
- Linux desktop entry and icon installation as part of the per-user installer.
- Portable Linux archive with deterministic packaging metadata and SHA-256.
- Windows resource/icon embedding and macOS `.app` bundle staging.
- GitHub Actions native builds and downloadable workflow artifacts for Linux,
  Windows and macOS, plus an aggregate release-integrity artifact.
- Minisign-compatible detached-signature scripts and release manifest tooling.
- Committed production Minisign public key (`B504F2189BB21013`); private key
  stays outside the repository and is not yet configured as a CI secret.
- Non-interactive `--version`, `--self-test` and `--smoke-test-gui` modes.
- Output-length selector with Automatic, Custom limit, and No limit modes.
  No limit generates until EOS, cancellation, or the model's positional
  capacity; the CPU KV cache grows geometrically for long generations.
- Real-model test generated 600 tokens (beyond the previous 512 ceiling) and
  verified cancellation after 520 tokens in unlimited mode.
- Cross-platform archive/manifest/signature verifier
  (`scripts/verify-artifacts.py`) that checks actual bytes, executable formats,
  architectures, checksums and detached signatures.
- Native CI smoke tests of the extracted Linux archive under Xvfb, Windows ZIP
  and macOS `.app` bundle, including a real temporary GUI window.
- Release-integrity/signing preparation documentation and scripts.

### Changed

- CPU Automatic precision now uses the Q8 path for NLLB 3.3B as well as 1.3B.
  On the benchmark host this reduced median 3.3B latency from 26.6 s to
  3.6 s and peak RSS from 6.54 GB to 4.57 GB on a 12-case probe, with
  identical metrics.
- Application version is centralized in the Cargo workspace and derived by
  build/package scripts and the GUI About string.
- Settings layout uses aligned grids and equal model cards.
- Source and translation panels reserve matching footer space and heights.
- Single, larger language swap control with consistent header control sizes.

### Fixed

- Installed launch now creates the `.desktop` entry and icon automatically.
- `XDG_BIN_HOME` and custom XDG locations are respected.
- Packaging works from checkout paths containing spaces.

## 0.1.0 — previous published version

Initial public/development release with local NLLB inference, Q8 CPU weights,
AVX2 with scalar fallback, fused attention/cache, cancellation, benchmark
harness and Linux installer foundation.
