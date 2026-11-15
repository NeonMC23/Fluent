# Model Manager

Fluent can discover, download, verify, load, unload and remove its four
translation models and two chat models from the **Model Manager** in the
application header (or **Settings → Manage models…**).

Models are always **user-supplied**. Fluent does not bundle weights in the
application, installer, portable archive or CI artifacts.

## Catalogue

| Model | Architecture | Storage | Download | License | Runtime |
|---|---|---:|---:|---|---|
| NLLB-200 1.3B | NLLB-200 | 5.2 GB | ~5.5 GB | CC-BY-NC-4.0 | CPU Q8; CUDA/Metal if built |
| NLLB-200 3.3B | NLLB-200 | 17.5 GB | ~17.6 GB | CC-BY-NC-4.0 | CPU Q8; CUDA/Metal if built |
| MADLAD-400 3B | T5 | 1.7 GB | ~1.7 GB | Apache-2.0 | CPU q4_k greedy only |
| SMaLL-100 | M2M-100 (12/3) | 1.3 GB | 1.34 GB | MIT | CPU Q8; CUDA/Metal if built |
| Qwen2.5 1.5B Instruct | Qwen2 (chat) | 1.1 GB | 1.12 GB | Apache-2.0 | CPU GGUF Q4_K_M |
| Qwen2.5 0.5B Instruct | Qwen2 (chat) | 0.4 GB | 0.43 GB | Apache-2.0 | CPU GGUF Q4_0 |

The catalogue is defined once in `crates/fluent-core/src/model/registry.rs`.
Required files, sizes, SHA-256 values, repository and pinned revision are in
that registry. The GUI, verifier and downloader consume the same definitions.

## Remote catalogue

The **Catalogue** row in the Model Manager fetches
`catalogue/models-v1.json` from the official Fluent repository. The document is
versioned (`schema_version: 1`), strict and fail-closed:

- HTTPS only, with a 256 KiB body limit and bounded connect/read timeouts.
- Unknown schema versions, unknown backends, unsafe file names, non-HTTPS
  URLs, invalid SHA-256 values and malformed entries are rejected.
- Every entry must agree with the compiled registry: id, backend, pinned
  revision, licence and the exact file manifest (name, size, SHA-256, total
  size and pinned Hugging Face URL). A mismatch fails the whole refresh.
- A `minimum_app_version` newer than the running Fluent is rejected with a
  clear message instead of silently ignoring fields the build does not know.
- A failed refresh never disables installed models; Fluent falls back to the
  built-in metadata.

This guarantees that a remote document can never point Fluent at arbitrary
weights or introduce a backend the binary does not implement. Adding a genuinely
new model therefore still requires a Fluent update that implements its backend
and registers it locally; the catalogue is a refreshable, tamper-evident mirror
of that registry, not an executable plugin mechanism. The schema and maintainer
workflow are documented in [CATALOGUE.md](CATALOGUE.md).

## States

The manager distinguishes:

- **Not installed** — no required files, or an empty/missing directory.
- **Incomplete** — some required files are missing or have the wrong size.
- **Installed** — all required files pass the size check.
- **Installed · verified** — every required file passed SHA-256 verification.
- **Needs repair** — verification found a corrupt or modified file.
- **Downloading / Verifying** — a background task is active.
- **Loaded** — the inference engine currently holds the model.

A size check is deliberately cheap and runs whenever the catalogue is shown.
Use **Verify** for the full checksum pass. A model is never reported as
downloaded/installed before its required files pass verification.

## Chat models

The **Chat models** section uses the same native installer and verifier as the
translation models: pinned revisions, exact sizes and SHA-256, resumable
`.part` downloads, disk checks, atomic promotion, cancellation and safe
removal. Chat models are intentionally separate from translation models and do
not appear in the translation selector.

- **Download/Repair/Verify** work exactly like the translation cards.
- There is no **Load** action in the manager: the Chat view loads the selected
  chat model on the first message.
- To remove a chat model, switch to **Translate** first. That unloads the chat
  engine; removal is refused while it is loaded or generating.
- Chat history is never stored in the model directory or settings.

## Installation

1. Open **Models** in the header.
2. Choose a model and click **Download** (or **Repair** for an incomplete
   installation).
3. Review the licence, exact download size, free space and destination.
4. Confirm. The task runs in a worker thread with progress, speed, ETA and a
   **Cancel** action.
5. After the download, every required file is SHA-256 verified before the model
   becomes available for loading.

Downloads use the official Hugging Face repositories at pinned revisions,
write to a `.part` file, resume with HTTP Range requests where the server
supports them, and rename only after size/hash verification. A failed download
does not replace a working file. Invalid existing files are preserved with an
`.invalid-<timestamp>` suffix rather than deleted.

If disk space is insufficient, the task stops before writing the downloaded
files. The check includes a safety margin.

The native downloader is Rust (`ureq` + `sha2`); the application has no Python
runtime dependency. `scripts/fetch-madlad.py` remains an optional command-line
fallback for MADLAD.

## Existing installations

Existing paths from `config.json` are used when no custom storage root is set:

```json
"models": {
  "fast": "/home/you/.local/share/fluent/models/nllb-200-1.3b",
  "accurate": "/home/you/.local/share/fluent/models/nllb-200-3.3b",
  "madlad": "/home/you/.local/share/fluent/models/madlad400-3b-mt"
}
```

The manager detects those directories without downloading or moving anything.
Extra files from a Hugging Face download are ignored. Existing model
directories are never renamed or deleted by verification.

## Storage

The default root is the parent of the configured NLLB directory, normally:

```text
~/.local/share/fluent/models/        Linux
~/Library/Application Support/io.fluent.Fluent/models/   macOS
%APPDATA%\fluent\Fluent\data\models\ Windows
```

The manager shows the path, free space, total usage and per-model usage.

A custom storage folder can be entered and applied. It is validated for
creation and write access. **Existing models are not moved automatically**;
the custom root changes where Fluent looks for model subdirectories. Clear the
field and click **Use config paths** to return to the existing configuration.
This keeps the operation reversible and avoids silently relocating weights.

Downloads are refused if the destination is inside a Git repository.

## Load, unload and switch

- **Load** selects the model and asks the worker to load it. Existing memory
  checks apply.
- **Unload** releases the engine state.
- Switching models uses the same explicit manual selection as the main
  interface; there is no automatic routing.
- While a download/verification task is active, load/remove actions are
  disabled.
- A failed load leaves the model in an error state and can be retried.

## Remove

**Remove** requires confirmation and shows the exact model directory. It is
refused while the model is loaded; unload it first. The manager only removes
the selected managed model directory. It never removes the storage root,
configuration, another model, or unrelated files.

Removed weights must be downloaded again. Fluent does not offer automatic
cleanup.

## Licenses

- NLLB-200: **CC-BY-NC-4.0** (non-commercial). See
  [MODEL_PROVENANCE.md](MODEL_PROVENANCE.md).
- MADLAD-400-3B-MT: **Apache-2.0**. See [MADLAD.md](MADLAD.md).
- Fluent source code: **MIT**; this does not change the model licenses.

The download confirmation displays the model license before acquisition.
Model pages: [NLLB-200 1.3B](https://huggingface.co/facebook/nllb-200-1.3B),
[NLLB-200 3.3B](https://huggingface.co/facebook/nllb-200-3.3B),
[MADLAD-400-3B-MT](https://huggingface.co/google/madlad400-3b-mt).

## Troubleshooting

| Symptom | Action |
|---|---|
| Incomplete | Click **Repair**; only missing/invalid files are requested |
| Catalogue refresh failed | Installed models keep working; check the network and retry |
| Needs repair | Click **Verify**, then **Repair** |
| Download interrupted | Click **Download** again; the `.part` file resumes |
| Not enough disk space | Free space or choose a different storage folder |
| Model folder not found | Check **Storage** and `config.json`; download the model |
| Custom root has no models | Clear it to return to config paths, or download there |
| Cannot remove | Unload the model first; verify the folder is the managed directory |

After installation, the model is fully offline. Fluent performs no background
network activity; downloads occur only after the user confirms them.
