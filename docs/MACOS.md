# macOS Build Guide

**Status: not validated.** No macOS machine or Apple target toolchain was
available in Cycle 6. The source is expected to compile, but nothing below has
been executed on macOS.

## Prerequisites

- macOS 12 or newer on Apple Silicon or Intel.
- Xcode Command Line Tools (`xcode-select --install`).
- Rust stable for `aarch64-apple-darwin` and/or `x86_64-apple-darwin`
  (<https://rustup.rs>).
- Enough RAM/disk for the model you intend to use.

## Build and run

CPU-only build:

```bash
git clone https://github.com/NeonMC23/Fluent.git
cd Fluent
cargo build --release --locked -p fluent-gui
./target/release/fluent
```

Optional Metal build:

```bash
cargo build --release --locked -p fluent-gui --features metal
```

Metal is an optional feature; the default build does not require it. When the
feature is enabled, select the Metal device in **Settings → Device**.

## CPU behaviour on Apple Silicon

The Q8 kernels are AVX2/FMA implementations guarded by `cfg(target_arch =
"x86_64")`. On Apple Silicon the engine therefore uses the portable scalar Q8
kernel. It is correct but slower than the AVX2 path measured on x86-64. There
is no NEON Q8 kernel yet; that is a future performance task, not a distribution
blocker. Runtime CPU detection remains safe because no x86 instructions are
compiled into the ARM path.

## Configuration and models

The application searches for `config.json` in this order:

1. `FLUENT_CONFIG`
2. the directory containing the executable
3. its parent directory
4. `~/Library/Application Support/io.fluent.Fluent/config.json`
5. the current working directory

Settings live in the same directory as `settings.json`. Documented model
location:

```text
~/Library/Application Support/io.fluent.Fluent/config.json
~/Library/Application Support/io.fluent.Fluent/settings.json
~/Library/Application Support/io.fluent.Fluent/models/nllb-200-1.3b/
~/Library/Application Support/io.fluent.Fluent/models/nllb-200-3.3b/
```

The Linux `install.sh` has a best-effort macOS branch that installs the payload
under `~/Library/Application Support/io.fluent.Fluent/app`, but **it has not
been tested**. Manual `cargo build` plus a config file is the supported path
for now.

## CI, application ZIP and validation status

The CI workflow defines a native `macos-latest` job that runs
`cargo test --workspace`, a release GUI build, `scripts/package-macos.sh` and
`scripts/package-macos-archive.py`. It uploads the `fluent-macos` workflow
artifact containing an unsigned `Fluent-<version>-macos-<arch>.zip` (14-day
retention; the current runner is arm64). It executed successfully in GitHub
Actions run `43621221912` (commit `09a71e9`), including a plist/Mach-O
validation and a bundled GUI smoke test on the runner. Metal, Intel macOS,
universal binaries, signing and notarization remain untested; this is not
notarized Gatekeeper validation.

The ZIP contains the staged `Fluent.app` bundle (Info.plist, executable,
Fluent.icns, PkgInfo). Models and user configuration are excluded. Intel macOS is not produced by the current matrix.

## Packaging, signing and notarization

## Application icon and bundle staging

`scripts/package-macos.sh` stages:

```text
dist/Fluent.app/Contents/MacOS/fluent
dist/Fluent.app/Contents/Resources/Fluent.icns
dist/Fluent.app/Contents/Info.plist
dist/Fluent.app/Contents/PkgInfo
```

`Info.plist` uses bundle identifier `io.fluent.Fluent`, executable `fluent`,
icon `Fluent.icns`, display name `Fluent`, minimum macOS 12.0 and high-DPI
support. The ICNS is validated independently on Linux; the bundle layout and
plist can be staged and parsed on Linux, but only a macOS build can produce a
usable executable. No macOS build or launch has been performed.

## Packaging, signing and notarization

The `.app` bundle and `.icns` are produced by the staging script; there is no
universal binary, code signature, hardened-runtime entitlement, or
notarization workflow. The ZIP is unsigned; Gatekeeper friction may apply. A
future distribution needs:

- an `Info.plist` with the `io.fluent.Fluent` bundle identifier;
- a `.icns` icon derived from the official brand asset (none exists yet);
- ad-hoc or Developer ID signing;
- hardened runtime and the minimum entitlements for the OpenGL/Metal path;
- notarization and stapling for distribution outside the App Store;
- separate `aarch64-apple-darwin` and `x86_64-apple-darwin` builds or a
  universal binary;
- validation on real Apple hardware.

Do not treat the current binary as a distributable macOS application.
