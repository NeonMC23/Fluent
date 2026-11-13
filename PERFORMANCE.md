# Fluent CPU Performance — Cycle 4

Cycle 4 focused on the cost of the quantized (Q8) CPU path: the unexpectedly
slow prefill kernel inherited from Cycle 3, the language-model head, and the
beam-search bookkeeping. The largest measured defect was not arithmetic: beam
search was copying the entire KV cache on every step to apply an *identity*
permutation. Fixing the cache layout removed essentially all of that cost.

## 1. Summary

- **Beam-search cache reorder: 2.30 s → 0.003 s per 200-token run**
  (median of 3 repetitions). The old implementation copied every destination
  beam on every step. The cache now keeps a physical-slot permutation and only
  copies rows when two beams genuinely share a parent. Beam parents were
  identity in every measured workload, so the common case became free.
- **17/17 corpus cases produce token-identical output between Q8 and f32**,
  including accents, numbers, proper nouns, repeated terms, paragraph
  boundaries, and forced 32/64/128/200-token continuations.
- **11/11 previously recorded Q8 outputs are unchanged** by the cache change.
- **14/14 scalar-kernel outputs match the AVX2 outputs** token for token.
- **The tiled Q8 prefill kernel was implemented and measured, then rejected.**
  Tile sizes 4/8/16/24/32 were benchmarked, but the in-situ A/B showed no
  reliable end-to-end win: the quantized prefill is arithmetic-bound, not
  weight-bandwidth-bound, because the FFN weights fit in L3. The previous
  kernel is retained.
- Lifecycle, cancellation and repeatability are now covered by integration
  tests: unload/reload returns identical output, five repeated translations
  are identical with +1 MB RSS, and encoder cancellation is observed within
  ~32 ms of the signal.
- 200-token segment: **17.61 s** in this session (17.11–17.66 s, 11.36 tok/s)
  versus **23.21 s** (20.11–24.64 s) for the pre-change tree. The machine was
  in a slower state during the baseline run (every compute stage was ~20%
  slower), so the attributable gain is the measured reorder delta of
  ~1.8–2.3 s, not the full 5.6 s total.

## 2. Environment

| Item | Value |
|---|---|
| CPU | Intel Core i5-11400H (6 cores / 12 threads, 2.70 GHz base) |
| RAM | 15,757 MB |
| OS | Nobara Linux 44 (KDE Plasma), kernel 7.3.1-202.nobara.fc44.x86_64 |
| Rust | rustc 1.99.0 / cargo 1.99.0 |
| Inference stack | Candle 0.11.0 |
| Model | NLLB-200 1.3B, f32 checkpoint, 24 encoder + 24 decoder layers, d_model 1024, FFN 8192, 16 heads, vocabulary 256,206 |
| SIMD | AVX2/FMA selected at runtime; `powersave` CPU governor |
| Repository | `/tmp/opencode/fluent-cycle2`, branch `finalization/cycle-4`, based on `f062bda` |
| Model path | `/home/neon/Documents/CODE LOCAL/Fluent/models/nllb-200-1.3b` (read-only) |

The machine is shared and the governor is `powersave`, so absolute timings vary
by more than 20% between sessions. Comparisons below therefore use (a) the
stage-level measurements, which isolate the changed code, and (b) median of
three repetitions within one session. The same binary produced 19.0 s and
23.2 s for the same 200-token workload minutes apart in this cycle.

## 3. Method and harness

The harness is `crates/fluent-core/examples/bench.rs`. Cycle 4 added:

- Encoder sub-stage timers (`encode_attn`, `encode_ffn`), search sub-stage
  timers (`search_logits`, `search_extract`, `search_topk`) and a separate
  `reorder` timer. All are reported in the JSON and the stage tables.
- A rewritten `--matmul-bench` covering prefill-sized `m` (1, 4, 8, 16, 32,
  64, 128, 267) for 1024×1024 and 8192×1024 weights, plus 256206×1024 for
  `m = 1, 4, 8`; it also measures candle's encoder self-attention and the
  logits post-processing path.
- Five corpus fixtures (accents, numbers/currency, proper nouns, repeated
  terms, paragraph boundary) and generated token ids in the JSON so quality
  comparisons are exact rather than text-only.

Model loading, tokenization and detokenization are measured separately from
generation. Every case uses the same fixture text, target language, beam count
and generation settings across implementations.

The reference corpus is now 13 product-like cases plus the forced
32/64/128/200-token matrix on the long English paragraph, 17 cases in total.
Product-like cases stop on EOS (heuristic early stopping); the forced matrix
uses `EarlyStopping::Never` so this is a pure throughput measurement, not a
user-visible latency claim.

## 4. Where the time went

200 generated tokens, 267-token source, Q8, median of 3 (`c4-final`):

| Stage | Pre-change tree | Final tree |
|---|---:|---:|
| Tokenization | 0.4 ms | 0.4 ms |
| Encoder | 2,095 ms | 2,244 ms |
| Cross-attention cache preparation | 287 ms | 254 ms |
| Prefill (start + language token) | 94 ms | 87 ms |
| Decoder (24 layers × 200 steps) | 13,899 ms | 11,157 ms |
| └ FFN | 6,859 ms | 5,685 ms |
| └ Q/K/V/out projections | 3,210 ms | 2,475 ms |
| └ Cross-attention | 2,028 ms | 1,588 ms |
| └ Self-attention | 1,685 ms | 1,306 ms |
| └ KV-cache append | 95 ms | 89 ms |
| Language-model head | 2,754 ms | 2,396 ms |
| Beam search | 1,794 ms | 1,488 ms |
| └ logits/log-softmax + extraction | 1,573 ms | 1,347 ms |
| └ top-k | 222 ms | 196 ms |
| **Beam cache reorder** | **2,302 ms** | **3 ms** |
| **Generation total** | **23,214 ms** | **17,609 ms** |

The pre-change run was collected when the CPU was slower: every arithmetic
stage is 15–25% higher, so the two totals are not a controlled A/B. The cache
reorder is the one stage that changed by orders of magnitude and is measured
directly by the same instrumentation in both runs.

Encoder split at 267 source tokens: attention ≈ 0.63 s, FFN ≈ 1.61 s,
everything else ≈ 5 ms.

## 5. Implemented: physical-slot beam cache (kept)

### Problem

`CpuDecoderCache` stores `[beam][head][capacity][head_dim]` K/V rows and used to
implement `reorder(parents)` by copying every destination beam from its parent
through a scratch buffer, then swapping. With 4 beams, 24 layers, 16 heads and
200 positions this copies roughly 150 MB per step and ~30 GB over a 200-token
run.

Profiling showed that beam parents were `[0, 1, 2, 3]` on every step of every
measured workload: each beam's own best candidate already occupied the pooled
top-k slots. The old code was therefore copying the whole cache onto itself.
Even for non-identity permutations the copy was wasteful: distinct parents need
no movement at all.

### Design

- `CpuDecoderCache` now keeps `phys: Vec<usize>`, the physical slot holding each
  logical beam's rows.
- `append` and self-attention address rows through `phys`.
- `reorder(parents)` rewrites `phys[dest] = phys[parent]` and then
  `ensure_unique_slots()`: beams whose parent is distinct keep their slot at
  zero cost; aliases are materialized by copying into unused slots.
- `fused_attention` takes a slot map instead of an implicit beam index.
  Cross-attention passes an empty map because all beams share one K/V row.
- `repeat()` sets all logical beams to the prefill slot; the same
  `ensure_unique_slots()` materializes the copies once.
- Cache buffers remain preallocated; no per-step reconstruction or new
  allocation proportional to sequence length was introduced.

### Correctness tests

`crates/fluent-core/src/model/nllb.rs`:

- `cpu_cache_physical_slots_match_reference_bookkeeping` drives prefill,
  `repeat`, three reorders with duplicate parents (`[2,0,2,2]`, `[1,1,3,0]`,
  `[0,0,0,0]`) and appends, comparing every beam's K/V rows against a compact
  reference cache that copies on every reorder. It also asserts that physical
  slots stay distinct.
- `cpu_cache_attention_uses_the_physical_slot` checks that after `[1,0]` the
  logical beam attends to the rows the other beam accumulated, not its own old
  rows.
- Existing `cpu_cache_repeat_copies_the_prefill_beam` and
  `cpu_cache_reorder_moves_positions_between_beams` were updated to resolve
  logical beams through `phys`.
- `fused_attention_matches_candle_for_broadcast_cross_attention` and the
  per-beam self-attention test still compare against candle's reference.

The change is bit-identical for the cache contents and attention arithmetic:
the reference-bookkeeping test compares exact f32 slices, and the attention
test compares exact output vectors.

## 6. Investigated and rejected

### 6.1 Tiled Q8 prefill kernel (implemented, measured, reverted)

The Cycle 3 report identified Q8 prefill as slower than f32 GEMM. A tiled
kernel was implemented for both the scalar and AVX2 paths: process `T` input
rows while each quantized weight block is decoded once, keep `T` f32
accumulators, and handle partial final tiles. The previous row-at-a-time
kernels were retained and a `FLUENT_QUANT_LEGACY=1` diagnostic switch allowed
an in-binary A/B. Exactness tests asserted bit-identical outputs for
`m = 1,4,7,8,9,15,16,17,23,31,32,33,64,67,70` on both kernels.

Tile sizes 4, 8, 16, 24 and 32 were measured on the kernel micro-benchmark
(8192×1024 weights, prefill-sized `m`). Tile 16 was the best of the set
(m = 16: 1.36 ms / 197 GFLOP/s; m = 267: 22.7 ms / 198 GFLOP/s), but the
end-to-end A/B was noise-dominated:

| Round | Workload | Legacy encode | Tiled encode |
|---|---|---:|---:|
| A | 32 tokens | 2,390 ms | 1,997 ms |
| A | 200 tokens | 1,993 ms | 1,974 ms |
| B | 32 tokens | 2,110 ms | 2,374 ms |
| B | 200 tokens | 2,033 ms | 2,340 ms |

The tiled kernel is a faithful, exact implementation, but it does not
outperform the original reliably. The explanation is visible in the profile:
the 9 MB FFN weight matrix fits in the 12 MB L3, so re-reading it does not hit
DRAM, and the quantized dot product is arithmetic-bound at roughly
140–190 GFLOP/s. Per the cycle policy, the previous kernel is retained and the
alternative is documented rather than merged. `crates/fluent-core/src/model/quant.rs`
is unchanged.

### 6.2 Cross-attention batching (implemented, measured, reverted)

Cross-attention re-reads the source K/V once per beam. A specialized kernel
processed all beams for one head and halved/tquartered the source traffic.
Measured cross-attention stayed at 1.57–1.64 s versus 1.57 s for the original
kernel: reducing the task count from 64 to 32/16 costs more parallelism than
the traffic saves on this CPU. Reverted.

### 6.3 LM head tiling (investigated, no safe win)

`lm_head` is 2.40 s over 200 steps, i.e. ~12 ms per step for a
`[4][256206] × [256206][1024]` product: 2.1 GFLOP per step at ~180 GFLOP/s and
~24 GB/s of weight traffic. That matches the Cycle 3 micro-benchmark
(11.1 ms/call, 189 GFLOP/s) and the measured kernel roofline. The existing
kernel already tiles across the four beams; no safe change was found that
improves the end-to-end number. Beam-search semantics were not touched.

### 6.4 Logits post-processing (investigated, deferred)

Per 200-token run the logits path costs 1.34 s: candle `log_softmax` ~5.8 ms
per step, extraction 0.2–0.3 ms, top-k 0.75 ms in isolation. A manual
logsumexp costs 2.8 ms per step, but changes the summation order compared with
candle's reduction. The expected saving is ~0.4–0.6 s per 200-token run (2–3%);
it was left out because it would introduce a numerical difference in the
search scores without a demonstrated quality benefit. The measurement and the
reason are recorded here so Cycle 5 can decide with the quality corpus in
place.

## 7. Benchmarks

### 7.1 Product-like cases (Q8, median of 3, heuristic EOS stop)

| Case | Source tokens | Generated | Median | Min | Max | tok/s | ms/token |
|---|---:|---:|---:|---:|---:|---:|---:|
| FR → EN | 13 | 7 | 0.83 s | 0.83 | 0.89 | 8.40 | 119.1 |
| EN → FR | 15 | 16 | 1.28 s | 1.26 | 1.50 | 12.54 | 79.7 |
| EN → DE | 15 | 19 | 1.51 s | 1.51 | 1.56 | 12.56 | 79.6 |
| ES → EN | 16 | 11 | 0.98 s | 0.96 | 0.98 | 11.25 | 88.9 |
| IT → EN | 20 | 20 | 1.58 s | 1.56 | 1.59 | 12.66 | 79.0 |
| EN → FR medium | 81 | 64 | 5.53 s | 4.95 | 5.69 | 11.57 | 86.4 |
| EN → FR long | 267 | 64 | 7.33 s | 7.17 | 7.57 | 8.73 | 114.6 |
| DE → EN long | 193 | 64 | 6.32 s | 6.23 | 6.49 | 10.12 | 98.8 |
| FR → EN accented | 40 | 21 | 1.92 s | 1.88 | 1.95 | 10.92 | 91.6 |
| EN → FR numbers | 25 | 27 | 2.13 s | 2.13 | 2.13 | 12.68 | 78.9 |
| EN → DE names | 26 | 27 | 2.22 s | 2.12 | 2.23 | 12.16 | 82.3 |
| ES → EN repeated terms | 27 | 24 | 1.98 s | 1.95 | 2.04 | 12.11 | 82.6 |
| EN → FR paragraphs | 45 | 55 | 4.21 s | 4.16 | 4.34 | 13.07 | 76.5 |

### 7.2 Forced budgets (long EN → FR, `EarlyStopping::Never`)

| Budget | Pre-change Q8 | Final Q8 | Final min–max | tok/s | ms/token | f32 control (Cycle 3) |
|---:|---:|---:|---:|---:|---:|---:|
| 32 | 5.93 s | 4.67 s | 4.66–4.81 | 6.86 | 145.9 | 13.14 s |
| 64 | 8.57 s | 7.00 s | 6.92–7.05 | 9.14 | 109.4 | 25.08 s |
| 128 | 12.20 s | 11.57 s | 11.56–11.63 | 11.06 | 90.4 | 50.08 s |
| 200 | 23.21 s | 17.61 s | 17.11–17.66 | 11.36 | 88.0 | 80.18 s |

The f32 column is the committed Cycle 3 control, measured with the same
harness and hardware; the f32 inference path was not modified in Cycle 4. A
Cycle 4 single-repetition f32 run on the same corpus produced 14.52 / 28.98 /
62.77 / 93.26 s, confirming the ~3–5× Q8 advantage; it is slower than the
Cycle 3 control because of the machine state, not a code change.

The pre-change Q8 column was collected with the same binary instrumentation but
while the CPU was ~20% slower; the reorder stage is the directly comparable
part (2,302 ms → 3 ms). The remaining total difference should not be attributed
to the cache change alone.

Model load: 8.35 s for Q8 (1,586 MB RSS, 2,431 MB peak during quantization)
versus 5.97 s for f32 (5,334 MB). Load time is excluded from all generation
numbers.

### 7.3 Kernel micro-benchmark

With AVX2, 8192×1024 weights, `m = 267`: ~22.7 ms per call, ~198 GFLOP/s.
256,206×1024, `m = 4`: ~21 ms per call at the prefill-sized micro-benchmark
(the warm in-pipeline value is ~12 ms, as reported above). These numbers vary
with the machine state; they are used for relative kernel comparisons only.

## 8. Translation quality

### Method

The corpus covers FR→EN, EN→FR, EN→DE, ES→EN and IT→EN; short, medium, long
and two-paragraph inputs; accents; punctuation; numbers, currency and dates;
proper nouns; repeated terminology; EOS-stopped generation; forced
32/64/128/200-token continuation on a 267-token source. All texts are
original and redistributable, and token ids are recorded in the benchmark
JSON.

### Results

- **Q8 vs f32: 17/17 cases token-identical**, including the forced 200-token
  continuation.
- **Pre-change Q8 vs final Q8: 11/11 outputs identical** (the overlapping
  benchmark cases; the five new fixtures were added in this cycle).
- **Scalar vs AVX2 Q8: 14/14 cases token-identical** (13 fixtures + the
  32-token forced case).
- Examples from the new fixtures:
  - FR→EN accented: “Last summer in Montreal, we visited an exhibition of the
    work of a very famous Quebec artist.”
  - EN→FR numbers: “La facture de 1 250,75 euros a été émise le 3 mars 2024 et
    doit être payée dans les 30 jours.”
  - ES→EN repeated terms: “The committee reviewed the budget, approved the
    budget and published the revised budget on the committee's website.”

Byte-identical output is stronger than the cycle required, but it is an
empirical result on a small corpus, not a guarantee. No BLEU-like score is
reported: the outputs are identical, so a corpus metric would add noise without
information. The corpus is single-model and does not cover domain-specific
terminology beyond the fixtures above.

## 9. Memory reliability

| Measurement | f32 | Q8 |
|---|---:|---:|
| RSS after load | 5,334 MB | 1,586 MB |
| Peak during load | 5,334 MB | 2,431 MB |
| Peak during benchmark | 5,643 MB | 2,431 MB |

`crates/fluent-core/tests/reliability.rs` (runs against
`FLUENT_TEST_MODEL`; skips cleanly without it):

- **Lifecycle**: load → translate → `unload()` → translate (auto-reload) →
  identical text. RSS 1,635 MB loaded, 1,385 MB after unload; the model state
  is released, the remaining RSS is allocator retention.
- **Repeated translations**: five runs, identical text each time, RSS
  1,689 MB → 1,690 MB.
- **Cancellation**: an already-cancelled call returns immediately; a signal
  200 ms into the encoder returns `Error::Cancelled` after 232 ms total
  (~32 ms post-signal); the engine remains usable afterwards.
- **Cache correctness**: the reference-bookkeeping and slot-attention tests
  above run on every `cargo test`, and the fused attention kernels are checked
  against candle.

## 10. Scalar and AVX2 behaviour

- Runtime detection is unchanged; `FLUENT_QUANT_SIMD=off` forces the scalar
  path.
- The cache change is scalar-independent (plain Rust slices) and both paths
  passed the full unit suite under `FLUENT_QUANT_SIMD=off`.
- Scalar is ~1.75× slower than AVX2 (32-token forced run: 8.15 s vs 4.67 s)
  and produced the same tokens on all 14 compared cases.
- No AVX-512 or other new instruction set was introduced.

## 11. Remaining technical debt

1. **Q8 prefill is still slower than f32 GEMM** (2.2 s vs ~1.1 s encoder at
   267 tokens). The kernel is arithmetic-bound; a faster dot product
   (e.g. wider integer multiply-accumulate or different block scaling) is the
   next kernel-level opportunity. It costs ~1 s on a 200-token run.
2. **Logits exponentiation is the largest search cost** (1.29 s per 200-token
   run). A faster, numerically validated logsumexp could recover ~0.4–0.6 s.
3. **Decoder FFN and projections dominate** (5.7 s + 2.5 s); they are close to
   the measured memory roofline. Further gains need different weight layouts
   or a lower-bit format, not more threads.
4. **Cross-attention is recomputed for every beam** (1.6 s). Batching was
   measured and rejected on this CPU; a SIMD dot product would change the
   trade-off.
5. **Load time**: 8.35 s for Q8, ~2.4 s of which is one-time quantization.
6. **No AVX-512/VNNI path**, no quantized GPU path, and no end-to-end CI
   performance gate; the required runner would need a fixed governor, a
   pinned CPU set and a stable thermal state.
7. **The quality corpus is still small** (17 cases, one model). It is enough to
   catch regressions in the changed code but not to claim general equivalence.
8. **Cycle 2 debt remains**: packaging/signing, model acquisition flow,
   Windows/macOS validation, and the NLLB weight license boundary.

## 12. Quality gates (final tree)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | Pass (no diff) |
| `cargo check --workspace` | Pass |
| `cargo test --workspace` | Pass: 44 core + 3 reliability (skipped without model) + 7 GUI = 54 tests, 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass |
| `cargo build --release -p fluent-gui` | Pass |
| `FLUENT_QUANT_SIMD=off cargo test -p fluent-core` | Pass: 44 + 3 tests |
| `FLUENT_TEST_MODEL=… cargo test --release -p fluent-core --test reliability` | Pass: 3/3, lifecycle/repeat/cancellation results above |
| Q8 benchmark, 3 repetitions, 17 cases | Pass, JSON outside the repo |
| Q8 vs f32 corpus comparison | Pass: 17/17 token-identical |
| Scalar vs AVX2 corpus comparison | Pass: 14/14 token-identical |
| GUI launch smoke test (8 s, Wayland) | Pass: process stayed up, no error output |
| `--matmul-bench` (Q8 kernels, attention, logits) | Pass |

Benchmark JSON and logs are not committed: they contain machine-specific
timings and absolute paths.

## 13. Reproducing the results

```bash
cargo build --release -p fluent-core --example bench

# Q8 (default on CPU f32)
./target/release/examples/bench models/nllb-200-1.3b \
    --reps 3 --budgets 32,64,128,200 --json /tmp/fluent-bench.json

# f32 control
FLUENT_QUANT=off ./target/release/examples/bench models/nllb-200-1.3b \
    --reps 1 --budgets 32,64,128,200 --json /tmp/fluent-bench-f32.json

# Scalar fallback
FLUENT_QUANT_SIMD=off ./target/release/examples/bench models/nllb-200-1.3b \
    --reps 1 --budgets 32 --json /tmp/fluent-bench-scalar.json

# Kernel micro-benchmarks
./target/release/examples/bench --matmul-bench

# Lifecycle, cancellation and repeatability (needs weights)
FLUENT_TEST_MODEL=models/nllb-200-1.3b \
    cargo test --release -p fluent-core --test reliability -- --nocapture
```

To compare quality, diff the `tokens` array for each `(id, budget,
early_stopping)` key in the two JSON reports.

## 14. Recommendation for Cycle 5

The measured order is: (1) quantized prefill arithmetic, (2) logits
post-processing, (3) decoder FFN/projection bandwidth. The highest-value next
step is a faster Q8 dot product for prefill (integer multiply-accumulate with
validated scaling and a scalar fallback), because it is the only remaining
regression against f32 and it affects every long source. It should be
benchmarked in situ, not only with the micro-benchmark, and gated on
token-identical corpus output. A validated logsumexp is the second step; LM
head tiling has no measured headroom on this CPU and should wait for a
different weight layout or hardware.
