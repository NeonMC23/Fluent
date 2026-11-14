# Model assets are not stored in this repository

Fluent's MIT license covers the application source only. The NLLB-200 weights,
tokenizer, and configuration files are third-party assets under
**CC-BY-NC-4.0** and are deliberately not committed here.

Place each model in its own directory:

```text
models/
├── nllb-200-1.3b/
│   ├── config.json
│   ├── tokenizer.json
│   └── pytorch_model.bin        (or model.safetensors / sharded checkpoint)
└── nllb-200-3.3b/
    ├── config.json
    ├── tokenizer.json
    └── pytorch_model-*-of-*.bin + pytorch_model.bin.index.json
```

Only `config.json`, `tokenizer.json`, and a complete checkpoint are required by
the engine. Extra files from a Hugging Face download can remain; they are
ignored. See [docs/MODEL_PACKAGE.md](../docs/MODEL_PACKAGE.md) for the exact
contract, [docs/MODEL_PROVENANCE.md](../docs/MODEL_PROVENANCE.md) for licensing,
and [docs/LINUX.md](../docs/LINUX.md) for installation.

Do not commit model weights or tokenizer files to this repository.
