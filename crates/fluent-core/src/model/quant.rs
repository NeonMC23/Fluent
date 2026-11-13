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
        let avx2 =
            !simd_disabled() && is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma");
        #[cfg(not(target_arch = "x86_64"))]
        let avx2 = false;

        let (small, large) = select_kernels(avx2);
        let kernel = if m <= 8 { small } else { large };
        (0..chunks).into_par_iter().for_each(|index| {
            let from = index * chunk;
            let to = ((index + 1) * chunk).min(self.rows);
            // SAFETY: the chunks write disjoint, in-bounds output ranges and all
            // input pointers stay valid for the duration of the loop.
            unsafe { kernel(&args, from, to) };
        });

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

type Kernel = unsafe fn(&MatmulArgs, usize, usize);

fn select_kernels(simd: bool) -> (Kernel, Kernel) {
    #[cfg(target_arch = "x86_64")]
    {
        if simd {
            return (run_small_avx2, run_large_avx2);
        }
    }
    let _ = simd;
    (run_small_scalar, run_large_scalar)
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

/// Scalar kernel for prefill (many input rows).
unsafe fn run_large_scalar(a: &MatmulArgs, from: usize, to: usize) {
    let nb = a.blocks_per_row;
    let cols = a.cols;
    for row in from..to {
        let wrow = a.qs.add(row * nb) as *const i8;
        let srow = a.scales.add(row * nb);
        for input in 0..a.m {
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

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn run_large_avx2(a: &MatmulArgs, from: usize, to: usize) {
    use std::arch::x86_64::*;

    let nb = a.blocks_per_row;
    let cols = a.cols;
    for row in from..to {
        let wrow = a.qs.add(row * nb) as *const i8;
        let srow = a.scales.add(row * nb);
        for input in 0..a.m {
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
