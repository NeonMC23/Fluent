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
- **Cycle 11:** the workflow now extracts and runs each packaged application,
  validates archive structure/checksums and verifies signatures when present.
  The execution result and artifact metadata are recorded here after the run.

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
- [ ] Windows native build produced a ZIP and executable metadata (CI run).
- [ ] macOS native build produced `Fluent.app` ZIP (arm64 only currently).
- [ ] Artifact names, contents and executable architectures verified by CI.
- [ ] SHA-256 files generated from actual archives.
- [x] Minisign public key committed (`B504F2189BB21013`).
- [ ] `MINISIGN_SECRET_KEY` configured and signatures verified (owner action).
- [ ] Windows Authenticode decision documented.
- [ ] macOS signing/notarization decision documented.
- [ ] Model/package/licensing notices current.
- [ ] Branding rights confirmed (not yet documented as legally cleared).
- [x] README, CHANGELOG and VERSIONING current.
- [ ] Current cycle CI run successful and artifacts independently downloaded.
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
