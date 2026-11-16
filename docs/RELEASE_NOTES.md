# Fluent 0.2.0 — Release Notes

**Status: release candidate, not published.** No Git tag, GitHub Release or
public asset upload is implied by this document.

## Highlights

- **Local multilingual translation** with NLLB-200 1.3B, NLLB-200 3.3B,
  MADLAD-400 3B and SMaLL-100.
- **Local AI chat** with Qwen2.5 1.5B Instruct and Qwen2.5 0.5B Instruct,
  including streaming, cancellation, multi-message context and session-only
  history.
- **Native Model Manager** with pinned metadata, resumable downloads,
  progress/speed/ETA, SHA-256 verification, atomic promotion, repair, safe
  removal and a custom storage root.
- **Platform-native model storage** on Linux, Windows and macOS. Portable
  installations no longer derive the model root from the process working
  directory, and existing portable/legacy model directories are detected
  without moving files.
- **One heavyweight model at a time**: Translate and Chat unload the other
  engine when switching views.
- First-run storage guidance, clearer model states and actionable storage
  errors.

## Translation

| Model | Languages | Download | Licence |
|---|---:|---:|---|
| NLLB-200 1.3B | 202 | 5.51 GB | CC-BY-NC-4.0 |
| NLLB-200 3.3B | 202 | 17.60 GB | CC-BY-NC-4.0 |
| MADLAD-400 3B | 72 | 1.68 GB | Apache-2.0 |
| SMaLL-100 | 98 | 1.34 GB | MIT |

NLLB-200 and MADLAD-400 expose Simplified and Traditional Chinese; NLLB-200
also exposes Yue/Cantonese. SMaLL-100 exposes Simplified Chinese only.

## Chat

| Model | Download | Quantization | Context used | Licence |
|---|---:|---|---:|---|
| Qwen2.5 1.5B Instruct | 1.12 GB | Q4_K_M | 4096 tokens | Apache-2.0 |
| Qwen2.5 0.5B Instruct | 0.44 GB | Q4_0 | 4096 tokens | Apache-2.0 |

Chat is CPU-only in the validated release, generates at most 512 tokens per
reply, has no temperature/top-p controls and does not provide tool calling,
retrieval, web access, vision or persistent history. Both models are compact
and can produce inaccurate or fabricated information.

## Platforms

- **Linux x86-64:** physically runtime-validated; glibc 2.35 baseline;
  portable tar.gz; locally Minisign-signed candidate.
- **Windows x86-64:** CI-built, packaged and path-tested; **physical GUI/runtime
  validation NOT EXECUTED**; unsigned, no Authenticode certificate.
- **macOS ARM64:** CI-built, `.app`/plist/Mach-O/smoke-tested; **physical
  Finder/Terminal validation NOT EXECUTED**; unsigned and not notarized.
- Intel macOS and universal binaries are not produced.

## Installation

Application archives contain no models or user configuration. Install a model
through **Models** after launch, or point `config.json` at an existing model
directory. The default model roots are:

- Linux: `$XDG_DATA_HOME/fluent/models`, fallback
  `~/.local/share/fluent/models`;
- Windows: `%APPDATA%\fluent\Fluent\data\models`;
- macOS: `~/Library/Application Support/io.fluent.Fluent/models`.

See the [README](../README.md) and [MODEL_MANAGER.md](MODEL_MANAGER.md) for
installation, verification and storage guidance.

## Verification

- Verify each archive with its adjacent SHA-256 file.
- The local Linux candidate is Minisign-signed with public key
  `B504F2189BB21013`; verify against [the committed public key](keys/minisign.pub).
- CI artifacts are currently **unsigned** because `MINISIGN_SECRET_KEY` is not
  configured.
- Windows/macOS archives are unsigned and not notarized.

## Known limitations

- Windows and macOS physical runtime validation is pending.
- No Windows installer, macOS DMG, AppImage or Flatpak.
- No GPU-validated chat inference.
- No persistent chat history, tool calling, RAG or web access.
- NLLB-200 is non-commercial; Fluent's MIT source licence does not relicense
  model assets.
- Historical Git commits still contain NLLB tokenizer/configuration assets;
  branding redistribution rights require owner confirmation. See
  [MODEL_PROVENANCE.md](MODEL_PROVENANCE.md) and
  [RELEASE_INTEGRITY.md](RELEASE_INTEGRITY.md).

## Publication

Publication remains a separate owner-authorized action. Before publishing,
complete the manual Windows and macOS checks, decide the signing requirements,
confirm model/branding rights, and authorize any release tag or GitHub Release.
