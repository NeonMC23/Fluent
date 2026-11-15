# Translation Quality and Performance Benchmarks (Cycle 14)

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
