# Fluent CPU Performance — Cycle 3

This document records the performance work done in Cycle 3 on
`finalization/cycle-3`. It separates measured facts from estimates, and it
includes the experiments that were rejected as well as the ones that were kept.

The goal for the cycle was narrow: make long-form CPU translation substantially
faster without changing generation correctness, translation quality, memory
safety, cancellation, or the short-input latency achieved in Cycle 2.

## 1. Summary

| Workload (NLLB-200 1.3B, CPU) | Cycle 2 reference | f32 control on this tree | Q8 final | Q8 vs f32 |
|---|---:|---:|---:|---:|
| FR → EN, 7 generated tokens | 2.83 s | 2.89 s | **0.64 s** | 4.5× |
| 32 generated tokens (267-token source) | — | 13.14 s | **4.38 s** | 3.0× |
| 64 generated tokens (267-token source) | — | 25.08 s | **6.56 s** | 3.8× |
| 128 generated tokens (267-token source) | — | 50.08 s | **11.19 s** | 4.5× |
| 200 generated tokens (267-token source) | ≈ 124.5 s | 80.18 s | **16.84 s** | 4.8× |
| Medium paragraph, 64 generated tokens | — | 21.04 s | **4.73 s** | 4.5× |
| Long paragraph, 64 generated tokens | — | 25.19 s | **6.58 s** | 3.8× |

The 200-token acceptance target (< 60 s) is met with a wide margin on the
reference CPU. The short FR → EN target (< 5 s) is met at 0.64 s.

All 12 benchmark outputs are byte-identical between the Q8 path and the f32
control, including a forced 200-token continuation and a source near the
512-token truncation limit.

## 2. Environment

Measured on the reference-class laptop, not on a synthetic CI runner.

| Item | Value |
|---|---|
| CPU | 11th Gen Intel Core i5-11400H @ 2.70 GHz (6 cores / 12 threads) |
| RAM | 15,757 MB |
| OS | Nobara Linux 44 (KDE Plasma Desktop Edition), x86-64 |
| Rust | 1.99.0 (Fedora 1.99.0-1.fc44) |
| Cargo | 1.99.0 |
| Candle | 0.11.0 (`candle-core`, `candle-nn`) |
| Model | NLLB-200 1.3B, `d_model=1024`, 24 encoder + 24 decoder layers, FFN 8192, 16 heads, vocab 256,206, shared embedding, ReLU, `max_position_embeddings=1024` |
| Model file | local `pytorch_model.bin` (5.48 GB), f32 |
| CPU features used | `avx2` + `fma`, selected at runtime; scalar fallback always available |
| CPU features present but unused | `avx512f` (see rejected experiments) |
| DVFS governor | `powersave` (not changed by the benchmark) |

The machine is shared with a desktop session. Absolute numbers drift by a few
percent between runs; the reported figures use medians of repeated runs where
noted.

## 3. Benchmark harness

The checked-in harness is `crates/fluent-core/examples/bench.rs`. It calls the
same `Nllb` and `NllbTokenizer` APIs as the product, so it measures the real
inference path rather than a synthetic approximation.

```bash
cargo run --release -p fluent-core --example bench -- \
    models/nllb-200-1.3b --reps 3 --budgets 32,64,128,200 \
    --json /tmp/fluent-bench.json
```

Properties:

- **Fixed corpus in source.** Five short cases (FR→EN, EN→FR, EN→DE, ES→EN,
  IT→EN), one medium EN→FR paragraph, and two long paragraphs (EN→FR 267 source
  tokens, DE→EN 193 source tokens). The texts are original and
  redistributable.
- **Product cases** use the default heuristic early stopping and the default
  beam count (4), so they reflect what a user sees.
- **Forced-budget cases** run the long EN→FR fixture with
  `EarlyStopping::Never` at 32, 64, 128 and 200 tokens. These measure raw
  throughput and deliberately do not stop early.
- **Timing windows.** `generation_ms` covers tokenization, encoder, cache
  preparation, prefill, all decoder steps, logits, search and detokenization —
  i.e. everything the product does per request. `end_to_end_ms` is measured
  around the same sequence at the harness level. Model loading is timed
  separately and is never mixed into generation throughput.
- **Cold or warm?** The model is loaded once, then all cases run in the same
  process. Each case is repeated `--reps` times and the median is reported;
  the first repetition is therefore effectively a warm-up for the process and
  the page cache, but no case is artificially discarded.
- **Output.** Human-readable tables (per-case generation time, end-to-end time,
  min/max, tokens/s, ms/token, stage breakdown, decoder sub-stages) plus a
  machine-readable JSON document (`schema: "fluent-bench/1"`) containing every
  sample, environment data, model identity, load time and RSS.
- **Explicit unavailable data.** RSS is `null` on platforms where it cannot be
  read; the harness never invents a value.
- **Kernel micro-benchmark.** `--matmul-bench` times the Q8 kernel and the
  candle attention path without loading the model.

The f32 control is the same harness and the same binary with
`FLUENT_QUANT=off`, which disables the Q8 weight path:

```bash
FLUENT_QUANT=off cargo run --release -p fluent-core --example bench -- \
    models/nllb-200-1.3b --reps 1 --budgets 32,64,128,200 \
    --json /tmp/fluent-bench-f32.json
```

### Baseline caveat

The harness itself was introduced in Cycle 3, so there is no pristine
Cycle 2 harness run. The Cycle 2 figures above are historical observations made
with the previous ad-hoc methodology; they are context, not a controlled
comparison. The **f32 control on the current tree** is the apples-to-apples
reference for the Q8 effect: it uses the original candle f32 inference path and
the original cache representation. It does include Cycle 3's step-0 logits
shortcut, which makes it a slightly *faster* baseline than the literal Cycle 2
code, so the reported Q8 speedups are conservative.

## 4. Where the time went

Before optimization, profiling on the 200-token workload showed the decoder
forward pass and the language-model head dominated; tokenization was
negligible. The measured final breakdown on the long EN→FR case confirms the
shape of the problem:

| Stage (200 tokens, 267-token source) | f32 control | Q8 final |
|---|---:|---:|
| Tokenization | 0.4 ms | 0.4 ms |
| Encoder | 1,083 ms | 2,008 ms |
| Cross-attention cache preparation | 99 ms | 226 ms |
| Prefill | 359 ms | 74 ms |
| Decoder (24 layers × 200 steps) | 60,784 ms | 9,424 ms |
| └ FFN | 26,501 ms | 4,787 ms |
| └ Q/K/V/out projections | 9,050 ms | 2,083 ms |
| └ Cross-attention | 18,611 ms | 1,339 ms |
| └ Self-attention | 4,917 ms | 1,108 ms |
| └ KV-cache append/copy | 1,812 ms | 92 ms |
| Logits (`lm_head`) | 13,297 ms | 2,140 ms |
| Beam search (top-k, reorder, EOS) | 1,273 ms | 1,380 ms |
| Detokenization (preview) | 0.1 ms | 0.1 ms |
| **Generation total** | **80,181 ms** | **16,842 ms** |

Two conclusions drove the work:

1. Roughly 90% of the time was streaming f32 weights (decoder FFN,
   projections, `lm_head`) through the CPU. On this class of hardware that is a
   memory-bandwidth problem, not an arithmetic problem.
2. Cross-attention through the general candle broadcasting path was expensive
   (18.6 s over 200 steps) and the KV cache was rebuilt per step (1.8 s).

Tokenization and detokenization were measured and deliberately left alone.

## 5. Implemented optimizations

### 5.1 In-memory Q8 weight quantization (kept)

`crates/fluent-core/src/model/quant.rs`

- Weights are stored as Q8 blocks of 32 values with one f32 scale per block
  (the well-tested `Q8_0` layout). This is 9 bytes per 32 values instead of
  128 bytes, about 3.5× less traffic and resident memory.
- Activations are quantized per block at call time; the inner loop is an
  `i8 × i8 → i32` dot product accumulated in f32 per block. This keeps the
  arithmetic in f32 at block granularity without a full f32 copy of the weights.
- Two kernels: a register-blocked one for the decoding case (`m ≤ 8`, weights
  loaded once per block and reused across beams) and a prefill kernel for many
  input rows. Both have AVX2/FMA and scalar implementations.
- The AVX2 path is selected at runtime with
  `is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma")`. The
  portable build does not require `-C target-cpu` and always retains a scalar
  fallback.
- Quantization happens once during model load and is **never written back to
  disk**. The f32 tensor is dropped immediately after conversion, so the format
  is internal to the process.
- Enabled automatically only for the CPU f32 path. CUDA, Metal and explicit f16
  keep the previous candle path unchanged.
- Overrides for diagnosis and safety: `FLUENT_QUANT=off` disables the Q8 path;
  `FLUENT_QUANT_SIMD=off` forces the scalar kernels.

Why it is safe: the model file is untouched, the scheme is per-block and
symmetric, no new model asset or dependency on a quantized checkpoint format is
introduced, and the scalar fallback is always compiled in.

### 5.2 Fused CPU attention and preallocated KV cache (kept)

`crates/fluent-core/src/model/nllb.rs`

The general candle path transposes, broadcasts and concatenates tensors every
step. For the quantized CPU path, attention now runs through a small fused
kernel and a preallocated cache:

- `CpuDecoderCache` stores self-attention keys/values for all beams in a
  `[beams][heads][capacity][head_dim]` layout with a per-beam length. The cache
  is appended in place, repeated once after prefill, reordered when beams
  reorder, and advanced exactly once per decoder pass.
- Cross-attention keys/values are encoded once into the same compact layout and
  broadcast over beams without copying.
- The fused kernel computes scores, softmax and value accumulation without
  intermediate transposes or per-step allocation.
- The `Tensors` cache representation and the original candle attention remain
  the path for f32, CUDA and Metal. The change is scoped to the new CPU path.

Regression tests compare the fused kernel against the candle reference with
broadcast cross-attention, per-beam self-attention and stride padding, and test
cache repeat/reorder semantics.

### 5.3 Cancellation inside the encoder (kept)

`crates/fluent-core/src/model/nllb.rs`

The decoder already checked cancellation between steps, but a long source could
block inside `encoder.forward` for more than a second after the user pressed
Cancel. `Nllb::encode_with_cancel` now checks the flag between encoder layers;
the public `encode` remains as a non-cancellable wrapper. Post-signal latency
dropped from about 1.5 s to 17–62 ms in repeated measurements.

### 5.4 Step-0 logits shortcut (kept)

The first decoding step only needs the last beam's logits for candidate
selection; the harness/product do not need the full `lm_head` projection for
every beam at step 0. `decode_hidden` returns the hidden state and the head is
skipped where it cannot affect the result. This is a small but measurable
saving on every request and applies to both f32 and Q8.

### 5.5 Memory estimate follows the weight path (kept)

`crates/fluent-core/src/engine.rs`, `lib.rs`, `crates/fluent-gui/src/app.rs`

`estimate_memory` was conservative and ignored quantization, so the GUI warned
about ~5 GB when a Q8 CPU load actually needs ~1.6 GB. The new
`estimate_memory_for_choice` takes the device into account: CPU f32 uses the
9/32 weight factor, while unknown/GPU devices keep the conservative estimate.
`estimate_memory` remains the conservative public entry point. This only
affects the pre-load advisory; it does not change loading behaviour.

### 5.6 Instrumentation and harness (kept)

`GenerationStats` / `GenerationTimers` in `nllb.rs` record encode,
cross-cache preparation, prefill, decoder, `lm_head`, search, total, step count
and beam count, plus decoder sub-stages (embedding, projections, KV cache,
self-attention, cross-attention, FFN). They are zero-cost when unused and are
only collected through `generate_with_stats`; the product path is unchanged.
The harness described in section 3 consumes them.

## 6. Experiment log

| ID | Hypothesis | Change | Result | Decision |
|---|---|---|---|---|
| E1 | The decoder and `lm_head` dominate; tokenization is irrelevant | Added stage timers and a checked-in harness | Decoder + head ≈ 90% of 200-token time; tokenization 0.4 ms | Keep |
| E2 | CPU decoding is bandwidth-bound on f32 weights | In-memory Q8 blocks + AVX2/scalar kernels | 4.5–4.8× faster long-form; RSS 5.3 GB → 1.6 GB; identical outputs | Keep |
| E3 | General candle attention rebuilds and broadcasts too much per step | Fused CPU attention + preallocated per-beam cache | Cross-attention 18.6 s → 1.34 s at 200 tokens; KV cache 1.8 s → 0.09 s | Keep |
| E4 | The scalar fallback must remain correct on non-AVX2 CPUs | Forced `FLUENT_QUANT_SIMD=off` and ran the full test suite plus a benchmark | All tests pass; outputs identical; ~2.4× slower than AVX2, still faster than f32 | Keep |
| E5 | A long source delays cancellation inside the encoder | Check the flag between encoder layers | Post-signal latency 1.5 s → 17–62 ms | Keep |
| E6 | The GUI memory warning is wrong for Q8 | Device-aware weight factor | Advisory drops to the real footprint; conservative form retained | Keep |
| E7 | AVX-512 would add throughput on this CPU | Runtime-detected AVX-512 kernel | Not implemented: extra code paths and validation for a workload already 3.5× inside target; AVX2 is sufficient | Reject |
| E8 | Keeping the encoder in f32 would fix its prefill regression | Dense encoder + quantized decoder | Adds ~2 GB RSS and only saves ~1 s on long inputs; violates the memory goal | Reject |
| E9 | Fewer beams (1 or 2) would speed up decoding | Measure beam counts 1/2/4 | Beam search is a product-quality decision; no quality evidence justified changing the default, and the acceptance target is already met | Reject for this cycle |
| E10 | Thread count should be raised or pinned | Change rayon pool / `OMP_NUM_THREADS` | No measured benefit over the default pool; risks oversubscription on 6C/12T and on other machines | Reject |
| E11 | Dequantizing weights once per call would simplify the kernel | Dense f32 tiles per matmul | Reintroduces the f32 traffic that caused the bottleneck; rejected on first principles and by the stage profile | Reject |
| E12 | Quantize CUDA/Metal weights too | Extend Q8 to GPU paths | Untested backends in this environment and no CPU benefit; outside the cycle scope and risk budget | Reject |

## 7. Performance comparison

Same binary, same harness, same model, same machine. Q8 = 3 repetitions
(median); f32 control = 1 repetition because it is 5× slower and only used to
isolate the optimization.

### Short and product-like cases

| Case | Source tokens | Generated | f32 control | Q8 final | Speedup |
|---|---:|---:|---:|---:|---:|
| FR → EN | 13 | 7 | 2.89 s | 0.64 s | 4.50× |
| EN → FR | 15 | 16 | 5.45 s | 1.18 s | 4.63× |
| EN → DE | 15 | 19 | 6.32 s | 1.36 s | 4.65× |
| ES → EN | 16 | 11 | 4.02 s | 0.89 s | 4.52× |
| IT → EN | 20 | 20 | 6.66 s | 1.45 s | 4.61× |
| EN → FR medium paragraph | 81 | 64 | 21.04 s | 4.73 s | 4.45× |
| EN → FR long paragraph | 267 | 64 | 25.19 s | 6.58 s | 3.83× |
| DE → EN long paragraph | 193 | 64 | 23.17 s | 5.89 s | 3.93× |

### Forced token budgets (long EN → FR, `EarlyStopping::Never`)

| Budget | f32 control | Q8 final | Speedup | Q8 ms/token | Q8 tokens/s |
|---:|---:|---:|---:|---:|---:|
| 32 | 13.14 s | 4.38 s | 3.00× | 137.0 | 7.30 |
| 64 | 25.08 s | 6.56 s | 3.82× | 102.5 | 9.76 |
| 128 | 50.08 s | 11.19 s | 4.47× | 87.4 | 11.44 |
| 200 | 80.18 s | 16.84 s | 4.76× | 84.2 | 11.88 |

Dispersion at 200 tokens was 16.82–16.84 s across three repetitions. The
32-token case has a lower speedup because the fixed encoder cost is a larger
share of a short run, and the Q8 prefill kernel is currently slower than the
f32 GEMM at that matrix shape (see section 10).

### Cycle 2 context

The Cycle 2 reference observations were 2.83 s for short FR → EN, 15.80 s for a
44-token paragraph, 67.06 s for 84 generated tokens and ≈124.5 s for 200
generated tokens. They were produced with an earlier, less controlled
methodology, so only the direction is comparable:

- short FR → EN: 2.83 s → 0.64 s (4.4×)
- 200 generated tokens: ≈124.5 s → 16.84 s (7.4×)
- 84 generated tokens: 67.06 s in Cycle 2; the nearest measured points on the
  final tree are 64 tokens in 6.56 s and 128 tokens in 11.19 s

### Kernel micro-benchmark (`--matmul-bench`, m = 4)

| Matrix | Q8 ms/call | Effective GB/s | GFLOP/s |
|---|---:|---:|---:|
| 1024 × 1024 | 0.090 | 13.2 | 93.4 |
| 8192 × 1024 (FFN) | 0.325 | 29.0 | 206.5 |
| 256,206 × 1024 (`lm_head`) | 11.094 | 26.6 | 189.2 |

The `lm_head` cost matches the end-to-end measurement: 11.1 ms × 200 steps ≈
2.2 s, which is what the stage breakdown reports.

## 8. Translation quality

### Method

The fixed corpus covers:

- EN → FR (short, medium and long)
- FR → EN
- EN → DE
- ES → EN
- IT → EN
- punctuation, accents, numbers, proper nouns, paragraph boundaries, repeated
  terms, EOS behaviour and a forced 200-token continuation
- an additional manual near-truncation case: a 512-token English source
  truncated explicitly, translated EN → DE with a forced 32-token budget

### Result

- **All 12 harness cases produced byte-identical output between Q8 and f32**,
  including the 200-token forced continuation.
- **The near-truncation case was byte-identical**, with `truncated=true` in both
  runs. Q8 took 6.81 s; f32 took 18.62 s.
- The scalar Q8 fallback produced identical output to the AVX2 Q8 path on the
  same cases.

Byte-identical output is a stronger result than was required. It is a
consequence of the per-block Q8 scheme and the beam decisions staying stable;
it is not guaranteed for every input, and numerical behaviour should be
re-evaluated if the block size or kernel accumulation order changes in a future
cycle.

No BLEU-like score was used: the corpus is small, the outputs are identical, and
a corpus-level metric would add noise without adding information. This is a
limitation of the evidence, not a hidden regression.

## 9. Memory and reliability

### Memory

| Measurement | f32 control | Q8 final |
|---|---:|---:|
| RSS after load | 5,337 MB | 1,586 MB |
| Peak RSS while loading | 5,337 MB | 2,433 MB |
| Peak RSS over the whole run | 5,673 MB | 2,433 MB |
| RSS at end of run | 5,373 MB | 1,587 MB |
| Model load time | 5.72 s | 7.48 s |

Q8 uses about 3.4× less resident memory. The load is ~1.8 s slower because the
weights are quantized once at load; that cost is paid once per process and is
not included in generation timing. The Q8 peak during loading is higher than
steady state because the f32 tensor and the Q8 copy briefly coexist during
conversion; the f32 peak during the run comes from ordinary activation and
cache allocations.

### Repeated runs and cache correctness

- Three consecutive 48-token runs on the same source produced identical tokens
  and text; RSS stayed at 1,613–1,614 MB across runs, ending at 1,587 MB.
- Tests cover fused attention against the candle reference, cache
  repeat/reorder, per-beam cache state, early stopping, EOS discovery, source
  truncation and maximum token limits.
- No panics, deadlocks, shape errors or cross-translation contamination were
  observed.

### Cancellation

With cancellation signalled 200 / 500 / 1,500 ms into a 200-token run, the
translation returned in 230 / 517 / 1,563 ms respectively; post-signal latency
was **30 / 17 / 62 ms** after encoder-layer checks were added. Before that
change the same test showed ~1.5 s of post-signal latency.

### Model unloading

Model loading was exercised repeatedly by the benchmark process and the
diagnostic runs. A dedicated in-process unload/reload measurement was not added:
the product unloads by dropping the engine and, in practice, by ending the
process, so there is no user-facing path that keeps several models resident.
The absence of that measurement is recorded as a limitation rather than
presented as a pass.

## 10. Known limitations and remaining debt

1. **Q8 prefill is slower than f32 GEMM at large `m`.** On the 267-token
   source, encode is 2.01 s in Q8 versus 1.08 s in f32, and cross-cache
   preparation is 0.23 s versus 0.10 s. This costs about 1.05 s on a 200-token
   run. The decoder kernel is register-blocked across beams, but the prefill
   kernel loops inputs outside blocks and re-reads weight blocks per input; a
   tiled/register-blocked prefill kernel is the obvious next improvement.
   It does not threaten acceptance because the decoder and `lm_head` savings
   are much larger.
2. **Decoder is still bandwidth-bound.** Effective weight streaming is
   ~19–28 GB/s against a measured kernel peak of ~29 GB/s on this machine.
   Further gains need larger tiles, fewer passes over `lm_head`, or a different
   head-selection strategy — not more threads.
3. **Beam search overhead is now visible.** Search is 1.38 s of the 16.84 s
   run (8%). Beam reordering and top-k are single-threaded.
4. **Load time increased** by ~1.8 s because of one-time quantization.
5. **No AVX-512 path.** `avx512f` is present on the reference CPU but was not
   used; the added code paths and validation cost were disproportionate once
   the target was met.
6. **Long-standing Cycle 2 debt remains:** heuristic language/generation
   detection, GPU quantized paths untested, no packaging/CI/signing, no model
   acquisition flow, and the repository's MIT license coexists with
   CC BY-NC 4.0 NLLB weights.
7. **The f32 control is not a pristine Cycle 2 binary.** It is the current tree
   with quantization disabled, as explained in section 3.

## 11. Quality gates (final tree)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | Pass (no diff) |
| `cargo check --workspace` | Pass |
| `cargo test --workspace` | Pass: 42 core + 7 GUI = 49 tests, 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass (no warnings) |
| `cargo build --release -p fluent-gui` | Pass |
| `FLUENT_QUANT_SIMD=off cargo test -p fluent-core` | Pass: 42 tests |
| Q8 benchmark, 3 repetitions, 12 cases | Pass, JSON saved outside the repo |
| f32 control benchmark, 1 repetition | Pass |
| Near-truncation EN → DE comparison | Pass, byte-identical |

Benchmark JSON and logs are deliberately not committed. They contain absolute
paths and machine-specific data and would be stale as soon as the model or
compiler changes.

## 12. Reproducing the results

```bash
# Build the harness
cargo build --release -p fluent-core --example bench

# Q8 (default on CPU f32)
./target/release/examples/bench models/nllb-200-1.3b \
    --reps 3 --budgets 32,64,128,200 --json /tmp/fluent-bench.json

# f32 control
FLUENT_QUANT=off ./target/release/examples/bench models/nllb-200-1.3b \
    --reps 1 --budgets 32,64,128,200 --json /tmp/fluent-bench-f32.json

# Scalar fallback
FLUENT_QUANT_SIMD=off ./target/release/examples/bench models/nllb-200-1.3b \
    --reps 1 --budgets 32

# Kernel and attention micro-benchmarks
./target/release/examples/bench --matmul-bench
```

To compare outputs between paths, run both JSON files and compare the `output`
field for each `(id, budget, early_stopping)` key.

## 13. Recommendation for the next cycle

The measured next bottleneck is the quantized prefill kernel, followed by
`lm_head` and beam-search overhead. The highest-value next step is a
register-blocked/tiled prefill kernel (process 4–8 input rows per weight pass)
to remove the ~1 s encoder regression and improve 32-token latency. After that,
evaluate a tiled `lm_head` and a cheaper beam-reorder path. These are
incremental optimizations; the current cycle already meets its acceptance
targets with identical output quality.
