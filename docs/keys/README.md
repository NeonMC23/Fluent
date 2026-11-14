# Minisign Public Key

The production Minisign **public** key belongs here once the repository owner
generates the signing keypair. No key has been generated or committed in this
cycle.

Expected files after key generation:

```text
docs/keys/minisign.pub
```

Never commit `minisign.key`. Configure the private key contents as the
protected GitHub Actions secret `MINISIGN_SECRET_KEY`; see
[../RELEASE.md](../RELEASE.md). CI installs the Minisign-compatible `rsign`
tool from the pinned `rsign2` crate because Ubuntu 22.04 does not package
`minisign`.
