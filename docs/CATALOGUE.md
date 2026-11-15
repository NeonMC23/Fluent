# Remote Model Catalogue

Fluent's Model Manager can refresh a versioned JSON catalogue from the official
repository:

```text
https://raw.githubusercontent.com/NeonMC23/Fluent/main/catalogue/models-v1.json
```

The document mirrors the acquisition metadata compiled into
`crates/fluent-core/src/model/registry.rs`. It exists so descriptions, download
sizes, licence links and availability notes can be reviewed and refreshed
without shipping a new binary.

## Guarantees and boundaries

- **HTTPS only.** The endpoint and every file URL must use `https://`.
- **Small and bounded.** The document is limited to 256 KiB and the request
  uses bounded connect, response and per-read timeouts.
- **Strict schema.** Unknown fields and unknown schema versions are rejected.
- **Fail closed.** A malformed entry, unsupported backend, unsafe file name,
  invalid hash or non-HTTPS URL aborts the refresh with a clear error.
- **No arbitrary code.** The catalogue cannot execute code, load a shared
  library or introduce a backend that the binary does not implement.
- **Consistent with the build.** Every entry must agree with the compiled
  registry on id, backend, pinned revision, licence and the exact file manifest
  (name, size, SHA-256 and pinned Hugging Face URL). This prevents a remote
  document from substituting weights or changing a checksum.
- **Non-destructive failure.** A failed refresh leaves installed models and
  local metadata untouched; the Model Manager falls back to the built-in
  registry.

Adding a genuinely new model still requires a Fluent update that implements its
backend and registers it in the local registry. The catalogue is a refreshable,
tamper-evident mirror of that registry, not an executable plugin system.

## Schema version 1

```json
{
  "schema_version": 1,
  "generated_at": "2026-11-15T00:00:00Z",
  "minimum_app_version": "0.2.0",
  "models": [
    {
      "id": "small100",
      "display_name": "SMaLL-100",
      "description": "Lightweight · 98 mapped languages · fast CPU translation",
      "backend": "small100",
      "revision": "8ab680e26a596d2e3d2d2d17ae0f68df1037328c",
      "license": "MIT",
      "license_url": "https://opensource.org/licenses/MIT",
      "model_page": "https://huggingface.co/alirezamsh/small100",
      "deprecated": false,
      "total_size": 1337109573,
      "languages": ["eng_Latn", "fra_Latn", "deu_Latn", "zho_Hans"],
      "files": [
        {
          "name": "model.safetensors",
          "size": 1330973772,
          "sha256": "dd3b845a36ea4ed90437fd0b9b477e30c21f144d3658679fd5c945e3c96b0fbc",
          "url": "https://huggingface.co/alirezamsh/small100/resolve/8ab680e26a596d2e3d2d2d17ae0f68df1037328c/model.safetensors"
        }
      ]
    }
  ]
}
```

The example abbreviates `files`; a real entry lists every required file for the
model (the committed catalogue lists all six SMaLL-100 files).

| Field | Rules |
|---|---|
| `schema_version` | Must be `1`. |
| `minimum_app_version` | Optional; the running Fluent must be at least this version (default `0.2.0`). |
| `models[].id` | 1–64 chars, lowercase ASCII, digits, `.` and `-`; must match a built-in model. |
| `models[].backend` | `nllb200`, `t5` or `small100`. |
| `models[].revision` | 40 hexadecimal characters; must match the registry. |
| `models[].license` | Must match the registry (case-insensitive). |
| `models[].languages` | 1–204 unique Fluent/NLLB codes supported by the backend. |
| `models[].files` | 1–64 unique entries; names must be plain filenames. |
| `files[].size` | Non-zero, at most 64 GiB; must match the registry. |
| `files[].sha256` | Exactly 64 lowercase hexadecimal characters; must match the registry. |
| `files[].url` | HTTPS; must equal the pinned Hugging Face URL from the registry. |
| `models[].total_size` | Must equal the sum of `files[].size`. |
| `deprecated` | Optional boolean; shown as a badge in the Model Manager. |

Unknown top-level or per-entry fields are rejected. Omitted optional fields use
their defaults (`generated_at`, `description`, `deprecated`, links).

## Maintainer workflow

1. Update the model in `crates/fluent-core/src/model/registry.rs` (revision,
   file sizes, SHA-256 values, licence, languages).
2. Regenerate the catalogue:

   ```bash
   cargo run --release -p fluent-core --example gen_catalogue \
     > catalogue/models-v1.json
   ```

3. Run the catalogue tests, which parse the committed file and compare it with
   the registry:

   ```bash
   cargo test -p fluent-core --lib catalogue
   ```

4. Commit both the registry change and the regenerated catalogue.

A stale catalogue fails CI, so the two cannot drift silently.

## Testing the failure paths

The unit tests cover unknown schema versions, unknown backends, unsafe file
names, non-HTTPS URLs, invalid hashes, entries that disagree with the build,
unknown model ids and oversized documents. Network failures are reported as
`Error::Catalogue` without touching installed models.

For a local mirror, set `FLUENT_CATALOGUE_URL` to an HTTPS URL. HTTP and
loopback/private-IP hosts are rejected even through the override.
