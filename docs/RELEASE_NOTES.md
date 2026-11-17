# Fluent 0.2.0 — Release Notes

**Status: prepared for owner-authorized manual publication; not published.**
No Git tag, GitHub Release or public asset upload is implied by this document.

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
- **Windows x86-64:** automated CI build, package and path validation passed;
  **physical GUI/runtime and inference validation NOT EXECUTED**; not
  Authenticode-signed.
- **macOS ARM64:** automated CI build, bundle validation and runner GUI smoke
  test passed; **physical Finder/Dock/Gatekeeper and inference validation NOT
  EXECUTED**; not Developer ID-signed or notarized.
- Intel macOS and universal binaries are not produced.

## Installation

Application archives contain no models, tokenizer files or user configuration.
Models are downloaded only after explicit confirmation in **Models**, which
shows each model’s licence before download, or can be supplied from an existing
local directory through `config.json`. The default model roots are:

- Linux: `$XDG_DATA_HOME/fluent/models`, fallback
  `~/.local/share/fluent/models`;
- Windows: `%APPDATA%\fluent\Fluent\data\models`;
- macOS: `~/Library/Application Support/io.fluent.Fluent/models`.

See the [README](https://github.com/NeonMC23/Fluent/blob/main/README.md) and [MODEL_MANAGER.md](https://github.com/NeonMC23/Fluent/blob/main/docs/MODEL_MANAGER.md) for
installation, verification and storage guidance.

## Release artifacts

Built from CI run `43986327987` at artifact source revision `b57e6bc`. Later
release-preparation documentation and test-tooling commits do not change the
application binaries.

| File | Bytes | SHA-256 |
|---|---:|---|
| `Fluent-v0.2.0-linux-x86_64.tar.gz` | 9,731,954 | `ebfd0750a3c1bbb01d9829798c253ba9cf38e6096f8766c9d6f2e637d691fb28` |
| `Fluent-v0.2.0-windows-x86_64.zip` | 7,627,018 | `50e6564dde071f6c28bcd9e530638f9ceecb0460cb2796d18f551ac73080bcdc` |
| `Fluent-v0.2.0-macos-aarch64.zip` | 7,911,087 | `b1ef4acadb360b9f498b5b271ef15db47bf0a010cd311665d9682df1901f0fc4` |

These checksums and the matching `SHA256SUMS` refer to the exact files supplied
with this release.

## Verification

- Verify the directory with `sha256sum -c SHA256SUMS`.
- All three final files have detached Minisign signatures made locally with
  public key ID `B504F2189BB21013`; verify with the included `minisign.pub`.
- CI-produced artifacts are **unsigned** because `MINISIGN_SECRET_KEY` is not
  configured. The detached signatures in `release-v0.2.0/` were added after
  download.
- Windows and macOS archives are **not Authenticode-signed, Developer ID-signed
  or notarized**. Gatekeeper/SmartScreen may warn; do not claim OS-level
  signing or physical platform validation.

## Known limitations

- Windows and macOS physical runtime validation was not performed; the owner
  has chosen automated CI validation for these platforms.
- No Windows installer, macOS DMG, AppImage or Flatpak.
- No GPU-validated chat inference.
- No persistent chat history, tool calling, RAG or web access.
- NLLB-200 is non-commercial; Fluent's MIT source licence does not relicense
  model assets.
- Historical Git commits still contain NLLB tokenizer/configuration assets;
  branding redistribution rights require owner confirmation. See
  [MODEL_PROVENANCE.md](https://github.com/NeonMC23/Fluent/blob/main/docs/MODEL_PROVENANCE.md) and
  [RELEASE_INTEGRITY.md](https://github.com/NeonMC23/Fluent/blob/main/docs/RELEASE_INTEGRITY.md).

## Publication

Publication remains a separate owner-authorized action. Before publishing,
confirm branding redistribution rights, confirm that the intended distribution
model is compatible with NLLB-200’s non-commercial licence, decide the Windows
Authenticode and macOS Developer ID/notarization requirements, and authorize any
release tag or GitHub Release. Physical Windows/macOS testing is not a
prerequisite under the owner’s selected automated-CI policy.

The step-by-step owner procedure is in [PUBLISHING.md](https://github.com/NeonMC23/Fluent/blob/main/docs/PUBLISHING.md).
