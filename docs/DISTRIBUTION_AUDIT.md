# Distribution Audit — Cycles 6–7

Status: **Linux source and portable archive validated against a glibc 2.35
baseline; NLLB assets removed from HEAD; Windows and macOS CI definitions exist
but have not run; public Git-history release still blocked by assets retained in
history.**

This document records what was verified, what changed in Cycle 6, and what
remains unknown. It is intentionally conservative: source code compiling on
Linux is not evidence that Windows or macOS binaries work.

## 1. Verified facts

| Finding | Evidence |
|---|---|
| The Rust GUI is independent of the Python/Flask components. | `crates/fluent-gui` and `crates/fluent-core` contain no references to `backend/`, `frontend/`, or `start.py`; the release GUI starts and runs without them. |
| The application needs no files from the source checkout at runtime. | The release binary reads `config.json` from the executable directory, its parent, the platform config directory, or the current directory. Models are ordinary files referenced by that config. |
| Model weights are not committed. | `models/*/pytorch_model*.bin` are 1-byte placeholders; the real weights used for testing live outside the repository. |
| NLLB tokenizer/model-config assets are **not** in the current tree. | They were removed in Cycle 7; `git ls-files models` now lists only `models/README.md`. Earlier commits still contain them. See `MODEL_PROVENANCE.md`. |
| Settings already follow platform conventions. | `directories::ProjectDirs::from("io", "fluent", "Fluent")` in `crates/fluent-gui/src/settings.rs`; Linux path verified as `~/.config/fluent/settings.json`. |
| The GUI can start without any model installed. | Installed release binary launched for 6 s under Wayland with a temp HOME and empty model directories; it stayed up and emitted no errors. |
| A missing model produces an actionable error. | `Error::ModelNotFound(path)` and `Error::MissingModelFile { dir, hint }` are surfaced in the GUI error banner. |
| CPU feature detection remains safe. | `quant.rs` selects AVX2/FMA through `is_x86_feature_detected!`; the scalar path is always compiled and was exercised in Cycle 5. |
| CUDA and Metal are optional. | Workspace/gui features default to empty; `engine.rs` compiles CPU fallbacks when the features are absent. |
| The legacy `install.sh` mixed application, configuration, models, and launcher in `~/.fluent`. | Audit of the previous script; replaced in Cycle 6. |
| Branding is integrated for the GUI header/window, Linux desktop, Windows resource and macOS bundle staging. | `src/logo/generated`, embedded header/window RGBA, installer default PNG, `build.rs`/`winres`, `scripts/package-macos.sh`; actual Linux GUI screenshot captured. Windows/macOS runtime still unvalidated. |

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
| Source build | Yes — host and Ubuntu 22.04 container (`cargo build --release -p fluent-gui`) | CI job defined; not executed | CI job defined; not executed |
| Unit/workspace tests | Yes | Not run | Not run |
| GUI launch | Yes — installed payload on host and Ubuntu 22.04 container under Wayland | Not run | Not run |
| Installer | Yes (temp HOMEs, spaces, uninstall) | Not provided | Best-effort script; **untested** |
| Desktop entry | Yes (`desktop-file-validate`) | N/A | N/A |
| Portable archive build | Yes — glibc 2.35 Ubuntu build, checksum verified, extracted and launched; baseline gate passes | N/A | N/A |
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
- **Implemented:** a portable application-only `tar.gz` via
  `scripts/package-linux.sh` (binary, `LICENSE`, runtime README; no models or
  configuration). The canonical build is produced in Ubuntu 22.04 and requires
  at most `GLIBC_2.35`, enforced by `scripts/check-linux-baseline.sh`. It has
  deterministic tar metadata, path remapping, a SHA-256 file, and validated
  extraction/launch plus Ubuntu-container GUI startup.
- **AppImage:** deferred. Requires `linuxdeploy`/`appimagetool`, additional
  validation, and a decision on how models and config are supplied.
- **Flatpak:** deferred. Requires a manifest, runtime selection, portal
  permissions, and a model-storage strategy outside the sandbox.
- No artifact was uploaded to a GitHub Release. Workflow artifact upload is defined but unexecuted; the local archive remains untracked under `dist/`.

## 6. Remaining blockers

1. **NLLB licensing in Git history** — the working tree no longer contains
   NLLB assets (Cycle 7), but earlier commits do. A public Git repository or
   full-history clone still redistributes them. A release decision is needed:
   rewrite history for a release mirror, or distribute the source archive from
   HEAD only. The model weights remain CC-BY-NC-4.0 and user-supplied.
2. **Windows runtime validation** — no MSVC toolchain or Windows machine.
3. **macOS build/bundle/signing** — no macOS machine; no `.app`, `.icns`,
   universal binary, codesigning, or notarization.
4. **Official icon/logo** — no brand asset exists; the `--icon` path is ready
   but intentionally unused.
5. **Model acquisition** — no downloader; see `MODEL_PROVENANCE.md` for the
   required consent/checksum workflow.
6. **Release signing/checksums** — the archive has a SHA-256 file, but no
   signature or trusted release channel yet.
7. **CI execution** — Linux/Windows/macOS jobs are defined but have not run on
   GitHub; only YAML syntax and local commands were validated here.

## 7. Unverified assumptions

- `eframe`/`glow` will build and run on a current Windows and macOS machine
  with the documented prerequisites.
- The Windows MSVC runtime is present on supported systems (or shipped by the
  user's environment).
- macOS Apple Silicon CPU inference is functional through the scalar Q8 path.
  It has not been executed on Apple hardware.
- The NLLB tokenizer/config files were covered by the CC-BY-NC-4.0 release
  terms. This was the most plausible reading of the model card but was not
  reviewed by counsel; the files are now out of the working tree regardless.
- The packaged source archive at HEAD is free of NLLB-derived assets; the Git
  history is not.
- The portable archive targets glibc 2.35 or newer; older distributions still
  need a different baseline. Linux distributions other than the build/runtime
  environments remain untested.
