# Windows Build Guide

**Status: not validated.** There is no MSVC toolchain or Windows machine in the
Cycle 6 environment, and no Windows installer. This guide describes the intended
build path and the path conventions the code uses. Do not treat it as a
compatibility guarantee.

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

Use doubled backslashes in JSON. Models are not downloaded or bundled.

## CI and validation status

The CI workflow defines a `windows-latest` job that runs
`cargo test --workspace` and a release GUI build. It has **not executed on
GitHub**, and no Windows GUI has been launched by this project. Compilation is
not runtime validation.

## What is missing

- No `.exe` installer, Start Menu shortcut, or uninstaller is provided.
- No Windows runtime validation has been performed.
- The state of the MSVC runtime on end-user machines has not been audited.
- CUDA has not been built or run on Windows in this project.
- The Windows `install.sh` branch of the old installer was untested and has
  been replaced by an explicit unsupported message.

These are release blockers, not claims that the source is incompatible.
