# Translation Quality and Performance Benchmarks

Sections 1–9 record Cycle 14. Section 10 adds the Cycle 16 lightweight-model
comparison over all 42 directed pairs among the seven required languages.

This document records measured results on the canonical development host. It is
a snapshot, not a universal performance claim. Raw JSON and metric outputs are
kept outside Git; the corpus sample and harness are committed.

## 1. Hardware and software

| Component | Value |
|---|---|
| CPU | Intel Core i5-11400H, 6 cores / 12 threads |
| RAM | 15 GiB total; ~9.2 GiB available at benchmark start |
| Swap | 24 GiB total; ~21 GiB free |
| GPU | NVIDIA RTX 3050 Laptop, 4 GiB; **not used** |
| OS | Nobara Linux 44 (KDE), x86-64 |
| Rust | 1.99.0 |
| Candle | 0.11 |
| NLLB 1.3B | `facebook/nllb-200-1.3B`, user-supplied PyTorch checkpoint |
| NLLB 3.3B | `facebook/nllb-200-3.3B`, user-supplied sharded PyTorch checkpoint |
| MADLAD | `google/madlad400-3b-mt` revision `fa184c675da0b5c9e1c8694fccd4e12e2d422094`, q4_k GGUF |

All timings are CPU. The build has no CUDA/Metal feature enabled. MADLAD is
CPU-only by design.

## 2. Harness and methodology

Harness: `crates/fluent-core/examples/eval.rs`.

It uses the same `Engine` path as the GUI, loads one model per process, records
load time and RSS, warms up once, then records per-case latency, time to first
decoded token, source/output token counts, output text and peak RSS as JSON.

```bash
cargo build --release -p fluent-core --example eval

./target/release/examples/eval \
  --mode fast \
  --model ~/.local/share/fluent/models/nllb-200-1.3b \
  --corpus benchmarks/flores-5x10.json \
  --limit 128 --beams 4 --reps 1 \
  --out /tmp/nllb13.json
```

MADLAD uses `--mode madlad --model .../madlad400-3b-mt`. The harness ignores
the beam argument for MADLAD (greedy decoding only).

Metric summarizer: `scripts/eval-metrics.py`, using SacreBLEU 2.6.0 in a
separate venv. chrF++ uses word order 2; BLEU uses the 13a tokenizer. No COMET
was run because the environment had no compatible PyTorch/COMET runtime.

Corpora:

- `benchmarks/flores-5x10.json`: first five FLORES-200 devtest sentences for
  twelve directions (60 cases), CC-BY-SA-4.0.
- `benchmarks/fluent-cases.json`: 20 original English→French/German cases
  covering conversation, technical documentation, instructions, a long
  paragraph, numbers/dates, proper nouns, lists, ambiguity and formality.
  Reference-free; evaluated by human review.

`--warmup` runs one untimed translation before the measured cases. For the
warm subset, `--reps 3` gives run-to-run ranges.

## 3. Quality: FLORES-200 sample

Macro averages over the twelve directions (5 sentences each):

| Model | chrF++ | BLEU |
|---|---:|---:|
| NLLB 1.3B, beams=4 | 60.55 | 35.54 |
| NLLB 1.3B, beams=1 | 60.55 | 35.54 |
| MADLAD, greedy, 128-token cap | 59.74 | 30.46 |
| NLLB 3.3B Q8, beams=4 | **63.46** | **38.88** |

NLLB 1.3B beams=4 and beams=1 produced identical output on all 60 cases while
beams=1 was about 1.5× faster. That is a useful speed opportunity, not proof
that beam search is unnecessary in general.

Per-pair chrF++ / BLEU:

| Pair | NLLB 1.3B | MADLAD | NLLB 3.3B Q8 |
|---|---:|---:|---:|
| arb→eng | 69.3 / 46.2 | 65.0 / 40.5 | 74.1 / 56.6 |
| deu→eng | 60.5 / 30.7 | 57.9 / 23.2 | 61.0 / 31.1 |
| eng→arb | 53.9 / 22.6 | 54.2 / 20.4 | 55.5 / 22.0 |
| eng→deu | 59.7 / 36.1 | 57.4 / 23.6 | 58.3 / 31.9 |
| eng→fra | 73.4 / 54.7 | **73.7 / 49.7** | 68.8 / 49.6 |
| eng→jpn | 22.5 / 0.0 | 30.6 / 0.0 | 32.2 / 0.0 |
| eng→spa | 58.4 / 31.4 | 56.9 / 28.0 | 56.9 / 27.0 |
| eng→swh | 68.9 / 44.9 | **71.1 / 46.9** | 68.6 / 46.2 |
| fra→eng | 70.0 / 46.4 | 62.8 / 29.1 | **74.1 / 54.4** |
| jpn→eng | 55.2 / 25.3 | 59.7 / 35.1 | **66.1 / 43.6** |
| spa→eng | 63.6 / 36.8 | 56.7 / 21.5 | **65.8 / 41.2** |
| swh→eng | 71.2 / 51.4 | 70.8 / 47.6 | **80.1 / 63.0** |

MADLAD hit the 128-token cap in 6/60 cases with coherent but unfinished text.
Rerunning at 256 tokens reduced macro chrF++ to 58.92 and BLEU to 28.75 and
left 1/60 at the cap, so the 128-token comparison was retained. Longer output
is not automatically better output.

NLLB 3.3B at 128 tokens never hit the cap. On the 12-case f16/Q8 probe, 11/12
outputs were byte-identical; the metrics were identical to two decimals.

These are small samples. Automatic metrics are not a definitive human-quality
judgment, especially for Japanese where BLEU is 0 for all models.

## 4. Human review: Fluent-oriented corpus

Representative observations:

| Case | NLLB 1.3B | MADLAD | NLLB 3.3B Q8 |
|---|---|---|---|
| Conversation en→fr | Omitted the greeting and final question | Preserved all three sentences | Omitted the greeting and final question |
| Formal en→de | Accurate, older spelling “Ausschuß” | Accurate, complete | Accurate, complete |
| Invoice en→fr | Numbers and reference preserved | Preserved but added a receipt/closing sentence | Numbers and reference preserved |
| Proper nouns en→fr | Omitted the Nobel Prize sentence | Complete; “leur recherche… vaut” grammar is awkward | Complete and grammatical |
| List en→fr | Omitted the second sentence | Repeated part of the list | Omitted the second sentence |
| Ambiguous “bank” en→de | Omitted the river-bank sentence | Added “San Francisco” and boat details (hallucination) | Omitted the river-bank sentence |
| Informal en→de | Partial, dropped the repayment clause | Complete but “ein paar Kaffees” changes quantity | Partial |

Trade-off summary:

- **NLLB 1.3B** is fastest and competitive on FLORES French/Swahili, but
  frequently drops clauses or whole sentences on the review corpus.
- **NLLB 3.3B Q8** has the best FLORES chrF++/BLEU and the most reliable
  reverse-direction output, but also omits content in several review cases.
- **MADLAD** is often the most complete on conversational/technical text, but
  has the highest risk of added content and repetition. English→Japanese is
  weak for all three.

No model is globally best. The manual model selector remains appropriate.

## 5. Performance and memory

Full 60-case FLORES run, one measured repetition after warm-up:

| Model | Load | Peak RSS | Median total | Range | TTFT | Tokens/s |
|---|---:|---:|---:|---:|---:|---:|
| NLLB 1.3B beams=4 | 9.56 s | 2.29 GB | 3.29 s | 1.47–6.53 s | 0.47 s | 14.06 |
| NLLB 1.3B beams=1 | 7.21 s | 2.29 GB | 2.20 s | 0.98–4.44 s | 0.42 s | 20.95 |
| MADLAD q4_k | 1.95 s | 2.75 GB | 30.19 s | 9.84–92.59 s | 3.02 s | 1.83 |
| NLLB 3.3B Q8 | 14.83 s | 4.57 GB | 5.87 s | 2.98–11.66 s | 0.95 s | 7.82 |
| NLLB 3.3B f16, 12 cases | 15.56 s | 6.54 GB | 26.63 s | 20.58–29.21 s | 2.37 s | 1.05 |

Warm subset (5 representative cases, 3 repetitions):

| Model | Median total | Range | TTFT | Tokens/s |
|---|---:|---:|---:|---:|
| NLLB 1.3B beams=4 | 1.79 s | 1.26–1.99 s | 0.32 s | 15.43 |
| MADLAD q4_k | 13.37 s | 11.52–17.79 s | 2.00 s | 2.18 |
| NLLB 3.3B Q8 | 3.76 s | 2.99–4.03 s | 0.71 s | 7.72 |

Model storage:

- NLLB 1.3B: 5.2 GB PyTorch checkpoint; ~1.6 GB RSS after Q8 load.
- NLLB 3.3B: 17.5 GB sharded checkpoint; ~4.6 GB peak under Q8.
- MADLAD q4_k: 1.70 GB including tokenizer; ~2.7 GB peak.

GPU/VRAM: 0 MiB used by Fluent. The RTX 3050 was present but the benchmarks
used CPU backends only.

## 6. Long-generation reliability

Test-only EOS suppression is used so the decoder cannot finish early; this
exercises the production generation loop, not a mock.

| Test | Result |
|---|---|
| MADLAD finite 520 tokens | PASS — 520 tokens in 124.0 s, peak 2.98 GB |
| MADLAD unlimited, cancel after 515 | PASS — cancelled in 124.5 s, peak 3.22 GB |
| Translation immediately after cancellation | PASS — “The library opens at eight.” |
| Drop and reload model | PASS — reload translated correctly |
| NLLB 1.3B 600 tokens | PASS |
| NLLB 1.3B unlimited cancel after 520 | PASS |

No artificial 512-token ceiling was observed. Memory did not approach the
machine limit; further growth was stopped at 520 tokens to keep the desktop
safe. Allocation-failure behavior was not deliberately induced.

After dropping the MADLAD model, process RSS remained ~2.25 GB; this is
allocator retention, not proof that live model tensors remain. A reload
succeeded.

## 7. Optimization: CPU auto-precision for NLLB 3.3B

**Change:** `Precision::Auto` on CPU now selects the Q8 path for both NLLB
sizes instead of f16 for models with `d_model >= 2048`. GPU auto still selects
f16. Explicit F32/F16 selections are unchanged.

Before/after on the 12-case FLORES probe:

| Metric | Auto f16 (before) | Auto Q8 (after) |
|---|---:|---:|
| Load | 15.56 s | 14.28–14.83 s |
| Peak RSS | 6.54 GB | 4.57 GB |
| Median latency | 26.63 s | 3.57 s |
| Tokens/s | 1.05 | 7.77 |
| Output match | — | 11/12 byte-identical; metrics equal |

This is a 7.5× latency improvement and ~2 GB lower peak RSS with no measured
quality regression on the probe. The 60-case Q8 macro chrF++ is 63.46, the
best of the tested models.

A preview-decoding throttle in MADLAD was also tested and rejected: one
128-token case took 81.6 s versus 78.0 s before, within run noise, so the
change was reverted. No speculative SIMD or threading changes were made.

## 8. Language mapping

All 72 MADLAD entries are validated at tokenizer load; every prefix exists as
a single token. Actual inference was run on the twelve FLORES directions and
20 additional targets:

Italian, Portuguese, Russian, Chinese Simplified/Traditional, Korean, Hindi,
Turkish, Vietnamese, Indonesian, Thai, Polish, Dutch, Ukrainian, Persian,
Hebrew, Tagalog, Albanian, Latvian, Azerbaijani.

All produced non-empty, script-appropriate output. This proves tokenizer and
prefix compatibility, not uniformly high quality. Alias mappings include
`tgl_Latn→<2fil>`, `als_Latn→<2sq>`, `zsm_Latn→<2ms>`, `lvs_Latn→<2lv>`,
`azj_Latn→<2az>`, and `nob_Latn→<2no>`.

NLLB retains its full 204-language list; MADLAD exposes only the curated 72.
Unsupported selections are explicitly rejected and the GUI filters the picker.

## 9. Limitations

- Five sentences per pair is a small quality sample; results are directional.
- No COMET or human-expert scoring was performed.
- No GPU backend was used or benchmarked.
- MADLAD real inference was not run on Windows/macOS.
- The 72 mappings are tokenizer-verified; not all have expert-reviewed quality.
- Allocation failure and true memory exhaustion were not induced.
- Long-generation tests used test-only EOS suppression to reach >512 tokens.
- Allocator-retained RSS after unload is not live model state.
- The manual model selector is intentionally unchanged; no automatic routing
  was added.

## 10. Cycle 16: SMaLL-100 and all 42 directed pairs

### 10.1 Corpus and method

- Corpus: `benchmarks/flores-7x7.json`, CC-BY-SA-4.0, derived from the first
  three FLORES-200 devtest sentences for each of the **42 directed pairs** among
  `eng_Latn`, `fra_Latn`, `deu_Latn`, `spa_Latn`, `por_Latn`, `ita_Latn` and
  `zho_Hans` (126 cases). The archive is not committed.
- Harness: `crates/fluent-core/examples/eval.rs`, one model per process, same
  `Engine` path as the GUI, one warm-up translation, one measured repetition,
  output ceiling 256 tokens, source limit 512 tokens.
- Beams: 4 for NLLB and SMaLL-100; MADLAD is greedy by design (effective
  beams = 1).
- Metrics: SacreBLEU 2.6.0, chrF++ (word order 2) and BLEU (13a). Macro values
  are the unweighted mean of the 42 pair scores.
- Host: Intel Core i5-11400H (6C/12T), 15 GiB RAM, 24 GiB swap, Nobara Linux
  44, CPU only, Rust 1.99.0, Candle 0.11.

### 10.2 Quality

Primary comparison: one sentence per pair (42 cases), identical case ids for
every model. MADLAD was measured on this sample because its CPU cost makes the
three-sentence run impractical on this host.

| Model | Parameters | Download | chrF++ (macro) | BLEU (macro) | Cases |
|---|---:|---:|---:|---:|---:|
| SMaLL-100, beams=4 | 330M | 1.34 GB | 43.35 | 18.64 | 42 |
| NLLB-200 1.3B, beams=4 | 1.3B | 5.50 GB | 44.40 | 20.79 | 42 |
| NLLB-200 3.3B Q8, beams=4 | 3.3B | 17.60 GB | **47.56** | **24.28** | 42 |
| MADLAD-400 3B q4_k, greedy | 3B | 1.68 GB | **47.65** | 22.50 | 42 |

Extended sample: three sentences per pair (126 cases) for the three models that
could complete it in reasonable time.

| Model | chrF++ (macro) | BLEU (macro) | Cases |
|---|---:|---:|---:|
| SMaLL-100, beams=4 | 50.06 | 24.60 | 126 |
| NLLB-200 1.3B, beams=4 | 51.04 | 27.61 | 126 |
| NLLB-200 3.3B Q8, beams=4 | **53.83** | **30.80** | 126 |

Differences of a few chrF++ points are within noise at this sample size. The
per-pair table is the useful view.

### 10.3 Performance

Same 42-case sample. All timings are CPU-only.

| Model | Load | RSS after load | Peak RSS | Median latency | TTFT (median) | Generated tok/s | Median output |
|---|---:|---:|---:|---:|---:|---:|---:|
| SMaLL-100 | **0.96 s** | **0.42 GB** | **1.67 GB** | **0.44 s** | **98 ms** | **59.9** | 27 tok |
| NLLB-200 1.3B | 10.46 s | 1.63 GB | 2.29 GB | 2.08 s | 406 ms | 12.0 | 25 tok |
| NLLB-200 3.3B Q8 | 19.08 s | 3.75 GB | 4.57 GB | 4.43 s | 852 ms | 6.1 | 27 tok |
| MADLAD-400 3B q4_k | 1.19 s | 2.68 GB | 2.72 GB | 16.87 s | 2.91 s | 1.8 | 31 tok |

Extended 126-case performance (three models):

| Model | Median latency | p95 latency | TTFT (median) | Generated tok/s |
|---|---:|---:|---:|---:|
| SMaLL-100 | 0.67 s | 1.05 s | 122 ms | 61.6 |
| NLLB-200 1.3B | 3.10 s | 5.19 s | 484 ms | 12.2 |
| NLLB-200 3.3B Q8 | 6.01 s | 10.31 s | 998 ms | 6.2 |

Latency is the per-case wall time including tokenization, encoding and beam
search. TTFT is the first decoding-progress update, not a hardware-level token
timestamp. Peak RSS includes checkpoint mapping and Q8 conversion.

### 10.4 Per-pair chrF++

42-case common sample:

| Pair | SMaLL-100 | NLLB 1.3B | NLLB 3.3B | MADLAD |
|---|---:|---:|---:|---:|
| deu→eng | 34.4 | 49.5 | 49.5 | 45.4 |
| deu→fra | 47.8 | 47.9 | 47.9 | 51.4 |
| deu→ita | 54.6 | 55.1 | 68.7 | 49.6 |
| deu→por | 39.6 | 33.7 | 39.6 | 40.3 |
| deu→spa | 29.6 | 29.6 | 40.0 | 34.6 |
| deu→zho | 22.3 | 18.8 | 17.8 | 26.2 |
| eng→deu | 45.0 | 53.7 | 45.0 | 52.4 |
| eng→fra | 57.7 | 63.9 | 63.9 | 64.7 |
| eng→ita | 49.7 | 54.9 | 55.0 | 41.6 |
| eng→por | 58.3 | 72.6 | 66.6 | 63.6 |
| eng→spa | 38.0 | 50.8 | 43.4 | 51.5 |
| eng→zho | 16.3 | 18.8 | 9.7 | 22.1 |
| fra→deu | 44.6 | 53.2 | 52.2 | 52.2 |
| fra→eng | 46.6 | 55.2 | 64.0 | 64.9 |
| fra→ita | 54.5 | 57.6 | 56.0 | 57.6 |
| fra→por | 50.4 | 47.1 | 56.2 | 56.8 |
| fra→spa | 36.5 | 31.6 | 40.6 | 40.6 |
| fra→zho | 21.8 | 18.8 | 9.7 | 40.6 |
| ita→deu | 65.3 | 56.4 | 56.8 | 55.5 |
| ita→eng | 41.3 | 46.3 | 59.6 | 48.9 |
| ita→fra | 53.0 | 52.1 | 50.2 | 62.1 |
| ita→por | 50.0 | 42.2 | 49.4 | 44.7 |
| ita→spa | 56.6 | 49.9 | 51.5 | 56.6 |
| ita→zho | 12.4 | 6.7 | 8.5 | 17.1 |
| por→deu | 46.1 | 48.7 | 53.6 | 53.4 |
| por→eng | 43.5 | 55.0 | 67.3 | 63.4 |
| por→fra | 65.2 | 61.3 | 68.0 | 67.7 |
| por→ita | 63.1 | 56.8 | 55.0 | 63.1 |
| por→spa | 45.8 | 41.6 | 43.0 | 51.5 |
| por→zho | 14.4 | 7.8 | 17.8 | 16.9 |
| spa→deu | 61.0 | 54.1 | 61.8 | 55.5 |
| spa→eng | 50.9 | 58.7 | 57.1 | 55.4 |
| spa→fra | 58.7 | 60.3 | 54.3 | 61.4 |
| spa→ita | 80.8 | 86.9 | 91.0 | 82.9 |
| spa→por | 62.9 | 53.6 | 63.4 | 57.0 |
| spa→zho | 10.6 | 6.5 | 12.0 | 17.2 |
| zho→deu | 41.5 | 41.7 | 53.5 | 44.9 |
| zho→eng | 27.8 | 36.9 | 37.9 | 33.5 |
| zho→fra | 34.3 | 32.6 | 38.8 | 40.1 |
| zho→ita | 28.8 | 25.1 | 50.3 | 39.0 |
| zho→por | 26.0 | 42.8 | 40.0 | 23.5 |
| zho→spa | 33.4 | 28.1 | 30.5 | 33.8 |

These are single-sentence scores and therefore noisy at the pair level; the
macro figures above are the reliable summary. The 126-case run gave a smoother
picture (SMaLL 50.06, NLLB 1.3B 51.04, NLLB 3.3B 53.83).


### 10.5 Stability and cancellation

- SMaLL-100: two identical requests produced identical text and token counts;
  unload/reload reproduced the same output. Cancellation during generation was
  observed within ~100 ms.
- The NLLB and MADLAD lifecycle, cancellation and long-generation tests from
  Cycle 14 remain unchanged and pass.

### 10.6 Interpretation

- SMaLL-100 is dramatically lighter (1.34 GB, 0.42 GB RSS after load) and
  ~4.7x faster than NLLB 1.3B on this CPU (0.44 s vs 2.08 s median on the
  42-case sample), with macro chrF++ about one point lower (43.35 vs 44.40;
  50.06 vs 51.04 on 126 cases).
- SMaLL-100 is ~38x faster than MADLAD on the same 42 cases (0.44 s vs 16.87 s
  median) and uses ~6x less resident memory, at a cost of ~4.3 chrF++ points.
- NLLB 3.3B and MADLAD are effectively tied on chrF++ in this sample (47.56 vs
  47.65); NLLB 3.3B has the higher BLEU and is ~3.8x faster than MADLAD. NLLB
  3.3B remains the quality ceiling but costs 17.6 GB and 3.75 GB RSS.
- On the larger 126-case sample, SMaLL-100 is notably stronger than NLLB 1.3B
  on Chinese→English and English→Chinese, and weaker on Romance→English.
- The 42-case per-pair differences are noisy; use the macro figures and the
  126-case sample for decisions.

### 10.7 Limitations

- The primary comparison uses one sentence per pair (42 cases); the extended
  sample uses three (126 cases) for the three models that could complete it.
  Neither substitutes for a full FLORES devtest evaluation.
- MADLAD's 16.9 s median makes the three-sentence run impractical on this CPU;
  its per-pair scores are single-sentence and noisy. It was run with a
  128-token ceiling; its longest output was 40 tokens, so the cap did not bind.
- One NLLB 1.3B case in the 126-case run reached the 256-token ceiling
  (`zho_Hans→eng_Latn`, sentence 2); its output is truncated.
- BLEU is 0 for Chinese targets with three references because no 4-gram
  matches; chrF++ is the more informative number there.
- No COMET, human evaluation or GPU benchmark was performed.
- Windows and macOS SMaLL-100 runtime validation was not executed; CI compiles
  the code only.
