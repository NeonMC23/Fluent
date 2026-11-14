# Linux Installation

Linux is the validated distribution target for Cycle 6. The installer is a
per-user installation: it does not need root, does not install system packages,
and never downloads models.

## Requirements

Building from source:

- Rust 1.85 or newer and Cargo.
- `bash` (the installer uses bash, not POSIX sh).
- A C/C++ toolchain, `pkg-config` and `cmake`.
- X11 or Wayland development headers and an OpenGL-capable driver.

Fedora / Nobara:

```bash
sudo dnf install gcc gcc-c++ make cmake pkgconf-pkg-config \
    libxkbcommon-devel wayland-devel mesa-libGL-devel fontconfig-devel \
    libX11-devel libXcursor-devel libXi-devel libXrandr-devel
```

Debian / Ubuntu:

```bash
sudo apt install build-essential pkg-config cmake \
    libxkbcommon-dev libwayland-dev libgl1-mesa-dev libegl1-mesa-dev \
    libfontconfig1-dev libx11-dev libxcursor-dev libxi-dev libxrandr-dev
```

The installer itself does not install these packages and will stop with an
error if `cargo` is missing. The GUI needs a graphical session with OpenGL 3.3+
(software rendering may be very slow).

## Install

```bash
git clone https://github.com/NeonMC23/Fluent.git
cd Fluent
./install.sh
```

The default layout is:

```text
~/.local/lib/fluent/fluent                 application binary
~/.local/lib/fluent/LICENSE
~/.local/bin/fluent                        launcher symlink
~/.config/fluent/config.json               configuration (user-owned)
~/.config/fluent/settings.json             preferences (created by the GUI)
~/.local/share/fluent/models/              model files (user-owned)
~/.local/share/applications/io.fluent.Fluent.desktop
~/.local/share/icons/hicolor/scalable/apps/io.fluent.Fluent.svg   (optional)
~/.local/state/fluent/install-manifest     used by uninstall
```

`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME` and `HOME` are respected.
The binary can be launched directly or from the desktop menu.

## Installer options

| Option | Purpose |
|---|---|
| `--cuda` | Build with NVIDIA CUDA support (CUDA toolkit required). |
| `--metal` | Rejected on Linux. |
| `--binary FILE` | Install a prebuilt binary instead of building. |
| `--skip-build` | Use `target/release/fluent` without running Cargo. |
| `--models-src DIR` | Directory containing `nllb-200-1.3b/` and `nllb-200-3.3b/`. Default: `./models`, which is empty in a fresh checkout. A legacy `~/.fluent/models` is also checked. |
| `--skip-models` | Create empty model directories only. |
| `--icon FILE` | Install an SVG icon and reference it from the desktop entry. |
| `--no-desktop` | Do not create a desktop entry or icon. |
| `--reset-config` | Regenerate `config.json` (existing configuration is otherwise preserved). |
| `--uninstall` | Remove application files; configuration and models are preserved. |
| `--app-dir`, `--bin-dir`, `--config-dir`, `--models-dir` | Override individual locations. `--install-dir` is a legacy alias for `--app-dir`. |

Environment equivalents: `FLUENT_APP_DIR`, `FLUENT_BIN_DIR`,
`FLUENT_CONFIG_DIR`, `FLUENT_MODELS_DIR`, `FLUENT_MODELS_SRC`,
`FLUENT_INSTALL_DIR`.

## Models

Fluent does not download or bundle NLLB, and this repository commits no NLLB
assets. Put the complete checkpoint, configuration, and tokenizer files in:

```text
~/.local/share/fluent/models/nllb-200-1.3b/
~/.local/share/fluent/models/nllb-200-3.3b/
```

Each directory needs `config.json`, `tokenizer.json`, and a real checkpoint
(`pytorch_model.bin`, `model.safetensors`, or complete shards plus an index).
The installer rejects placeholder or incomplete directories and never
overwrites a working model. See [MODEL_PACKAGE.md](MODEL_PACKAGE.md) for the exact contract.

If you already have a `~/.fluent/models` directory from the previous
single-directory installer, the new installer uses it as a read-only source
when the new model directory is empty. It hard-links when possible and copies
otherwise. The old directory is not modified or deleted; remove it manually
once you have verified the new installation.

Models are subject to the NLLB license. See [MODEL_PROVENANCE.md](MODEL_PROVENANCE.md).

## Portable archive

A reproducible application-only archive can be produced locally:

```bash
./scripts/package-linux.sh
```

The output is `dist/fluent-<version>-linux-<arch>.tar.gz` plus a `.sha256`
file. It contains only:

```text
fluent-<version>-linux-<arch>/
├── bin/fluent
├── LICENSE
└── README.txt
```

Models, tokenizer files, configuration and user data are **not** included. The
archive README explains how to place models next to it. The tar metadata is
normalised (sorted entries, fixed timestamp, numeric owner) and the packaging
build remaps Cargo/repository paths out of the binary. Re-running the script
with the same binary produces the same SHA-256. The Rust binary itself is not
guaranteed bit-reproducible across hosts or toolchains.

Verify an archive from the directory containing it:

```bash
sha256sum -c fluent-<version>-linux-<arch>.tar.gz.sha256
```

A checksum proves integrity relative to the channel that supplied it; it is not
a signature.

### glibc compatibility baseline

The canonical Cycle 8 archive is built in an **Ubuntu 22.04** container and its
strongest symbol requirement is **`GLIBC_2.35`**. The gate is:

```bash
./scripts/check-linux-baseline.sh target/release/fluent 2.35
```

The binary was also launched successfully inside the Ubuntu 22.04 container
under Wayland (software GL) with no model installed: it stayed up for the
6-second timeout and emitted no errors. This is a real runtime check on the
glibc 2.35 baseline, not only a compile check.

The previous host-built archive (glibc 2.43) is preserved locally under
`dist/legacy-glibc243/`; it is not the canonical artifact and should not be
distributed as the compatible build. The current host can also build a binary,
but it will inherit the host's glibc; use the Ubuntu 22.04 procedure for
releases.

The binary is a dynamically linked x86-64 PIE with no GLIBCXX/CXXABI
requirements. X11/Wayland, OpenGL and font libraries are loaded at runtime by
`winit`/`glutin`; they do not all appear in `ldd`, so the graphical session
still needs the runtime packages listed above. There is no RPATH/RUNPATH
pointing into the build tree.

The Ubuntu build is currently a local container procedure, not an automated
release pipeline. CI now runs on `ubuntu-22.04` and enforces the same baseline
check, but GitHub Actions has not executed yet.

CPU dispatch is unchanged: the binary contains AVX2/FMA kernels selected by
runtime detection, with a scalar fallback. It is not compiled with
`target-cpu=native`; installing it on a CPU without AVX2 is intended to work
through the scalar path and was covered by the scalar test suite.

This archive has only been executed on the host described in the validation
section. Other distributions and older glibc versions remain untested.

## Launch and desktop integration

```bash
~/.local/bin/fluent
```

If `~/.local/bin` is not on your `PATH`, add it in your shell configuration.
The installer writes `io.fluent.Fluent.desktop` with an absolute `Exec` path
(quoted correctly, including spaces) and `StartupWMClass=io.fluent.Fluent`.
If `desktop-file-validate`, `update-desktop-database` or
`gtk-update-icon-cache` are available, the installer uses them; otherwise it
continues without failing.

## Uninstall

```bash
./uninstall.sh
# or: ./install.sh --uninstall
```

Uninstall removes the binary, launcher symlink (only if it points to this
installation), the desktop entry marked `X-Fluent-Installer=1`, an icon installed
by the installer, and the manifest. It **always preserves** `config.json`,
`settings.json`, and the model directory. Delete those directories yourself only
when you no longer need them.

## Migration from the old `~/.fluent` layout

The previous installer placed everything in `~/.fluent`. Cycle 6 does not delete
that directory. To migrate:

1. Run the new installer. Existing models are found automatically.
2. Launch the new installation and translate a sentence.
3. If everything works, remove `~/.fluent` manually.

Custom settings were already stored in `~/.config/fluent/settings.json`, so they
carry over. If you had edited `~/.fluent/config.json`, compare it with the new
`~/.config/fluent/config.json` and copy any non-model preferences.

## Troubleshooting

| Symptom | Check |
|---|---|
| `cargo was not found` | Install Rust from <https://rustup.rs> or your distribution, then reopen the shell. |
| Native build failure | Install the development packages listed above. |
| `binary not found` with `--skip-build` | Build first: `cargo build --release -p fluent-gui`. |
| `already exists and is not Fluent's launcher` | Another `~/.local/bin/fluent` exists. Move it away or use `--bin-dir`. |
| `cannot create ... check ownership and permissions` | The target directory is not writable by your user. |
| Model not found in the GUI | Confirm the path shown in the error and that `config.json` points to the model directory. |
| Desktop entry does not appear | Log out/in or run `update-desktop-database ~/.local/share/applications`. |
| Wayland/X11 startup failure | Ensure an OpenGL driver is installed; try an X11 session to isolate compositor issues. |

## Validation performed in Cycle 6

The reproducible test script is `scripts/test-installer.sh`; it uses only
temporary HOMEs, a fake executable and a fake 2 MB checkpoint. None of the
tests touched the real user environment.

- Fresh install with spaces in `HOME` and model source paths: passed.
- Repeat install: binary updated, configuration and model files preserved.
- Uninstall: app files, launcher, desktop entry and icon removed; config and
  models preserved.
- `desktop-file-validate`: passed (one hint about multiple main categories was
  fixed).
- Missing binary, missing icon, launcher conflict, read-only HOME, missing
  uninstall manifest: each produced a clear non-zero error.
- Placeholder `models/` content: rejected with a warning instead of installing
  broken files.
- Legacy `~/.fluent/models` migration: model hard-linked, old directory intact.
- `--skip-models`, `--reset-config`, `--no-desktop`, no-icon desktop entry:
  passed.
- Installed release binary launched under Wayland for 6 s with no model and
  with the config file removed: stayed up, no error output.
- `scripts/test-installer.sh`: passes all fresh/repeat/uninstall, missing
  binary, `--no-desktop`, `--skip-models` and placeholder-model checks.
- `scripts/package-linux.sh`: archive created, contents inspected, SHA-256
  verified, extraction and GUI launch tested from a directory containing
  spaces. No model or tokenizer asset was present. Runtime dependency/glibc
  inspection performed with `file`, `readelf`, `ldd` and `objdump`.
- Ubuntu 22.04 container build: complete GUI built against glibc 2.35; maximum
  required symbol `GLIBC_2.35`; GUI launched under Wayland in the container.
  `scripts/check-linux-baseline.sh` passes for that binary and fails for the
  old glibc 2.43 host build, as intended.
- Portable archive rebuilt from the Ubuntu binary; content and checksum
  verified, extraction into a path with spaces, GUI launch on the host.

Not validated: other Linux distributions, non-glibc systems, ARM Linux, distro
packaging, AppImage/Flatpak, and desktop environments other than the KDE/Wayland
session used here. A binary built on this machine is not guaranteed to run on
older glibc systems; building from source is the supported path.
