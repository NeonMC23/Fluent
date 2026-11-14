# Versioning

## Source of truth

The Rust workspace version in `Cargo.toml` is authoritative:

```toml
[workspace.package]
version = "0.2.0"
```

Both crates use `version.workspace = true`. `scripts/version.sh` reads this
single value for packaging and CI; no workflow or packaging script keeps its
own version constant.

| Consumer | How it gets the version |
|---|---|
| GUI About string | `env!("CARGO_PKG_VERSION")` |
| Linux installer desktop entry | `X-Fluent-Version` parsed from `Cargo.toml` |
| Linux archive name | `scripts/version.sh` |
| Windows executable metadata | `CARGO_PKG_VERSION` in `build.rs` |
| macOS `Info.plist` | `@VERSION@` substituted by `package-macos.sh` |
| CI artifact names | `scripts/version.sh` |
| Release manifest | explicit `--version`, generated from actual files |

## Current status

- **Last published version:** `0.1.0`.
- **Current unreleased development version:** `0.2.0`.
- **Git tag for 0.2.0:** intentionally absent. A tag is created only after all
  release gates pass and the release is authorized.
- Historical `0.1.0` references in changelog/provenance records are kept as
  history, not replaced.

## Release rule

Bump the workspace version in one commit, rebuild all platform artifacts from
that commit, verify checksums/signatures, then tag/release. Do not reuse an
artifact built from a different commit. The current mission does not publish a
release or create a tag.
