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
  tar.gz. Uploads `fluent-linux-x86_64`.
- **Windows** (`windows-latest`, x86-64): workspace tests, release GUI build
  with embedded ICO/metadata, portable ZIP. Uploads
  `fluent-windows-x86_64`.
- **macOS** (`macos-latest`, currently arm64): workspace tests, release GUI
  build, `Fluent.app` staging, app ZIP. Uploads `fluent-macos`; the filename
  records the runner architecture. Intel macOS is not separately built.
- **Release integrity** (only non-PR events): downloads all three artifacts,
  optionally signs archives with Minisign when `MINISIGN_SECRET_KEY` is
  configured, generates `release-manifest.json`, and uploads the combined
  `fluent-release-candidate` artifact.

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

The workflow is defined and YAML-validated locally. **It has not been executed
on GitHub in this cycle.** The environment used for development has no `gh`
CLI/API token, so run IDs and artifact download results are not available.
Open the Actions tab above to inspect the first run.

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
| Aggregate | `fluent-release-candidate` | all archives, `.sha256`, optional `.minisig`, `release-manifest.json` |

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
pinned `rsign2`; local users may use either. The repository currently contains
**no production public key**; `docs/keys/README.md` is the placeholder.

### Owner key setup

With Minisign:

```bash
minisign -G -p minisign.pub -s minisign.key
# Store minisign.key offline/password-manager protected. Never commit it.
# Commit only the public key as docs/keys/minisign.pub.
```

With `rsign` for a non-interactive CI key:

```bash
rsign generate -p minisign.pub -s minisign.key --unencrypted
# The unencrypted private key is only acceptable inside a protected CI secret
# or an offline signing machine.
```

Configure the GitHub Actions secret `MINISIGN_SECRET_KEY` with the contents of
`minisign.key`. The signing job only runs on non-PR events; pull requests from
forks never receive the secret. An encrypted key that prompts for a password
will not work non-interactively in CI; use a protected CI-only key or extend
the workflow with an approved password mechanism.

### Sign and verify locally

```bash
MINISIGN_SECRET_KEY_FILE=/secure/minisign.key \
  ./scripts/sign-artifacts.sh dist/fluent-*.tar.gz dist/*.zip
MINISIGN_PUBLIC_KEY_FILE=docs/keys/minisign.pub \
  ./scripts/verify-artifacts.sh dist/fluent-*.tar.gz dist/*.zip
```

CI produces `.minisig` files when the secret is present; otherwise it uploads
artifacts plus `UNSIGNED.txt` and a manifest with `"minisign_status":
"unsigned"`. No fake signatures are generated. The scripts were tested with a
throwaway `rsign` key during development; that key was deleted and is not the
production key.

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
- **Linux:** no package-level signature; Minisign covers the archive when the
  owner configures the key.

## 7. Release-candidate checklist

- [ ] Version in `Cargo.toml` and GUI About is correct (`0.2.0`).
- [ ] Linux Ubuntu 22.04 build passes and reports `GLIBC_2.35`.
- [ ] Linux installer/archive/checksum verified.
- [ ] Windows native build produced a ZIP and executable metadata.
- [ ] macOS native build produced `Fluent.app` ZIP (arm64 only currently).
- [ ] Artifact names and contents match this document.
- [ ] SHA-256 files generated from actual archives.
- [ ] Minisign public key committed and signatures verified (owner action).
- [ ] Windows Authenticode decision documented.
- [ ] macOS signing/notarization decision documented.
- [ ] Model/package/licensing notices current.
- [ ] Branding rights confirmed (not yet documented as legally cleared).
- [ ] README, CHANGELOG and VERSIONING current.
- [ ] GitHub Actions run successful and artifacts downloaded/inspected.
- [ ] Release published/tagged only after explicit owner authorization.

## 8. Current limitations

- Workflow has not executed on GitHub; compile/build results are unverified.
- No Minisign production key or signatures yet.
- No Windows/macOS runtime launch validation.
- macOS artifact is arm64 only; Intel/universal not produced.
- Windows ZIP is portable only; no installer.
- No DMG is produced; ZIP is the initial macOS format.
