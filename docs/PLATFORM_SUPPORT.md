# Platform Support

This is the honest support matrix for the 0.2.0 release candidate (Cycle 22
release validation; documentation reviewed in Cycle 22). It distinguishes local
builds, CI definitions, CI execution and actual GUI runtime validation.

## Summary

| Platform | Local build | CI job defined | CI executed | GUI runtime validated | Distribution artifact |
|---|---|---|---|---|---|
| Linux x86-64, glibc ≥ 2.35 | **Yes** (host and Ubuntu 22.04 container) | Yes (`ubuntu-22.04`, tar.gz + checksum + smoke) | **Yes** (run 43986327987) | **Yes** (Nobara host + Ubuntu 22.04 container + Xvfb) | Portable tar.gz |
| Linux AArch64 | No | No | No | No | No |
| Windows x86-64 | No toolchain | Yes (tests, release GUI, portable ZIP + self-test) | **Yes** (run 43986327987) | **Partial**: packaged `--version`/`--self-test` on runner; GUI skipped (no OpenGL); physical GUI **NOT EXECUTED** | Portable ZIP; no installer |
| macOS (current runner: arm64) | No toolchain | Yes (tests, release GUI, `.app` ZIP + smoke) | **Yes** (run 43986327987) | **CI runner only** (bundle GUI smoke); physical GUI **NOT EXECUTED** | `.app` ZIP; unsigned; Intel not produced |
| NLLB-200 1.3B / 3.3B (dense, quantised) | **Yes** on Linux x86-64 CPU | Yes (Linux/Windows/macOS builds) | **Yes** (run 43986327987) | **Yes** (Linux real translation, tiny FLORES regression, generated checks) | User-installed weights; CPU Q8 validated; CUDA/Metal builds not validated |
| MADLAD-400 3B (T5, q4_k GGUF) | **Yes** on Linux x86-64 CPU | Yes (Linux/Windows/macOS builds) | **Yes** (run 43986327987) | **Yes** (Linux real-model engine test + GUI inspection) | User-supplied GGUF; CPU only |
| SMaLL-100 (M2M-100, 330M) | **Yes** on Linux x86-64 CPU | Yes (Linux/Windows/macOS builds) | **Yes** (run 43986327987) | **Yes** (Linux real-model engine test + GUI download/verify) | User-supplied safetensors; CPU validated |
| Qwen2.5 Instruct chat (0.5B/1.5B) | **Yes** on Linux x86-64 CPU | Yes (Linux/Windows/macOS builds) | **Yes** (run 43986327987) | **Yes** (Linux GUI download/verify, English/French chat, 0.5B↔1.5B switch) | User-supplied GGUF; CPU only |

The final Cycle 22 validation run is `43986327987` (`b57e6bc`), including the
strengthened Windows/macOS package checks, MADLAD/T5, SMaLL-100, NLLB and
Qwen2.5 chat compilation, packaged application smoke tests, the Model Manager,
the remote catalogue, the unified mark-based icons, the language capability
registry, local chat and artifact verification. GitHub-runner execution is
still not physical-device validation.

Measured on the canonical host (Intel Core i5-11400H, 15 GiB RAM, CPU only):
the Qwen2.5 1.5B chat engine reached about 2.09 GB RSS while answering; real
NLLB-200 1.3B translation in the GUI reached about 1.74 GB. The default
allocator kept RSS near 0.96 GB after chat unload, but forcing trims
(`MALLOC_TRIM_THRESHOLD_=0 MALLOC_MMAP_THRESHOLD_=65536`) showed RSS falling to
~0.14 GB and ~0.18 GB after chat and translation unload respectively, so the
other heavyweight engine is not kept resident. Translation engines loaded in
2.1 s (MADLAD), 9.4 s (NLLB 1.3B) and 21.3 s (NLLB 3.3B) with peak RSS of
2.68 GB, 1.63 GB and 3.62 GB respectively.

## Linux

- **Canonical baseline:** Ubuntu 22.04, glibc 2.35, x86-64.
- **Build:** complete GUI compiled inside an Ubuntu 22.04 container with the
  committed `Cargo.lock`; no dependency updates.
- **Binary:** ELF 64-bit PIE, stripped, dynamically linked. Maximum required
  GLIBC symbol: `GLIBC_2.35`. No GLIBCXX/CXXABI dependency.
- **Direct NEEDED:** `libgcc_s.so.1`, `libm.so.6`, `libc.so.6`,
  `ld-linux-x86-64.so.2`.
- **Runtime:** GUI launched inside Ubuntu 22.04 under Wayland with software
  OpenGL for 6 seconds with no model; stayed up and emitted no errors. The
  Cycle 19 archive was extracted and passed `--version`, `--self-test` and
  `--smoke-test-gui` under Xvfb; it also ran the 1.5B/0.5B chat and four-model
  translation validation on the Nobara host.
- **Runtime-only libraries:** X11 or Wayland, OpenGL 3.3 driver, fontconfig and
  the usual desktop libraries loaded by `winit`/`glutin`; not all appear in
  `ldd`.
- **Branding:** the official 256 px PNG is installed automatically by the
  installer, the window icon is embedded in the binary, and the main window
  header shows the official mark next to the Fluent wordmark.
- **Archive:** `dist/fluent-0.2.0-linux-x86_64.tar.gz`; application-only
  (binary, LICENSE, README); no models or tokenizer files.
- **Not tested:** Debian/Fedora stable releases other than the build/runtime
  hosts, ARM Linux, non-glibc systems, real Wayland desktops in this cycle
  (X11/Xvfb was used for the packaged smoke test), and other desktop
  environments.
- **Signing:** the local Ubuntu 22.04 archive is Minisign-signed
  (`B504F2189BB21013`), verifies against `docs/keys/minisign.pub`, and its
  SHA-256 (`475b2a83…`) is byte-identical to the Cycle 19 CI inner archive.
- **Gate:** `scripts/check-linux-baseline.sh target/release/fluent 2.35`.

## Windows

- The workflow runs `cargo test --workspace`, a release GUI build, the portable
  ZIP, and executes the packaged `fluent.exe --version` / `--self-test`.
- Run `43986327987` passed those checks after the portable README fix.
  GitHub's hosted Windows runner has no OpenGL 2.0+ context, so the temporary
  GUI-window smoke is SKIPPED there. This is still not physical-device GUI
  validation.
- **Physical runtime: NOT EXECUTED.** An existing local `win11` libvirt domain
  is defined, but it cannot start because its attached
  `tiny11 23H2 x64.iso` is on the unavailable `/mnt/nas` mount. No Windows
  credentials, installer artifact or authorized fallback environment were
  available.
- No installer, Start Menu integration, DLL inventory, CUDA build, or
  uninstaller is provided.
- See [WINDOWS.md](WINDOWS.md) for prerequisites and paths.

## macOS

- The workflow runs `cargo test --workspace`, a release GUI build, `.app`
  staging, plist/Mach-O validation and a bundled GUI smoke test on
  `macos-latest`.
- Run `43986327987` passed all of those steps, so the arm64 `.app` is runtime
  validated on the CI runner. **Physical macOS runtime: NOT EXECUTED**; no
  Apple Silicon or Intel Mac is available in this environment. Metal, Intel
  macOS, universal binaries, signing and notarization remain unvalidated.
- See [MACOS.md](MACOS.md) for the CPU-only/Metal build commands and remaining
  bundle work.

## MADLAD-400 (T5)

- **Architecture:** T5 encoder-decoder, q4_k GGUF, 32 encoder/decoder layers.
- **Device:** CPU only; explicit CUDA/Metal selection is rejected rather than
  silently ignored.
- **Decoding:** deterministic greedy; beam search is an NLLB-only capability.
- **Languages:** 72 verified FLORES-200 mappings whose `<2xx>` prefixes exist
  in the official tokenizer. NLLB keeps its full language table.
- **Measured on the canonical host:** 1.89 s load, 2.72 GB peak RSS,
  ~2.4 generated tokens/s, 1.4–2.2 s to first output, 1.70 GB storage.
- **Validated pairs:** English→French, French→English, English→German,
  English→Spanish, English→Japanese, English→Arabic and English→Swahili.
- **Not executed:** physical Windows/macOS MADLAD runs, CUDA/Metal for MADLAD,
  and a 512+ token MADLAD generation.
- See [MADLAD.md](MADLAD.md) for acquisition, files and limitations.

## SMaLL-100 (M2M-100 family)

- **Architecture:** 12 encoder layers, 3 decoder layers, `d_model=1024`,
  shared 128,112-entry vocabulary, 330M parameters.
- **Device:** CPU Q8 is the validated path. The same backend compiles for
  CUDA/Metal, but neither was validated in this cycle.
- **Decoding:** beam search and greedy are supported; deterministic for a fixed
  beam count and prompt.
- **Languages:** 98 mappings between Fluent/NLLB codes and the M2M-100 short
  codes. Breton and Western Frisian are not in NLLB's table; `__zh__` maps to
  Simplified Chinese only.
- **Measured on the canonical host:** see [BENCHMARKS.md](BENCHMARKS.md).
  Roughly 1 s to load, ~0.4 GB resident after load, and ~5x faster generation
  than NLLB 1.3B.
- **Validated pairs:** all 42 directed pairs among English, French, German,
  Spanish, Portuguese, Italian and Simplified Chinese. See
  [BENCHMARKS.md](BENCHMARKS.md) and [SMALL100.md](SMALL100.md).
- **Not executed:** physical Windows/macOS SMaLL-100 runs and CUDA/Metal.

## Qwen2.5 chat (0.5B/1.5B)

- **Architecture:** Qwen2.5 Instruct GGUF, CPU-only, one chat engine resident
  at a time. Chat is independent of the translation mode.
- **Models:** 1.5B Q4_K_M (`1.12 GB` download, default) and 0.5B Q4_0
  (`0.44 GB`, fast option); both Apache-2.0.
- **Cycle 19 GUI download:** 1.5B downloaded through the Model Manager into a
  fresh root. 1,124,359,686 bytes total, SHA-256 verified for all three files,
  `Installed, verified=true, missing=0, bad=0`. Cancelling at 37.6 MB preserved
  the `.part` file and showed **Not installed**.
- **Measured on the canonical host:** 1.5B load ~1.9 s, ~1.95 GB RSS after
  load, ~2.02 GB peak, 3.3–4.9 s to first output and 4–7 generated tok/s;
  0.5B load ~1.0 s, ~0.94 GB RSS and 5–8 tok/s. The GUI RSS reached about
  2.03 GB while answering and stayed near 0.96 GB after switching to Translate
  under the default allocator; forced trims showed the engine was released
  (~0.14 GB).
- **Validated flow:** English answer, French follow-up with context, streaming,
  cancellation, context truncation, switching 0.5B→1.5B (new conversation),
  unload on Translate, reload on return to Chat.
- **Not executed:** physical Windows/macOS chat inference and GPU chat.
  Persistent conversation history is not implemented by design.
- See [CHAT.md](CHAT.md) for the lifecycle and limitations.

## Branding assets

The canonical application symbol is the turquoise F mark
(`src/logo/fluent-mark.png`), extracted from the immutable `logo.png` master.
The GUI header, window icon, Linux launcher PNG, Windows `.ico` and macOS
`.icns` are all generated from that mark by
`scripts/generate-branding-assets.py`. Structural validation runs without
Pillow (`scripts/verify-branding-assets.py`); byte-for-byte reproducibility is
checked with `--check` where Pillow is available. See [ASSETS.md](ASSETS.md).

## Language coverage

The selector, engine and catalogue use the same capability registry. Actual
counts are NLLB-200 1.3B/3.3B: 202; MADLAD-400 3B: 72; SMaLL-100: 98. See
[LANGUAGES.md](LANGUAGES.md) for mappings, Chinese variants and the NLLB
204→202 correction.

## CPU and GPU behaviour

- NLLB 1.3B and 3.3B both use CPU Q8 under Automatic precision. A measured
  12-case probe showed 3.3B Q8 at ~3.6 s median versus ~26.6 s for f16, with
  4.57 GB versus 6.54 GB peak RSS and equal metrics.
- MADLAD is CPU-only q4_k greedy. Its 520-token long-generation test passed;
  see [BENCHMARKS.md](BENCHMARKS.md).

- x86-64: AVX2/FMA kernels are selected at runtime with `is_x86_feature_detected!`;
  the scalar Q8 kernel is always available. The binary is not compiled with
  `target-cpu=native`.
- ARM: only the scalar Q8 kernel exists; it is correct but slower and has not
  been benchmarked on Apple Silicon.
- CUDA and Metal are optional build features and are not required for the CPU
  build. No GPU runtime validation was performed for this release; all
  documented performance and inference results are CPU-only.

## Model storage

- With no `config.json`, the default model root is platform-native:
  `$XDG_DATA_HOME/fluent/models` (or `~/.local/share/fluent/models`) on Linux,
  `%APPDATA%\fluent\Fluent\data\models` on Windows, and
  `~/Library/Application Support/io.fluent.Fluent/models` on macOS.
- The process working directory, `/models`, and the macOS `.app` bundle are
  never used as model storage.
- On Linux and Windows, a populated portable `models/` directory beside the
  executable or archive root is detected and reused without moving files when
  the platform root is empty. A historical `~/.fluent/models` directory is also
  considered. The macOS `.app` bundle is never used.
- An explicit `config.json` model path or GUI custom storage root takes
  precedence. See [MODEL_MANAGER.md](MODEL_MANAGER.md).

## Model assets

No model weights, tokenizer or model configuration is included in the
repository or the portable archive. The Model Manager downloads pinned files
only after explicit confirmation, or you can supply an existing package; see
[MODEL_PACKAGE.md](MODEL_PACKAGE.md) and [MODEL_PROVENANCE.md](MODEL_PROVENANCE.md).
