# Fluent Branding Assets

This document records the official Fluent branding sources, derived platform
icons, validation rules and remaining public-distribution caveats.

## 1. Source masters (immutable)

The two source files were copied byte-for-byte from the original local project
with explicit authorization. They are masters: **never re-encode, overwrite,
crop, vectorize or edit them in place.**

| File | Dimensions | Mode | Alpha | SHA-256 | Size |
|---|---|---|---|---|---|
| `src/logo/logo.png` | 1024×1024 | RGBA / sRGB | Yes; 0–255, non-opaque corners | `bce82ebd549e4e6a0001a605a7bb8ff3efc8bfedf30fc20c3803801db884b5e7` | 1,410,578 B |
| `src/logo/banner.png` | 1536×1024 | RGBA / sRGB | Yes; non-opaque corners | `ce3deeb9bab7a9cf847ba06568886e8cb5b10d88080bc1c55363300f35925545` | 876,488 B |

Visual notes:

- The logo contains the Fluent mark plus the “Fluent” wordmark on a soft dark
  gradient, with transparent outer padding. The non-transparent bounding box is
  approximately `(280,201)–(735,808)` within the 1024 canvas.
- Because the wordmark is part of the master, it becomes unreadable below
  roughly 32 px; at 16–24 px only the mark is distinguishable. The original was
  preserved unchanged; no crop or redesign was applied. This is a documented
  limitation, not a hidden asset substitution.
- The banner is landscape and carries the mark/wordmark on a dark transparent
  background. It is used in the README only; it is not an icon and is not
  embedded in the application.
- Neither source is a vector. No automatic vectorization was attempted.

License/provenance: these are **project-provided branding assets**. Their
authorship/license is not documented in the original project. Do not assume the
repository's MIT source license covers them; confirm redistribution rights
before public distribution.

## 2. Generated assets

Generated files live under `src/logo/generated/` and are committed so normal
builds and installers need no image tooling.

| Asset | Format | Dimensions / entries | Purpose | Validation |
|---|---|---|---|---|
| `png/fluent-16.png` … `fluent-512.png` | PNG, RGBA | 16, 24, 32, 48, 64, 128, 256, 512 px | Linux/source variants, documentation, ICO/ICNS inputs | IHDR dimensions checked |
| `window/fluent-256.rgba` | raw RGBA8 | 256×256, 262,144 B | Compile-time embedded window icon | Exact length checked; used by `main.rs` |
| `ui/fluent-mark-256.png` | PNG RGBA | 256×256 | Documentation/preview of the header mark | IHDR checked |
| `ui/fluent-mark-256.rgba` | raw RGBA8 | 256×256, 262,144 B | Compile-time embedded header mark | Exact length checked; used by `app.rs` |
| `windows/fluent.ico` | ICO | 7 entries: 16, 24, 32, 48, 64, 128, 256; 32-bit | Windows executable resource | Directory/BMP payloads parsed independently |
| `macos/Fluent.icns` | ICNS | 11 PNG chunks: 16, 32, 64, 128, 256, 512, 1024 plus 2× variants | macOS bundle icon | Header/chunk lengths and PNG dimensions parsed independently |

Key hashes:

```text
fluent.ico         d5459a9fc241815c2833bc1c325af12c4583ba11cf648d1fa171e6a6d0f03966
Fluent.icns        8016e6005b889bd463bb30c4afb4d3ff01a758a11a71b0ba5fc1965636150107
fluent-256.png     8b4212285d6d33cc786cc54d0dc0715da4ce9127757d946e2a23daca848c0de9
fluent-512.png     ca62ea2c784fd58cc32a5bf44e655c31a9ecaaee6028262fe0402fc00d7f4d4e
fluent-256.rgba    6eb9909db6a7b702a862731de84f5551e957a0c08cb3f31346e4139ad779d7a2
fluent-mark.png    061a732acbec76cf91c9108ffec65383bd44d7a6919c13599e2851a016759e60
fluent-mark.rgba   22c92da9c4d9eefaa1524300b7f3e961a1332f21bb5b6f0ce6dc67c6e5b45ac3
```

## 3. Generation and validation

Regenerate (development only; requires Pillow 12.3.0 / Python 3.14 here):

```bash
python3 scripts/generate-branding-assets.py
```

The script:

1. verifies the master is 1024×1024 RGBA;
2. resizes with Lanczos to the listed sizes, preserving aspect ratio and alpha;
3. writes optimized PNGs;
4. writes the raw 256 RGBA payload;
5. writes a 7-entry ICO with 32-bit BMP entries;
6. writes an 11-chunk ICNS containing PNG payloads.

Validate without Pillow (standard library only):

```bash
python3 scripts/verify-branding-assets.py
```

It checks master hashes/dimensions, every PNG size, ICO directory/payload
dimensions, ICNS header/chunks/PNG dimensions, and the raw RGBA length. The CI
workflow executed this check successfully in run 43605074905.

Reproducibility: the same masters, Pillow version and script produce the same
files here. Different Pillow/zlib versions may choose different PNG compression
bytes. The committed assets are authoritative for builds; regeneration is only
needed when the master changes.

## 4. Application and platform integration

| Target | Integration | Status |
|---|---|---|
| GUI window icon | `include_bytes!("…/window/fluent-256.rgba")` in `crates/fluent-gui/src/main.rs`; no runtime file lookup or image decoder | Implemented |
| GUI header | Mark-only `ui/fluent-mark-256.rgba` loaded once as an egui texture in `app.rs`; the “Fluent” wordmark remains text | Implemented and visible in a real app screenshot (`src/image1.png`) |
| Linux desktop entry | Installer copies the bundled `fluent-256.png` to `hicolor/256x256/apps/io.fluent.Fluent.png`; explicit SVG/PNG `--icon` still supported | Implemented and tested |
| Linux portable archive | Window icon embedded in the binary; no external icon files needed for running | Implemented |
| Windows executable | `build.rs` uses `winres` to embed `windows/fluent.ico`, ProductName, FileDescription, OriginalFilename and numeric file/product version | Prepared; no Windows build executed |
| macOS bundle | `scripts/package-macos.sh` stages `Fluent.app` with `Fluent.icns` and `Info.plist`; no signing/notarization | Prepared; no macOS build executed |
| README | Banner referenced from `src/logo/banner.png` | Implemented |
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
| Language picker | `docs/screenshots/language-picker.png` |
| Main interface, light theme | `docs/screenshots/main-light.png` |

The settings screenshots use a disposable container home directory so no user
paths or private text are exposed; they show the newest Output limit control. The main preview was captured with neutral sample
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
