//! In-memory Q8 weight quantization for CPU inference.
//!
//! Long-form decoding on CPU is memory-bandwidth bound: a 1.3B-parameter f32
//! model streams several gigabytes of weights per generated token. Storing the
//! weights as 8-bit blocks with one f32 scale per 32 values cuts that traffic
//! by roughly 3.5x while keeping the arithmetic in f32 at block granularity.
//!
//! The format and the dot product follow the well-tested ggml `Q8_0` scheme:
//! activations are quantized per block at call time and the inner loop is an
//! `i8 * i8 -> i32` dot product. The scalar kernel is always available; on
//! x86-64 the AVX2 kernel is selected through `is_x86_feature_detected!` so the
//! portable build does not require `-C target-cpu`.
//!
//! Weights are never written back to disk. Quantization happens once, while
//! loading, and the f32 tensor is dropped immediately afterwards.

use std::sync::Arc;

use candle_core::{DType, Device, DeviceLocation, Tensor};
use candle_nn::{Linear, Module, VarBuilder};
use rayon::prelude::*;

use crate::{Error, Result};

/// Number of values sharing one scale.
pub const BLOCK: usize = 32;

/// One 32-value block, 32-byte aligned so AVX2 can use aligned loads.
#[repr(C, align(32))]
#[derive(Clone, Copy)]
struct QBlock {
    qs: [i8; BLOCK],
}

/// A row-major matrix stored as Q8 blocks.
pub struct QuantizedMatrix {
    rows: usize,
    cols: usize,
    blocks_per_row: usize,
    qs: Vec<QBlock>,
    scales: Vec<f32>,
}

impl QuantizedMatrix {
    /// Quantizes a 2-D tensor. `cols` must be a multiple of [`BLOCK`].
    pub fn from_tensor(weight: &Tensor) -> Result<Self> {
        let (rows, cols) = weight
            .dims2()
            .map_err(|_| Error::Other("quantized matrix must be 2-D".to_string()))?;
        if cols % BLOCK != 0 {
            return Err(Error::Other(format!(
                "cannot quantize a matrix with {cols} columns (not a multiple of {BLOCK})"
            )));
        }
        let blocks_per_row = cols / BLOCK;
        // `to_vec1` materializes a contiguous f32 copy; the source tensor is
        // dropped by the caller as soon as quantization finishes.
        let data = weight
            .to_dtype(DType::F32)?
            .contiguous()?
            .flatten_all()?
            .to_vec1::<f32>()?;
        let data = &data[..rows * cols];

        let mut qs = vec![QBlock { qs: [0i8; BLOCK] }; rows * blocks_per_row];
        let mut scales = vec![0f32; rows * blocks_per_row];
        qs.par_chunks_mut(blocks_per_row)
            .zip(scales.par_chunks_mut(blocks_per_row))
            .enumerate()
            .for_each(|(row, (qrow, srow))| {
                quantize_row(&data[row * cols..(row + 1) * cols], qrow, srow);
            });

        Ok(Self {
            rows,
            cols,
            blocks_per_row,
            qs,
            scales,
        })
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Approximate number of bytes held by this matrix.
    pub fn size_bytes(&self) -> usize {
        self.qs.len() * std::mem::size_of::<QBlock>()
            + self.scales.len() * std::mem::size_of::<f32>()
    }

    /// Computes `x @ weightᵀ` for an f32 CPU tensor.
    pub fn matmul(&self, x: &Tensor) -> Result<Tensor> {
        self.matmul_with(x, Prefill::Tiled, !simd_disabled())
    }

    /// Test/reference entry point. `simd == false` forces the scalar kernels
    /// even on a machine that supports AVX2.
    fn matmul_with(&self, x: &Tensor, prefill: Prefill, simd: bool) -> Result<Tensor> {
        let dims = x.dims().to_vec();
        let cols = self.cols;
        let Some(&last) = dims.last() else {
            return Err(Error::Other("quantized matmul on a scalar".to_string()));
        };
        if last != cols {
            return Err(Error::Other(format!(
                "quantized matmul expects {cols} input columns, got {last}"
            )));
        }
        let m: usize = dims[..dims.len() - 1].iter().product();
        if m == 0 {
            let mut out_dims = dims[..dims.len() - 1].to_vec();
            out_dims.push(self.rows);
            return Ok(Tensor::zeros(out_dims, DType::F32, &Device::Cpu)?);
        }

        let data = x
            .to_dtype(DType::F32)?
            .to_device(&Device::Cpu)?
            .contiguous()?
            .flatten_all()?
            .to_vec1::<f32>()?;
        let data = &data[..m * cols];

        let nb = self.blocks_per_row;
        let mut aq = vec![0i8; m * cols];
        let mut ad = vec![0f32; m * nb];
        for row in 0..m {
            quantize_activations(
                &data[row * cols..(row + 1) * cols],
                &mut aq[row * cols..(row + 1) * cols],
                &mut ad[row * nb..(row + 1) * nb],
            );
        }

        let mut dst = vec![0f32; m * self.rows];
        let args = MatmulArgs {
            qs: self.qs.as_ptr(),
            scales: self.scales.as_ptr(),
            aq: aq.as_ptr(),
            ad: ad.as_ptr(),
            m,
            rows: self.rows,
            cols,
            blocks_per_row: nb,
            dst: dst.as_mut_ptr(),
        };

        let work = m.saturating_mul(self.rows).saturating_mul(cols);
        let workers = if work >= 1_000_000 {
            rayon::current_num_threads().max(1)
        } else {
            1
        };
        let chunk = self.rows.div_ceil(workers).max(1);
        let chunks = self.rows.div_ceil(chunk);

        #[cfg(target_arch = "x86_64")]
        let avx2 = simd && is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma");
        #[cfg(not(target_arch = "x86_64"))]
        let avx2 = {
            let _ = simd;
            false
        };

        if m <= 8 {
            let kernel = select_small(avx2);
            (0..chunks).into_par_iter().for_each(|index| {
                let from = index * chunk;
                let to = ((index + 1) * chunk).min(self.rows);
                // SAFETY: chunks write disjoint, in-bounds output ranges and
                // all input pointers stay valid for the duration of the loop.
                unsafe { kernel(&args, from, to) };
            });
        } else if prefill == Prefill::Legacy {
            let kernel = select_large_legacy(avx2);
            (0..chunks).into_par_iter().for_each(|index| {
                let from = index * chunk;
                let to = ((index + 1) * chunk).min(self.rows);
                // SAFETY: as above.
                unsafe { kernel(&args, 0, m, from, to) };
            });
        } else {
            let tile = PREFILL_TILE;
            let full_tiles = m / tile;
            let tail = m - full_tiles * tile;
            let task_count = (full_tiles + usize::from(tail > 0)) * chunks;
            // Parallelize over `(input tile, output chunk)` so each task walks
            // a contiguous output range while its activation tile stays hot in
            // L1. Every task writes a disjoint `(input, output)` rectangle.
            (0..task_count).into_par_iter().for_each(|task| {
                let tile_index = task / chunks;
                let chunk_index = task % chunks;
                let from = chunk_index * chunk;
                let to = ((chunk_index + 1) * chunk).min(self.rows);
                if tile_index < full_tiles {
                    let start = tile_index * tile;
                    run_tile(&args, avx2, start, from, to);
                } else {
                    // Partial final tile: at most `tile - 1` inputs, processed
                    // by the row-at-a-time kernel so no input is skipped.
                    let start = full_tiles * tile;
                    let kernel = select_large_legacy(avx2);
                    // SAFETY: as above.
                    unsafe { kernel(&args, start, tail, from, to) };
                }
            });
        }

        let mut out_dims = dims[..dims.len() - 1].to_vec();
        out_dims.push(self.rows);
        Ok(Tensor::from_vec(dst, out_dims, &Device::Cpu)?)
    }

    /// Looks up rows by token id, dequantizing only the requested rows.
    pub fn embedding(&self, ids: &Tensor) -> Result<Tensor> {
        let dims = ids.dims().to_vec();
        let count: usize = dims.iter().product();
        let ids = ids
            .to_dtype(DType::U32)?
            .to_device(&Device::Cpu)?
            .flatten_all()?;
        let ids = ids.to_vec1::<u32>()?;
        let cols = self.cols;
        let nb = self.blocks_per_row;
        let mut out = vec![0f32; count.saturating_mul(cols)];
        for (index, &id) in ids.iter().enumerate() {
            let row = id as usize;
            if row >= self.rows {
                return Err(Error::Other(format!(
                    "embedding id {row} is out of range for {} rows",
                    self.rows
                )));
            }
            let dst = &mut out[index * cols..(index + 1) * cols];
            for block in 0..nb {
                let scale = self.scales[row * nb + block];
                let qs = &self.qs[row * nb + block].qs;
                let dst = &mut dst[block * BLOCK..(block + 1) * BLOCK];
                for (value, &q) in dst.iter_mut().zip(qs.iter()) {
                    *value = q as f32 * scale;
                }
            }
        }
        let mut out_dims = dims;
        out_dims.push(cols);
        Ok(Tensor::from_vec(out, out_dims, &Device::Cpu)?)
    }
}

fn quantize_row(src: &[f32], qrow: &mut [QBlock], srow: &mut [f32]) {
    for (index, (block, scale)) in qrow.iter_mut().zip(srow.iter_mut()).enumerate() {
        let src = &src[index * BLOCK..(index + 1) * BLOCK];
        let mut amax = 0f32;
        for &value in src {
            amax = amax.max(value.abs());
        }
        let d = amax / 127.0;
        *scale = d;
        if d > 0.0 {
            let inv = 1.0 / d;
            for (q, &value) in block.qs.iter_mut().zip(src.iter()) {
                *q = (value * inv).round().clamp(-127.0, 127.0) as i8;
            }
        } else {
            block.qs.fill(0);
        }
    }
}

fn quantize_activations(src: &[f32], qs: &mut [i8], scales: &mut [f32]) {
    for (block, scale) in scales.iter_mut().enumerate() {
        let src = &src[block * BLOCK..(block + 1) * BLOCK];
        let dst = &mut qs[block * BLOCK..(block + 1) * BLOCK];
        let mut amax = 0f32;
        for &value in src {
            amax = amax.max(value.abs());
        }
        let d = amax / 127.0;
        *scale = d;
        if d > 0.0 {
            let inv = 1.0 / d;
            for (q, &value) in dst.iter_mut().zip(src.iter()) {
                *q = (value * inv).round().clamp(-127.0, 127.0) as i8;
            }
        } else {
            dst.fill(0);
        }
    }
}

struct MatmulArgs {
    qs: *const QBlock,
    scales: *const f32,
    aq: *const i8,
    ad: *const f32,
    m: usize,
    rows: usize,
    cols: usize,
    blocks_per_row: usize,
    dst: *mut f32,
}

// SAFETY: the struct only carries pointers into buffers that outlive the
// parallel loop, and each chunk writes a disjoint output range.
unsafe impl Send for MatmulArgs {}
unsafe impl Sync for MatmulArgs {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Prefill {
    Tiled,
    /// Row-at-a-time kernel, retained only as the reference used by the
    /// exactness tests.
    #[cfg_attr(not(test), allow(dead_code))]
    Legacy,
}

type SmallKernel = unsafe fn(&MatmulArgs, usize, usize);
type LargeKernel = unsafe fn(&MatmulArgs, usize, usize, usize, usize);

/// Number of input rows processed together while weights stream past.
///
/// Eight keeps the activation tile at 8 × 1024 × 4 B = 32 KiB, which fits in
/// L1 alongside the weight row, and leaves enough ymm registers for eight f32
/// accumulators plus the widened weight block.
const PREFILL_TILE: usize = 8;
// The AVX2 tile kernel uses eight explicit accumulator registers.
const _: () = assert!(PREFILL_TILE == 8);

fn select_small(simd: bool) -> SmallKernel {
    #[cfg(target_arch = "x86_64")]
    {
        if simd {
            return run_small_avx2;
        }
    }
    let _ = simd;
    run_small_scalar
}

fn select_large_legacy(simd: bool) -> LargeKernel {
    #[cfg(target_arch = "x86_64")]
    {
        if simd {
            return run_large_avx2;
        }
    }
    let _ = simd;
    run_large_scalar
}

fn run_tile(a: &MatmulArgs, simd: bool, input_start: usize, from: usize, to: usize) {
    match simd {
        #[cfg(target_arch = "x86_64")]
        true => unsafe { run_tile_avx2(a, input_start, from, to) },
        _ => unsafe { run_tile_scalar::<PREFILL_TILE>(a, input_start, from, to) },
    }
}

/// Scalar kernel for the decoding case (at most eight rows per call).
///
/// The weight block is loaded once and reused for every input row; the
/// accumulator stays in a register for the whole row instead of being reduced
/// block by block, which keeps the dependency chain short.
unsafe fn run_small_scalar(a: &MatmulArgs, from: usize, to: usize) {
    let nb = a.blocks_per_row;
    let cols = a.cols;
    let m = a.m;
    for row in from..to {
        let wrow = a.qs.add(row * nb) as *const i8;
        let srow = a.scales.add(row * nb);
        let mut acc = [0f32; 8];
        for block in 0..nb {
            let ws = *srow.add(block);
            let w = wrow.add(block * BLOCK);
            for (input, slot) in acc.iter_mut().enumerate().take(m) {
                let act = a.aq.add(input * cols + block * BLOCK);
                let mut sum = 0i32;
                for index in 0..BLOCK {
                    sum += (*w.add(index) as i32) * (*act.add(index) as i32);
                }
                *slot += ws * *a.ad.add(input * nb + block) * sum as f32;
            }
        }
        for (input, &value) in acc.iter().enumerate().take(m) {
            *a.dst.add(input * a.rows + row) = value;
        }
    }
}

/// Scalar kernel for prefill (row-at-a-time reference path).
unsafe fn run_large_scalar(
    a: &MatmulArgs,
    input_start: usize,
    input_count: usize,
    from: usize,
    to: usize,
) {
    let nb = a.blocks_per_row;
    let cols = a.cols;
    for row in from..to {
        let wrow = a.qs.add(row * nb) as *const i8;
        let srow = a.scales.add(row * nb);
        for input in input_start..input_start + input_count {
            let act = a.aq.add(input * cols);
            let drow = a.ad.add(input * nb);
            let mut acc = 0f32;
            for block in 0..nb {
                let w = wrow.add(block * BLOCK);
                let aptr = act.add(block * BLOCK);
                let mut sum = 0i32;
                for index in 0..BLOCK {
                    sum += (*w.add(index) as i32) * (*aptr.add(index) as i32);
                }
                acc += *srow.add(block) * *drow.add(block) * sum as f32;
            }
            *a.dst.add(input * a.rows + row) = acc;
        }
    }
}

/// Scalar tiled prefill: `T` input rows share each weight-block traversal.
///
/// The per-input arithmetic is identical to [`run_large_scalar`], including the
/// order of the scale multiplications, so results are bit-identical.
unsafe fn run_tile_scalar<const T: usize>(
    a: &MatmulArgs,
    input_start: usize,
    from: usize,
    to: usize,
) {
    let nb = a.blocks_per_row;
    let cols = a.cols;
    for row in from..to {
        let wrow = a.qs.add(row * nb) as *const i8;
        let srow = a.scales.add(row * nb);
        let mut acc = [0f32; T];
        for block in 0..nb {
            let ws = *srow.add(block);
            let w = wrow.add(block * BLOCK);
            for (slot, input) in acc.iter_mut().zip(0..T) {
                let act = a.aq.add((input_start + input) * cols + block * BLOCK);
                let mut sum = 0i32;
                for index in 0..BLOCK {
                    sum += (*w.add(index) as i32) * (*act.add(index) as i32);
                }
                *slot += ws * *a.ad.add((input_start + input) * nb + block) * sum as f32;
            }
        }
        for (input, &value) in acc.iter().enumerate() {
            *a.dst.add((input_start + input) * a.rows + row) = value;
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn run_small_avx2(a: &MatmulArgs, from: usize, to: usize) {
    use std::arch::x86_64::*;

    let nb = a.blocks_per_row;
    let cols = a.cols;
    let m = a.m;
    for row in from..to {
        let wrow = a.qs.add(row * nb) as *const i8;
        let srow = a.scales.add(row * nb);
        let mut acc = [_mm256_setzero_ps(); 8];
        for block in 0..nb {
            let ws = *srow.add(block);
            let wv = _mm256_loadu_si256(wrow.add(block * BLOCK) as *const __m256i);
            let w_lo = _mm256_cvtepi8_epi16(_mm256_castsi256_si128(wv));
            let w_hi = _mm256_cvtepi8_epi16(_mm256_extracti128_si256::<1>(wv));
            for (input, slot) in acc.iter_mut().enumerate().take(m) {
                let av =
                    _mm256_loadu_si256(a.aq.add(input * cols + block * BLOCK) as *const __m256i);
                let a_lo = _mm256_cvtepi8_epi16(_mm256_castsi256_si128(av));
                let a_hi = _mm256_cvtepi8_epi16(_mm256_extracti128_si256::<1>(av));
                let products =
                    _mm256_add_epi32(_mm256_madd_epi16(w_lo, a_lo), _mm256_madd_epi16(w_hi, a_hi));
                let d = ws * *a.ad.add(input * nb + block);
                *slot = _mm256_fmadd_ps(_mm256_cvtepi32_ps(products), _mm256_set1_ps(d), *slot);
            }
        }
        for (input, slot) in acc.iter().enumerate().take(m) {
            *a.dst.add(input * a.rows + row) = hsum_ps(*slot);
        }
    }
}

/// Row-at-a-time AVX2 prefill kernel (reference path for A/B and partial tiles).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn run_large_avx2(
    a: &MatmulArgs,
    input_start: usize,
    input_count: usize,
    from: usize,
    to: usize,
) {
    use std::arch::x86_64::*;

    let nb = a.blocks_per_row;
    let cols = a.cols;
    for row in from..to {
        let wrow = a.qs.add(row * nb) as *const i8;
        let srow = a.scales.add(row * nb);
        for input in input_start..input_start + input_count {
            let act = a.aq.add(input * cols);
            let drow = a.ad.add(input * nb);
            let mut acc = _mm256_setzero_ps();
            for block in 0..nb {
                let wv = _mm256_loadu_si256(wrow.add(block * BLOCK) as *const __m256i);
                let av = _mm256_loadu_si256(act.add(block * BLOCK) as *const __m256i);
                let w_lo = _mm256_cvtepi8_epi16(_mm256_castsi256_si128(wv));
                let w_hi = _mm256_cvtepi8_epi16(_mm256_extracti128_si256::<1>(wv));
                let a_lo = _mm256_cvtepi8_epi16(_mm256_castsi256_si128(av));
                let a_hi = _mm256_cvtepi8_epi16(_mm256_extracti128_si256::<1>(av));
                let products =
                    _mm256_add_epi32(_mm256_madd_epi16(w_lo, a_lo), _mm256_madd_epi16(w_hi, a_hi));
                let d = *srow.add(block) * *drow.add(block);
                acc = _mm256_fmadd_ps(_mm256_cvtepi32_ps(products), _mm256_set1_ps(d), acc);
            }
            *a.dst.add(input * a.rows + row) = hsum_ps(acc);
        }
    }
}

/// Widens one Q8 block and accumulates its scaled integer dot product.
///
/// Kept as a macro so the tile size is a compile-time constant and every
/// accumulator stays in a ymm register. The arithmetic is exactly the same as
/// [`run_large_avx2`]: `madd_epi16` pairs, one `i32 -> f32` conversion, one
/// fused multiply-add per block, horizontal sum at the end.
#[cfg(target_arch = "x86_64")]
macro_rules! tile_step {
    ($slot:ident, $input:expr, $w_lo:expr, $w_hi:expr, $ws:expr, $a:expr, $nb:expr, $cols:expr, $block:expr) => {{
        use std::arch::x86_64::*;
        let av =
            _mm256_loadu_si256($a.aq.add(($input) * $cols + ($block) * BLOCK) as *const __m256i);
        let a_lo = _mm256_cvtepi8_epi16(_mm256_castsi256_si128(av));
        let a_hi = _mm256_cvtepi8_epi16(_mm256_extracti128_si256::<1>(av));
        let products = _mm256_add_epi32(
            _mm256_madd_epi16($w_lo, a_lo),
            _mm256_madd_epi16($w_hi, a_hi),
        );
        let d = $ws * *$a.ad.add(($input) * $nb + ($block));
        $slot = _mm256_fmadd_ps(_mm256_cvtepi32_ps(products), _mm256_set1_ps(d), $slot);
    }};
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn run_tile_avx2(a: &MatmulArgs, input_start: usize, from: usize, to: usize) {
    use std::arch::x86_64::*;

    let nb = a.blocks_per_row;
    let cols = a.cols;
    let base = input_start;
    for row in from..to {
        let wrow = a.qs.add(row * nb) as *const i8;
        let srow = a.scales.add(row * nb);
        let mut acc0 = _mm256_setzero_ps();
        let mut acc1 = _mm256_setzero_ps();
        let mut acc2 = _mm256_setzero_ps();
        let mut acc3 = _mm256_setzero_ps();
        let mut acc4 = _mm256_setzero_ps();
        let mut acc5 = _mm256_setzero_ps();
        let mut acc6 = _mm256_setzero_ps();
        let mut acc7 = _mm256_setzero_ps();
        for block in 0..nb {
            let ws = *srow.add(block);
            let wv = _mm256_loadu_si256(wrow.add(block * BLOCK) as *const __m256i);
            let w_lo = _mm256_cvtepi8_epi16(_mm256_castsi256_si128(wv));
            let w_hi = _mm256_cvtepi8_epi16(_mm256_extracti128_si256::<1>(wv));
            tile_step!(acc0, base, w_lo, w_hi, ws, a, nb, cols, block);
            tile_step!(acc1, base + 1, w_lo, w_hi, ws, a, nb, cols, block);
            tile_step!(acc2, base + 2, w_lo, w_hi, ws, a, nb, cols, block);
            tile_step!(acc3, base + 3, w_lo, w_hi, ws, a, nb, cols, block);
            tile_step!(acc4, base + 4, w_lo, w_hi, ws, a, nb, cols, block);
            tile_step!(acc5, base + 5, w_lo, w_hi, ws, a, nb, cols, block);
            tile_step!(acc6, base + 6, w_lo, w_hi, ws, a, nb, cols, block);
            tile_step!(acc7, base + 7, w_lo, w_hi, ws, a, nb, cols, block);
        }
        *a.dst.add(base * a.rows + row) = hsum_ps(acc0);
        *a.dst.add((base + 1) * a.rows + row) = hsum_ps(acc1);
        *a.dst.add((base + 2) * a.rows + row) = hsum_ps(acc2);
        *a.dst.add((base + 3) * a.rows + row) = hsum_ps(acc3);
        *a.dst.add((base + 4) * a.rows + row) = hsum_ps(acc4);
        *a.dst.add((base + 5) * a.rows + row) = hsum_ps(acc5);
        *a.dst.add((base + 6) * a.rows + row) = hsum_ps(acc6);
        *a.dst.add((base + 7) * a.rows + row) = hsum_ps(acc7);
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn hsum_ps(value: std::arch::x86_64::__m256) -> f32 {
    use std::arch::x86_64::*;
    let hi = _mm256_extractf128_ps::<1>(value);
    let lo = _mm256_castps256_ps128(value);
    let sum = _mm_add_ps(lo, hi);
    let sum = _mm_hadd_ps(sum, sum);
    let sum = _mm_hadd_ps(sum, sum);
    _mm_cvtss_f32(sum)
}

// ---------------------------------------------------------------------------
// Linear layers
// ---------------------------------------------------------------------------

/// A linear layer that is either a regular candle [`Linear`] or a quantized
/// matrix. Keeping both makes the GPU/f32 paths identical to the previous
/// implementation.
pub enum Projection {
    Dense {
        weight: Linear,
    },
    Quant {
        matrix: Arc<QuantizedMatrix>,
        bias: Option<Tensor>,
    },
}

impl Projection {
    pub fn load(vb: VarBuilder, in_dim: usize, out_dim: usize, quantize: bool) -> Result<Self> {
        let weight = vb.get((out_dim, in_dim), "weight")?;
        let bias = if vb.contains_tensor("bias") {
            Some(vb.get(out_dim, "bias")?)
        } else {
            None
        };
        if quantize {
            let matrix = QuantizedMatrix::from_tensor(&weight)?;
            Ok(Self::Quant {
                matrix: Arc::new(matrix),
                bias,
            })
        } else {
            Ok(Self::Dense {
                weight: Linear::new(weight, bias),
            })
        }
    }

    pub fn from_matrix(matrix: Arc<QuantizedMatrix>, bias: Option<Tensor>) -> Self {
        Self::Quant { matrix, bias }
    }

    pub fn from_linear(weight: Linear) -> Self {
        Self::Dense { weight }
    }

    pub fn is_quantized(&self) -> bool {
        matches!(self, Self::Quant { .. })
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        match self {
            Self::Dense { weight } => Ok(weight.forward(x)?),
            Self::Quant { matrix, bias } => {
                let out = matrix.matmul(x)?;
                match bias {
                    Some(bias) => Ok(out.broadcast_add(bias)?),
                    None => Ok(out),
                }
            }
        }
    }
}

/// The shared input/output embedding.
pub enum Embedding {
    Dense(Tensor),
    Quant(Arc<QuantizedMatrix>),
}

impl Embedding {
    pub fn from_tensor(tensor: Tensor, quantize: bool) -> Result<Self> {
        if quantize {
            Ok(Self::Quant(Arc::new(QuantizedMatrix::from_tensor(
                &tensor,
            )?)))
        } else {
            Ok(Self::Dense(tensor))
        }
    }

    pub fn embedding(&self, ids: &Tensor) -> Result<Tensor> {
        match self {
            Self::Dense(weight) => Ok(weight.embedding(ids)?),
            Self::Quant(matrix) => matrix.embedding(ids),
        }
    }

    pub fn lm_head(&self) -> Result<Projection> {
        match self {
            Self::Dense(weight) => Ok(Projection::from_linear(Linear::new(weight.clone(), None))),
            Self::Quant(matrix) => Ok(Projection::from_matrix(matrix.clone(), None)),
        }
    }

    pub fn is_quantized(&self) -> bool {
        matches!(self, Self::Quant(_))
    }

    pub fn device(&self) -> &Device {
        match self {
            Self::Dense(tensor) => tensor.device(),
            Self::Quant(_) => &Device::Cpu,
        }
    }
}

/// Returns the SIMD backend selected for quantized matmuls.
///
/// Exposed for benchmark diagnostics and for testing the scalar fallback.
pub fn simd_backend() -> &'static str {
    if simd_disabled() {
        return "scalar (forced)";
    }
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            return "avx2";
        }
    }
    "scalar"
}

fn simd_disabled() -> bool {
    std::env::var("FLUENT_QUANT_SIMD")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            value == "off" || value == "0" || value == "false" || value == "scalar"
        })
        .unwrap_or(false)
}

/// Whether weights should be quantized in memory.
///
/// Quantization is limited to CPU f32 builds: the kernels produce f32 CPU
/// tensors and the CUDA/Metal backends already use their own reduced precision.
/// `FLUENT_QUANT=off` disables it for diagnostics and quality comparisons.
pub fn quantization_enabled(device: &Device, dtype: DType) -> bool {
    let disabled = std::env::var("FLUENT_QUANT")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            value == "off" || value == "0" || value == "false" || value == "none"
        })
        .unwrap_or(false);
    !disabled && device.location() == DeviceLocation::Cpu && dtype == DType::F32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix(rows: usize, cols: usize, data: Vec<f32>) -> QuantizedMatrix {
        let tensor = Tensor::from_vec(data, (rows, cols), &Device::Cpu).unwrap();
        QuantizedMatrix::from_tensor(&tensor).unwrap()
    }

    #[test]
    fn quantized_matmul_is_close_to_f32() {
        let rows = 7;
        let cols = 64;
        let weights: Vec<f32> = (0..rows * cols)
            .map(|index| ((index * 37 % 101) as f32 - 50.0) / 50.0)
            .collect();
        let input: Vec<f32> = (0..2 * cols)
            .map(|index| ((index * 13 % 29) as f32 - 14.0) / 14.0)
            .collect();
        let quantized = matrix(rows, cols, weights.clone());
        let x = Tensor::from_vec(input.clone(), (2, cols), &Device::Cpu).unwrap();
        let out = quantized.matmul(&x).unwrap().to_vec2::<f32>().unwrap();

        let reference = Tensor::from_vec(weights, (rows, cols), &Device::Cpu)
            .unwrap()
            .matmul(
                &Tensor::from_vec(input, (2, cols), &Device::Cpu)
                    .unwrap()
                    .t()
                    .unwrap(),
            )
            .unwrap()
            .to_vec2::<f32>()
            .unwrap();

        for row in 0..2 {
            for col in 0..rows {
                // `reference` is `[rows, batch]`; `out` is `[batch, rows]`.
                let expected = reference[col][row];
                let actual = out[row][col];
                // Both operands are quantized, so the dot product has roughly
                // the ~1/128 block resolution of ggml Q8_0.
                assert!(
                    (expected - actual).abs() <= 0.08 * expected.abs().max(1.0),
                    "row {row} col {col}: expected {expected}, got {actual}"
                );
            }
        }
    }

    fn pseudo_random(count: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..count)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 8) as f32 / 8_388_608.0 - 1.0
            })
            .collect()
    }

    fn bits(matrix: &QuantizedMatrix, x: &Tensor, prefill: Prefill, simd: bool) -> Vec<u32> {
        matrix
            .matmul_with(x, prefill, simd)
            .unwrap()
            .flatten_all()
            .unwrap()
            .to_vec1::<f32>()
            .unwrap()
            .into_iter()
            .map(f32::to_bits)
            .collect()
    }

    #[test]
    fn tiled_prefill_is_bit_identical_to_the_reference_kernel() {
        // One block, three blocks and four blocks; several output-row counts
        // so the rayons chunks and the partial final input tile are exercised.
        for (rows, cols) in [(5usize, 64usize), (37, 64), (23, 96), (41, 128)] {
            let weights = pseudo_random(rows * cols, 7);
            let quantized = matrix(rows, cols, weights);
            for m in [1usize, 3, 7, 8, 9, 15, 16, 17, 23, 31, 32, 33, 67] {
                let x =
                    Tensor::from_vec(pseudo_random(m * cols, 11), (m, cols), &Device::Cpu).unwrap();
                for simd in [false, true] {
                    let reference = bits(&quantized, &x, Prefill::Legacy, simd);
                    let tiled = bits(&quantized, &x, Prefill::Tiled, simd);
                    assert_eq!(
                        reference, tiled,
                        "rows={rows} cols={cols} m={m} simd={simd}"
                    );
                }
            }
        }
    }

    #[test]
    fn tiled_prefill_handles_extreme_magnitudes_bit_identically() {
        let (rows, cols) = (9usize, 64usize);
        let extreme = [0.0f32, 1e-30, -1e-30, 1.0, -1.0, 1e30, -1e30, 127.0, -127.0];
        let weights: Vec<f32> = (0..rows * cols)
            .map(|index| extreme[(index * 7 + 3) % extreme.len()])
            .collect();
        let quantized = matrix(rows, cols, weights);
        for m in [1usize, 8, 9, 17] {
            let input: Vec<f32> = (0..m * cols)
                .map(|index| extreme[(index * 5 + 1) % extreme.len()])
                .collect();
            let x = Tensor::from_vec(input, (m, cols), &Device::Cpu).unwrap();
            for simd in [false, true] {
                let reference = bits(&quantized, &x, Prefill::Legacy, simd);
                let tiled = bits(&quantized, &x, Prefill::Tiled, simd);
                // Scales can overflow f32 for absurd inputs; that is the
                // existing f32 behaviour and both kernels must agree on it,
                // including infinities and NaNs.
                assert_eq!(reference, tiled, "m={m} simd={simd}");
            }
        }
    }

    #[test]
    fn integer_dot_products_fit_in_i32_for_supported_dimensions() {
        // Each Q8 value is in [-127, 127], so a full block of 32 products is
        // bounded by 32 * 127 * 127 = 516,128, well inside i32. The kernel
        // accumulates *scaled* block results in f32, so no cross-block integer
        // accumulator exists to overflow.
        let block_max = BLOCK as i64 * 127 * 127;
        assert_eq!(block_max, 516_128);
        // Even a single integer accumulator over every block of an 8192-wide
        // row would need 8192 * 127 * 127 = 132,128,768 < i32::MAX.
        assert!(8192i64 * 127 * 127 <= i32::MAX as i64);
        // The maximum amplitude used by the model is far below this: the
        // kernel never mixes more than one block before applying an f32 scale.
        assert!(block_max <= i32::MAX as i64);
    }

    #[test]
    fn embedding_lookup_matches_dequantized_rows() {
        let weights: Vec<f32> = (0..4 * 32).map(|index| index as f32 / 32.0).collect();
        let quantized = matrix(4, 32, weights.clone());
        let ids = Tensor::from_vec(vec![1u32, 3], (1, 2), &Device::Cpu).unwrap();
        let out = quantized.embedding(&ids).unwrap();
        assert_eq!(out.dims(), &[1, 2, 32]);
        let values = out.to_vec3::<f32>().unwrap();
        // Q8 blocks carry an f32 scale, so the error is bounded by half a step
        // (about 0.016 for these rows), not by float rounding.
        assert!((values[0][0][0] - 1.0).abs() < 0.02);
        assert!((values[0][1][0] - 3.0).abs() < 0.02);
    }

    #[test]
    fn rejects_matrices_that_cannot_be_blocked() {
        let tensor = Tensor::zeros((2, 33), DType::F32, &Device::Cpu).unwrap();
        assert!(QuantizedMatrix::from_tensor(&tensor).is_err());
    }

    #[test]
    fn out_of_range_embedding_ids_are_rejected() {
        let quantized = matrix(2, 32, vec![0.0; 64]);
        let ids = Tensor::from_vec(vec![2u32], 1, &Device::Cpu).unwrap();
        assert!(quantized.embedding(&ids).is_err());
    }
}
