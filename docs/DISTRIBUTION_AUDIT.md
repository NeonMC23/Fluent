# Distribution Audit — Cycle 6

Status: **Linux foundation implemented and validated; Windows and macOS
unvalidated; public distribution blocked by model-asset licensing.**

This document records what was verified, what changed in Cycle 6, and what
remains unknown. It is intentionally conservative: source code compiling on
Linux is not evidence that Windows or macOS binaries work.

## 1. Verified facts

| Finding | Evidence |
|---|---|
| The Rust GUI is independent of the Python/Flask components. | `crates/fluent-gui` and `crates/fluent-core` contain no references to `backend/`, `frontend/`, or `start.py`; the release GUI starts and runs without them. |
| The application needs no files from the source checkout at runtime. | The release binary reads `config.json` from the executable directory, its parent, the platform config directory, or the current directory. Models are ordinary files referenced by that config. |
| Model weights are not committed. | `models/*/pytorch_model*.bin` are 1-byte placeholders; the real weights used for testing live outside the repository. |
| Tokenizer and model-config assets **are** committed under `models/`. | `git ls-files models` lists tokenizer JSON/SentencePiece files and model cards. See `MODEL_PROVENANCE.md`. |
| Settings already follow platform conventions. | `directories::ProjectDirs::from("io", "fluent", "Fluent")` in `crates/fluent-gui/src/settings.rs`; Linux path verified as `~/.config/fluent/settings.json`. |
| The GUI can start without any model installed. | Installed release binary launched for 6 s under Wayland with a temp HOME and empty model directories; it stayed up and emitted no errors. |
| A missing model produces an actionable error. | `Error::ModelNotFound(path)` and `Error::MissingModelFile { dir, hint }` are surfaced in the GUI error banner. |
| CPU feature detection remains safe. | `quant.rs` selects AVX2/FMA through `is_x86_feature_detected!`; the scalar path is always compiled and was exercised in Cycle 5. |
| CUDA and Metal are optional. | Workspace/gui features default to empty; `engine.rs` compiles CPU fallbacks when the features are absent. |
| The legacy `install.sh` mixed application, configuration, models, and launcher in `~/.fluent`. | Audit of the previous script; replaced in this cycle. |

### Platform-sensitive code

- `memory_info()` reads `/proc/meminfo` only on Linux and returns empty values
  elsewhere (`crates/fluent-core/src/engine.rs`).
- Q8 SIMD kernels are x86-64 only; ARM and other CPUs use the portable scalar
  kernel.
- `settings_path()` uses `directories` and is portable.
- Path separators are handled by `PathBuf`; no hard-coded `/` in runtime code.
- The GUI uses `eframe` with `glow`; Linux-only Wayland/X11 features are
  enabled in the manifest but are conditionally compiled by `winit`.
- The old installer contained Windows/MSYS path handling; it was never
  validated and has been replaced by an explicit "not provided yet" message.

## 2. Distribution architecture

The installer now separates three ownership classes:

| Class | Linux | macOS | Windows (documented; no installer) |
|---|---|---|---|
| Application payload | `~/.local/lib/fluent/` | `~/Library/Application Support/io.fluent.Fluent/app/` | Manual build output |
| Launcher | `~/.local/bin/fluent` symlink | `~/.local/bin/fluent` symlink | Manual |
| Configuration | `${XDG_CONFIG_HOME:-~/.config}/fluent/config.json` | `~/Library/Application Support/io.fluent.Fluent/config.json` | `%APPDATA%\fluent\Fluent\config\config.json` |
| User settings | same directory, `settings.json` | same directory | same directory |
| Models | `${XDG_DATA_HOME:-~/.local/share}/fluent/models/` | `~/Library/Application Support/io.fluent.Fluent/models/` | `%APPDATA%\fluent\Fluent\data\models\` |
| Desktop entry | `${XDG_DATA_HOME:-~/.local/share}/applications/io.fluent.Fluent.desktop` | none (bundle work deferred) | none |
| Icon (optional) | `${XDG_DATA_HOME:-~/.local/share}/icons/hicolor/scalable/apps/io.fluent.Fluent.svg` | deferred | deferred |
| Install manifest | `${XDG_STATE_HOME:-~/.local/state}/fluent/install-manifest` | same XDG default | not used |

The payload contains only `fluent` and `LICENSE`. Configuration and models are
never stored inside the payload directory.

### Configuration search order

`ProjectConfig::load()` now searches:

1. `FLUENT_CONFIG` (explicit override),
2. `<executable directory>/config.json`,
3. `<executable parent>/config.json`,
4. the platform configuration directory (new in Cycle 6),
5. `<current directory>/config.json`.

This preserves portable/development runs while letting an installed
application read user-owned configuration from `~/.config/fluent`.

### Upgrade and uninstall behaviour

- Reinstalling replaces the binary and `LICENSE`, updates the launcher and
  desktop entry, and leaves `config.json` untouched unless `--reset-config` is
  passed.
- Existing valid model directories are never copied over, moved, or deleted.
- Uninstall removes only the binary, `LICENSE`, launcher symlink that points to
  the payload, a desktop entry carrying `X-Fluent-Installer=1`, an icon under
  the hicolor tree, and the install manifest. Configuration and models are
  always preserved.
- Old `~/.fluent` installations are left untouched. Their model directories are
  detected as an additional read-only source when the new model directory is
  empty, so upgrades can hard-link or copy existing models.

## 3. Installer audit (old → new)

| Concern | Old behaviour | New behaviour |
|---|---|---|
| Layout | Everything under `~/.fluent`, including config and models | Payload in `~/.local/lib/fluent`, config in XDG config, models in XDG data |
| Configuration | Overwritten on every install | Preserved by default; `--reset-config` is explicit |
| Models | Copied into install dir; placeholder files accepted | Validated checkpoint required; placeholders rejected; legacy source supported |
| Uninstall | None | `--uninstall` / `uninstall.sh`, manifest-based, preserves user data |
| Paths with spaces | Partly quoted | Quoted throughout; tested with spaces in HOME and model paths |
| Desktop entry | None | `io.fluent.Fluent.desktop`, validated when `desktop-file-validate` exists |
| Icon | None | Optional `--icon` SVG; omitted cleanly when absent |
| Root privileges | Not required | Not required; no `sudo` |
| Package installation | None | None; missing tools are reported |
| Windows | Untested MSYS path | Explicit unsupported message; manual guide |
| Repeated installs | Mostly safe | Tested; config/models preserved, launcher conflict detected |

## 4. Validation matrix

| Area | Linux | Windows | macOS |
|---|---|---|---|
| Source build | Yes — `cargo build --release -p fluent-gui` | Not available (no MSVC/target) | Not available (no target) |
| Unit/workspace tests | Yes | Not run | Not run |
| GUI launch | Yes (Wayland, installed payload) | Not run | Not run |
| Installer | Yes (temp HOMEs, spaces, uninstall) | Not provided | Best-effort script; **untested** |
| Desktop entry | Yes (`desktop-file-validate`) | N/A | N/A |
| Models/config separation | Yes | Documented only | Documented only |
| CUDA | Feature compiles? Not built this cycle | Not run | N/A |
| Metal | N/A | N/A | Not run |

Cross-target checks failed because the host has only the
`x86_64-unknown-linux-gnu` standard library:

```text
cargo check -p fluent-core --target x86_64-pc-windows-gnu
error[E0463]: can't find crate for `core`
cargo check -p fluent-core --target aarch64-apple-darwin
error[E0463]: can't find crate for `core`
```

`rustup` is not installed on this host, so no target could be added without
installing a new toolchain; that was out of scope.

## 5. Packaging decision

- **Chosen now:** per-user source build + installer + desktop integration.
  This is validated and does not require root.
- **Next format:** a portable `tar.gz` containing the release binary, `LICENSE`,
  a configuration template, and a short README. It is straightforward and can
  be tested locally without FUSE or sandbox runtimes. Not implemented this
  cycle.
- **AppImage:** deferred. Requires `linuxdeploy`/`appimagetool`, additional
  validation, and a decision on how models and config are supplied.
- **Flatpak:** deferred. Requires a manifest, runtime selection, portal
  permissions, and a model-storage strategy outside the sandbox.
- No artifact was produced, uploaded, or published in this cycle.

## 6. Remaining blockers

1. **Model and tokenizer licensing** — NLLB-200 assets are CC-BY-NC-4.0. The
   repository currently commits tokenizer/model-card files; public
   redistribution needs legal review or removal with a working alternative.
2. **Windows runtime validation** — no MSVC toolchain or Windows machine.
3. **macOS build/bundle/signing** — no macOS machine; no `.app`, `.icns`,
   universal binary, codesigning, or notarization.
4. **Official icon/logo** — no brand asset exists; the `--icon` path is ready
   but intentionally unused.
5. **Model acquisition** — no downloader; see `MODEL_PROVENANCE.md` for the
   required consent/checksum workflow.
6. **Installer signing/checksums** — source installs do not need them, but
   packaged artifacts will.
7. **CI execution** — a workflow is added, but it cannot run locally; only
   syntax was validated here.

## 7. Unverified assumptions

- `eframe`/`glow` will build and run on a current Windows and macOS machine
  with the documented prerequisites.
- The Windows MSVC runtime is present on supported systems (or shipped by the
  user's environment).
- macOS Apple Silicon CPU inference is functional through the scalar Q8 path.
  It has not been executed on Apple hardware.
- The tokenizer assets are covered by the NLLB CC-BY-NC-4.0 release terms. This
  is the most plausible reading of the model card but has not been reviewed by
  counsel.
