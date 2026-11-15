# Fluent Branding Assets

This document records the official Fluent branding sources, derived platform
icons, validation rules and remaining public-distribution caveats.

## 1. Source masters and the canonical symbol

The two source files were copied byte-for-byte from the original local project
with explicit authorization. They are masters: **never re-encode, overwrite,
crop, vectorize or edit them in place.**

The canonical application symbol is the **turquoise stylized F mark**. It is
extracted from `logo.png` with the same `(270, 190, 745, 665)` crop the GUI
header has always used and committed as `src/logo/fluent-mark.png` (475×475
RGBA). Every application and platform icon derives from that mark. The full
`logo.png` and `banner.png` remain unchanged as the original brand artwork.

| File | Dimensions | Mode | Alpha | SHA-256 | Size |
|---|---|---|---|---|---|
| `src/logo/logo.png` | 1024×1024 | RGBA / sRGB | Yes; 0–255, non-opaque corners | `bce82ebd549e4e6a0001a605a7bb8ff3efc8bfedf30fc20c3803801db884b5e7` | 1,410,578 B |
| `src/logo/banner.png` | 1536×1024 | RGBA / sRGB | Yes; non-opaque corners | `ce3deeb9bab7a9cf847ba06568886e8cb5b10d88080bc1c55363300f35925545` | 876,488 B |
| `src/logo/fluent-mark.png` | 475×475 | RGBA / sRGB | Yes; non-opaque corners | `6ac61dc6b89694f3a1d6aa900b2b6604c100464c79e352e43e5de5b0c5e24df4` | generated from `logo.png` |

Visual notes:

- The full logo master contains the Fluent mark plus the “Fluent” wordmark on a
  soft dark gradient, with transparent outer padding. The non-transparent
  bounding box is approximately `(280,201)–(735,808)`.
- The mark crop `(270,190,745,665)` contains only the turquoise F. It is the
  authoritative application symbol and is never redrawn.
- The banner is landscape and carries the same mark/wordmark. It is used in the
  README only; it is not an application icon and is not embedded in the binary.
  It is documented here because it is the one remaining wordmark composition.
- Neither source is a vector. No automatic vectorization was attempted.
- The mark is the only artwork used for window, taskbar, launcher, executable,
  bundle and dock icons.

License/provenance: these are **project-provided branding assets**. Their
authorship/license is not documented in the original project. Do not assume the
repository's MIT source license covers them; confirm redistribution rights
before public distribution.

## 2. Generated assets

Generated files live under `src/logo/generated/` and are committed so normal
builds and installers need no image tooling.

| Asset | Format | Dimensions / entries | Purpose | Validation |
|---|---|---|---|---|
| `png/fluent-16.png` … `fluent-512.png` | PNG, RGBA | 16, 24, 32, 48, 64, 128, 256, 512 px | Linux/source variants, documentation, ICO/ICNS inputs | All derive from the mark; dimensions checked |
| `window/fluent-256.rgba` | raw RGBA8 | 256×256, 262,144 B | Compile-time embedded window icon | Byte-identical to the header mark; used by `main.rs` |
| `ui/fluent-mark-256.png` | PNG RGBA | 256×256 | Documentation/preview of the header mark | IHDR checked |
| `ui/fluent-mark-256.rgba` | raw RGBA8 | 256×256, 262,144 B | Compile-time embedded header mark | Exact length checked; used by `app.rs` |
| `windows/fluent.ico` | ICO | 7 entries: 16, 24, 32, 48, 64, 128, 256; 32-bit | Windows executable resource | Generated from the mark; directory/BMP payloads parsed independently |
| `macos/Fluent.icns` | ICNS | 11 PNG chunks: 16, 32, 64, 128, 256, 512, 1024 plus 2× variants | macOS bundle icon | Generated from the mark; header/chunk lengths and PNG dimensions parsed independently |

Key hashes:

```text
fluent-mark.png    6ac61dc6b89694f3a1d6aa900b2b6604c100464c79e352e43e5de5b0c5e24df4
fluent.ico         bd50855683fd9eaba549f058fa8de0315abeb987474091b652db67df5b8e2c6f
Fluent.icns        b1c5163c0eed7132987accc35c3fb950813926adfb541de5878bcfc33083ce44
fluent-256.png     061a732acbec76cf91c9108ffec65383bd44d7a6919c13599e2851a016759e60
fluent-512.png     a4b830d076bf8b27648ef11fc44fae4d445ee842ed9ac2470041367c1590b521
fluent-256.rgba    22c92da9c4d9eefaa1524300b7f3e961a1332f21bb5b6f0ce6dc67c6e5b45ac3
fluent-mark.rgba   22c92da9c4d9eefaa1524300b7f3e961a1332f21bb5b6f0ce6dc67c6e5b45ac3
```

## 3. Generation and validation

Regenerate (development only; requires Pillow 12.3.0 / Python 3.14 here):

```bash
python3 scripts/generate-branding-assets.py
```

The script:

1. verifies the master is 1024×1024 RGBA;
2. extracts the canonical mark with the fixed crop and writes
   `src/logo/fluent-mark.png`;
3. resizes the **mark** with Lanczos to the listed sizes;
4. writes optimized PNGs;
5. writes the raw 256 RGBA window payload and the matching header mark;
6. writes a 7-entry ICO with 32-bit BMP entries from the mark;
7. writes an 11-chunk ICNS containing PNG payloads from the mark.

Validate structurally without Pillow (standard library only):

```bash
python3 scripts/verify-branding-assets.py
```

It checks the master/banner/mark hashes and dimensions, every PNG size, ICO
directory/payload dimensions, ICNS header/chunks/PNG dimensions, the raw RGBA
lengths, and that the window icon and header mark are byte-identical.

Verify reproducibility (requires Pillow):

```bash
python3 scripts/generate-branding-assets.py --check
```

It regenerates all 14 outputs into a temporary directory and compares them
byte-for-byte with the committed files. The same master, Pillow version and
script produce the same files here. The CI structural check does not require
Pillow.

## 4. Application and platform integration

| Target | Integration | Status |
|---|---|---|
| GUI window icon | `include_bytes!("…/window/fluent-256.rgba")` in `crates/fluent-gui/src/main.rs`; now the same mark as the header, with no runtime file lookup or image decoder | Implemented and verified byte-identical to the header mark |
| GUI header | Mark-only `ui/fluent-mark-256.rgba` loaded once as an egui texture in `app.rs`; the “Fluent” wordmark remains text | Implemented and visible in a real app screenshot (`src/image1.png`) |
| Linux desktop entry | Installer copies the bundled `fluent-256.png` (the mark) to `hicolor/256x256/apps/io.fluent.Fluent.png`; explicit SVG/PNG `--icon` still supported | Implemented and tested |
| Linux portable archive | Window icon embedded in the binary; no external icon files needed for running | Implemented |
| Windows executable | `build.rs` uses `winres` to embed the mark-based `windows/fluent.ico`, ProductName, FileDescription, OriginalFilename and numeric file/product version | CI-compiled; physical Windows runtime not executed |
| macOS bundle | `scripts/package-macos.sh` stages `Fluent.app` with the mark-based `Fluent.icns` and `Info.plist`; no signing/notarization | CI-built; physical macOS runtime not executed |
| README | Banner referenced from `src/logo/banner.png`; it is the only wordmark composition and is not used as an icon | Implemented and documented |
| In-app banner | Deliberately not added; it would consume permanent UI space and the screenshots already show the interface | Not used |

The Windows icon embedding is only exercised when the build script runs on a
Windows host. Cross-compiling from Linux would need additional resource-tool
setup and is not claimed.

## 5. Interface screenshots

Captured from the actual running application on the canonical Linux host:

| View | Path |
|---|---|
| Main interface, dark theme | `src/image1.png` |
| Settings page, dark theme, output limit | `docs/screenshots/settings.png` |
| Settings page, light theme | `docs/screenshots/settings-light.png` |
| Settings page, MADLAD selected | `docs/screenshots/settings-madlad.png` |
| MADLAD CPU/greedy/output controls | `docs/screenshots/settings-madlad-output.png` |
| MADLAD filtered target-language picker | `docs/screenshots/language-picker-madlad.png` |
| Main interface with SMaLL-100 selected and Translate/Chat navigation | `docs/screenshots/main-small.png` |
| Chat empty state and model status | `docs/screenshots/chat.png` |
| Chat conversation with a French follow-up | `docs/screenshots/chat-conversation.png` |
| Chat model picker | `docs/screenshots/chat-model-picker.png` |
| Model Manager chat models | `docs/screenshots/model-manager-chat.png` |
| Chat model download progress with speed and ETA | `docs/screenshots/chat-download-progress.png` |
| Downloaded chat model installed in a custom storage root | `docs/screenshots/chat-download.png` |
| Model selector with explicit model identities | `docs/screenshots/model-selector.png` |
| Language picker filtered to SMaLL-100 (Simplified Chinese only) | `docs/screenshots/language-picker-small.png` |
| Language picker for NLLB-200 (Simplified and Traditional Chinese) | `docs/screenshots/language-picker-nllb.png` |
| Model-switch language adjustment notice | `docs/screenshots/language-adjusted.png` |
| Model Manager, storage, catalogue and four models | `docs/screenshots/model-manager.png` |
| Model Manager, SMaLL-100 installed and verified | `docs/screenshots/model-manager-small.png` |
| Model Manager, download confirmation and licence | `docs/screenshots/model-manager-confirm.png` |
| Model Manager, live download progress, speed, ETA and cancel | `docs/screenshots/model-manager-download.png` |
| Model Manager, cancelled download shown as Incomplete | `docs/screenshots/model-manager-cancelled.png` |
| Language picker | `docs/screenshots/language-picker.png` |
| Main interface, light theme | `docs/screenshots/main-light.png` |

The settings screenshots use a disposable container home directory so no user
paths or private text are exposed; they show the four-model selector, MADLAD
runtime capabilities and the Output limit control. The main preview was captured with neutral sample
text. These are documentation captures, not mockups.

## 6. Platform output requirements

For a future release:

- **Linux:** 256/512 PNGs (present) or the ICNS/ICO not used; desktop entry
  `Icon=io.fluent.Fluent`.
- **Windows:** multi-size ICO (present) embedded by the build; verify Explorer
  rendering on a real Windows machine.
- **macOS:** ICNS (present), `Info.plist` with `CFBundleIconFile=Fluent.icns`,
  a proper `.app`, and signing/notarization on Apple hardware.
- **Store/social:** banner (present), README image; no icon derived from the
  banner.

## 6. Public-distribution caveat

The logo/banner are project-provided but their license is not documented here.
Before publishing binaries or the repository, confirm the branding rights in
addition to the unresolved NLLB model/tokenizer question. No legal clearance is
claimed.
