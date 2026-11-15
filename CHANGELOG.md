# Changelog

All notable changes to Fluent are documented here. Versions are development
versions until a release is explicitly published.

## 0.2.0 — unreleased

### Added

- Native Model Manager with a three-model catalogue, download/repair,
  SHA-256 verification, progress/cancellation, safe removal, storage usage and
  an optional custom storage root.
- Central registry manifests with pinned repositories, revisions, file sizes
  and SHA-256 values for NLLB-200 and MADLAD-400.
- Reproducible product-level benchmark harness (`examples/eval`), FLORES-200
  sample and Fluent-oriented review corpus, and chrF++/BLEU summarizer.
- Cycle 14 long-generation tests: MADLAD 520 tokens, cancellation after 515,
  post-cancellation translation and drop/reload.
- MADLAD-400-3B-MT (T5) backend with the pinned Apache-2.0 q4_k GGUF,
  SentencePiece-compatible tokenizer, target-language prefixes, curated
  language mapping, deterministic greedy decoding, output limits and
  cancellation.
- Central model registry used by the GUI for NLLB and MADLAD names,
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
