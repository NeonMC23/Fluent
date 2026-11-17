# Manual GitHub Publication Procedure — Fluent v0.2.0

**This is a manual owner procedure. It has not been executed.** Publication,
tag creation and asset upload require explicit owner authorization outside this
documentation cycle.

## 1. Confirm the release source

- Recommended artifact source: `b57e6bc050ab967e0d96a3fea43266c949852923`.
  This is the source revision for all three prepared archives.
- The current documentation `main` may be later. After that revision, only
  release documentation, CI validation and verification tooling changed; no
  application source did.
- Confirm the repository is clean and synchronized:

```bash
git status
git rev-parse HEAD
git ls-remote origin refs/heads/main
```

- Confirm the intended tag/commit decision. The strictest provenance choice is
  to tag `v0.2.0` at the artifact source commit above. If the owner prefers the
  final documentation commit, record explicitly that only documentation and
  validation tooling changed after the artifact build.

## 2. Verify the local release directory

```bash
cd release-v0.2.0
sha256sum -c SHA256SUMS
minisign -Vm Fluent-v0.2.0-linux-x86_64.tar.gz -p minisign.pub
minisign -Vm Fluent-v0.2.0-windows-x86_64.zip -p minisign.pub
minisign -Vm Fluent-v0.2.0-macos-aarch64.zip -p minisign.pub
```

Expected files:

```text
Fluent-v0.2.0-linux-x86_64.tar.gz
Fluent-v0.2.0-linux-x86_64.tar.gz.minisig
Fluent-v0.2.0-windows-x86_64.zip
Fluent-v0.2.0-windows-x86_64.zip.minisig
Fluent-v0.2.0-macos-aarch64.zip
Fluent-v0.2.0-macos-aarch64.zip.minisig
SHA256SUMS
minisign.pub
```

Expected archive SHA-256 values:

| File | SHA-256 |
|---|---|
| `Fluent-v0.2.0-linux-x86_64.tar.gz` | `ebfd0750a3c1bbb01d9829798c253ba9cf38e6096f8766c9d6f2e637d691fb28` |
| `Fluent-v0.2.0-windows-x86_64.zip` | `50e6564dde071f6c28bcd9e530638f9ceecb0460cb2796d18f551ac73080bcdc` |
| `Fluent-v0.2.0-macos-aarch64.zip` | `b1ef4acadb360b9f498b5b271ef15db47bf0a010cd311665d9682df1901f0fc4` |

Do not upload a directory or a ZIP containing the release directory. Upload the
individual files.

## 3. Create the release

Only after explicit owner authorization:

1. Create and push the `v0.2.0` tag from the approved commit.
2. Open **Releases → Draft a new release**.
3. Choose tag `v0.2.0`.
4. Set the title to `Fluent v0.2.0`.
5. Copy the prepared description from
   [RELEASE_NOTES.md](RELEASE_NOTES.md).
6. Upload all three archives.
7. Upload all three `.minisig` files.
8. Upload `SHA256SUMS` and `minisign.pub`.
9. Confirm GitHub lists the exact expected filenames and sizes.
10. Do not mark the release public until the owner has completed the remaining
    branding and NLLB distribution decisions.

## 4. Verify the published assets

- Download the three uploaded archives from the draft release.
- Run `sha256sum -c SHA256SUMS`.
- Verify each signature against `minisign.pub`.
- Confirm the release description states:
  - Linux x86-64 is physically runtime-validated;
  - Windows x86-64 was validated by automated CI, not physical runtime;
  - macOS ARM64 was validated by automated CI, not physical Mac/Finder/Dock/
    Gatekeeper;
  - Windows is not Authenticode-signed;
  - macOS is not Developer ID-signed or notarized;
  - NLLB-200 is CC-BY-NC-4.0 and non-commercial.
- Do not claim signing, notarization, legal clearance or physical validation
  that has not occurred.

## 5. Remaining owner decisions

Before making the release public, the owner must confirm:

1. Branding asset redistribution rights.
2. That the intended distribution model is compatible with NLLB-200’s
   non-commercial licence.
3. Whether Windows Authenticode signing is required.
4. Whether macOS Developer ID signing and notarization are required.

No publication action is performed by this repository procedure itself.
