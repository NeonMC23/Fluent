# Windows Build Guide

**Status: CI-built and smoke-tested; physical GUI validation NOT EXECUTED.**
There is no MSVC toolchain or Windows machine in this development environment
and no Windows installer. GitHub's `windows-latest` runner builds the release
EXE and executes the packaged `--version` and `--self-test`, but its hosted
runner cannot create an OpenGL context, so no window is rendered there. This
guide describes the build path and the path conventions the code uses. Do not
treat it as a physical-compatibility guarantee.

## Prerequisites

- Windows 10 or 11, 64-bit.
- Rust stable for the `x86_64-pc-windows-msvc` target (<https://rustup.rs>).
- Visual Studio 2022 Build Tools with the **Desktop development with C++**
  workload, including the Windows 10/11 SDK and MSVC linker.
- A GPU driver with OpenGL 3.3 support (the GUI uses `eframe` with `glow`).
- Enough RAM/disk for the model you intend to use.

CUDA is optional and requires a compatible NVIDIA driver plus the CUDA toolkit
and `nvcc`:

```powershell
cargo build --release -p fluent-gui --features cuda
```

## Build and run

From a Developer PowerShell (or any shell with MSVC on `PATH`):

```powershell
git clone https://github.com/NeonMC23/Fluent.git
cd Fluent
cargo build --release --locked -p fluent-gui
.\target\release\fluent.exe
```

No administrator privileges are needed to build or run from a user directory.

## Application icon

`crates/fluent-gui/build.rs` embeds
`src/logo/generated/windows/fluent.ico` through the `winres` build dependency on
Windows hosts. The ICO has 16, 24, 32, 48, 64, 128 and 256 px entries and is
independently validated by `scripts/verify-branding-assets.py`. Product name,
file description, original filename and numeric file/product versions are set.
The icon and metadata have **not** been inspected in Windows Explorer, and no
physical Windows runtime has been executed here. CI has built the EXE.

## Configuration and models

The application searches for `config.json` in this order:

1. `%FLUENT_CONFIG%`
2. the directory containing `fluent.exe`
3. its parent directory
4. `%APPDATA%\fluent\Fluent\config\config.json`
5. the current working directory

For a manual installation, either place `config.json` next to `fluent.exe` or
set `FLUENT_CONFIG` to an absolute path. The configuration directory used by
`directories` is:

```text
%APPDATA%\fluent\Fluent\config\config.json
%APPDATA%\fluent\Fluent\config\settings.json
```

The Windows per-user data directory is `%APPDATA%\fluent\Fluent\data`; the
default model root is `%APPDATA%\fluent\Fluent\data\models`. The portable ZIP
contains no `config.json`, so a fresh extraction uses that stable per-user
location rather than the process working directory. A populated `models/`
folder next to `fluent.exe` is detected and used without moving files. Use
**Models → Storage** and **Apply folder** to choose an explicit model root. If
a configuration is present, its model paths take precedence.

A minimal configuration with absolute paths:

```json
{
  "models": {
    "fast": "C:\\Users\\you\\AppData\\Roaming\\fluent\\Fluent\\data\\models\\nllb-200-1.3b",
    "accurate": "C:\\Users\\you\\AppData\\Roaming\\fluent\\Fluent\\data\\models\\nllb-200-3.3b"
  },
  "generation": { "max_length": 512, "num_beams": 4 }
}
```

Use doubled backslashes in JSON. Fluent bundles no models: the native Model
Manager downloads translation and chat models only after confirmation, stores
them outside the application payload, and verifies SHA-256 before reporting them
installed. With an installer-written configuration the default is the per-user
data directory; the portable ZIP uses the same per-user data directory unless
an existing model folder is detected or you choose another folder.

## CI, portable ZIP and validation status

The CI workflow defines a native `windows-latest` job that runs
`cargo test --workspace`, a release GUI build with the embedded icon/resource,
and `scripts/package-windows.py`. It uploads the
`fluent-<version>-windows-x86_64.zip` workflow artifact (14-day retention).
The packaged check now verifies PE32+ format and x86-64 architecture, the
non-empty `.rsrc` icon/version resource section, exact archive contents and
SHA-256 sidecar, product metadata, the associated icon, CLI help/version,
`--self-test`, an unrelated working directory, a Unicode/space-containing
extraction path, explicit Unicode config paths, default per-user model storage
and the absence of automatic model files. The GUI-window smoke is SKIPPED
because the hosted runner has no OpenGL 2.0+ context; this is not
physical-device GUI validation.

Windows PE archives are not byte-reproducible across rebuilds because the
linker embeds build-specific data; always verify the `.sha256` published with
the exact ZIP you downloaded.

**Physical runtime was not executed.** An existing local `win11` libvirt domain
cannot start because its attached `tiny11 23H2 x64.iso` lives on the
unavailable `/mnt/nas` mount, and no authorized physical Windows machine or
credentials were available. GUI startup, fonts, window sizing, Downloads,
model-file locking and real inference therefore remain unvalidated on Windows.

The ZIP contains `fluent.exe`, `LICENSE` and `README.txt`; models and user
configuration are excluded. There is no authenticated installer or Start Menu
integration.

## Manual physical validation procedure

Run the following on a normal Windows 10/11 x86-64 machine. It requires no
administrator privileges and does not download a model:

1. Download the `fluent-windows-x86_64` workflow artifact and extract the ZIP
   to a normal user folder (preferably a path containing a space and a
   non-ASCII character).
2. Verify the adjacent `.sha256` with:
   `Get-FileHash .\fluent-0.2.0-windows-x86_64.zip -Algorithm SHA256`.
3. Run `fluent.exe`. Confirm the Fluent name, icon, window and taskbar identity.
4. If SmartScreen appears, only continue for a verified artifact from a trusted
   source. Do not disable Windows security globally.
5. Run `fluent.exe --self-test` and confirm `SELF-TEST OK`, the default
   `%APPDATA%\fluent\Fluent\data\models` path and no model list entry claims
   an automatic download.
6. Open **Models**, confirm the storage path and free space, and cancel before
   downloading anything. Confirm no download starts on its own.
7. Close the window, relaunch `fluent.exe`, and confirm it starts without a
   model.

Record which steps executed. Steps not performed must be reported as
**NOT EXECUTED**, not inferred from CI.

## What is missing

- No `.exe` installer, Start Menu shortcut, or uninstaller is provided.
- No physical Windows runtime validation has been performed.
- The ZIP is unsigned (Authenticode is not configured). SmartScreen reputation
  is not claimed. Signing requires a real code-signing certificate, an
  RFC 3161 timestamp service and a Windows signing host; none is available, so
  no signing command is presented as active.
- The state of the MSVC runtime on end-user machines has not been audited.
- CUDA has not been built or run on Windows in this project.
- The Windows `install.sh` branch of the old installer was untested and has
  been replaced by an explicit unsupported message.

These are release blockers, not claims that the source is incompatible.
