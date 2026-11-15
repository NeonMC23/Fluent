# Builds, GitHub Actions Artifacts and Release Verification

Fluent has one authoritative development version in `Cargo.toml`; see
[VERSIONING.md](VERSIONING.md). The current unreleased version is **0.2.0**;
the last published version was **0.1.0**. No tag or GitHub Release is created
by the current mission.

## 1. GitHub Actions

Workflow: `.github/workflows/ci.yml` — **CI and build artifacts**.

### What it does

- **Linux** (`ubuntu-22.04`, x86-64): formatting, check, tests, Clippy,
  release GUI build, `GLIBC_2.35` gate, installer regression tests, portable
  tar.gz. Extracts the archive and runs `--version`, `--self-test` and a real
  `--smoke-test-gui` window under Xvfb. Uploads `fluent-linux-x86_64`.
- **Windows** (`windows-latest`, x86-64): workspace tests, release GUI build
  with embedded ICO/metadata, portable ZIP. Extracts the ZIP and runs
  `--version` and `--self-test`. The hosted runner has no OpenGL 2.0+ context,
  so `--smoke-test-gui` is reported as SKIPPED there; any other GUI error fails
  the job. Uploads `fluent-windows-x86_64`.
- **macOS** (`macos-latest`, currently arm64): workspace tests, release GUI
  build, `Fluent.app` staging, app ZIP. Unpacks the ZIP, validates the plist
  with `plutil`, inspects the Mach-O architecture with `lipo`, and runs
  `--version`, `--self-test` and `--smoke-test-gui`. Uploads `fluent-macos`;
  the filename records the runner architecture. Intel macOS is not separately
  built.
- **Release integrity** (only non-PR events): downloads all three artifacts,
  optionally signs archives with Minisign when `MINISIGN_SECRET_KEY` is
  configured, verifies every signature against the committed public key,
  checks archive structure and checksums with
  `scripts/verify-artifacts.py`, generates `release-manifest.json`, and
  uploads the combined `fluent-release-candidate` artifact including
  `minisign.pub`.

### Triggers

- Push to `main` or any `finalization/**` branch.
- Pull requests (build/test/unsigned artifacts only; signing job is skipped).
- Manual `workflow_dispatch`.

### Finding and downloading a build

1. Open <https://github.com/NeonMC23/Fluent/actions>.
2. Select **CI and build artifacts** and the run for your branch/commit.
3. Wait for the run to complete successfully.
4. Scroll to **Artifacts** at the bottom of the run summary.
5. Download the platform artifact or `fluent-release-candidate`.
6. Artifacts are retained for **14 days**. They are workflow artifacts, not
   GitHub Release assets.

A workflow artifact is an internal/test build. A GitHub Release is a separate,
manually authorized publication. Do not treat a downloaded artifact as a
release until a maintainer has verified checksums/signatures and created the
release explicitly.

### Workflow artifacts vs releases

| | Workflow artifact | GitHub Release |
|---|---|---|
| Location | Actions run summary | Repository Releases page |
| Lifetime | 14 days | Until deleted |
| Signing | Only if secret configured | Must be verified/signed before publication |
| Audience | Testers/maintainers | Public users |
| Created by | Every successful non-PR/PR run | Manual, authorized step |

### Actual execution status

The workflow has run successfully on GitHub:

- **Runs:** [43605074905](https://github.com/NeonMC23/Fluent/actions/runs/43605074905)
  on code commit `ae1dfcb9…`, the documentation follow-up
  [43606018556](https://github.com/NeonMC23/Fluent/actions/runs/43606018556)
  on `b104b398…`, and the final path-remapping build
  [43606646499](https://github.com/NeonMC23/Fluent/actions/runs/43606646499)
  on `af7dbca0…`.
- **Result:** all four jobs succeeded in every run — Linux, Windows, macOS and
  Release integrity.
- **Artifacts from the final run (14-day retention):**
  `fluent-linux-x86_64` (7,894,084 B), `fluent-windows-x86_64` (5,669,939 B),
  `fluent-macos` (6,581,141 B), and `fluent-release-candidate`
  (20,146,036 B). Sizes differ slightly between runs because embedded build
  paths and compression vary.
- Those historical runs predate the committed production public key and had no
  `MINISIGN_SECRET_KEY`; their artifacts are **unsigned**. The public key now
  exists, but the secret is still not configured, so current CI artifacts
  remain unsigned until the owner performs the one command in section 5.
- This development environment has no `gh`, token or stored credential, so it
  cannot download GitHub artifact bytes. The local Linux candidate is verified
  directly from its bytes, including a real GUI smoke test and a detached
  signature.
- The macOS artifact is arm64 on the current `macos-latest` runner.
- **Cycle 17 run (branding, model identity and language accuracy):**
  [43747547053](https://github.com/NeonMC23/Fluent/actions/runs/43747547053)
  on `23a7c944…`. All four jobs succeeded with the canonical turquoise F mark,
  explicit model names, the 202-language NLLB correction and the per-model
  language capability registry.
- **Cycle 17 archive SHA-256:** Linux
  `c425e87012ee84a44a2513d53ad9ac4832057ee2daad430e35cc7138370b1016`
  (9,395,169 B), Windows
  `5fb92fc9e0faa646361631b8bf5e949f8d0bc3b8d01aa4d165fc7e870b1d63e5`
  (7,259,029 B), macOS
  `03d5b9ff9292ef1a3b72dc95a330953cf907baf0a52694332f3d333f0d112dd2`
  (7,607,140 B). CI remains unsigned until the Minisign secret is configured.
  The locally built and Minisign-signed Linux archive is byte-identical to the
  CI inner archive (`c425e87012ee84a44a2513d53ad9ac4832057ee2daad430e35cc7138370b1016`),
  and the verifier accepted the signature and manifest.
- **Cycle 16 run (SMaLL-100 and remote catalogue):**
  [43727254932](https://github.com/NeonMC23/Fluent/actions/runs/43727254932)
  on `5200ab5a…`. All four jobs succeeded with the SMaLL-100 backend, the
  98-language mapping, the validated remote catalogue, the GUI catalogue
  refresh and the four-model Model Manager.
- **Cycle 16 archive SHA-256:** Linux
  `db1fcdaaf9071f43afac439640b9fe389af3dcb7a5a7a6764ff498a85bd74fb6`
  (9,419,066 B), Windows
  `e5c9188a1a9e394e5c649ea50383df88a072d2c2b997f011011828e82c744756`
  (7,201,614 B), macOS
  `e5d78d4d12988a715d814794acc74655b807fd598eae4596ba69c8e6aa909b02`
  (7,942,940 B). CI remains unsigned until the Minisign secret is configured.
  The locally built and Minisign-signed Linux archive from `5200ab5` is
  byte-identical to the CI inner archive
  (`db1fcdaaf9071f43afac439640b9fe389af3dcb7a5a7a6764ff498a85bd74fb6`), and the
  verifier accepted the signature and manifest.
- **Cycle 15 run (Model Manager):**
  [43710941404](https://github.com/NeonMC23/Fluent/actions/runs/43710941404)
  on `6f06329a…`. All four jobs succeeded with the native model manager,
  registry manifests, downloader, custom storage UI and stalled-read timeout.
  The earlier Cycle 15 run 43706890466 also passed.
- **Cycle 15 archive SHA-256:** Linux
  `95e4fd923b4db762a86c7e8231d21d905d05f06f57a1b45e2df433957ab854c3`
  (9,355,181 B), Windows
  `3d36aa0a323a6cf98ee71bbdcf135caa7c32364035642cf89ea42d5f3ef5721e`
  (7,130,370 B), macOS
  `541417b441052b4c96b5accca94dae5208129134c2362b9f5eda2e172d2e570b`
  (7,879,732 B). CI still reports unsigned until the Minisign secret is set.
- **Cycle 14 run (quality/performance):**
  [43682638997](https://github.com/NeonMC23/Fluent/actions/runs/43682638997)
  on `d554f5d2…`. All four jobs succeeded with the NLLB CPU Q8 auto-precision
  change, benchmark/test additions and packaged smoke tests.
- **Cycle 14 archive SHA-256:** Linux
  `9a540311934e3c800f75bc4c39d0368ad7d58bca0ba091cfffc1d72cff1d4436`
  (8,341,103 B), Windows
  `bdfe5acf0b089fbcd77a80bd74fda4242d2885d7b4f044fa98d5aa945f0a91a8`
  (6,129,208 B), macOS
  `6c3b93bb33b706526968130e764d233cd3e181114691db2a28f31c55b2f3e788`
  (6,967,812 B). The Linux archive matches the locally signed candidate; CI
  remains `unsigned` until `MINISIGN_SECRET_KEY` is configured.
- **Cycle 13 run (MADLAD):**
  [43666915254](https://github.com/NeonMC23/Fluent/actions/runs/43666915254)
  on `e5fda1fb…`. All four jobs succeeded after adding the Candle T5 backend:
  Linux, Windows and macOS built and ran their packaged checks, and the
  release-integrity job verified the archives and manifest.
- **Cycle 13 archive SHA-256:** Linux
  `f47f7a44b7b5b9cb9e387c8ef6e381ad648031fd54e5b71b3977016ab568732b`
  (8,341,621 B), Windows
  `f357f4ae2d8bd870c96bb90d8a1eda82769251dfa9afdf1f75942171ae19dbee`
  (6,128,430 B), macOS
  `e70faf638fb1dc868a5a9c48e2e96416fc7265ef3191fb36d6ae3871499d5330`
  (6,967,983 B). The Linux archive is byte-identical to the locally built and
  Minisign-signed candidate; CI still correctly reports `unsigned` because the
  Actions secret is not configured.
- **Cycle 12 run (current `main`):**
  [43658280679](https://github.com/NeonMC23/Fluent/actions/runs/43658280679)
  on `8186e04f…`. All four jobs succeeded, including the new output-limit
  tests, Linux/macOS GUI smoke tests, Windows packaged self-test, artifact
  verification and checksum publication.
- **Cycle 12 archive SHA-256:** Linux
  `a833d2a4e439a5f37af8268b842aa88a47509fbe23fa3e91d804343d1bf43614`
  (7,930,772 B), Windows
  `946395a4cb43882c6b5001dc9b92f45cdc4fa06a0be111f16427de8b94bf4454`
  (5,697,155 B), macOS
  `e89ba066724bdee66796badbb0c0441fe2786f60e05ddfe084efdf74352830f7`
  (6,605,287 B). The Linux archive is byte-identical to the locally built and
  Minisign-signed candidate, so its detached signature verifies; CI itself
  still reports `signing=unsigned` because the Actions secret is not set.
- **Cycle 11 final run:**
  [43621221912](https://github.com/NeonMC23/Fluent/actions/runs/43621221912)
  on `09a71e94…`. All four jobs succeeded. Linux and macOS ran the packaged
  GUI smoke test; Windows ran `--version` and `--self-test` and reported the
  GUI window as SKIPPED because the hosted runner has no OpenGL 2.0+ context.
- **Verified inner archive SHA-256 (from the run’s checksum notice):**
  - `fluent-0.2.0-linux-x86_64.tar.gz` — 7,924,697 B,
    `512167036b12f735f60399fb0cdd44ffe5af6303a7b8f1fb30364ea0a7011a23`
  - `fluent-0.2.0-windows-x86_64.zip` — 5,693,060 B,
    `3369029402bc7102bb7dfe07eb2a336e4c9ffb59a43f16ae632ea16e474f2a78`
  - `Fluent-0.2.0-macos-arm64.zip` — 6,602,402 B,
    `92cf43ad11a9c622fc7091023d694ed42c003d2bec5a5397908b7df8d45bd34f`
- **GitHub artifact ZIP digests (uploaded bundles, not inner archives):**
  Linux `efcb9fb9…`, Windows `a86edc34…`, macOS `116b593f…`,
  release candidate `c8f4d657…`.
- **Signing status of that run:** `unsigned`, `signatures_verified=false`;
  no secret was configured. The verifier still checked every archive’s exact
  contents, format, architecture, checksum and manifest.

## 2. Local packaging commands

### Linux (Ubuntu 22.04, glibc 2.35)

```bash
./scripts/package-linux.sh
(cd dist && sha256sum -c fluent-*.tar.gz.sha256)
```

To build the compatibility binary in a container, run `cargo build --release
--locked -p fluent-gui` in Ubuntu 22.04 with the dev packages listed in
[LINUX.md](LINUX.md), then:

```bash
./scripts/package-linux.sh --binary /path/to/ubuntu/fluent
./scripts/check-linux-baseline.sh /path/to/ubuntu/fluent 2.35
```

### Windows (native)

```powershell
cargo build --release --locked -p fluent-gui
$v = (Select-String Cargo.toml '^version = "([^"]+)"').Matches.Groups[1].Value
python scripts/package-windows.py --binary target/release/fluent.exe --version $v
```

Output: `fluent-<version>-windows-x86_64.zip` plus `.sha256`.

### macOS (native, arm64 on current runners)

```bash
cargo build --release --locked -p fluent-gui
./scripts/package-macos.sh --skip-build
./scripts/package-macos-archive.py --app dist/Fluent.app \
    --version "$(./scripts/version.sh)" --arch "$(uname -m)"
```

Output: `Fluent-<version>-macos-<arch>.zip` plus `.sha256`. The `.app` is
unsigned/not notarized. Intel macOS and universal binaries are not produced by
the current matrix.

## 3. Artifact contents and names

| Platform | Artifact | Contents |
|---|---|---|
| Linux | `fluent-0.2.0-linux-x86_64.tar.gz` | `bin/fluent`, `LICENSE`, `README.txt` |
| Windows | `fluent-0.2.0-windows-x86_64.zip` | `fluent.exe`, `LICENSE`, `README.txt` |
| macOS | `Fluent-0.2.0-macos-<arch>.zip` | `Fluent.app` with Info.plist, executable, ICNS, PkgInfo |
| Aggregate | `fluent-release-candidate` | all archives, `.sha256`, optional `.minisig`, `release-manifest.json`, `minisign.pub` |

No artifact contains model weights, tokenizer assets, user configuration,
caches or development files. The application needs a user-supplied model
package; see [MODEL_PACKAGE.md](MODEL_PACKAGE.md).

## 4. Checksum verification

Linux/macOS:

```bash
sha256sum -c fluent-0.2.0-linux-x86_64.tar.gz.sha256
shasum -a 256 -c Fluent-0.2.0-macos-arm64.zip.sha256
```

Windows PowerShell:

```powershell
Get-FileHash .\fluent-0.2.0-windows-x86_64.zip -Algorithm SHA256
# Compare with the value in the adjacent .sha256 file.
```

A checksum proves integrity relative to the channel that supplied it; it is
not a signature or proof of authorship.

## 5. Minisign signatures

Minisign is the chosen detached-signature format. The scripts accept either
the `minisign` C tool or the Minisign-compatible pure-Rust `rsign` CLI
(`rsign2` crate). Ubuntu 22.04 does not package `minisign`, so CI installs
pinned `rsign2`; local users may use either.

The repository now contains the production **public** key at
`docs/keys/minisign.pub`:

- key ID `B504F2189BB21013`
- public-key file SHA-256
  `bd07631dfc69be884ad2edd405fe5238bae27748f4d1ab3cbff7f1331501e713`

The private key is kept outside the repository with owner-only permissions.
The `MINISIGN_SECRET_KEY` Actions secret is **not configured yet**, so current
CI artifacts are unsigned. The workflow writes `UNSIGNED.txt` and a manifest
with `"minisign_status": "unsigned"` rather than inventing a signature.

### Enable CI signing

```bash
gh secret set MINISIGN_SECRET_KEY \
  --repo NeonMC23/Fluent \
  --body-file "$HOME/.config/fluent-signing/minisign.key"
```

`gh` must be authenticated for the repository owner. The key is read through
stdin and is never printed. Once the secret exists, the non-PR
`release-integrity` job signs every archive, verifies each signature against
`docs/keys/minisign.pub`, and records `"signatures_verified": true` in the
manifest. Fork pull requests never receive the secret.

### Sign and verify locally

```bash
MINISIGN_SECRET_KEY_FILE=/secure/minisign.key \
  ./scripts/sign-artifacts.sh dist/fluent-*.tar.gz dist/*.zip
MINISIGN_PUBLIC_KEY_FILE=docs/keys/minisign.pub \
  ./scripts/verify-artifacts.sh dist/fluent-*.tar.gz dist/*.zip

# Full structural, checksum, manifest and signature verification:
python3 scripts/verify-artifacts.py \
  --dir dist \
  --version 0.2.0 \
  --public-key docs/keys/minisign.pub \
  --require-signatures
```

Omit `--require-signatures` for an unsigned build; the verifier reports
`UNSIGNED` explicitly. It never treats a sidecar file's mere existence as
proof: each `.minisig` is checked with the public key. No fake signatures are
generated.

### Key rotation

If a private key is compromised: remove/replace the GitHub secret, replace the
committed public key, publish the new fingerprint, re-sign all current
artifacts, and explicitly announce that old signatures are no longer trusted.

## 6. Native platform signing status

- **Windows Authenticode:** not implemented. No certificate, timestamping or
  SmartScreen reputation is claimed. The portable ZIP is unsigned.
- **macOS Developer ID/notarization:** not implemented. No Apple Developer
  account, hardened-runtime signing, notarization or stapling is performed.
  Unsigned macOS apps may require Gatekeeper bypass steps; do not claim
  frictionless installation.
- **Linux:** no distribution-package signature. Minisign covers the portable
  archive once the Actions secret is configured; the public key is committed.

## 7. Release-candidate checklist

- [x] Version in `Cargo.toml` and GUI About is correct (`0.2.0`).
- [x] Linux Ubuntu 22.04 build passes and reports `GLIBC_2.35`.
- [x] Linux installer/archive/checksum verified locally.
- [x] Linux extracted archive passes `--version`, `--self-test` and Xvfb GUI
  smoke tests.
- [x] Windows native build produced a ZIP; CI verified its PE32+ x86-64
  executable, contents and checksum and ran `--version`/`--self-test`.
- [x] macOS native build produced `Fluent.app` ZIP (arm64); CI validated the
  plist, Mach-O architecture, contents and checksum.
- [x] Artifact names, contents, architectures and checksums verified by CI.
- [ ] SHA-256 files generated from actual archives.
- [x] Minisign public key committed (`B504F2189BB21013`).
- [ ] `MINISIGN_SECRET_KEY` configured and signatures verified (owner action).
- [ ] Windows Authenticode decision documented.
- [ ] macOS signing/notarization decision documented.
- [ ] Model/package/licensing notices current.
- [ ] Branding rights confirmed (not yet documented as legally cleared).
- [x] README, CHANGELOG and VERSIONING current.
- [x] Cycle 17 CI run 43747547053 succeeded on `main` for all four jobs after
  the branding, model-identity and language-capability integration; inner
  archive checksums were published. Cycle 16 run 43727254932 also passed.
- [x] Unlimited output generated 600 real-model tokens and cancelled after 520
  in the gated integration test.
- [ ] Artifact bytes independently downloaded and inspected (requires GitHub
  authentication).
- [ ] Release published/tagged only after explicit owner authorization.

## 8. Current limitations

- CI artifact download requires GitHub authentication. This development
  environment has no `gh` installation, token or stored credential, so the
  uploaded bytes cannot be re-downloaded here. The local Linux candidate is
  verified directly from its actual bytes.
- `MINISIGN_SECRET_KEY` is not configured; CI artifacts remain unsigned even
  though the public key is committed.
- Windows and macOS runtime validation is limited to native GitHub runners,
  not physical devices. The Windows runner cannot create an OpenGL 2.0+
  context, so its GUI-window smoke is SKIPPED; `--version` and `--self-test`
  still execute the real packaged `fluent.exe`. No SmartScreen or Gatekeeper
  acceptance is claimed.
- macOS artifact is arm64 only; Intel/universal are not produced.
- Windows ZIP is portable only; no installer.
- No DMG is produced; ZIP is the initial macOS format.
