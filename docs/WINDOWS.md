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
The icon and metadata have **not** been inspected in Windows Explorer, and a
Windows build has not been executed here.

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

The Windows data directory is `%APPDATA%\fluent\Fluent\data`; the documented
model location for a future installer is:

```text
%APPDATA%\fluent\Fluent\data\models\nllb-200-1.3b\
%APPDATA%\fluent\Fluent\data\models\nllb-200-3.3b\
```

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
them outside the application folder (by default under the per-user application
data directory) and verifies SHA-256 before reporting them installed.

## CI, portable ZIP and validation status

The CI workflow defines a native `windows-latest` job that runs
`cargo test --workspace`, a release GUI build with the embedded icon/resource,
and `scripts/package-windows.py`. It uploads the
`fluent-<version>-windows-x86_64.zip` workflow artifact (14-day retention).
It executed successfully in GitHub Actions run `43781201073` (commit
`27a4a3d`), uploaded the ZIP artifact, and ran the packaged `fluent.exe`
`--version` and `--self-test`. The GUI-window smoke is SKIPPED because the
hosted runner has no OpenGL 2.0+ context; this is not physical-device GUI
validation. The Cycle 19 inner ZIP is 7,618,508 B with SHA-256
`eafbc65ac0ae06133cecf3dd8550493298c405fc894e606194641e9a3be4b7fa`.

**Physical runtime was not executed.** An existing local `win11` libvirt domain
cannot start because its attached `tiny11 23H2 x64.iso` lives on the
unavailable `/mnt/nas` mount, and no authorized physical Windows machine or
credentials were available. GUI startup, fonts, window sizing, Downloads,
model-file locking and real inference therefore remain unvalidated on Windows.

The ZIP contains `fluent.exe`, `LICENSE` and `README.txt`; models and user
configuration are excluded. There is no authenticated installer or Start Menu
integration.

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
