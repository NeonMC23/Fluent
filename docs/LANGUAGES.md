# Language Support and Capability Registry

Fluent does not assume that every model speaks the same languages. The source
and target selectors are filtered by the selected model, and the engine rejects
an unsupported code before it reaches a backend.

## 1. Single source of truth

`crates/fluent-core/src/model/registry.rs` owns the capability API:

| Function | Purpose |
|---|---|
| `supports_language(mode, code)` | Whether the selected backend can represent this Fluent code. |
| `language_list(mode)` | The exact language list the GUI and CLI should expose. |
| `resolve_language_selection(mode, source, target)` | Clamps a saved/selected pair to the model, with an `adjusted` flag. |

The model-specific pieces are:

- NLLB: the Fluent/NLLB table in `crates/fluent-core/src/languages.tsv`, loaded
  by `languages.rs`.
- SMaLL-100: `LANGUAGES` in `crates/fluent-core/src/model/small100.rs`, mapping
  Fluent/NLLB codes to M2M-100 short codes and `__xx__` tokens.
- MADLAD: `MADLAD_LANGUAGES` in `registry.rs`, mapping Fluent codes to `<2xx>`
  prefixes.

The GUI (`app.rs`), Model Manager and catalogue all consume these functions.
There is no independent language list in the interface.

## 2. Actual coverage

| Model | Fluent codes | Backend identifier | Chinese variants | Decoding |
|---|---:|---|---|---|
| NLLB-200 1.3B | **202** | NLLB `xxx_Yyyy` tokens in `tokenizer.json` | `zho_Hans`, `zho_Hant`, `yue_Hant` | beam search |
| NLLB-200 3.3B | **202** | same tokenizer as 1.3B | `zho_Hans`, `zho_Hant`, `yue_Hant` | beam search |
| MADLAD-400 3B | **72** | `<2xx>` target prefix | `zho_Hans` (`<2zh>`), `zho_Hant` (`<2zh_Hant>`) | greedy |
| SMaLL-100 | **98** | `__xx__` target token | `zho_Hans` (`__zh__`) only | beam search |

### NLLB-200 correction (Cycle 17)

The previous table contained 204 codes, but the NLLB-200 tokenizer has 202
language tokens. Three codes were not present:

- `arb_Latn` (Romanized Arabic) — not in NLLB-200.
- `min_Arab` (Minangkabau in Arabic script) — not in NLLB-200.
- `sat_Olck` (Santali in Ol Chiki) — not in NLLB-200.

The tokenizer does contain `sat_Beng` (Santali in Bengali script). The table now
matches the tokenizer exactly: 202 codes. `NllbTokenizer::from_dir` validates
all 202 at load and fails with an actionable error if an installed tokenizer is
incomplete.

This is exactly the kind of mismatch the capability registry is meant to
prevent: the old UI could offer three languages that the model could not
translate.

### SMaLL-100

98 of the upstream 100 codes are mapped. Breton (`br`) and Western Frisian
(`fy`) are not in NLLB's language table and are not offered. `__zh__` is a
single upstream token; Fluent maps it to `zho_Hans` and does not claim
`zho_Hant`.

### MADLAD-400 3B

72 curated mappings whose `<2xx>` prefixes exist as single tokens in the
official tokenizer. Both Chinese scripts are mapped. The tokenizer is validated
at load: every advertised prefix must be a single token.

## 3. Required-language mappings

| Language | Fluent code | NLLB token | SMaLL token | MADLAD prefix |
|---|---|---|---|---|
| English | `eng_Latn` | `eng_Latn` | `__en__` | `<2en>` |
| French | `fra_Latn` | `fra_Latn` | `__fr__` | `<2fr>` |
| German | `deu_Latn` | `deu_Latn` | `__de__` | `<2de>` |
| Spanish | `spa_Latn` | `spa_Latn` | `__es__` | `<2es>` |
| Portuguese | `por_Latn` | `por_Latn` | `__pt__` | `<2pt>` |
| Italian | `ita_Latn` | `ita_Latn` | `__it__` | `<2it>` |
| Chinese (Simplified) | `zho_Hans` | `zho_Hans` | `__zh__` | `<2zh>` |
| Chinese (Traditional) | `zho_Hant` | `zho_Hant` | **not supported** | `<2zh_Hant>` |

`Mode::supports_language` is the only check used by the selector and the
engine. The engine's `translate` method returns `Error::UnknownLanguage` for an
unsupported explicit source or target before loading a model.

## 4. Selector behaviour

- Opening the source or target picker uses `language_list(selected_mode)`.
- Switching models calls `resolve_language_selection`:
  - a supported selection is preserved;
  - an unsupported explicit source becomes `auto`;
  - an unsupported target becomes the first supported code in a stable order
    (`eng_Latn`, `fra_Latn`, `deu_Latn`, `spa_Latn`, `zho_Hans`, then the first
    available language);
  - the change is announced in the status area and persisted.
- A settings file that outlives a model change is clamped on startup and the
  adjustment is announced.
- `auto` is always a valid source. SMaLL-100 and MADLAD condition on the target
  language, so a detected source outside their list does not break inference;
  NLLB detection only returns codes from its 202-language table.

All four backends translate every supported source/target pair in both
directions. No asymmetric pair restrictions are implemented or claimed.

## 5. Display names

The selector and pickers show human-readable names and native names. Internal
codes are no longer displayed in the language button or picker rows. Search
still matches the code, so maintainers can type `zho_Hant` if needed.

| Model | Selector name | Secondary description |
|---|---|---|
| SMaLL-100 | **SMaLL-100** | Lightweight and fast |
| NLLB-200 1.3B | **NLLB-200 1.3B** | Balanced CPU translation |
| NLLB-200 3.3B | **NLLB-200 3.3B** | Higher-quality, resource-intensive translation |
| MADLAD-400 3B | **MADLAD-400 3B** | Alternative multilingual translation model |

Internal identifiers (`fast`, `accurate`, `madlad`, `small`), configuration keys
and model directories are unchanged for compatibility.

## 6. Adding a language

1. Confirm the code exists in the model's tokenizer/prefix table.
2. Add or update the mapping in the appropriate model-specific table.
3. Keep the Fluent/NLLB code as the canonical identifier.
4. Add the display name and native name to `languages.tsv` if it is a new
   Fluent language.
5. Run the capability tests:

   ```bash
   cargo test -p fluent-core --lib registry
   cargo test -p fluent-core --test language_coverage
   FLUENT_TEST_MODELS_ROOT="$HOME/.local/share/fluent/models" \
     cargo test --release -p fluent-core --test language_coverage
   ```

The last command loads the installed tokenizers and checks every advertised
code end to end.

## 7. Tests

- `registry.rs`: counts, uniqueness, required languages, Chinese variants,
  model-switch clamping, unknown-code fallback.
- `tests/language_coverage.rs`: static lists, Chinese behaviour, SMaLL mapping
  uniqueness, and gated real-tokenizer coverage for all four backends.
- `engine.rs`: unsupported source/target rejected before a model load.
- GUI: picker filtering by selected model and the startup clamp.
- Real inference: `tests/small100.rs`, `tests/madlad.rs`,
  `tests/reliability.rs`, plus the Cycle 16 42-pair benchmark.
