# Fluent CPU Performance — Cycle 5 (historical)

> **Historical snapshot.** This document records the Cycle 5 optimisation work
> for NLLB-200 1.3B on CPU. It is retained as engineering history. Current
> translation quality/performance is in [docs/BENCHMARKS.md](docs/BENCHMARKS.md);
> current chat performance and limits are in [docs/CHAT.md](docs/CHAT.md).

Cycle 5 attacked the two remaining CPU bottlenecks identified in Cycle 4: the
quantized encoder/prefill path and the per-step logits normalization. Both
changes are arithmetic-preserving: the new prefill kernel is bit-identical to
the previous Q8 kernel, and the logits path reproduces candle's `log_softmax`
bit for bit. That let the cycle optimize aggressively without buying speed by
changing translation behaviour.

## 1. Summary

- **Cache-tiled Q8 prefill: encoder −24% on a 267-token source.** The old
  row-at-a-time kernel rescanned the whole activation matrix once per output
  row. Processing eight input rows per weight traversal keeps the activation
  tile in L1 and removes that traffic. Interleaved A/B: **2,149 ms → 1,622 ms**
  (median of four rounds). Kernel microbenchmark at 8192×1024, m = 267:
  **30.2 ms → 19.7 ms per call** (median of five interleaved runs).
- **On-demand logits normalization: search −25% and logits −38%.** Beam search
  now stores raw logits plus one max and one log-sum-exp per row, and computes
  a log-softmax value only for candidates it actually visits. Interleaved A/B:
  search **1,579 ms → 1,190 ms**, logits **1,365 ms → 849 ms** per 200-token
  run. The values are bit-identical to candle's reference path, including
  infinities and NaNs.
- **200-token workload: 14.28 s** in the final three-repetition run
  (14.27–14.28 s), **14.01 tok/s**, 71.4 ms/token. The same binary measured
  16.63 s earlier in the session and the start-of-session baseline measured
  19.55 s (17.63–19.80 s): machine state moves the number by more than either
  code change. The interleaved A/B is the controlled comparison and attributes
  ~1.0 s to the changes (encoder −0.52 s, search −0.52 s); the rest is
  run-to-run conditions.
- **Quality: 21/21 corpus cases token-identical between Q8 and f32**, 17/17
  overlapping cases identical to the pre-change Q8 outputs. Scalar vs AVX2 is
  17/18: one new punctuation fixture picks a synonym (see §8), a pre-existing
  reduction-order difference between the two kernels, not a tiling effect.
- **Rejected/deferred:** cross-block integer accumulation (mathematically
  invalid with per-block scales), AVX-512 VNNI (would break bit-identity;
  documented as a Cycle 6 candidate), and running the 3.3B model (f32
  checkpoint exceeds available RAM).
- Lifecycle, repeatability and cancellation behaviour is unchanged and was
  re-verified.

## 2. Environment

| Item | Value |
|---|---|
| CPU | Intel Core i5-11400H (6 cores / 12 threads, 2.70 GHz base, 12 MB L3) |
| RAM | 15,757 MB (25 GB swap configured, heavily used by the shared host) |
| OS | Nobara Linux 44 (KDE Plasma), kernel 7.3.1-202.nobara.fc44.x86_64 |
| Rust | rustc 1.99.0 / cargo 1.99.0 |
| Inference stack | Candle 0.11.0 (default build, no global `target-feature`) |
| CPU features | AVX2, FMA, AVX-512F/DQ/BW/VL/VNNI detected; only AVX2/FMA used |
| Model | NLLB-200 1.3B, f32 checkpoint, 24 encoder + 24 decoder layers, d_model 1024, FFN 8192, 16 heads, vocabulary 256,206 |
| Governor | `powersave` (unchanged); benchmarks pinned with `taskset -c 0-11` |
| Repository | `/tmp/opencode/fluent-cycle2`, branch `finalization/cycle-5`, based on `4e31b45` |
| Model path | `/home/neon/Documents/CODE LOCAL/Fluent/models/nllb-200-1.3b` (read-only) |

The host is shared and the governor is `powersave`; absolute timings move by
more than 20% between sessions, and the load average ranged from 2 to 10 during
this cycle. All before/after claims therefore come from interleaved runs of the
same session. Only stage deltas that appear in the interleaved A/B are
attributed to code.

## 3. Method and harness

The harness is `crates/fluent-core/examples/bench.rs`; it was not changed in
this cycle. It separates model load from generation and reports encoder,
encoder-attention, encoder-FFN, cross-cache preparation, prefill, decoder,
LM-head, search, log-its, top-k, reorder and total generation timings, plus RSS
and token ids for exact comparisons.

Cycle 5 used three comparison modes:

1. **Kernel A/B**: one binary, `FLUENT_QUANT_LEGACY=1` temporarily selected the
   row-at-a-time kernel. This switch existed only while measuring and was
   removed from the final tree.
2. **Search A/B**: the Cycle 4 binary and the Cycle 5 binary were run
   alternately (old, new, old, new) with identical inputs, budgets and
   repetitions.
3. **Final benchmark**: three repetitions of the full corpus, used for absolute
   tables. Ranges are min–max across repetitions.

Every comparison kept the model, source text, beam count (4), early-stopping
policy and token budget identical. Model load was excluded from generation
timings. Raw JSON and logs stayed outside Git.

## 4. Where the time went

200 generated tokens from a 267-token source, Q8, median of 3:

| Stage | Pre-change | Final | Interleaved A/B delta |
|---|---:|---:|---:|
| Tokenization | 0.5 ms | 0.5 ms | unchanged |
| Encoder | 2,188 ms | 1,512 ms | **−24.6%** (2,149 → 1,622) |
| └ encoder attention | 617 ms | 441 ms | included above |
| └ encoder FFN | 1,567 ms | 1,067 ms | included above |
| Cross-attention cache preparation | 236 ms | 177 ms | not measured separately |
| Prefill (start + language token) | 79 ms | 75 ms | unchanged code |
| Decoder (24 layers × 200 steps) | 12,851 ms | 9,432 ms | +0.1% in A/B |
| LM head | 2,589 ms | 2,135 ms | +2.5% in A/B |
| Beam search | 1,609 ms | 949 ms | **−24.6%** (1,579 → 1,190) |
| └ logits normalization + copy | 1,399 ms | 734 ms | **−37.9%** (1,365 → 849) |
| └ top-k | 212 ms | 219 ms | scan overhead, see §6 |
| Beam cache reorder | 3.5 ms | 2.9 ms | unchanged |
| **Generation total** | **19,550 ms** | **14,278 ms** | −27.0% |

The final run is faster in *every* stage, including unchanged code: decoder
−26.6% and LM head −17.5% relative to the start-of-session baseline, while the
interleaved A/B shows decoder +0.1% and LM head +2.5%. That is machine drift,
not a Cycle 5 result. The attributable gain is ~520 ms from the encoder and
~520 ms from search; the rest of the 5.3 s total difference is run-to-run
conditions. An earlier final run of the same binary, before the host sped up,
measured 16.63 s with the same per-stage attribution.

## 5. Tiled Q8 prefill (kept)

### Previous kernel

`run_large_avx2` and `run_large_scalar` process one output row at a time. For
each output row they scan every input row and every block:

```text
for output_row:
    for input_row:
        for block:
            dot(weight[output_row, block], activation[input_row, block])
```

The weight row (1 KiB) stays in L1 across the input loop, but the activation
matrix (m × 4 KiB) is re-read for every output row. At m = 267 and 8192 output
rows that is ~8.7 GiB of activation traffic per projection, served mostly from
L2/L3. The kernel was therefore cache-traffic bound, not integer-arithmetic
bound. Cycle 4's experiment EB tried to amortize weight-block decoding across
input rows while keeping this loop order; that did not reduce activation
traffic, which is why it produced no reliable gain.

### New kernel

The prefill path is blocked in both dimensions:

```text
for tile of 8 input rows:
    for output row:
        for block:
            decode weight block once
            for input in tile:
                dot(weight_block, activation[input, block])
```

- The 8 × 1024 f32 activation tile is 32 KiB and stays in L1 while the output
  row's quantized weights stream past. Weight traffic is one row per output row
  per tile; activations are re-read from L1 instead of L2/L3.
- The weight block is widened once and reused by all eight input rows. Across a
  tile that removes the repeated weight widening: 32 `vpmovsxbw` conversions
  become 18 (2 for the weight + 2 per input row) per block.
- Work is parallelized over `(input tile, output chunk)` pairs. Each task
  writes a disjoint `(input, output)` rectangle, so the existing unsafe
  disjoint-write contract is preserved. The partial final tile (0–7 inputs)
  falls back to the row-at-a-time kernel.
- Accumulators are stack-resident ymm registers; no per-call allocation was
  added and peak memory is unchanged.

### Integer and accumulator safety

- Weights and activations are quantized to `[-127, 127]`.
- A block dot product is at most `32 × 127 × 127 = 516,128`, inside `i32`.
- `_mm256_madd_epi16` sums two products into an `i32` lane; the largest pair
  value is `2 × 127 × 127 = 32,258`, and the two madd result vectors sum to at
  most 516,128. No `i16` accumulator exists, so no saturation or wrapping.
- The kernel applies each block's `weight_scale × activation_scale` in f32 and
  accumulates the scaled results in f32. It deliberately does **not**
  accumulate unscaled block integers across blocks: every block has its own
  scale, so cross-block integer accumulation would be mathematically invalid.
  For reference, even a hypothetical full-row integer accumulation at the
  widest NLLB matrix (8192 columns) would be at most
  `8192 × 127 × 127 = 132,128,768 < i32::MAX`, but the implementation does not
  rely on that.
- No rounding mode, saturation, NaN or infinity behaviour was changed: the
  per-block order, conversion and scale multiply are identical to the previous
  kernel.

### Correctness tests

`crates/fluent-core/src/model/quant.rs`:

- `tiled_prefill_is_bit_identical_to_the_reference_kernel` compares every output
  bit against the row-at-a-time kernel for rows 5/23/37/41, columns 64/96/128,
  and m = 1, 3, 7, 8, 9, 15, 16, 17, 23, 31, 32, 33, 67, on both the AVX2 and
  scalar paths. This covers full tiles, partial tiles, a single row, chunks
  smaller than the thread count, and non-multiple-of-tile widths.
- `tiled_prefill_handles_extreme_magnitudes_bit_identically` uses values from
  ±1e-30 to ±1e30, ±127, zero and mixed signs. Results remain bit-identical,
  including f32 infinities where the scales overflow; that is the existing f32
  behaviour.
- `integer_dot_products_fit_in_i32_for_supported_dimensions` pins the bound
  analysis above as an executable assertion.

### Measurements

Kernel microbenchmark, 8192×1024, five interleaved runs (same binary):

| m | Row-at-a-time median | Tiled median | Speedup |
|---:|---:|---:|---:|
| 128 | 14.0 ms | 9.6 ms | 1.44× |
| 267 | 30.2 ms | 19.7 ms | 1.53× |

1024×1024 at m = 267: 5.29 ms → 3.18 ms (1.66×). Tiled throughput at
8192×1024, m = 267 reached 217–269 GFLOP/s across runs (up from 133–173).

In-situ interleaved A/B, four rounds, 200-token generation, 267-token source:

| Round | Row-at-a-time encoder | Tiled encoder |
|---|---:|---:|
| 1 | 2,423 ms | 1,653 ms |
| 2 | 2,214 ms | 1,837 ms |
| 3 | 2,084 ms | 1,590 ms |
| 4 | 2,080 ms | 1,582 ms |
| **Median** | **2,149 ms** | **1,622 ms** |

The encoder gain is consistent in every round and is concentrated in the FFN
(1,509–1,708 ms → 1,103–1,246 ms) and the encoder projections. Generation total
median moved from 16.81 s to 16.53 s in the same A/B because decoding dominates
and is untouched.

Memory impact: none. Load RSS 1,586 MB and peak 2,433 MB are identical to
Cycle 4; the tile adds only registers and stack accumulators.

**Decision: kept.** The row-at-a-time kernels remain as the reference used by
the exactness tests, but production `matmul` uses the tiled path.

## 6. On-demand logits normalization (kept)

### What beam search needs

For each beam row, cumulative scores are
`beam_score + log_softmax(logits)[token]`. The previous implementation asked
candle for a full `log_softmax`, materialized the `[4][256,206]` result as a
tensor, copied it into nested `Vec<f32>` rows, and then scanned each row for the
top candidates. That allocates three full-size temporaries (`diff`, `exp`,
`log_sm`) plus the copied rows and makes several passes over ~4 MiB per step.

### New path

`prepare_logits` copies the raw logits once and computes, per row, the maximum
and

```text
log_norm = ln( Σ exp(x - max) )
```

The top-k scan then evaluates `(x - max) - log_norm` only for tokens it visits.
The full normalized vocabulary is never created. The operations and their order
match candle's reference:

1. maximum of the row (starting from the first element, so an all-NaN row
   behaves the same),
2. `exp(x - max)` in element order,
3. a sequential f32 sum,
4. `ln(sum)`,
5. `(x - max) - log_norm`, then widening to f64 and adding `beam_score`
   exactly as before.

Candle's CPU reduction is a sequential f32 loop when the crate is not built
with a global `avx2` target feature; the default Fluent build is in that
configuration. The comparison tests pin the result.

### Correctness tests

`crates/fluent-core/src/model/nllb.rs`:

- `manual_log_softmax_matches_candle_bit_for_bit` compares every value of a
  random 4 × 4,096 tensor against candle, using `f32::to_bits`.
- `manual_log_softmax_matches_candle_for_adversarial_rows` covers ±1e35
  magnitudes, repeated values, one-ULP differences, `+inf`, `-inf`, `NaN`,
  `f32::MAX`, `f32::MIN`, and an all-NaN row. All 3,072 values match bit for
  bit, including NaN payloads.
- `prepare_logits_repeats_the_single_prefill_row` checks the first-step beam
  replication.
- `optimized_top_candidates_match_the_candle_reference_path` compares the full
  candidate list (token, beam, score bits) against a reference built from
  candle log-probabilities.

EOS handling, per-beam pooling, tie-breaking, early-stopping policies, length
penalty, cancellation and score accumulation were not changed.

### Measurements

Interleaved old/new binaries, 200-token run, four rounds:

| Metric | Pre-change median | Final median | Change |
|---|---:|---:|---:|
| Search total | 1,579 ms | 1,190 ms | −24.6% |
| Logits normalization + copy | 1,365 ms | 849 ms | −37.9% |

The first version regressed top-k from ~215 ms to ~347 ms because the score
expression indexed three arrays per token. Hoisting the row slice and its two
constants into locals restored top-k to 0.24–0.31 s depending on run (the final
three-repetition median was 0.31 s) without changing the arithmetic; the
intermediate version is not in the final tree.

One caveat: the bit-identity relies on candle's CPU reduction order. Building
Candle with a global `avx2` target feature changes its summation order; the
bitwise tests would fail, although the candidate decisions would still be
numerically stable. The shipped build does not use global target features.

**Decision: kept.** The gain is ~0.5 s per 200-token run (roughly 3%) with no
observed behavioural change and stronger correctness coverage than the
reference path had.

## 7. Benchmarks

### 7.1 Product-like cases (Q8, median of 3, heuristic EOS stop)

| Case | Source tokens | Generated | Median | Min | Max | tok/s | ms/token |
|---|---:|---:|---:|---:|---:|---:|---:|
| FR → EN | 13 | 7 | 0.62 s | 0.61 | 0.63 | 11.29 | 88.6 |
| EN → FR | 15 | 16 | 1.12 s | 1.10 | 1.15 | 14.29 | 70.0 |
| EN → DE | 15 | 19 | 1.27 s | 1.27 | 1.29 | 14.90 | 67.1 |
| ES → EN | 16 | 11 | 0.88 s | 0.87 | 0.90 | 12.44 | 80.4 |
| IT → EN | 20 | 20 | 1.43 s | 1.42 | 1.46 | 13.95 | 71.7 |
| EN → FR medium | 81 | 64 | 4.37 s | 4.37 | 4.38 | 14.64 | 68.3 |
| EN → FR long | 267 | 64 | 5.93 s | 5.73 | 5.99 | 10.79 | 92.7 |
| DE → EN long | 193 | 64 | 5.41 s | 5.09 | 5.46 | 11.82 | 84.6 |
| FR → EN accented | 40 | 21 | 1.59 s | 1.58 | 1.64 | 13.23 | 75.6 |
| EN → FR numbers | 25 | 27 | 1.83 s | 1.81 | 1.83 | 14.76 | 67.7 |
| EN → DE names | 26 | 27 | 1.84 s | 1.84 | 1.86 | 14.69 | 68.1 |
| ES → EN repeated | 27 | 24 | 1.67 s | 1.66 | 1.69 | 14.38 | 69.5 |
| EN → FR paragraphs | 45 | 55 | 3.57 s | 3.56 | 3.57 | 15.42 | 64.8 |
| EN → FR punctuation/quotes | 61 | 61 | 4.15 s | 4.03 | 4.29 | 14.69 | 68.1 |
| EN → DE measurements | 42 | 47 | 3.09 s | 3.08 | 3.10 | 15.23 | 65.6 |
| EN → FR near boundary | 443 | 64 | 7.06 s | 7.03 | 7.12 | 9.07 | 110.3 |
| EN → FR truncated | 512 | 64 | 7.65 s | 7.63 | 7.65 | 8.37 | 119.5 |

The 443-token case is below the 512-token limit (`truncated=false`); the
512-token case is at the limit and reports `truncated=true`. Source text is
truncated by the tokenizer, not by the benchmark.

### 7.2 Forced budgets (long EN → FR, `EarlyStopping::Never`)

| Budget | Start-of-session Q8 | Final Q8 | Final range | tok/s | ms/token |
|---:|---:|---:|---|---:|---:|
| 32 | 4.66 s | 3.70 s | 3.70–3.71 | 8.64 | 115.7 |
| 64 | 7.02 s | 5.65 s | 5.64–5.67 | 11.32 | 88.3 |
| 128 | 11.45 s | 9.66 s | 9.63–9.68 | 13.25 | 75.5 |
| 200 | 19.55 s | 14.28 s | 14.27–14.28 | 14.01 | 71.4 |

The pre-change column is the Cycle 4 binary measured at the start of this
session, not the Cycle 4 report's number. Both columns move together with
machine state: an earlier final run of the same binary measured 4.05 / 6.05 /
11.58 / 16.63 s. The interleaved A/B is the authoritative comparison and shows
the encoder −24.6% and search −24.6%; the absolute before/after difference also
includes the host becoming much quieter during the session.

### 7.3 Load and memory

| Metric | Pre-change Q8 | Final Q8 | f32 |
|---|---:|---:|---:|
| Load time | 9.61 s | 7.83 s | 6.40 s |
| RSS after load | 1,587 MB | 1,586 MB | 5,334 MB |
| Peak during load | 2,433 MB | 2,433 MB | 5,334 MB |
| Peak during benchmark | 2,433 MB | 2,433 MB | 5,619 MB |

Load time is excluded from all generation numbers.

### 7.4 Kernel microbenchmark (final tree)

8192×1024, m = 267: 19.9 ms median per call (19.9, 17.2, 20.3 across three
final runs), 221–261 GFLOP/s. 1024×1024, m = 267: 3.1 ms median. These are
relative kernel numbers; the in-situ encoder measurements above are the
product-level result.

## 8. Translation quality

### Corpus

21 cases: FR→EN, EN→FR, EN→DE, ES→EN, IT→EN; short sentences, a medium
paragraph, a long paragraph (267 tokens), a second long paragraph (193 tokens),
a multi-paragraph document, accents and Unicode, numbers/currency/dates,
measurements, proper nouns, repeated terminology, abbreviations, parentheses
and typographic quotations, EOS-stopped generation, forced
32/64/128/200-token continuations, a 443-token source just below the 512-token
limit, and a 512-token source that reports `truncated=true`. Token ids and the
truncation flag are stored in the benchmark JSON.

### Results

- **Q8 vs f32: 21/21 cases token-identical**, including the near-boundary and
  truncated inputs.
- **Pre-change Q8 vs final Q8: 17/17 overlapping cases identical.** The four
  fixtures added in this cycle have no pre-change counterpart; the other 17
  cases were already in the Cycle 4 corpus.
- **Scalar Q8 vs AVX2 Q8: 17/18 identical.** The one difference is
  `quotes-en-fr`, token 21: AVX2 selects “quelque” and scalar selects
  “environ” (“… indique que quelque/environ 12 000 résidents …”). Both are
  valid French and the sentence remains fluent.
  - This is a pre-existing numerical difference between the two kernels, not a
    Cycle 5 regression. The scalar kernel sums each 32-value block into a
    single `i32`; the AVX2 kernel keeps eight `i32` lanes and performs a
    horizontal add, so near-tie candidates can round differently. The new
    tests prove tiled-scalar == legacy-scalar and tiled-AVX2 == legacy-AVX2
    bit for bit, so tiling did not create the divergence.
  - The default AVX2 path matches f32 on all 21 cases; the scalar path is a
    portability fallback and was previously compared on a corpus that did not
    contain this punctuation-heavy case.
- No Q8-vs-f32 differences were observed, so there is nothing to explain or
  justify for the default path.

The corpus is still small and covers one model. Byte identity is an empirical
result, not a guarantee; it depends on the Q8 block format and on not changing
the summation order. No BLEU-style metric is reported because the Q8/f32
outputs are identical.

## 9. Reliability and scalar behaviour

`crates/fluent-core/tests/reliability.rs` (needs `FLUENT_TEST_MODEL`, skips
otherwise):

- **Lifecycle**: load → translate → `unload()` → translate (auto-reload) →
  identical text. RSS 1,666 MB loaded and 1,351 MB after unload; the remainder
  is allocator retention.
- **Repeated translations**: five runs, identical text, RSS 1,724 MB → 1,724
  MB.
- **Cancellation**: an already-cancelled call returns in 0.0 ms; a signal
  200 ms into the encoder returns `Error::Cancelled` at 202 ms total and the
  engine remains usable.
- **One difference from Cycle 4**: the encoder cancellation latency measured
  2 ms after the signal this time versus ~32 ms in Cycle 4. The check is
  per-layer, so the number is bounded by one layer's runtime and varies with
  where the signal lands; both are far below the 10 s test bound.

Cache correctness is covered by the physical-slot reference tests added in
Cycle 4, which still pass, plus the new prefill and logits exactness tests.

Scalar fallback: the tiled scalar kernel is exercised by the same exactness
tests; `FLUENT_QUANT_SIMD=off` ran the full unit suite and the 18-case quality
run (17 product-like cases plus the 32-token forced case). Scalar matches AVX2
on 17 of 18 cases; the single synonym difference is documented in §8. On the
32-token forced case scalar took 6.03 s versus 3.70 s for AVX2 in adjacent runs
(~1.63×; the host state moved between the runs, so treat the ratio as
indicative).

## 10. Second model investigation

`/home/neon/Documents/CODE LOCAL/Fluent/models/nllb-200-3.3b` contains a
complete-looking checkpoint: `config.json` (d_model 2048, 24+24 layers, vocab
256,206, f32), `pytorch_model.bin.index.json` and three
`pytorch_model-0000{1,2,3}-of-00003.bin` shards totalling 17,577,465,405 bytes
(~16.4 GiB). The loader discovers it through the index, so the directory is
readable.

It was **not run**. The f32 checkpoint is larger than the machine's 15,757 MB
of RAM, and the host already had ~12 GB resident and ~13 GB of swap in use.
Quantization requires the f32 tensors during loading, so a run would thrash or
be killed. Producing a bounded test safely would require a larger machine or a
pre-quantized checkpoint. No weights were copied or downloaded.

## 11. Rejected and deferred

### 11.1 Cross-block integer accumulation (rejected)

Tempting simplification: accumulate `i32` block dot products across the whole
row and multiply by one scale at the end. This is invalid because each 32-value
block has its own weight scale and each activation block has its own scale.
The product of the two scales differs per block, so no common factor can be
pulled out. The kernel keeps one f32 scaled accumulation step per block; this
is also what makes the result bit-identical to the reference.

### 11.2 AVX-512 VNNI (deferred)

This CPU supports AVX-512 VNNI. A `vpdpbusd` kernel would compute 64 `u8 × i8`
products per instruction, but Q8_0 activations are signed. The conversion
`a_u8 = a_i8 + 128` requires subtracting `128 × Σw_block` per block, i.e. an
extra per-block weight-sum term and a different accumulation order. The result
would no longer be bit-identical to the AVX2 path, and the fallback matrix
(AVX2 and scalar) would still have to be maintained and validated. After the
tiling change the encoder is no longer L2-bound, so the remaining upside is
smaller than the Cycle 4 estimate. Deferred to Cycle 6 with the requirement
that any VNNI path be gated on the full quality corpus and a token-identity (or
explicitly justified) result.

### 11.3 Cycle 4 tiling versus Cycle 5 tiling

Cycle 4 amortized weight-block decoding across input rows but kept the output
row as the outer loop, so the activation matrix was still rescanned per output
row. Cycle 5 makes the input tile the outer loop and parallelizes `(tile,
output chunk)`, which is what removes the activation traffic. The two changes
are complementary; only the second produced a reliable end-to-end gain.

### 11.4 LM head tiling (still rejected)

The LM head remains at ~2.4 s / 200 steps, near the measured memory roofline
(~180 GFLOP/s, ~24 GB/s). No safe change was found in this cycle either.

## 12. Remaining technical debt (ranked)

1. **Q8 prefill is still slower than f32** (~1.5–1.7 s run-dependent, 1.62 s in
   the interleaved A/B, vs ~1.08 s f32 encoder at 267 tokens). The tiled kernel
   removed cache-traffic overhead; the remaining gap is quantized arithmetic
   throughput. A carefully validated VNNI or wider-integer dot product is the
   next candidate, but it must preserve quality.
2. **Logits top-k scan** is now ~0.24–0.31 s / 200 steps. Selecting on raw
   logits would be faster but could alter tie behaviour; not worth the risk for
   ~0.1 s without a dedicated equivalence proof.
3. **Decoder FFN and projections** remain ~5.7 s + 2.5 s / 200 steps and are at
   the bandwidth roofline for this weight format.
4. **Cross-attention** ~1.6 s / 200 steps; batching was measured and rejected,
   a SIMD dot product could change the trade-off.
5. **Load time** ~8.3 s for Q8, about 2.4 s of one-time quantization.
6. **No reliable CI performance gate**; see §15 for the required runner.
7. **Corpus size** — 21 cases, one model.
8. **3.3B model validation is blocked by RAM**, not by code.
9. **Cycle 2 packaging/licensing debt** remains out of scope.

## 13. Quality gates (final tree)

| Command / check | Result |
|---|---|
| `cargo fmt --all -- --check` | Pass |
| `cargo check --workspace` | Pass |
| `cargo test --workspace` | Pass: 51 core + 3 reliability (skip without model) + 7 GUI = 61, 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass |
| `cargo build --release -p fluent-gui` | Pass (2m24s) |
| `FLUENT_QUANT_SIMD=off cargo test -p fluent-core` | Pass: 51 + 3 |
| `FLUENT_TEST_MODEL=… cargo test --release -p fluent-core --test reliability` | Pass 3/3 (42 s) |
| Prefill exactness: tiled vs row-at-a-time | Pass: bit-identical on all tested shapes, AVX2 and scalar |
| Logits exactness: manual vs candle | Pass: bit-identical, including ±inf/NaN rows |
| Candidate equality vs candle reference | Pass: token, beam and score bits |
| Q8 vs f32 corpus | Pass: 21/21 token-identical |
| Pre-change Q8 vs final Q8 | Pass: 17/17 overlapping cases identical |
| Scalar vs AVX2 Q8 | 17/18 identical; one synonym difference documented in §8 |
| Final Q8 benchmark, 3 repetitions, 21 cases | Pass; JSON outside Git |
| `--matmul-bench` | Pass |
| GUI launch smoke (Wayland, 8 s timeout) | Pass: stayed up, no error output |

## 14. Reproducing the results

```bash
cargo build --release -p fluent-core --example bench

# Q8 final benchmark (3 repetitions, all 21 cases)
./target/release/examples/bench models/nllb-200-1.3b \
    --reps 3 --budgets 32,64,128,200 --json /tmp/fluent-bench.json

# f32 control
FLUENT_QUANT=off ./target/release/examples/bench models/nllb-200-1.3b \
    --reps 1 --budgets 32,64,128,200 --json /tmp/fluent-bench-f32.json

# Scalar Q8
FLUENT_QUANT_SIMD=off ./target/release/examples/bench models/nllb-200-1.3b \
    --reps 1 --budgets 32 --json /tmp/fluent-bench-scalar.json

# Kernel microbenchmark
./target/release/examples/bench --matmul-bench
```

The Cycle 5 A/B switches were removed from the final tree. To reproduce the
kernel or search A/B, build the Cycle 4 commit (`4e31b45`) and the Cycle 5 HEAD
and alternate runs as described in §3; the logs used here are:

- `/tmp/opencode/c5-baseline.json` (Cycle 4 binary, start of session)
- `/tmp/opencode/c5-ab-tiled-*.json` / `c5-ab-legacy-*.json` (kernel)
- `/tmp/opencode/c5-ab2-old-*.json` / `c5-ab2-new-*.json` (search)
- `/tmp/opencode/c5-final.json`, `/tmp/opencode/c5-quality-*.json`

## 15. Future performance-runner specification

No CI workflow was added: wall-clock thresholds on this shared host would be
noise. A meaningful gate needs:

- A dedicated runner with a fixed CPU model, at least six physical cores, SMT
  disabled or pinned, and the process pinned with a cpuset (`taskset`).
- Governor `performance`, fixed turbo/thermal state, and no other jobs.
- A pinned Rust toolchain (`rust-toolchain.toml` + committed `Cargo.lock`).
- A model checked by SHA-256, stored in a pre-baked cache; no model download in
  ordinary CI.
- The fixed corpus and budgets, plus `EarlyStopping::Never` for throughput.
- At least five repetitions, reporting median and interquartile range; a
  regression gate such as “median must stay within 5–10% of the stored
  baseline, confirmed over three consecutive runs”.
- A separate correctness job that runs the unit, exactness and quality corpus
  tests without asserting unstable wall-clock values.

## 16. Recommendation for Cycle 6

With the cache-traffic and logits overheads removed, CPU inference is close to
the practical limit of the current Q8 format. The next highest-value kernel
work is a numerically validated wider dot product (AVX-512 VNNI with an
explicit per-block correction and a maintained AVX2/scalar fallback), measured
in situ and gated on token identity. In parallel, distribution preparation
deserves attention: packaging, model acquisition/checksums, license disclosure
and platform validation are now larger release blockers than another few
percent of CPU speed. A 3.3B validation should wait for a machine with enough
RAM or a pre-quantized checkpoint.
