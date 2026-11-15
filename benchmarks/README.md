# Benchmark corpora

## FLORES-200 sample

`flores-5x10.json` is a small sample of the FLORES-200 devtest set, licensed
**CC-BY-SA-4.0**. It contains the first five sentences for twelve language
directions (60 cases). The original archive is available from:

<https://dl.fbaipublicfiles.com/nllb/flores200_dataset.tar.gz>

The full archive is not committed. Regenerate the sample with:

```bash
python3 scripts/make-flores-corpus.py \
  --flores-dir /path/to/flores200_dataset/devtest \
  --sentences 5 --output benchmarks/flores-5x10.json
```

Attribution: FLORES-200 / NLLB team, Meta AI. See the dataset README for the
full attribution and share-alike terms.

## Fluent-oriented review corpus

`fluent-cases.json` contains original English texts written for this project
(MIT) covering conversation, technical documentation, instructions, a long
paragraph, numbers and dates, proper nouns, lists and punctuation, ambiguity,
and formal/informal phrasing. Each source is used for English→French and
English→German. References are intentionally empty; these cases are for human
review, not automatic metrics.

## Results

Generated benchmark JSON and metric outputs are intentionally **not** committed.
Curated summaries and methodology live in `docs/BENCHMARKS.md`.
