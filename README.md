# Fluent

<div align="center">

### Private AI Translation · 190+ Languages · Local First

Translate text locally with Meta's NLLB-200 models and a modern native interface.

</div>

---

## Overview

Fluent is a local AI translation application built in Rust. It runs NLLB-200
directly on your machine, so your text never leaves your computer: there is no
cloud API, no account and no background server.

The desktop interface is written with [egui](https://github.com/emilk/egui) /
eframe and works on Linux, Windows 10/11 and macOS.

Key points:

- 190+ languages and dialects
- Automatic language detection
- Fast and Accurate translation modes
- Native, responsive interface with light, dark and system themes
- Custom accent colors
- GPU acceleration (CUDA or Metal) when available
- CPU fallback with no external runtime dependencies

---

## Quick install

Build and install Fluent into a self-contained folder under your home directory:

```bash
chmod +x install.sh
./install.sh
```

Requirements: Rust/cargo and a C toolchain (see
[Building from source](#building-from-source)). No Python or web server is
needed for the installed application.

The installer detects the current platform:

| Platform | Installation directory | Executable |
| --- | --- | --- |
| Linux (Nobara/Fedora, etc.) | `~/.fluent` | `~/.fluent/bin/fluent` |
| macOS | `~/.fluent` | `~/.fluent/bin/fluent` |
| Windows (Git Bash/MSYS2) | `%USERPROFILE%\.fluent` | `%USERPROFILE%\.fluent\bin\fluent.exe` |

It performs the following steps:

- builds `fluent-gui` in release mode;
- installs the binary into `<install-dir>/bin/`;
- writes `<install-dir>/config.json` with model paths relative to the install folder;
- creates `<install-dir>/models/nllb-200-1.3b/` and `<install-dir>/models/nllb-200-3.3b/`;
- hard-links the model files from `./models` when they are on the same filesystem,
  and copies them otherwise. Use `--skip-models` to skip this step.

The installer never downloads models. If `./models` contains checkpoint folders
they are installed automatically; otherwise place the model files in
`<install-dir>/models/` after installation.

Run the installed application with:

```bash
~/.fluent/bin/fluent
```

Useful options:

```bash
./install.sh --cuda            # NVIDIA build (CUDA toolkit + nvcc required)
./install.sh --metal           # Apple Silicon build
./install.sh --skip-build      # reuse target/release/fluent
./install.sh --skip-models     # create empty model folders
./install.sh --install-dir DIR # install somewhere other than ~/.fluent
```

`FLUENT_MODELS_SRC=/path/to/models ./install.sh` reads model files from another
directory. `FLUENT_INSTALL_DIR` overrides the installation directory.

### Installed layout

```text
~/.fluent/
├── bin/
│   └── fluent
├── config.json
└── models/
    ├── nllb-200-1.3b/
    └── nllb-200-3.3b/
```

The application loads `config.json` from the executable directory or its parent
before looking in the current directory, so the installed copy always uses its
own `models/` folder, regardless of where you launch it from.

To uninstall, remove the installation folder:

```bash
rm -rf ~/.fluent
```

Platform settings such as the selected theme and last source text are stored
separately (see [Usage](#usage)).

---

## Translation engines

| Mode | Model | Weights | Best for |
| --- | --- | --- | --- |
| Fast | `facebook/nllb-200-1.3B` | ~5.2 GB | CPUs and low-memory machines |
| Accurate | `facebook/nllb-200-3.3B` | ~17.5 GB | GPUs and machines with plenty of RAM |

Models are loaded on demand and can be unloaded from the settings panel to free
memory.

### Model files

For a source build, place the model directories next to `config.json` (or update
the paths in that file). The installer places them in the installation folder
instead (`~/.fluent/models/` by default).

Each model directory must contain:

```text
models/nllb-200-1.3b/
├── config.json
├── tokenizer.json
├── pytorch_model.bin          # or model.safetensors
└── sentencepiece.bpe.model    # optional, not used by the Rust engine
```

Sharded checkpoints (`pytorch_model.bin.index.json`, `model.safetensors.index.json`)
are supported too.

`config.json` at the repository root points to the two model directories:

```json
{
  "models": {
    "fast": "models/nllb-200-1.3b",
    "accurate": "models/nllb-200-3.3b"
  },
  "generation": {
    "max_length": 512,
    "num_beams": 4
  }
}
```

The `host` and `port` fields are only used by the legacy Python server.

---

## Building from source

### Requirements

- Rust 1.85 or newer (`rustup` is recommended)
- A C toolchain for the target platform
- Linux: X11 or Wayland development libraries and an OpenGL driver

### Run from source

```bash
git clone <repository-url> fluent
cd fluent

# Build and run the desktop application
cargo run --release -p fluent-gui
```

The binary is written to `target/release/fluent` (`fluent.exe` on Windows).

### Linux (Fedora / Nobara)

```bash
sudo dnf install gcc gcc-c++ make cmake \
    libxkbcommon-devel wayland-devel mesa-libGL-devel \
    fontconfig-devel
cargo build --release -p fluent-gui
./target/release/fluent
```

### Windows

Install the MSVC build tools and Rust (`x86_64-pc-windows-msvc`), then run:

```powershell
cargo build --release -p fluent-gui
.\target\release\fluent.exe
```

### macOS

Install Xcode Command Line Tools and Rust, then run:

```bash
cargo build --release -p fluent-gui
./target/release/fluent
```

### GPU acceleration (optional)

The default build is CPU-only and runs everywhere. GPU support is compiled in
explicitly:

```bash
# NVIDIA (CUDA toolkit and nvcc required)
cargo build --release -p fluent-gui --features cuda

# Apple Silicon (Metal)
cargo build --release -p fluent-gui --features metal
```

Select `Automatic`, `Processor (CPU)`, `NVIDIA GPU (CUDA)` or
`Apple GPU (Metal)` in **Settings → Device**. CUDA/Metal entries are only
available in builds that include the corresponding feature.

---

## Usage

1. Choose a source language, or keep **Automatic detection**.
2. Choose the target language. Use the search field to filter by English name,
   native name, FLORES-200 code or ISO code.
3. Paste or type your text. Long text scrolls inside the editor instead of
   overflowing the panel.
4. Click **Translate** or press `Ctrl + Enter`.
5. Use **Copy** to put the translation on the clipboard.

The model is loaded on the first translation. Loading and generation show live
progress and can be cancelled at any time.

Settings are stored in the platform configuration directory:

| Platform | Path |
| --- | --- |
| Linux | `~/.config/fluent/settings.json` |
| Windows | `%APPDATA%\fluent\Fluent\settings.json` |
| macOS | `~/Library/Application Support/io.fluent.Fluent/settings.json` |

---

## Command-line example

The core engine also ships with a small CLI example:

```bash
cargo run --release -p fluent-core --example translate -- \
    "Hello, how are you?" eng_Latn fra_Latn models/nllb-200-1.3b \
    cpu auto
```

Optional environment variables:

- `FLUENT_BEAMS` — beam width (default `4`)
- `FLUENT_MAX_NEW` — maximum generated tokens

---

## Project layout

```text
.
├── Cargo.toml                 # Cargo workspace
├── config.json                # model paths and defaults
├── install.sh                 # build + self-contained installer
├── crates/
│   ├── fluent-core/           # NLLB-200 engine, tokenizer, language data
│   │   ├── src/
│   │   └── examples/translate.rs
│   └── fluent-gui/            # egui desktop application
│       └── src/
├── backend/                   # legacy Python/Flask implementation
├── frontend/                  # legacy web interface
└── models/                    # local NLLB-200 checkpoints
```

The original Python implementation is kept in `backend/`, `frontend/` and
`start.py` for reference. The Rust application does not depend on it.

---

## Technology

- **Language:** Rust
- **Interface:** egui / eframe (glow renderer)
- **Inference:** candle (CPU by default, optional CUDA/Metal)
- **Models:** Meta NLLB-200 (M2M100 architecture)
- **Tokenizer:** Hugging Face `tokenizers`
- **Language detection:** `whatlang`
- **Configuration:** JSON + serde

## Privacy

All inference happens on the local device. Fluent does not send text, prompts or
statistics over the network. The only network access is the one you perform
yourself when downloading model weights.

## License

MIT — see [LICENSE](LICENSE).
