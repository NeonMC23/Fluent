use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

use candle_core::{DType, Device, Tensor};
use candle_nn::{layer_norm, LayerNorm, Module, VarBuilder};
use rayon::prelude::*;
use serde::Deserialize;

use crate::tokenizer::NllbTokenizer;
use crate::{Error, Result};

use super::checkpoint::Checkpoint;
use super::quant::{quantization_enabled, Embedding, Projection};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    #[default]
    Relu,
    Gelu,
    Silu,
}

impl Activation {
    fn apply(self, x: &Tensor) -> Result<Tensor> {
        Ok(match self {
            Self::Relu => x.relu()?,
            Self::Gelu => x.gelu()?,
            Self::Silu => x.silu()?,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NllbConfig {
    pub d_model: usize,
    pub encoder_attention_heads: usize,
    pub decoder_attention_heads: usize,
    pub encoder_ffn_dim: usize,
    pub decoder_ffn_dim: usize,
    pub encoder_layers: usize,
    pub decoder_layers: usize,
    pub vocab_size: usize,
    #[serde(default = "default_max_positions")]
    pub max_position_embeddings: usize,
    #[serde(default = "default_pad")]
    pub pad_token_id: u32,
    #[serde(default = "default_eos")]
    pub eos_token_id: u32,
    #[serde(default = "default_decoder_start")]
    pub decoder_start_token_id: u32,
    #[serde(default = "default_true")]
    pub scale_embedding: bool,
    #[serde(default)]
    pub activation_function: Activation,
}

fn default_max_positions() -> usize {
    1024
}
fn default_pad() -> u32 {
    1
}
fn default_eos() -> u32 {
    2
}
fn default_decoder_start() -> u32 {
    2
}
fn default_true() -> bool {
    true
}

impl NllbConfig {
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let path = dir.join("config.json");
        let raw = std::fs::read_to_string(&path).map_err(|error| {
            Error::Config(format!("could not read {}: {error}", path.display()))
        })?;
        let config: Self = serde_json::from_str(&raw)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.d_model == 0 || self.vocab_size == 0 {
            return Err(Error::Config("invalid NLLB configuration".to_string()));
        }
        if self.d_model % self.encoder_attention_heads != 0 {
            return Err(Error::Config(format!(
                "d_model={} is not divisible by encoder_attention_heads={}",
                self.d_model, self.encoder_attention_heads
            )));
        }
        if self.d_model % self.decoder_attention_heads != 0 {
            return Err(Error::Config(format!(
                "d_model={} is not divisible by decoder_attention_heads={}",
                self.d_model, self.decoder_attention_heads
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Attention
// ---------------------------------------------------------------------------

struct Attention {
    q_proj: Projection,
    k_proj: Projection,
    v_proj: Projection,
    out_proj: Projection,
    num_heads: usize,
    head_dim: usize,
    d_model: usize,
    scaling: f64,
}

impl Attention {
    fn load(vb: VarBuilder, d_model: usize, num_heads: usize, quantize: bool) -> Result<Self> {
        let head_dim = d_model / num_heads;
        Ok(Self {
            q_proj: Projection::load(vb.pp("q_proj"), d_model, d_model, quantize)?,
            k_proj: Projection::load(vb.pp("k_proj"), d_model, d_model, quantize)?,
            v_proj: Projection::load(vb.pp("v_proj"), d_model, d_model, quantize)?,
            out_proj: Projection::load(vb.pp("out_proj"), d_model, d_model, quantize)?,
            num_heads,
            head_dim,
            d_model,
            scaling: 1.0 / (head_dim as f64).sqrt(),
        })
    }

    fn split_heads(&self, x: Tensor) -> Result<Tensor> {
        let (batch, seq, _) = x.dims3()?;
        Ok(x.reshape((batch, seq, self.num_heads, self.head_dim))?
            .transpose(1, 2)?)
    }

    fn project_q(&self, x: &Tensor) -> Result<Tensor> {
        let q = self.q_proj.forward(x)?;
        self.split_heads(q)
    }

    fn project_kv(&self, x: &Tensor) -> Result<(Tensor, Tensor)> {
        let k = self.k_proj.forward(x)?;
        let v = self.v_proj.forward(x)?;
        Ok((self.split_heads(k)?, self.split_heads(v)?))
    }

    fn project_qkv(&self, x: &Tensor) -> Result<(Tensor, Tensor, Tensor)> {
        let q = self.q_proj.forward(x)?;
        let k = self.k_proj.forward(x)?;
        let v = self.v_proj.forward(x)?;
        Ok((
            self.split_heads(q)?,
            self.split_heads(k)?,
            self.split_heads(v)?,
        ))
    }

    fn merge_heads(&self, x: Tensor) -> Result<Tensor> {
        let (batch, _heads, seq, _head_dim) = x.dims4()?;
        Ok(x.transpose(1, 2)?
            .reshape((batch, seq, self.d_model))?
            .contiguous()?)
    }

    fn attend(&self, q: &Tensor, k: &Tensor, v: &Tensor) -> Result<Tensor> {
        let q = (q * self.scaling)?;
        let scores = q.matmul(&k.transpose(2, 3)?)?;
        let probs = candle_nn::ops::softmax_last_dim(&scores)?;
        let out = probs.matmul(v)?;
        let out = self.merge_heads(out)?;
        self.out_proj.forward(&out)
    }

    fn attend_cross(&self, q: &Tensor, k: &Tensor, v: &Tensor) -> Result<Tensor> {
        let q = (q * self.scaling)?;
        // k/v have a batch of 1: broadcast over the beams.
        let scores = q.broadcast_matmul(&k.transpose(2, 3)?)?;
        let probs = candle_nn::ops::softmax_last_dim(&scores)?;
        let out = probs.broadcast_matmul(v)?;
        let out = self.merge_heads(out)?;
        self.out_proj.forward(&out)
    }
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

struct EncoderLayer {
    self_attn: Attention,
    self_attn_layer_norm: LayerNorm,
    fc1: Projection,
    fc2: Projection,
    final_layer_norm: LayerNorm,
    activation: Activation,
}

impl EncoderLayer {
    fn load(vb: VarBuilder, cfg: &NllbConfig, quantize: bool) -> Result<Self> {
        Ok(Self {
            self_attn: Attention::load(
                vb.pp("self_attn"),
                cfg.d_model,
                cfg.encoder_attention_heads,
                quantize,
            )?,
            self_attn_layer_norm: layer_norm(cfg.d_model, 1e-5, vb.pp("self_attn_layer_norm"))?,
            fc1: Projection::load(vb.pp("fc1"), cfg.d_model, cfg.encoder_ffn_dim, quantize)?,
            fc2: Projection::load(vb.pp("fc2"), cfg.encoder_ffn_dim, cfg.d_model, quantize)?,
            final_layer_norm: layer_norm(cfg.d_model, 1e-5, vb.pp("final_layer_norm"))?,
            activation: cfg.activation_function,
        })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let residual = x;
        let h = self.self_attn_layer_norm.forward(x)?;
        let (q, k, v) = self.self_attn.project_qkv(&h)?;
        let h = self.self_attn.attend(&q, &k, &v)?;
        let x = (residual + h)?;

        let residual = &x;
        let h = self.final_layer_norm.forward(&x)?;
        let h = self.fc1.forward(&h)?;
        let h = self.activation.apply(&h)?;
        let h = self.fc2.forward(&h)?;
        Ok((residual + h)?)
    }
}

struct Encoder {
    layers: Vec<EncoderLayer>,
    layer_norm: LayerNorm,
}

impl Encoder {
    fn load(
        vb: VarBuilder,
        cfg: &NllbConfig,
        loaded: &mut usize,
        progress: &mut dyn FnMut(usize),
        cancel: &AtomicBool,
        quantize: bool,
    ) -> Result<Self> {
        let mut layers = Vec::with_capacity(cfg.encoder_layers);
        for index in 0..cfg.encoder_layers {
            if cancel.load(AtomicOrdering::Relaxed) {
                return Err(Error::Cancelled);
            }
            layers.push(EncoderLayer::load(
                vb.pp(format!("layers.{index}")),
                cfg,
                quantize,
            )?);
            *loaded += 1;
            progress(*loaded);
        }
        Ok(Self {
            layers,
            layer_norm: layer_norm(cfg.d_model, 1e-5, vb.pp("layer_norm"))?,
        })
    }

    fn forward(&self, x: Tensor, cancel: &AtomicBool) -> Result<Tensor> {
        let mut x = x;
        for layer in &self.layers {
            // The encoder takes a meaningful share of the wall-clock time on
            // long inputs; checking between layers keeps cancellation
            // responsive instead of waiting for the whole source to encode.
            if cancel.load(AtomicOrdering::Relaxed) {
                return Err(Error::Cancelled);
            }
            x = layer.forward(&x)?;
        }
        Ok(self.layer_norm.forward(&x)?)
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

struct DecoderLayer {
    self_attn: Attention,
    self_attn_layer_norm: LayerNorm,
    encoder_attn: Attention,
    encoder_attn_layer_norm: LayerNorm,
    fc1: Projection,
    fc2: Projection,
    final_layer_norm: LayerNorm,
    activation: Activation,
}

impl DecoderLayer {
    fn load(vb: VarBuilder, cfg: &NllbConfig, quantize: bool) -> Result<Self> {
        Ok(Self {
            self_attn: Attention::load(
                vb.pp("self_attn"),
                cfg.d_model,
                cfg.decoder_attention_heads,
                quantize,
            )?,
            self_attn_layer_norm: layer_norm(cfg.d_model, 1e-5, vb.pp("self_attn_layer_norm"))?,
            encoder_attn: Attention::load(
                vb.pp("encoder_attn"),
                cfg.d_model,
                cfg.decoder_attention_heads,
                quantize,
            )?,
            encoder_attn_layer_norm: layer_norm(
                cfg.d_model,
                1e-5,
                vb.pp("encoder_attn_layer_norm"),
            )?,
            fc1: Projection::load(vb.pp("fc1"), cfg.d_model, cfg.decoder_ffn_dim, quantize)?,
            fc2: Projection::load(vb.pp("fc2"), cfg.decoder_ffn_dim, cfg.d_model, quantize)?,
            final_layer_norm: layer_norm(cfg.d_model, 1e-5, vb.pp("final_layer_norm"))?,
            activation: cfg.activation_function,
        })
    }

    fn forward(
        &self,
        x: &Tensor,
        index: usize,
        cache: &mut DecoderCache,
        timers: &mut GenerationTimers,
    ) -> Result<Tensor> {
        let residual = x;
        let started = Instant::now();
        let h = self.self_attn_layer_norm.forward(x)?;
        let (q, k_new, v_new) = self.self_attn.project_qkv(&h)?;
        timers.projections += started.elapsed();

        let started = Instant::now();
        let (k_all, v_all) = match &cache.self_kv[index] {
            Some((k, v)) => (Tensor::cat(&[k, &k_new], 2)?, Tensor::cat(&[v, &v_new], 2)?),
            None => (k_new, v_new),
        };
        cache.self_kv[index] = Some((k_all.clone(), v_all.clone()));
        timers.kv_cache += started.elapsed();

        let started = Instant::now();
        let h = self.self_attn.attend(&q, &k_all, &v_all)?;
        let x = (residual + h)?;
        timers.self_attn += started.elapsed();

        let residual = &x;
        let started = Instant::now();
        let h = self.encoder_attn_layer_norm.forward(&x)?;
        let q = self.encoder_attn.project_q(&h)?;
        timers.projections += started.elapsed();

        let started = Instant::now();
        let (k, v) = &cache.cross_kv[index];
        let h = self.encoder_attn.attend_cross(&q, k, v)?;
        let x = (residual + h)?;
        timers.cross_attn += started.elapsed();

        let residual = &x;
        let started = Instant::now();
        let h = self.final_layer_norm.forward(&x)?;
        let h = self.fc1.forward(&h)?;
        let h = self.activation.apply(&h)?;
        let h = self.fc2.forward(&h)?;
        let out = (residual + h)?;
        timers.ffn += started.elapsed();
        Ok(out)
    }

    /// CPU fast path used with quantized weights: fused attention over
    /// preallocated key/value buffers instead of generic candle matmuls.
    fn forward_cpu(
        &self,
        x: &Tensor,
        index: usize,
        cache: &mut CpuDecoderCache,
        timers: &mut GenerationTimers,
    ) -> Result<Tensor> {
        let residual = x;
        let started = Instant::now();
        let h = self.self_attn_layer_norm.forward(x)?;
        let (q, k_new, v_new) = self.self_attn.project_qkv(&h)?;
        timers.projections += started.elapsed();

        let started = Instant::now();
        let q = as_f32_vec(&q)?;
        let k_new = as_f32_vec(&k_new)?;
        let v_new = as_f32_vec(&v_new)?;
        cache.append(index, &k_new, &v_new);
        timers.kv_cache += started.elapsed();

        let started = Instant::now();
        let attended = cache.self_attention(index, &q, self.self_attn.scaling as f32);
        let h = Tensor::from_vec(
            attended,
            (cache.active_beams, 1, cache.d_model),
            &Device::Cpu,
        )?;
        let h = self.self_attn.out_proj.forward(&h)?;
        let x = (residual + h)?;
        timers.self_attn += started.elapsed();

        let residual = &x;
        let started = Instant::now();
        let h = self.encoder_attn_layer_norm.forward(&x)?;
        let q = self.encoder_attn.project_q(&h)?;
        timers.projections += started.elapsed();

        let started = Instant::now();
        let q = as_f32_vec(&q)?;
        let attended = cache.cross_attention(index, &q, self.encoder_attn.scaling as f32);
        let h = Tensor::from_vec(
            attended,
            (cache.active_beams, 1, cache.d_model),
            &Device::Cpu,
        )?;
        let h = self.encoder_attn.out_proj.forward(&h)?;
        let x = (residual + h)?;
        timers.cross_attn += started.elapsed();

        let residual = &x;
        let started = Instant::now();
        let h = self.final_layer_norm.forward(&x)?;
        let h = self.fc1.forward(&h)?;
        let h = self.activation.apply(&h)?;
        let h = self.fc2.forward(&h)?;
        let out = (residual + h)?;
        timers.ffn += started.elapsed();
        Ok(out)
    }
}

struct Decoder {
    layers: Vec<DecoderLayer>,
    layer_norm: LayerNorm,
}

impl Decoder {
    fn load(
        vb: VarBuilder,
        cfg: &NllbConfig,
        loaded: &mut usize,
        progress: &mut dyn FnMut(usize),
        cancel: &AtomicBool,
        quantize: bool,
    ) -> Result<Self> {
        let mut layers = Vec::with_capacity(cfg.decoder_layers);
        for index in 0..cfg.decoder_layers {
            if cancel.load(AtomicOrdering::Relaxed) {
                return Err(Error::Cancelled);
            }
            layers.push(DecoderLayer::load(
                vb.pp(format!("layers.{index}")),
                cfg,
                quantize,
            )?);
            *loaded += 1;
            progress(*loaded);
        }
        Ok(Self {
            layers,
            layer_norm: layer_norm(cfg.d_model, 1e-5, vb.pp("layer_norm"))?,
        })
    }

    fn prepare_cross(&self, encoder_hidden: &Tensor) -> Result<Vec<(Tensor, Tensor)>> {
        self.layers
            .iter()
            .map(|layer| layer.encoder_attn.project_kv(encoder_hidden))
            .collect()
    }

    fn forward(
        &self,
        mut x: Tensor,
        cache: &mut DecoderCache,
        timers: &mut GenerationTimers,
    ) -> Result<Tensor> {
        for (index, layer) in self.layers.iter().enumerate() {
            x = layer.forward(&x, index, cache, timers)?;
        }
        Ok(self.layer_norm.forward(&x)?)
    }

    fn forward_cpu(
        &self,
        mut x: Tensor,
        cache: &mut CpuDecoderCache,
        timers: &mut GenerationTimers,
    ) -> Result<Tensor> {
        for (index, layer) in self.layers.iter().enumerate() {
            x = layer.forward_cpu(&x, index, cache, timers)?;
        }
        Ok(self.layer_norm.forward(&x)?)
    }
}

struct DecoderCache {
    self_kv: Vec<Option<(Tensor, Tensor)>>,
    cross_kv: Vec<(Tensor, Tensor)>,
}

impl DecoderCache {
    fn new(cross_kv: Vec<(Tensor, Tensor)>) -> Self {
        let self_kv = (0..cross_kv.len()).map(|_| None).collect();
        Self { self_kv, cross_kv }
    }

    fn seq_len(&self) -> usize {
        self.self_kv
            .iter()
            .flatten()
            .next()
            .and_then(|(k, _)| k.dim(2).ok())
            .unwrap_or(0)
    }

    fn repeat(&self, n: usize) -> Result<Self> {
        let mut self_kv = Vec::with_capacity(self.self_kv.len());
        for entry in &self.self_kv {
            self_kv.push(match entry {
                Some((k, v)) => Some((k.repeat((n, 1, 1, 1))?, v.repeat((n, 1, 1, 1))?)),
                None => None,
            });
        }
        Ok(Self {
            self_kv,
            cross_kv: self.cross_kv.clone(),
        })
    }

    fn reorder(&self, parents: &Tensor) -> Result<Self> {
        let mut self_kv = Vec::with_capacity(self.self_kv.len());
        for entry in &self.self_kv {
            self_kv.push(match entry {
                Some((k, v)) => Some((k.index_select(parents, 0)?, v.index_select(parents, 0)?)),
                None => None,
            });
        }
        Ok(Self {
            self_kv,
            cross_kv: self.cross_kv.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// CPU decoder cache with fused attention
// ---------------------------------------------------------------------------
//
// Candle's generic batched matmul is optimized for large matrices. At decode
// time the attention operands are tiny (`[beams, heads, 1, head_dim]` against
// `[1, heads, seq, head_dim]`), and the per-call setup dominates the actual
// arithmetic. When the weights are quantized we therefore keep the self- and
// cross-attention keys/values in plain f32 buffers and run a fused
// score/softmax/value kernel. The buffers are preallocated for the whole token
// budget, so appending a position never copies the cache.

struct CpuLayerKv {
    k: Vec<f32>,
    v: Vec<f32>,
}

struct CpuDecoderCache {
    layers: Vec<CpuLayerKv>,
    /// Per layer `[heads][src_len][head_dim]`, shared by every beam.
    cross: Vec<(Vec<f32>, Vec<f32>)>,
    src_len: usize,
    len: usize,
    cap: usize,
    active_beams: usize,
    max_beams: usize,
    heads: usize,
    head_dim: usize,
    d_model: usize,
    scratch_k: Vec<f32>,
    scratch_v: Vec<f32>,
}

impl CpuDecoderCache {
    fn new(
        decoder: &Decoder,
        encoder_hidden: &Tensor,
        cfg: &NllbConfig,
        max_beams: usize,
        cap: usize,
    ) -> Result<Self> {
        let heads = cfg.decoder_attention_heads;
        let head_dim = cfg.d_model / heads;
        let mut cross = Vec::with_capacity(decoder.layers.len());
        let mut src_len = 0;
        for layer in &decoder.layers {
            let (k, v) = layer.encoder_attn.project_kv(encoder_hidden)?;
            src_len = k.dim(2)?;
            cross.push((as_f32_vec(&k)?, as_f32_vec(&v)?));
        }
        let per_beam = heads.saturating_mul(cap).saturating_mul(head_dim);
        let total = per_beam.saturating_mul(max_beams);
        let layers = (0..decoder.layers.len())
            .map(|_| CpuLayerKv {
                k: vec![0.0; total],
                v: vec![0.0; total],
            })
            .collect();
        Ok(Self {
            layers,
            cross,
            src_len,
            len: 0,
            cap,
            active_beams: 1,
            max_beams,
            heads,
            head_dim,
            d_model: cfg.d_model,
            scratch_k: vec![0.0; total],
            scratch_v: vec![0.0; total],
        })
    }

    fn advance(&mut self) {
        self.len += 1;
    }

    /// Stores one new key/value position for every layer.
    fn append(&mut self, index: usize, k_new: &[f32], v_new: &[f32]) {
        let (heads, head_dim, cap) = (self.heads, self.head_dim, self.cap);
        let pos = self.len;
        let layer = &mut self.layers[index];
        for beam in 0..self.active_beams {
            for head in 0..heads {
                let src = (beam * heads + head) * head_dim;
                let dst = ((beam * heads + head) * cap + pos) * head_dim;
                layer.k[dst..dst + head_dim].copy_from_slice(&k_new[src..src + head_dim]);
                layer.v[dst..dst + head_dim].copy_from_slice(&v_new[src..src + head_dim]);
            }
        }
    }

    fn repeat(&mut self, beams: usize) {
        if beams <= self.active_beams {
            return;
        }
        assert!(
            beams <= self.max_beams,
            "cache was sized for {} beams but {beams} were requested",
            self.max_beams
        );
        let (heads, head_dim, cap, len) = (self.heads, self.head_dim, self.cap, self.len);
        for layer in &mut self.layers {
            for beam in 1..beams {
                for head in 0..heads {
                    let src = head * cap * head_dim;
                    let dst = ((beam * heads + head) * cap) * head_dim;
                    layer.k.copy_within(src..src + len * head_dim, dst);
                    layer.v.copy_within(src..src + len * head_dim, dst);
                }
            }
        }
        self.active_beams = beams;
    }

    /// Reorders the cache so that destination beam `i` inherits `parents[i]`.
    fn reorder(&mut self, parents: &[usize]) {
        let (heads, head_dim, cap, len) = (self.heads, self.head_dim, self.cap, self.len);
        let bytes = len * head_dim;
        for layer in &mut self.layers {
            for (dest, &parent) in parents.iter().enumerate() {
                for head in 0..heads {
                    let src = ((parent * heads + head) * cap) * head_dim;
                    let dst = ((dest * heads + head) * cap) * head_dim;
                    self.scratch_k[dst..dst + bytes].copy_from_slice(&layer.k[src..src + bytes]);
                    self.scratch_v[dst..dst + bytes].copy_from_slice(&layer.v[src..src + bytes]);
                }
            }
            std::mem::swap(&mut layer.k, &mut self.scratch_k);
            std::mem::swap(&mut layer.v, &mut self.scratch_v);
        }
    }

    fn self_attention(&self, index: usize, q: &[f32], scale: f32) -> Vec<f32> {
        let layer = &self.layers[index];
        let seq = self.len + 1;
        fused_attention(
            q,
            &layer.k,
            &layer.v,
            self.active_beams,
            self.active_beams,
            self.heads,
            self.head_dim,
            self.cap,
            seq,
            scale,
        )
    }

    fn cross_attention(&self, index: usize, q: &[f32], scale: f32) -> Vec<f32> {
        let (k, v) = &self.cross[index];
        fused_attention(
            q,
            k,
            v,
            self.active_beams,
            1,
            self.heads,
            self.head_dim,
            self.src_len,
            self.src_len,
            scale,
        )
    }
}

fn as_f32_vec(tensor: &Tensor) -> Result<Vec<f32>> {
    Ok(tensor
        .to_dtype(DType::F32)?
        .to_device(&Device::Cpu)?
        .contiguous()?
        .flatten_all()?
        .to_vec1::<f32>()?)
}

/// `softmax(scale * q kᵀ) v` for a single query position.
///
/// `q` is `[beams][heads][head_dim]`, `k`/`v` are `[kv_beams][heads][stride][head_dim]`
/// (cross-attention uses a single, shared `kv_beams`). The result is laid out
/// `[beams][heads * head_dim]`, ready for the output projection.
#[allow(clippy::too_many_arguments)]
fn fused_attention(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    beams: usize,
    kv_beams: usize,
    heads: usize,
    head_dim: usize,
    stride: usize,
    seq: usize,
    scale: f32,
) -> Vec<f32> {
    let mut out = vec![0f32; beams * heads * head_dim];
    out.par_chunks_mut(head_dim)
        .enumerate()
        .map_init(Vec::<f32>::new, |scores, (task, dst)| {
            let beam = task / heads;
            let head = task % heads;
            let kv_beam = beam.min(kv_beams.saturating_sub(1));
            let q_row = &q[(beam * heads + head) * head_dim..][..head_dim];
            let base = ((kv_beam * heads + head) * stride) * head_dim;

            scores.clear();
            scores.resize(seq, 0.0);
            for (position, score) in scores.iter_mut().enumerate() {
                let k_row = &k[base + position * head_dim..][..head_dim];
                let mut dot = 0f32;
                for (a, b) in q_row.iter().zip(k_row.iter()) {
                    dot += a * b;
                }
                *score = dot * scale;
            }

            let mut max = f32::NEG_INFINITY;
            for &score in scores.iter() {
                max = max.max(score);
            }
            let mut sum = 0f32;
            for score in scores.iter_mut() {
                *score = (*score - max).exp();
                sum += *score;
            }
            let inverse = 1.0 / sum;
            for score in scores.iter_mut() {
                *score *= inverse;
            }

            dst.fill(0.0);
            for (position, &weight) in scores.iter().enumerate() {
                let v_row = &v[base + position * head_dim..][..head_dim];
                for (acc, value) in dst.iter_mut().zip(v_row.iter()) {
                    *acc += weight * value;
                }
            }
        })
        .for_each(|_| {});
    out
}

/// Either the generic candle cache or the fused CPU cache.
///
/// The variant is chosen once per generation call from the weight format, so
/// the search loop stays identical for both paths.
enum CacheState {
    Tensors(DecoderCache),
    Cpu(CpuDecoderCache),
}

impl CacheState {
    fn seq_len(&self) -> usize {
        match self {
            Self::Tensors(cache) => cache.seq_len(),
            Self::Cpu(cache) => cache.len,
        }
    }

    /// Replicates the single prefill beam across the requested beam count.
    fn repeat(&mut self, beams: usize) -> Result<()> {
        match self {
            Self::Tensors(cache) => {
                let repeated = cache.repeat(beams)?;
                *cache = repeated;
            }
            Self::Cpu(cache) => cache.repeat(beams),
        }
        Ok(())
    }

    /// Reorders the cache so that destination beam `i` inherits `parents[i]`.
    fn reorder(&mut self, parents: &Tensor, parents_usize: &[usize]) -> Result<()> {
        match self {
            Self::Tensors(cache) => {
                let reordered = cache.reorder(parents)?;
                *cache = reordered;
            }
            Self::Cpu(cache) => cache.reorder(parents_usize),
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

pub struct Nllb {
    config: NllbConfig,
    shared: Embedding,
    pos_table: Tensor,
    encoder: Encoder,
    decoder: Decoder,
    lm_head: Projection,
}

impl Nllb {
    pub fn load(model_dir: &Path, dtype: DType, device: &Device) -> Result<Self> {
        Self::load_with_progress(
            model_dir,
            dtype,
            device,
            &AtomicBool::new(false),
            &mut |_| {},
        )
    }

    pub fn load_with_progress(
        model_dir: &Path,
        dtype: DType,
        device: &Device,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize),
    ) -> Result<Self> {
        let config = NllbConfig::from_dir(model_dir)?;
        let checkpoint = Checkpoint::discover(model_dir)?;
        let vb = checkpoint.var_builder(dtype, device)?;

        // Embedding weights are shared between the encoder, decoder and lm_head.
        let shared = vb
            .get_unchecked("model.shared.weight")
            .or_else(|_| vb.get_unchecked("model.encoder.embed_tokens.weight"))
            .or_else(|_| vb.get_unchecked("lm_head.weight"))?;
        let expected = (config.vocab_size, config.d_model);
        if shared.dims() != [expected.0, expected.1] {
            return Err(Error::Config(format!(
                "unexpected embedding dimensions: {:?} (expected {:?})",
                shared.dims(),
                [expected.0, expected.1]
            )));
        }

        let quantize = quantization_enabled(device, dtype);
        let shared = Embedding::from_tensor(shared, quantize)?;
        let lm_head = shared.lm_head()?;

        let mut loaded = 1;
        progress(loaded);
        let encoder = Encoder::load(
            vb.pp("model").pp("encoder"),
            &config,
            &mut loaded,
            progress,
            cancel,
            quantize,
        )?;
        let decoder = Decoder::load(
            vb.pp("model").pp("decoder"),
            &config,
            &mut loaded,
            progress,
            cancel,
            quantize,
        )?;
        let pos_table = build_position_table(&config, dtype, device)?;

        Ok(Self {
            config,
            shared,
            pos_table,
            encoder,
            decoder,
            lm_head,
        })
    }

    /// `true` when the loaded weights are stored as Q8 blocks in memory.
    pub fn is_quantized(&self) -> bool {
        self.shared.is_quantized()
    }

    pub fn config(&self) -> &NllbConfig {
        &self.config
    }

    fn embed(&self, ids: &Tensor, position_start: usize) -> Result<Tensor> {
        let (batch, seq) = ids.dims2()?;
        // `embedding` expects 1-D ids and returns `[batch * seq, d_model]`.
        let x = self.shared.embedding(&ids.flatten_all()?)?.reshape((
            batch,
            seq,
            self.config.d_model,
        ))?;
        let scale = if self.config.scale_embedding {
            (self.config.d_model as f64).sqrt()
        } else {
            1.0
        };
        let x = (x * scale)?;
        // NLLB starts positions at padding_idx + 1, i.e. 2 without padding.
        let positions = self
            .pos_table
            .narrow(0, position_start + 2, seq)?
            .unsqueeze(0)?;
        Ok(x.broadcast_add(&positions)?)
    }

    pub fn encode(&self, input_ids: &Tensor) -> Result<Tensor> {
        self.encode_with_cancel(input_ids, &AtomicBool::new(false))
    }

    /// Encodes the source while observing the cancellation flag between layers.
    pub fn encode_with_cancel(&self, input_ids: &Tensor, cancel: &AtomicBool) -> Result<Tensor> {
        let x = self.embed(input_ids, 0)?;
        self.encoder.forward(x, cancel)
    }

    /// Runs one decoder step and returns the hidden state for the last token.
    ///
    /// Skipping the language-model head is useful for the mandatory start
    /// token, whose logits are discarded: the head reads the whole shared
    /// vocabulary matrix on every call.
    fn decode_hidden(
        &self,
        input_ids: &Tensor,
        cache: &mut CacheState,
        timers: &mut GenerationTimers,
    ) -> Result<Tensor> {
        let position_start = cache.seq_len();
        let started = Instant::now();
        let x = self.embed(input_ids, position_start)?;
        timers.embedding += started.elapsed();
        let x = match cache {
            CacheState::Tensors(cache) => self.decoder.forward(x, cache, timers)?,
            CacheState::Cpu(cache) => {
                let out = self.decoder.forward_cpu(x, cache, timers)?;
                // The fused path appends at `len`; advancing after the whole
                // stack makes the position visible to the next call.
                cache.advance();
                out
            }
        };
        Ok(x.narrow(1, x.dim(1)? - 1, 1)?)
    }

    fn decode_step(
        &self,
        input_ids: &Tensor,
        cache: &mut CacheState,
        timers: &mut GenerationTimers,
    ) -> Result<Tensor> {
        let started = Instant::now();
        let last = self.decode_hidden(input_ids, cache, timers)?;
        timers.decoder += started.elapsed();
        let started = Instant::now();
        let logits = self.lm_head.forward(&last)?;
        timers.lm_head += started.elapsed();
        Ok(logits.squeeze(1)?)
    }

    /// Translates a source token sequence into the target language.
    ///
    /// Beam search follows the Transformers configuration:
    /// `[eos, target_language]` prefix, length penalty 1.0.
    pub fn generate(
        &self,
        tokenizer: &NllbTokenizer,
        source_ids: &[u32],
        target_language: &str,
        options: GenerateOptions,
        progress: &mut dyn FnMut(GenerationProgress),
        cancel: &AtomicBool,
    ) -> Result<Vec<u32>> {
        let mut timers = GenerationTimers::default();
        self.generate_impl(
            tokenizer,
            source_ids,
            target_language,
            options,
            progress,
            cancel,
            &mut timers,
        )
    }

    /// Same as [`Nllb::generate`], but also returns a stage breakdown.
    ///
    /// The timings are wall-clock measurements of the actual inference calls,
    /// not estimates. They exist so the benchmark harness can attribute time to
    /// the encoder, the decoder stack, the language-model head and the beam
    /// search bookkeeping.
    pub fn generate_with_stats(
        &self,
        tokenizer: &NllbTokenizer,
        source_ids: &[u32],
        target_language: &str,
        options: GenerateOptions,
        progress: &mut dyn FnMut(GenerationProgress),
        cancel: &AtomicBool,
    ) -> Result<(Vec<u32>, GenerationStats)> {
        let mut timers = GenerationTimers::default();
        let tokens = self.generate_impl(
            tokenizer,
            source_ids,
            target_language,
            options,
            progress,
            cancel,
            &mut timers,
        )?;
        Ok((tokens, timers.finish()))
    }

    #[allow(clippy::too_many_arguments)]
    fn generate_impl(
        &self,
        tokenizer: &NllbTokenizer,
        source_ids: &[u32],
        target_language: &str,
        options: GenerateOptions,
        progress: &mut dyn FnMut(GenerationProgress),
        cancel: &AtomicBool,
        timers: &mut GenerationTimers,
    ) -> Result<Vec<u32>> {
        let total_started = Instant::now();
        let device = self.shared.device();
        let input_ids = Tensor::new(source_ids, device)?.unsqueeze(0)?;
        let started = Instant::now();
        let encoder_hidden = self.encode_with_cancel(&input_ids, cancel)?;
        timers.encode += started.elapsed();

        let beams = options.num_beams.max(1);
        let max_new = options.max_new_tokens.max(1);

        // The fused CPU cache is only used with quantized weights. The generic
        // path is byte-for-byte the previous implementation, so f32/CUDA/Metal
        // behaviour does not change.
        let started = Instant::now();
        let mut cache = if self.is_quantized() {
            CacheState::Cpu(CpuDecoderCache::new(
                &self.decoder,
                &encoder_hidden,
                &self.config,
                beams,
                max_new + 2,
            )?)
        } else {
            CacheState::Tensors(DecoderCache::new(
                self.decoder.prepare_cross(&encoder_hidden)?,
            ))
        };
        timers.cross_prepare += started.elapsed();
        let prefill_started = Instant::now();

        // Step 0: start token (eos), logits ignored. The head is skipped
        // entirely: its output would be discarded and it reads the full
        // 256k-entry shared vocabulary matrix.
        let eos = Tensor::new(&[tokenizer.eos_id()], device)?.unsqueeze(0)?;
        let _ = self.decode_hidden(&eos, &mut cache, timers)?;
        if cancel.load(AtomicOrdering::Relaxed) {
            return Err(Error::Cancelled);
        }

        // Step 1: the target language is forced as the first generated token.
        let target_id = tokenizer.lang_id(target_language)?;
        let target = Tensor::new(&[target_id], device)?.unsqueeze(0)?;
        let first_logits = self.decode_step(&target, &mut cache, timers)?;
        timers.prefill += prefill_started.elapsed();

        cache.repeat(beams)?;
        let mut scores = vec![0f64; beams];
        let mut sequences: Vec<Vec<u32>> = vec![Vec::new(); beams];
        let mut rows = logits_rows(&first_logits, beams)?;

        let eos_id = tokenizer.eos_id();
        let mut completed: Vec<(Vec<u32>, f64)> = Vec::new();
        let mut steps = 0usize;

        while steps < max_new {
            if cancel.load(AtomicOrdering::Relaxed) {
                return Err(Error::Cancelled);
            }

            // Search each beam separately before merging. A single global
            // top-k is dominated by the best beam, which can starve the other
            // beams of their EOS candidate and prevent beam search from ever
            // completing `num_beams` hypotheses.
            let search_started = Instant::now();
            let pooled = top_candidates_per_beam(&rows, &scores, beams * 2);
            let (active, finished) = collect_step(&pooled, eos_id, beams, &sequences);
            completed.extend(finished);
            trim_completed(&mut completed, beams);
            timers.search += search_started.elapsed();

            if active.is_empty() {
                break;
            }

            let parents: Vec<u32> = active.iter().map(|c| c.beam as u32).collect();
            let parents_usize: Vec<usize> = active.iter().map(|c| c.beam).collect();
            let parents_tensor = Tensor::new(parents.as_slice(), device)?;
            cache.reorder(&parents_tensor, &parents_usize)?;

            let tokens: Vec<u32> = active.iter().map(|c| c.token).collect();
            let input_ids = Tensor::new(tokens.as_slice(), device)?.unsqueeze(1)?;
            let logits = self.decode_step(&input_ids, &mut cache, timers)?;
            let rows_started = Instant::now();
            rows = logits_rows(&logits, 1)?;
            timers.search += rows_started.elapsed();

            let new_sequences: Vec<Vec<u32>> = active
                .iter()
                .map(|candidate| {
                    let mut sequence = sequences[candidate.beam].clone();
                    sequence.push(candidate.token);
                    sequence
                })
                .collect();
            sequences = new_sequences;
            scores = active.iter().map(|candidate| candidate.score).collect();
            steps += 1;

            let preview_index = scores
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.total_cmp(b))
                .map(|(index, _)| index)
                .unwrap_or(0);
            let preview = tokenizer
                .decode(&sequences[preview_index])
                .unwrap_or_default();
            progress(GenerationProgress {
                generated: steps,
                max: max_new,
                preview,
            });

            if steps >= max_new {
                for (sequence, score) in sequences.iter().zip(scores.iter()) {
                    let length = sequence.len().max(1) as f64;
                    completed.push((sequence.clone(), *score / length));
                }
                trim_completed(&mut completed, beams);
                break;
            }

            // Transformers-compatible stopping policy. The heuristic compares
            // the worst completed length-normalized score with the score the
            // best active hypothesis could reach if it terminated now. Future
            // tokens can only lower the accumulated log-probability, so this
            // is the standard optimistic bound; it is only applied once
            // `num_beams` hypotheses are complete, as in `BeamHypotheses`.
            if should_stop_early(
                &completed,
                &scores,
                steps + 1,
                beams,
                options.early_stopping,
            ) {
                break;
            }
        }

        if completed.is_empty() {
            return Err(Error::Other(
                "the model produced no translation tokens".to_string(),
            ));
        }

        timers.steps = steps;
        timers.beams = beams;
        timers.total = total_started.elapsed();
        Ok(best_completed(&completed).unwrap_or_default())
    }
}

/// Returns the highest-scoring completed hypothesis.
fn best_completed(completed: &[(Vec<u32>, f64)]) -> Option<Vec<u32>> {
    completed
        .iter()
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(sequence, _)| sequence.clone())
}

/// Keeps only the best `limit` completed hypotheses, as `BeamHypotheses` does.
///
/// Without this cap, every beam contributes a finished hypothesis on every
/// later step, the worst score collapses, and early stopping can never fire.
fn trim_completed(completed: &mut Vec<(Vec<u32>, f64)>, limit: usize) {
    while completed.len() > limit.max(1) {
        let worst = completed
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.1.total_cmp(&b.1))
            .map(|(index, _)| index);
        match worst {
            Some(index) => {
                completed.swap_remove(index);
            }
            None => break,
        }
    }
}

fn build_position_table(config: &NllbConfig, dtype: DType, device: &Device) -> Result<Tensor> {
    let dim = config.d_model;
    let half = dim / 2;
    let rows = config.max_position_embeddings + 2;
    let mut data = vec![0f32; rows * dim];
    let log_timescale = (10_000f64).ln() / ((half.saturating_sub(1)).max(1) as f64);

    for position in 0..rows {
        for index in 0..half {
            let frequency = (-log_timescale * index as f64).exp();
            let angle = position as f64 * frequency;
            data[position * dim + index] = angle.sin() as f32;
            data[position * dim + half + index] = angle.cos() as f32;
        }
        if dim % 2 == 1 {
            data[position * dim + dim - 1] = 0.0;
        }
    }
    // The padding row is zero in the NLLB implementation.
    if (config.pad_token_id as usize) < rows && dim > 0 {
        let start = config.pad_token_id as usize * dim;
        data[start..start + dim].fill(0.0);
    }

    Ok(Tensor::from_vec(data, (rows, dim), device)?.to_dtype(dtype)?)
}

fn logits_rows(logits: &Tensor, repeat: usize) -> Result<Vec<Vec<f32>>> {
    let logits = logits.to_dtype(DType::F32)?.to_device(&Device::Cpu)?;
    let log_probs = candle_nn::ops::log_softmax(&logits, 1)?;
    let rows = log_probs.to_vec2::<f32>()?;
    if repeat <= 1 || rows.len() >= repeat {
        Ok(rows)
    } else {
        let mut out = Vec::with_capacity(repeat);
        for _ in 0..repeat {
            out.push(rows[0].clone());
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    score: f64,
    beam: usize,
    token: u32,
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.score.total_cmp(&other.score) == Ordering::Equal
            && self.beam == other.beam
            && self.token == other.token
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| self.beam.cmp(&other.beam))
            .then_with(|| self.token.cmp(&other.token))
    }
}

#[cfg(test)]
fn top_candidates(rows: &[Vec<f32>], beam_scores: &[f64], k: usize) -> Vec<Candidate> {
    use std::cmp::Reverse;
    let mut heap: BinaryHeap<Reverse<Candidate>> = BinaryHeap::with_capacity(k + 1);
    for (beam, row) in rows.iter().enumerate() {
        let beam_score = beam_scores.get(beam).copied().unwrap_or(f64::NEG_INFINITY);
        for (token, log_prob) in row.iter().enumerate() {
            let candidate = Candidate {
                score: beam_score + *log_prob as f64,
                beam,
                token: token as u32,
            };
            if heap.len() < k {
                heap.push(Reverse(candidate));
            } else if let Some(Reverse(smallest)) = heap.peek() {
                if candidate.score > smallest.score {
                    heap.pop();
                    heap.push(Reverse(candidate));
                }
            }
        }
    }
    let mut candidates: Vec<Candidate> = heap.into_iter().map(|Reverse(c)| c).collect();
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    candidates
}

/// Returns the best `per_beam` candidates of every active beam, pooled and
/// sorted by cumulative score.
///
/// Searching globally instead lets the best beam occupy every slot, which can
/// hide another beam's end-of-sequence token.
fn top_candidates_per_beam(
    rows: &[Vec<f32>],
    beam_scores: &[f64],
    per_beam: usize,
) -> Vec<Candidate> {
    use std::cmp::Reverse;
    let per_beam = per_beam.max(1);
    let mut pooled = Vec::with_capacity(rows.len() * per_beam);
    for (beam, row) in rows.iter().enumerate() {
        let beam_score = beam_scores.get(beam).copied().unwrap_or(f64::NEG_INFINITY);
        let mut heap: BinaryHeap<Reverse<Candidate>> = BinaryHeap::with_capacity(per_beam + 1);
        for (token, log_prob) in row.iter().enumerate() {
            let candidate = Candidate {
                score: beam_score + *log_prob as f64,
                beam,
                token: token as u32,
            };
            if heap.len() < per_beam {
                heap.push(Reverse(candidate));
            } else if let Some(Reverse(smallest)) = heap.peek() {
                if candidate.score > smallest.score {
                    heap.pop();
                    heap.push(Reverse(candidate));
                }
            }
        }
        pooled.extend(heap.into_iter().map(|Reverse(candidate)| candidate));
    }
    pooled.sort_by(|a, b| b.score.total_cmp(&a.score));
    pooled
}

/// Splits one beam-search step into the hypotheses to keep active and the
/// hypotheses that emitted EOS.
fn collect_step(
    pooled: &[Candidate],
    eos_id: u32,
    beams: usize,
    sequences: &[Vec<u32>],
) -> (Vec<Candidate>, Vec<(Vec<u32>, f64)>) {
    let mut active = Vec::with_capacity(beams);
    let mut finished = Vec::new();
    let mut eos_recorded = vec![false; sequences.len()];
    for candidate in pooled {
        if candidate.token == eos_id {
            // Record at most one EOS per beam per step: the row contains one
            // EOS token, but duplicate rows must not create duplicate
            // hypotheses.
            if let Some(recorded) = eos_recorded.get_mut(candidate.beam) {
                if !*recorded {
                    *recorded = true;
                    let length = (sequences[candidate.beam].len() + 1).max(1);
                    finished.push((
                        sequences[candidate.beam].clone(),
                        candidate.score / length as f64,
                    ));
                }
            }
        } else if active.len() < beams {
            active.push(*candidate);
        }
    }
    (active, finished)
}

/// Decides whether beam search may stop before the generation budget.
///
/// `next_length` is the length a hypothesis would have if the next token were
/// EOS. Scores are length-normalized cumulative log-probabilities, so they are
/// negative; a higher value is better.
fn should_stop_early(
    completed: &[(Vec<u32>, f64)],
    active_scores: &[f64],
    next_length: usize,
    beams: usize,
    policy: EarlyStopping,
) -> bool {
    if policy == EarlyStopping::Never || completed.len() < beams {
        return false;
    }
    if policy == EarlyStopping::Always {
        return true;
    }
    let worst_completed = completed
        .iter()
        .map(|(_, score)| *score)
        .fold(f64::INFINITY, f64::min);
    let best_active = active_scores
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    if !best_active.is_finite() {
        return true;
    }
    let highest_attainable = best_active / next_length.max(1) as f64;
    worst_completed >= highest_attainable
}

/// Controls when beam search may stop before exhausting its token budget.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EarlyStopping {
    /// Stop as soon as `num_beams` hypotheses have finished.
    Always,
    /// Hugging Face's `early_stopping = false` heuristic: once `num_beams`
    /// hypotheses are complete, stop when the worst completed normalized score
    /// cannot be improved on by the best active hypothesis.
    #[default]
    Heuristic,
    /// Never stop early; generate until `max_new_tokens` (Hugging Face's
    /// `early_stopping = "never"`).
    Never,
}

/// Wall-clock breakdown of one generation call.
///
/// `decoder` covers the embedding lookup, the decoder stack and the KV-cache
/// updates; `lm_head` covers the shared-vocabulary projection only. `prefill`
/// contains the two mandatory prompt steps, including their share of
/// `decoder` and `lm_head`. `search` is beam bookkeeping and log-softmax.
#[derive(Debug, Clone, Copy, Default)]
pub struct GenerationStats {
    pub encode: Duration,
    /// Cross-attention key/value projection over the encoded source.
    pub cross_prepare: Duration,
    pub prefill: Duration,
    pub decoder: Duration,
    pub lm_head: Duration,
    pub search: Duration,
    pub total: Duration,
    pub steps: usize,
    pub beams: usize,
    /// Decoder sub-stages, accumulated inside `decoder`.
    pub embedding: Duration,
    pub projections: Duration,
    pub kv_cache: Duration,
    pub self_attn: Duration,
    pub cross_attn: Duration,
    pub ffn: Duration,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GenerationTimers {
    pub encode: Duration,
    pub cross_prepare: Duration,
    pub prefill: Duration,
    pub decoder: Duration,
    pub lm_head: Duration,
    pub search: Duration,
    pub total: Duration,
    pub steps: usize,
    pub beams: usize,
    pub embedding: Duration,
    pub projections: Duration,
    pub kv_cache: Duration,
    pub self_attn: Duration,
    pub cross_attn: Duration,
    pub ffn: Duration,
}

impl GenerationTimers {
    fn finish(self) -> GenerationStats {
        GenerationStats {
            encode: self.encode,
            cross_prepare: self.cross_prepare,
            prefill: self.prefill,
            decoder: self.decoder,
            lm_head: self.lm_head,
            search: self.search,
            total: self.total,
            steps: self.steps,
            beams: self.beams,
            embedding: self.embedding,
            projections: self.projections,
            kv_cache: self.kv_cache,
            self_attn: self.self_attn,
            cross_attn: self.cross_attn,
            ffn: self.ffn,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GenerateOptions {
    pub max_new_tokens: usize,
    pub num_beams: usize,
    pub early_stopping: EarlyStopping,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            max_new_tokens: 256,
            num_beams: 4,
            early_stopping: EarlyStopping::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GenerationProgress {
    pub generated: usize,
    pub max: usize,
    pub preview: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_candidates_are_sorted_by_cumulative_score() {
        let rows = vec![vec![0.0, -1.0], vec![-2.0, -0.5]];
        let beam_scores = vec![0.0, -0.1];
        let candidates = top_candidates(&rows, &beam_scores, 4);

        assert_eq!(candidates.len(), 4);
        let scores: Vec<f64> = candidates.iter().map(|candidate| candidate.score).collect();
        assert_eq!(scores, vec![0.0, -0.6, -1.0, -2.1]);
        assert_eq!(candidates[0].beam, 0);
        assert_eq!(candidates[0].token, 0);
        assert_eq!(candidates[1].beam, 1);
        assert_eq!(candidates[1].token, 1);
    }

    #[test]
    fn top_candidates_limits_the_result_size() {
        let rows = vec![vec![0.0, -1.0, -2.0]];
        let candidates = top_candidates(&rows, &[0.0], 2);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].token, 0);
        assert_eq!(candidates[1].token, 1);
    }

    #[test]
    fn default_activation_is_relu() {
        assert_eq!(Activation::default(), Activation::Relu);
    }

    fn candidate(score: f64, beam: usize, token: u32) -> Candidate {
        Candidate { score, beam, token }
    }

    #[test]
    fn per_beam_search_finds_eos_hidden_by_the_best_beam() {
        // Beam 0 dominates the global ranking; beam 1 can only finish if its
        // own EOS candidate is considered.
        let rows = vec![vec![-0.1, -0.2, -0.3, -10.0], vec![-5.0, -5.1, -5.2, -0.5]];
        let pooled = top_candidates_per_beam(&rows, &[0.0, 0.0], 2);

        let beam_one_eos = pooled
            .iter()
            .find(|candidate| candidate.beam == 1 && candidate.token == 3)
            .expect("beam 1 EOS should be pooled");
        assert!((beam_one_eos.score - -0.5).abs() < 1e-9);

        // The old global top-k would only contain beam 0's best tokens.
        let global = top_candidates(&rows, &[0.0, 0.0], 2);
        assert!(global.iter().all(|candidate| candidate.beam == 0));
    }

    #[test]
    fn collect_step_records_one_eos_per_beam_and_fills_active_slots() {
        let pooled = vec![
            candidate(-0.5, 0, 3),
            candidate(-0.7, 1, 3),
            candidate(-1.0, 0, 10),
            candidate(-1.2, 1, 11),
            candidate(-1.3, 0, 12),
        ];
        let sequences = vec![vec![7, 8, 9], vec![7, 8, 9]];
        let (active, finished) = collect_step(&pooled, 3, 2, &sequences);

        assert_eq!(finished.len(), 2);
        assert!((finished[0].1 - (-0.5 / 4.0)).abs() < 1e-9);
        assert!((finished[1].1 - (-0.7 / 4.0)).abs() < 1e-9);
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].token, 10);
        assert_eq!(active[1].token, 11);
    }

    #[test]
    fn collect_step_handles_multiple_children_from_one_parent() {
        let pooled = vec![
            candidate(-0.1, 0, 4),
            candidate(-0.2, 0, 5),
            candidate(-0.3, 1, 6),
        ];
        let sequences = vec![vec![2], vec![2]];
        let (active, finished) = collect_step(&pooled, 3, 2, &sequences);

        assert!(finished.is_empty());
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].beam, 0);
        assert_eq!(active[1].beam, 0);
    }

    #[test]
    fn heuristic_waits_for_enough_completed_hypotheses() {
        let completed = vec![(vec![1], -0.1)];
        assert!(!should_stop_early(
            &completed,
            &[-50.0],
            4,
            4,
            EarlyStopping::Heuristic
        ));
    }

    #[test]
    fn heuristic_stops_when_active_beams_cannot_overtake() {
        let completed = vec![
            (vec![1], -1.0),
            (vec![2], -1.1),
            (vec![3], -1.2),
            (vec![4], -1.3),
        ];
        // Best active normalized score would be -5.0, worse than -1.3.
        assert!(should_stop_early(
            &completed,
            &[-50.0],
            10,
            4,
            EarlyStopping::Heuristic
        ));
    }

    #[test]
    fn heuristic_continues_when_an_active_beam_can_still_overtake() {
        let completed = vec![
            (vec![1], -1.0),
            (vec![2], -1.1),
            (vec![3], -1.2),
            (vec![4], -1.3),
        ];
        // -2.0 / 10 = -0.2, better than the worst completed score.
        assert!(!should_stop_early(
            &completed,
            &[-2.0],
            10,
            4,
            EarlyStopping::Heuristic
        ));
    }

    #[test]
    fn never_policy_runs_to_the_budget() {
        let completed = vec![
            (vec![1], -1.0),
            (vec![2], -1.1),
            (vec![3], -1.2),
            (vec![4], -1.3),
        ];
        assert!(!should_stop_early(
            &completed,
            &[-50.0],
            10,
            4,
            EarlyStopping::Never
        ));
    }

    #[test]
    fn always_policy_stops_once_every_beam_completed() {
        let completed = vec![
            (vec![1], -1.0),
            (vec![2], -1.1),
            (vec![3], -1.2),
            (vec![4], -1.3),
        ];
        assert!(should_stop_early(
            &completed,
            &[0.0],
            10,
            4,
            EarlyStopping::Always
        ));
    }

    #[test]
    fn best_completed_selects_the_highest_normalized_score() {
        let completed = vec![
            (vec![10, 11], -1.4),
            (vec![20, 21, 22], -0.9),
            (vec![30], -1.1),
        ];
        assert_eq!(best_completed(&completed), Some(vec![20, 21, 22]));
        assert_eq!(best_completed(&[]), None);
    }

    #[test]
    fn completed_hypotheses_are_capped_to_the_beam_count() {
        let mut completed = vec![
            (vec![1], -2.0),
            (vec![2], -0.5),
            (vec![3], -1.0),
            (vec![4], -1.5),
        ];
        trim_completed(&mut completed, 2);
        assert_eq!(completed.len(), 2);
        assert!(completed.iter().any(|(sequence, _)| sequence == &vec![2]));
        assert!(completed.iter().any(|(sequence, _)| sequence == &vec![3]));
        assert!(!completed.iter().any(|(sequence, _)| sequence == &vec![1]));
    }

    #[test]
    fn early_eos_cannot_be_starved_into_a_512_step_stall() {
        // Regression for the reported FR->EN defect: beam 0 finishes early,
        // while the other beams only expose EOS outside a naive global top-k.
        let rows = vec![
            vec![-0.01, -0.02, -0.03, -0.005],
            vec![-8.0, -8.1, -8.2, -0.5],
            vec![-9.0, -9.1, -9.2, -0.6],
            vec![-10.0, -10.1, -10.2, -0.7],
        ];
        let mut pooled = top_candidates_per_beam(&rows, &[0.0, 0.0, 0.0, 0.0], 2);
        pooled.sort_by(|a, b| b.score.total_cmp(&a.score));

        let sequences = vec![vec![5], vec![5], vec![5], vec![5]];
        let (_, finished) = collect_step(&pooled, 3, 4, &sequences);
        assert_eq!(finished.len(), 4, "every beam's EOS must be recorded");
        assert!(should_stop_early(
            &finished,
            &[-100.0],
            2,
            4,
            EarlyStopping::Heuristic
        ));
    }

    // -- fused attention ---------------------------------------------------

    fn pseudo_random(count: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..count)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 8) as f32 / 8_388_608.0 - 1.0
            })
            .collect()
    }

    /// Reference `softmax(scale q kᵀ) v` built from candle ops.
    #[allow(clippy::too_many_arguments)]
    fn reference_attention(
        q: &[f32],
        k: &[f32],
        v: &[f32],
        beams: usize,
        kv_beams: usize,
        heads: usize,
        head_dim: usize,
        seq: usize,
        scale: f32,
    ) -> Vec<f32> {
        let device = Device::Cpu;
        let q = Tensor::from_vec(q.to_vec(), (beams, heads, 1, head_dim), &device).unwrap();
        let k = Tensor::from_vec(k.to_vec(), (kv_beams, heads, seq, head_dim), &device).unwrap();
        let v = Tensor::from_vec(v.to_vec(), (kv_beams, heads, seq, head_dim), &device).unwrap();
        let scores =
            (q.broadcast_matmul(&k.transpose(2, 3).unwrap()).unwrap() * scale as f64).unwrap();
        let probs = candle_nn::ops::softmax_last_dim(&scores).unwrap();
        probs
            .broadcast_matmul(&v)
            .unwrap()
            .flatten_all()
            .unwrap()
            .to_vec1::<f32>()
            .unwrap()
    }

    #[test]
    fn fused_attention_matches_candle_for_broadcast_cross_attention() {
        let (beams, kv_beams, heads, head_dim, seq, scale) = (2, 1, 3, 8, 5, 0.353_553_4);
        let q = pseudo_random(beams * heads * head_dim, 1);
        let k = pseudo_random(kv_beams * heads * seq * head_dim, 2);
        let v = pseudo_random(kv_beams * heads * seq * head_dim, 3);

        let actual = fused_attention(
            &q, &k, &v, beams, kv_beams, heads, head_dim, seq, seq, scale,
        );
        let expected =
            reference_attention(&q, &k, &v, beams, kv_beams, heads, head_dim, seq, scale);

        assert_eq!(actual.len(), expected.len());
        for (index, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
            assert!(
                (a - e).abs() < 1e-4,
                "index {index}: fused {a} vs candle {e}"
            );
        }
    }

    #[test]
    fn fused_attention_matches_candle_for_per_beam_self_attention() {
        let (beams, heads, head_dim, seq, scale) = (2, 2, 4, 3, 0.5);
        let q = pseudo_random(beams * heads * head_dim, 11);
        let k = pseudo_random(beams * heads * seq * head_dim, 12);
        let v = pseudo_random(beams * heads * seq * head_dim, 13);

        let actual = fused_attention(&q, &k, &v, beams, beams, heads, head_dim, seq, seq, scale);
        let expected = reference_attention(&q, &k, &v, beams, beams, heads, head_dim, seq, scale);

        for (index, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
            assert!(
                (a - e).abs() < 1e-4,
                "index {index}: fused {a} vs candle {e}"
            );
        }
    }

    #[test]
    fn fused_attention_ignores_cache_slots_past_the_sequence_length() {
        // The cache is preallocated for the whole token budget, so positions
        // beyond the current length contain stale data and must never be read.
        let (heads, head_dim, seq, stride, scale) = (2, 4, 3, 6, 0.25);
        let q = pseudo_random(heads * head_dim, 21);
        let mut k = vec![999.0; heads * stride * head_dim];
        let mut v = vec![999.0; heads * stride * head_dim];
        let compact_k = pseudo_random(heads * seq * head_dim, 22);
        let compact_v = pseudo_random(heads * seq * head_dim, 23);
        for head in 0..heads {
            let src = head * seq * head_dim;
            let dst = head * stride * head_dim;
            k[dst..dst + seq * head_dim].copy_from_slice(&compact_k[src..src + seq * head_dim]);
            v[dst..dst + seq * head_dim].copy_from_slice(&compact_v[src..src + seq * head_dim]);
        }

        let actual = fused_attention(&q, &k, &v, 1, 1, heads, head_dim, stride, seq, scale);
        let expected = reference_attention(
            &q, &compact_k, &compact_v, 1, 1, heads, head_dim, seq, scale,
        );

        for (index, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
            assert!(
                (a - e).abs() < 1e-4,
                "index {index}: fused {a} vs candle {e}"
            );
        }
    }

    // -- CPU cache bookkeeping --------------------------------------------

    fn test_cache(max_beams: usize, cap: usize, heads: usize, head_dim: usize) -> CpuDecoderCache {
        let total = heads * cap * head_dim * max_beams;
        CpuDecoderCache {
            layers: vec![CpuLayerKv {
                k: vec![0.0; total],
                v: vec![0.0; total],
            }],
            cross: Vec::new(),
            src_len: 0,
            len: 0,
            cap,
            active_beams: 1,
            max_beams,
            heads,
            head_dim,
            d_model: heads * head_dim,
            scratch_k: vec![0.0; total],
            scratch_v: vec![0.0; total],
        }
    }

    fn position_slice(cache: &CpuDecoderCache, beam: usize, head: usize, pos: usize) -> Vec<f32> {
        let start = ((beam * cache.heads + head) * cache.cap + pos) * cache.head_dim;
        cache.layers[0].k[start..start + cache.head_dim].to_vec()
    }

    #[test]
    fn cpu_cache_repeat_copies_the_prefill_beam() {
        let mut cache = test_cache(2, 4, 2, 2);
        // One beam, two heads of width two, two positions.
        cache.append(0, &[1.0, 2.0, 3.0, 4.0], &[0.0; 4]);
        cache.advance();
        cache.append(0, &[5.0, 6.0, 7.0, 8.0], &[0.0; 4]);
        cache.advance();

        cache.repeat(2);
        assert_eq!(cache.active_beams, 2);
        for head in 0..2 {
            assert_eq!(
                position_slice(&cache, 0, head, 0),
                position_slice(&cache, 1, head, 0)
            );
            assert_eq!(
                position_slice(&cache, 0, head, 1),
                position_slice(&cache, 1, head, 1)
            );
        }
    }

    #[test]
    fn cpu_cache_reorder_moves_positions_between_beams() {
        let mut cache = test_cache(2, 4, 2, 2);
        cache.append(0, &[1.0, 2.0, 3.0, 4.0], &[0.0; 4]);
        cache.advance();
        cache.repeat(2);
        // Two beams now: beam 0 and 1 both hold the prefill position. Append a
        // distinct position per beam.
        cache.append(
            0,
            &[10.0, 11.0, 12.0, 13.0, 20.0, 21.0, 22.0, 23.0],
            &[0.0; 8],
        );
        cache.advance();

        cache.reorder(&[1, 0]);
        for head in 0..2 {
            assert_eq!(
                position_slice(&cache, 0, head, 1),
                vec![20.0 + head as f32 * 2.0, 21.0 + head as f32 * 2.0]
            );
            assert_eq!(
                position_slice(&cache, 1, head, 1),
                vec![10.0 + head as f32 * 2.0, 11.0 + head as f32 * 2.0]
            );
            // The prefill position is identical in both beams, so reordering
            // must leave it untouched.
            assert_eq!(
                position_slice(&cache, 0, head, 0),
                position_slice(&cache, 1, head, 0)
            );
        }
    }
}
