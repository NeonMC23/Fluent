# Platform Support

This is the honest support matrix for Cycle 8. It distinguishes local builds,
CI definitions, CI execution and actual GUI runtime validation.

## Summary

| Platform | Local build | CI job defined | CI executed | GUI runtime validated | Distribution artifact |
|---|---|---|---|---|---|
| Linux x86-64, glibc ≥ 2.35 | **Yes** (host and Ubuntu 22.04 container) | Yes (`ubuntu-22.04`, tar.gz + checksum) | **No** | **Yes** (Nobara host + Ubuntu 22.04 container) | Portable tar.gz |
| Linux AArch64 | No | No | No | No | No |
| Windows x86-64 | No toolchain | Yes (tests, release GUI, portable ZIP) | **No** | **No** | Portable ZIP; no installer |
| macOS arm64 (Apple Silicon) | No toolchain | Yes (tests, release GUI, `.app` ZIP) | **No** | **No** | arm64 `.app` ZIP; unsigned |

“CI job defined” means the workflow contains the job; it does not mean it has
run or passed. No GitHub Actions run occurred in this cycle.

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

- The workflow now runs `cargo test --workspace` and a release GUI build on
  `windows-latest`.
- This is a compile/test definition only. It has not run on GitHub, and no
  Windows GUI has ever been launched by this project.
- No installer, Start Menu integration, DLL inventory, CUDA build, or
  uninstaller is provided.
- See [WINDOWS.md](WINDOWS.md) for prerequisites and paths.

## macOS

- The workflow now runs `cargo test --workspace` and a release GUI build on
  `macos-latest`.
- No macOS GUI has been launched, no `.app`/`.icns`/`Info.plist` exists, and
  Metal, universal binaries, signing and notarization are unvalidated.
- The macOS CI job compiles, tests and stages a ZIP, but is not runtime validation.
- See [MACOS.md](MACOS.md) for the CPU-only/Metal build commands and remaining
  bundle work.

## CPU and GPU behaviour

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
