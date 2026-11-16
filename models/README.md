# Model assets are not stored in this repository

Fluent's MIT license covers the application source only. Model weights,
tokenizers and model configuration are third-party assets with their own
licences and are deliberately not committed here.

The native **Model Manager** downloads and verifies the six supported models
after explicit confirmation. The default subdirectories are:

```text
models/
├── nllb-200-1.3b/            NLLB-200 1.3B       CC-BY-NC-4.0
├── nllb-200-3.3b/            NLLB-200 3.3B       CC-BY-NC-4.0
├── madlad400-3b-mt/          MADLAD-400 3B       Apache-2.0
├── small100/                 SMaLL-100            MIT
├── qwen2.5-1.5b-instruct/    Qwen2.5 1.5B chat   Apache-2.0
└── qwen2.5-0.5b-instruct/    Qwen2.5 0.5B chat   Apache-2.0
```

For a manual NLLB installation, a model directory needs `config.json`,
`tokenizer.json` and a complete checkpoint (`pytorch_model.bin`,
`model.safetensors`, or complete shards plus an index). Extra files from a
Hugging Face download can remain; they are ignored.

See [docs/MODEL_PACKAGE.md](../docs/MODEL_PACKAGE.md) for the exact NLLB
contract, [docs/MODEL_MANAGER.md](../docs/MODEL_MANAGER.md) for download and
storage behaviour, [docs/MODEL_PROVENANCE.md](../docs/MODEL_PROVENANCE.md) for
licensing, and [docs/LINUX.md](../docs/LINUX.md) for installation.

Do not commit model weights or tokenizer files to this repository.
