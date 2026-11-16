# macOS Build Guide

**Status: CI-built and smoke-tested on an arm64 runner; physical validation
NOT EXECUTED.** No macOS machine is available in this development environment.
GitHub's `macos-latest` runner builds the release binary and `.app`, validates
the plist and Mach-O architecture, and opens a GUI window. That is CI-runner
validation, not physical Apple-hardware validation.

## Prerequisites

- macOS 12 or newer on Apple Silicon for the packaged artifact. Intel builds
  are not produced or validated.
- Xcode Command Line Tools (`xcode-select --install`).
- Rust stable for `aarch64-apple-darwin` (<https://rustup.rs>).
- Enough RAM/disk for the model you intend to use.

## Build and run

CPU-only build:

```bash
git clone https://github.com/NeonMC23/Fluent.git
cd Fluent
cargo build --release --locked -p fluent-gui
./target/release/fluent
```

Optional Metal build (not validated in this release):

```bash
cargo build --release --locked -p fluent-gui --features metal
```

Metal is an optional feature; the default CPU build does not require it. When
the feature is enabled, select the Metal device in **Settings → Device**. CPU is
the only validated macOS path.

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

Settings live in `~/Library/Application Support/io.fluent.Fluent/`. A
configuration written there or next to the executable can point at model
directories. The packaged ZIP contains no `config.json`, so a fresh extraction
starts with `models/` relative to its working directory; use
**Models → Storage** and **Apply folder** to choose an explicit model root
before downloading. The documented per-user data location is:

```text
~/Library/Application Support/io.fluent.Fluent/config.json
~/Library/Application Support/io.fluent.Fluent/settings.json
~/Library/Application Support/io.fluent.Fluent/models/
```

The Linux `install.sh` has a best-effort macOS branch that installs the payload
under `~/Library/Application Support/io.fluent.Fluent/app`, but **it has not
been tested**. Manual `cargo build` plus a configuration file is the supported
path for now.

## CI, application ZIP and validation status

The CI workflow defines a native `macos-latest` job that runs
`cargo test --workspace`, a release GUI build, `scripts/package-macos.sh` and
`scripts/package-macos-archive.py`. It uploads the `fluent-macos` workflow
artifact containing an unsigned `Fluent-<version>-macos-<arch>.zip` (14-day
retention; the current runner is arm64). It executed successfully in GitHub
Actions run `43785992390` (commit `c91523c`), including plist/Mach-O
validation and a bundled GUI smoke test on the runner. The Cycle 19 inner ZIP
is 7,905,926 B with SHA-256
`d903b2c801acaabf8625bfaa6a54df3503962c7ff2c49648a0c8761b6291fa39`. Metal,
Intel macOS, universal binaries, signing and notarization remain untested;
this is not notarized Gatekeeper or physical-hardware validation.

The ZIP contains the staged `Fluent.app` bundle (Info.plist, executable,
Fluent.icns, PkgInfo). Models and user configuration are excluded. Intel macOS is not produced by the current matrix.

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
plist can be staged and parsed on Linux. CI has built the arm64 executable and
launched the bundle under the runner's GUI smoke test; no physical Mac has
launched it.

## Packaging, signing and notarization

The `.app` bundle and `.icns` are produced by the staging script; there is no
universal binary, code signature, hardened-runtime entitlement, or
notarization workflow. The ZIP is unsigned; Gatekeeper friction may apply. A
future distribution needs:

- a Developer ID Application certificate and code-signing identity;
- hardened runtime and the minimum entitlements for the OpenGL/Metal path;
- an Apple notarization account and a `notarytool` workflow with stapling;
- separate `aarch64-apple-darwin` and `x86_64-apple-darwin` builds or a
  universal binary;
- validation on real Apple hardware, including Gatekeeper behavior.

The existing bundle already has the `io.fluent.Fluent` identifier and an ICNS
derived from the official Fluent mark. No Developer ID certificate or Apple
notarization credentials are available, so the ZIP is unsigned and
Gatekeeper friction may apply. Do not treat the current ZIP as a notarized
macOS distribution.
