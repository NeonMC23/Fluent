# Minisign Public Key

This directory contains the public key used for detached signatures of Fluent
distribution archives.

```text
docs/keys/minisign.pub
```

## Current key

| Field | Value |
|---|---|
| Key ID | `B504F2189BB21013` |
| SHA-256 of public key file | `bd07631dfc69be884ad2edd405fe5238bae27748f4d1ab3cbff7f1331501e713` |
| Algorithm | Ed25519 (Minisign format) |
| Status | Generated for Fluent; CI secret **not yet configured** |

The matching private key is **not** in this repository. It is kept outside the
working tree with owner-only permissions. Never commit, print, paste or upload
`minisign.key`.

## Enable signing in GitHub Actions

1. Authenticate the GitHub CLI with access to `NeonMC23/Fluent`.
2. Store the private key as the protected Actions secret
   `MINISIGN_SECRET_KEY`, reading it from the protected file through stdin:

   ```bash
   gh secret set MINISIGN_SECRET_KEY \
     --repo NeonMC23/Fluent \
     --body-file "$HOME/.config/fluent-signing/minisign.key"
   ```

   Do not use `--body` with the key on a command line, and do not paste the key
   into chat, an issue or a commit.

3. Run the workflow on `finalization/professionalization`. The
   `release-integrity` job signs every archive, verifies each signature against
   this committed public key, and generates a manifest with
   `"signatures_verified": true`.

The secret is only exposed to the non-PR `release-integrity` job. Pull requests,
including fork PRs, do not receive it.

Until the secret is configured, CI deliberately uploads **unsigned** artifacts
and writes `UNSIGNED.txt`; this repository does not fabricate signatures.

## Verify a download

```bash
MINISIGN_PUBLIC_KEY_FILE=docs/keys/minisign.pub \
  ./scripts/verify-artifacts.sh fluent-*.tar.gz fluent-*.zip
```

Or verify structure, checksums, manifest and signatures together:

```bash
python3 scripts/verify-artifacts.py \
  --dir /path/to/downloaded/artifacts \
  --version 0.2.0 \
  --public-key docs/keys/minisign.pub \
  --require-signatures
```

Omit `--require-signatures` for an unsigned build; the script will report
`UNSIGNED` rather than pretend the archive is signed.

## Rotation

If the private key is exposed: remove or replace the GitHub secret, replace
`docs/keys/minisign.pub`, publish the new key ID, re-sign every current
archive, and explicitly announce that signatures from the old key are no
longer trusted.
