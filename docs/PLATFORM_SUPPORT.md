# Platform Support

This is the honest support matrix for Cycle 8. It distinguishes local builds,
CI definitions, CI execution and actual GUI runtime validation.

## Summary

| Platform | Local build | CI job defined | CI executed | GUI runtime validated | Distribution artifact |
|---|---|---|---|---|---|
| Linux x86-64, glibc ≥ 2.35 | **Yes** (host and Ubuntu 22.04 container) | Yes (`ubuntu-22.04`, tar.gz + checksum + smoke) | **Yes** (run 43727254932) | **Yes** (Nobara host + Ubuntu 22.04 container + Xvfb) | Portable tar.gz |
| Linux AArch64 | No | No | No | No | No |
| Windows x86-64 | No toolchain | Yes (tests, release GUI, portable ZIP + self-test) | **Yes** (run 43727254932) | **Partial**: self-test on runner; GUI skipped (no OpenGL) | Portable ZIP; no installer |
| macOS (current runner: arm64) | No toolchain | Yes (tests, release GUI, `.app` ZIP + smoke) | **Yes** (run 43727254932) | **Yes on CI runner** (bundle GUI smoke) | `.app` ZIP; unsigned; Intel not produced |
| MADLAD-400 3B (T5, q4_k GGUF) | **Yes** on Linux x86-64 CPU | Yes (Linux/Windows/macOS builds) | **Yes** (run 43727254932) | **Yes** (Linux real-model engine test + GUI inspection) | User-supplied GGUF; CPU only |
| SMaLL-100 (M2M-100, 330M) | **Yes** on Linux x86-64 CPU | Yes (Linux/Windows/macOS builds) | **Yes** (run 43727254932) | **Yes** (Linux real-model engine test + GUI download/verify) | User-supplied safetensors; CPU validated |

The current fully green `main` run is `43727254932` (`5200ab5`), including
MADLAD/T5 and SMaLL-100 compilation, packaged application smoke tests, the
Model Manager, the remote catalogue, benchmark harness compilation and artifact
verification. Earlier successful runs are
`43605074905`, `43606018556`, `43606646499` and `43620495032`. GitHub-runner
execution is still not physical-device validation.

## Linux

- **Canonical baseline:** Ubuntu 22.04, glibc 2.35, x86-64.
- **Build:** complete GUI compiled inside an Ubuntu 22.04 container with the
  committed `Cargo.lock`; no dependency updates.
- **Binary:** ELF 64-bit PIE, stripped, dynamically linked. Maximum required
  GLIBC symbol: `GLIBC_2.35`. No GLIBCXX/CXXABI dependency.
- **Direct NEEDED:** `libgcc_s.so.1`, `libm.so.6`, `libc.so.6`,
  `ld-linux-x86-64.so.2`.
- **Runtime:** GUI launched inside Ubuntu 22.04 under Wayland with software
  OpenGL for 6 seconds with no model; stayed up and emitted no errors. Also
  launched from an extracted archive on the Nobara host.
- **Runtime-only libraries:** X11 or Wayland, OpenGL 3.3 driver, fontconfig and
  the usual desktop libraries loaded by `winit`/`glutin`; not all appear in
  `ldd`.
- **Branding:** the official 256 px PNG is installed automatically by the
  installer, the window icon is embedded in the binary, and the main window
  header shows the official mark next to the Fluent wordmark.
- **Archive:** `dist/fluent-0.2.0-linux-x86_64.tar.gz`; application-only
  (binary, LICENSE, README); no models or tokenizer files.
- **Not tested:** Debian/Fedora stable releases other than the build/runtime
  hosts, ARM Linux, non-glibc systems, X11-only sessions (the container used
  Wayland), and other desktop environments.
- **Gate:** `scripts/check-linux-baseline.sh target/release/fluent 2.35`.

## Windows

- The workflow runs `cargo test --workspace`, a release GUI build, the portable
  ZIP, and executes the packaged `fluent.exe --version` / `--self-test`.
- Run `43710941404` passed those checks. GitHub's hosted Windows runner has no
  OpenGL 2.0+ context, so the temporary GUI-window smoke is SKIPPED there.
  This is still not physical-device GUI validation.
- No installer, Start Menu integration, DLL inventory, CUDA build, or
  uninstaller is provided.
- See [WINDOWS.md](WINDOWS.md) for prerequisites and paths.

## macOS

- The workflow runs `cargo test --workspace`, a release GUI build, `.app`
  staging, plist/Mach-O validation and a bundled GUI smoke test on
  `macos-latest`.
- Run `43710941404` passed all of those steps, so the arm64 `.app` is runtime
  validated on the CI runner. Metal, Intel macOS, universal binaries, signing
  and notarization remain unvalidated.
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
  build.
- No GPU runtime validation was performed in this cycle.

## Model assets

No NLLB weights, tokenizer or model configuration is included in the
repository or the portable archive. Users supply a model package; see
[MODEL_PACKAGE.md](MODEL_PACKAGE.md) and [MODEL_PROVENANCE.md](MODEL_PROVENANCE.md).
