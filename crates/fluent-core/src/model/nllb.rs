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
use crate::{default_generation_budget, Error, OutputLimit, Result};

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

    fn forward(&self, x: &Tensor, timers: &mut GenerationTimers) -> Result<Tensor> {
        let residual = x;
        let started = Instant::now();
        let h = self.self_attn_layer_norm.forward(x)?;
        let (q, k, v) = self.self_attn.project_qkv(&h)?;
        let h = self.self_attn.attend(&q, &k, &v)?;
        let x = (residual + h)?;
        timers.encode_attn += started.elapsed();

        let residual = &x;
        let started = Instant::now();
        let h = self.final_layer_norm.forward(&x)?;
        let h = self.fc1.forward(&h)?;
        let h = self.activation.apply(&h)?;
        let h = self.fc2.forward(&h)?;
        timers.encode_ffn += started.elapsed();
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

    fn forward(
        &self,
        x: Tensor,
        cancel: &AtomicBool,
        timers: &mut GenerationTimers,
    ) -> Result<Tensor> {
        let mut x = x;
        for layer in &self.layers {
            // The encoder takes a meaningful share of the wall-clock time on
            // long inputs; checking between layers keeps cancellation
            // responsive instead of waiting for the whole source to encode.
            if cancel.load(AtomicOrdering::Relaxed) {
                return Err(Error::Cancelled);
            }
            x = layer.forward(&x, timers)?;
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

fn try_zeroed(elements: usize, tokens: usize) -> Result<Vec<f32>> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(elements)
        .map_err(|_| Error::CacheAllocation(tokens))?;
    values.resize(elements, 0.0);
    Ok(values)
}

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
    /// Hard positional capacity. `UntilEos` starts smaller and grows
    /// geometrically up to this bound.
    max_cap: usize,
    active_beams: usize,
    max_beams: usize,
    heads: usize,
    head_dim: usize,
    d_model: usize,
    /// Physical slot holding each logical beam's rows.
    ///
    /// Beam search frequently asks several destination beams to inherit the
    /// same parent. Instead of copying every destination beam, the cache keeps
    /// the parent rows where they are and only materializes a copy for the
    /// duplicate parents. Attention and `append` go through this indirection,
    /// so the layout stays invisible to the rest of the decoder.
    phys: Vec<usize>,
}

impl CpuDecoderCache {
    fn new(
        decoder: &Decoder,
        encoder_hidden: &Tensor,
        cfg: &NllbConfig,
        max_beams: usize,
        cap: usize,
        max_cap: usize,
    ) -> Result<Self> {
        let cap = cap.max(1).min(max_cap.max(1));
        let max_cap = max_cap.max(cap);
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
            max_cap,
            active_beams: 1,
            max_beams,
            heads,
            head_dim,
            d_model: cfg.d_model,
            phys: (0..max_beams).collect(),
        })
    }

    fn advance(&mut self) {
        self.len += 1;
    }

    /// Grows the preallocated cache when the next decode step needs a new
    /// position. Growth is geometric, so long generations do not reallocate on
    /// every token. Allocation failures return an error instead of aborting.
    fn ensure_next_position(&mut self) -> Result<()> {
        let required = self.len.saturating_add(1);
        if required <= self.cap {
            return Ok(());
        }
        if required > self.max_cap {
            return Err(Error::PositionLimit(self.max_cap));
        }
        let mut new_cap = self.cap.max(1);
        while new_cap < required {
            new_cap = new_cap.saturating_mul(2).min(self.max_cap);
        }
        self.grow(new_cap)
    }

    fn grow(&mut self, new_cap: usize) -> Result<()> {
        if new_cap <= self.cap {
            return Ok(());
        }
        let old_cap = self.cap;
        let used = self.len;
        let total = self
            .max_beams
            .saturating_mul(self.heads)
            .saturating_mul(new_cap)
            .saturating_mul(self.head_dim);
        let mut grown = Vec::with_capacity(self.layers.len());
        for layer in &self.layers {
            let mut k = try_zeroed(total, new_cap)?;
            let mut v = try_zeroed(total, new_cap)?;
            for slot in 0..self.max_beams {
                for head in 0..self.heads {
                    let src = ((slot * self.heads + head) * old_cap) * self.head_dim;
                    let dst = ((slot * self.heads + head) * new_cap) * self.head_dim;
                    let bytes = used.saturating_mul(self.head_dim);
                    k[dst..dst + bytes].copy_from_slice(&layer.k[src..src + bytes]);
                    v[dst..dst + bytes].copy_from_slice(&layer.v[src..src + bytes]);
                }
            }
            grown.push(CpuLayerKv { k, v });
        }
        self.layers = grown;
        self.cap = new_cap;
        Ok(())
    }

    /// Stores one new key/value position for every layer.
    fn append(&mut self, index: usize, k_new: &[f32], v_new: &[f32]) {
        let (heads, head_dim, cap) = (self.heads, self.head_dim, self.cap);
        let pos = self.len;
        debug_assert!(pos < cap, "cache capacity was not grown before append");
        let layer = &mut self.layers[index];
        for beam in 0..self.active_beams {
            let slot = self.phys[beam];
            for head in 0..heads {
                let src = (beam * heads + head) * head_dim;
                let dst = ((slot * heads + head) * cap + pos) * head_dim;
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
        self.active_beams = beams;
        let source = self.phys[0];
        for slot in self.phys[..beams].iter_mut() {
            *slot = source;
        }
        self.ensure_unique_slots();
    }

    /// Reorders the cache so that destination beam `i` inherits `parents[i]`.
    ///
    /// Rows are addressed through `phys`, so a distinct parent costs nothing:
    /// the destination simply points at the same physical slot. Only beams
    /// that share a parent need a real copy, which is what beam search usually
    /// produces (two or three children of the best beam).
    fn reorder(&mut self, parents: &[usize]) {
        let beams = self.active_beams;
        debug_assert_eq!(parents.len(), beams);
        let previous = &self.phys[..beams];
        let mut next = vec![0usize; beams];
        for (dest, &parent) in parents.iter().enumerate() {
            next[dest] = previous[parent.min(beams - 1)];
        }
        self.phys[..beams].copy_from_slice(&next);
        self.ensure_unique_slots();
    }

    /// Gives every logical beam a distinct physical slot, copying rows only
    /// for beams that currently alias an earlier one.
    fn ensure_unique_slots(&mut self) {
        let beams = self.active_beams;
        let mut used = vec![false; self.max_beams];
        let mut remap = vec![usize::MAX; beams];
        for (beam, &slot) in self.phys[..beams].iter().enumerate() {
            if slot < self.max_beams && !used[slot] {
                used[slot] = true;
                remap[beam] = slot;
            }
        }
        let mut free: Vec<usize> = (0..self.max_beams).filter(|slot| !used[*slot]).collect();
        for (beam, slot) in remap.iter_mut().enumerate() {
            if *slot != usize::MAX {
                continue;
            }
            let source = self.phys[beam];
            let Some(&destination) = free.first() else {
                debug_assert!(false, "cache has no free physical slot");
                return;
            };
            free.remove(0);
            used[destination] = true;
            self.copy_slot(source, destination);
            *slot = destination;
        }
        self.phys[..beams].copy_from_slice(&remap);
    }

    /// Copies `len` positions of every head from one physical slot to another.
    fn copy_slot(&mut self, source: usize, destination: usize) {
        let (heads, head_dim, cap, len) = (self.heads, self.head_dim, self.cap, self.len);
        let bytes = len * head_dim;
        for layer in &mut self.layers {
            for head in 0..heads {
                let src = ((source * heads + head) * cap) * head_dim;
                let dst = ((destination * heads + head) * cap) * head_dim;
                // `copy_within` handles the case where two heads of the same
                // slot overlap; destinations are always unused slots.
                layer.k.copy_within(src..src + bytes, dst);
                layer.v.copy_within(src..src + bytes, dst);
            }
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
            &self.phys[..self.active_beams],
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
            &[],
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
/// `q` is `[beams][heads][head_dim]`, `k`/`v` are `[kv_beam][heads][stride][head_dim]`.
/// `slots[beam]` selects the key/value rows, which lets the cache keep beam
/// rows in place across reorders; an empty slice means every beam shares row 0
/// (cross-attention). The result is laid out `[beams][heads * head_dim]`,
/// ready for the output projection.
#[allow(clippy::too_many_arguments)]
fn fused_attention(
    q: &[f32],
    k: &[f32],
    v: &[f32],
    beams: usize,
    slots: &[usize],
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
            let kv_beam = slots.get(beam).copied().unwrap_or(0);
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
        let mut timers = GenerationTimers::default();
        self.encode_impl(input_ids, cancel, &mut timers)
    }

    fn encode_impl(
        &self,
        input_ids: &Tensor,
        cancel: &AtomicBool,
        timers: &mut GenerationTimers,
    ) -> Result<Tensor> {
        let x = self.embed(input_ids, 0)?;
        self.encoder.forward(x, cancel, timers)
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
                cache.ensure_next_position()?;
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
        let encoder_hidden = self.encode_impl(&input_ids, cancel, timers)?;
        timers.encode += started.elapsed();

        let beams = options.num_beams.max(1);
        let window = GenerationWindow::resolve(
            options.output_limit,
            source_ids.len(),
            self.config.max_position_embeddings,
        );
        let max_new = window.max_steps;
        let cache_capacity = max_new.saturating_add(2);

        // The fused CPU cache is only used with quantized weights. The generic
        // path is byte-for-byte the previous implementation, so f32/CUDA/Metal
        // behaviour does not change.
        let started = Instant::now();
        let mut cache = if self.is_quantized() {
            // Unlimited output starts smaller and grows on demand instead of
            // reserving the full positional capacity up front.
            let initial_capacity = if window.unlimited {
                cache_capacity.min(128)
            } else {
                cache_capacity
            };
            CacheState::Cpu(CpuDecoderCache::new(
                &self.decoder,
                &encoder_hidden,
                &self.config,
                beams,
                initial_capacity,
                cache_capacity,
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

        let repeat_started = Instant::now();
        cache.repeat(beams)?;
        timers.reorder += repeat_started.elapsed();
        let mut scores = vec![0f64; beams];
        let mut sequences: Vec<Vec<u32>> = vec![Vec::new(); beams];
        let first_rows_started = Instant::now();
        let mut rows = prepare_logits(&first_logits, beams, timers)?;
        timers.search_logits += first_rows_started.elapsed();

        let eos_id = tokenizer.stop_eos_id();
        let mut completed: Vec<(Vec<u32>, f64)> = Vec::new();
        let mut steps = 0usize;

        while window.allows(steps) {
            if cancel.load(AtomicOrdering::Relaxed) {
                return Err(Error::Cancelled);
            }

            // Search each beam separately before merging. A single global
            // top-k is dominated by the best beam, which can starve the other
            // beams of their EOS candidate and prevent beam search from ever
            // completing `num_beams` hypotheses.
            let search_started = Instant::now();
            let topk_started = Instant::now();
            let pooled = top_candidates_per_beam(&rows, &scores, beams * 2);
            timers.search_topk += topk_started.elapsed();
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
            let reorder_started = Instant::now();
            cache.reorder(&parents_tensor, &parents_usize)?;
            timers.reorder += reorder_started.elapsed();

            let tokens: Vec<u32> = active.iter().map(|c| c.token).collect();
            let input_ids = Tensor::new(tokens.as_slice(), device)?.unsqueeze(1)?;
            let logits = self.decode_step(&input_ids, &mut cache, timers)?;
            let rows_started = Instant::now();
            rows = prepare_logits(&logits, 1, timers)?;
            timers.search_logits += rows_started.elapsed();
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
                max: if window.unlimited { 0 } else { max_new },
                preview,
            });

            if steps >= max_new {
                if !window.unlimited {
                    // A finite ceiling is an expected completion condition.
                    for (sequence, score) in sequences.iter().zip(scores.iter()) {
                        let length = sequence.len().max(1) as f64;
                        completed.push((sequence.clone(), *score / length));
                    }
                    trim_completed(&mut completed, beams);
                }
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
            if window.unlimited {
                return Err(Error::PositionLimit(max_new));
            }
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

/// Per-row logits plus the constants needed to reproduce candle's
/// `log_softmax` value on demand.
///
/// Beam search only needs the top candidates of each row, not a fully
/// normalized vocabulary, so the normalized values are never materialized.
/// `log_prob` performs exactly the operations candle's reference path performs:
/// subtract the row maximum, exponentiate, sum sequentially in row order, take
/// the natural log, then subtract. The reduction order matches candle's CPU
/// `log_softmax` (which uses a scalar sum when the crate is built without a
/// global `avx2` target feature), so scores are bit-identical.
struct LogitsRows {
    data: Vec<f32>,
    row_len: usize,
    max: Vec<f32>,
    log_norm: Vec<f32>,
}

impl LogitsRows {
    fn rows(&self) -> usize {
        self.max.len()
    }

    fn row(&self, index: usize) -> &[f32] {
        &self.data[index * self.row_len..(index + 1) * self.row_len]
    }

    /// log-softmax value for one entry, identical to candle's result.
    #[cfg(test)]
    fn log_prob(&self, row: usize, token: usize) -> f32 {
        (self.data[row * self.row_len + token] - self.max[row]) - self.log_norm[row]
    }

    /// Test helper: wrap already-normalized rows so the candidate code sees
    /// `(v - 0) - 0 == v`, which makes the reference path easy to assert on.
    #[cfg(test)]
    fn from_log_probs(rows: &[Vec<f32>]) -> Self {
        let row_len = rows.first().map(Vec::len).unwrap_or(0);
        let data = rows.concat();
        let count = rows.len();
        Self {
            data,
            row_len,
            max: vec![0.0; count],
            log_norm: vec![0.0; count],
        }
    }
}

/// Copies the logits and computes the stable log-sum-exp constants per row.
///
/// `repeat` mirrors the beam-search prefill: the first target step produces a
/// single row that is shared by every beam.
fn prepare_logits(
    logits: &Tensor,
    repeat: usize,
    timers: &mut GenerationTimers,
) -> Result<LogitsRows> {
    let logits = logits.to_dtype(DType::F32)?.to_device(&Device::Cpu)?;
    let (rows, row_len) = logits.dims2()?;
    let started = Instant::now();
    let data = logits.flatten_all()?.to_vec1::<f32>()?;
    timers.search_extract += started.elapsed();

    let (data, out_rows) = if repeat > rows {
        let mut repeated = Vec::with_capacity(repeat * row_len);
        for _ in 0..repeat {
            repeated.extend_from_slice(&data[..row_len]);
        }
        (repeated, repeat)
    } else {
        (data, rows)
    };

    let mut max = Vec::with_capacity(out_rows);
    let mut log_norm = Vec::with_capacity(out_rows);
    for row in 0..out_rows {
        let values = &data[row * row_len..(row + 1) * row_len];
        // Start from the first element (rather than -inf) so an all-NaN row
        // behaves exactly like candle's maximum reduction.
        let mut row_max = values[0];
        for &value in &values[1..] {
            row_max = row_max.max(value);
        }
        let mut sum = 0f32;
        for &value in values {
            sum += (value - row_max).exp();
        }
        max.push(row_max);
        log_norm.push(sum.ln());
    }

    Ok(LogitsRows {
        data,
        row_len,
        max,
        log_norm,
    })
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
fn top_candidates(rows: &LogitsRows, beam_scores: &[f64], k: usize) -> Vec<Candidate> {
    use std::cmp::Reverse;
    let mut heap: BinaryHeap<Reverse<Candidate>> = BinaryHeap::with_capacity(k + 1);
    for beam in 0..rows.rows() {
        let beam_score = beam_scores.get(beam).copied().unwrap_or(f64::NEG_INFINITY);
        for token in 0..rows.row_len {
            let candidate = Candidate {
                score: beam_score + rows.log_prob(beam, token) as f64,
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
    rows: &LogitsRows,
    beam_scores: &[f64],
    per_beam: usize,
) -> Vec<Candidate> {
    use std::cmp::Reverse;
    let per_beam = per_beam.max(1);
    let mut pooled = Vec::with_capacity(rows.rows() * per_beam);
    for beam in 0..rows.rows() {
        let beam_score = beam_scores.get(beam).copied().unwrap_or(f64::NEG_INFINITY);
        let mut heap: BinaryHeap<Reverse<Candidate>> = BinaryHeap::with_capacity(per_beam + 1);
        // Hoist the row so the scan has no per-token indexing arithmetic; the
        // value is exactly the inlined `log_prob` result.
        let row = rows.row(beam);
        let row_max = rows.max[beam];
        let log_norm = rows.log_norm[beam];
        for (token, &value) in row.iter().enumerate() {
            let log_prob = (value - row_max) - log_norm;
            let candidate = Candidate {
                score: beam_score + log_prob as f64,
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
    /// Never stop early; generate until the output limit (Hugging Face's
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
    /// Encoder sub-stages, accumulated inside `encode`.
    pub encode_attn: Duration,
    pub encode_ffn: Duration,
    /// Cross-attention key/value projection over the encoded source.
    pub cross_prepare: Duration,
    pub prefill: Duration,
    pub decoder: Duration,
    pub lm_head: Duration,
    pub search: Duration,
    /// Beam-search sub-stages, accumulated inside `search` plus `reorder`.
    pub search_logits: Duration,
    /// Portion of `search_logits` spent copying the row into a Rust vector.
    pub search_extract: Duration,
    pub search_topk: Duration,
    pub reorder: Duration,
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
    pub encode_attn: Duration,
    pub encode_ffn: Duration,
    pub cross_prepare: Duration,
    pub prefill: Duration,
    pub decoder: Duration,
    pub lm_head: Duration,
    pub search: Duration,
    /// Beam-search sub-stages, accumulated inside `search` plus `reorder`.
    pub search_logits: Duration,
    /// Portion of `search_logits` spent copying the row into a Rust vector.
    pub search_extract: Duration,
    pub search_topk: Duration,
    pub reorder: Duration,
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
            encode_attn: self.encode_attn,
            encode_ffn: self.encode_ffn,
            cross_prepare: self.cross_prepare,
            prefill: self.prefill,
            decoder: self.decoder,
            lm_head: self.lm_head,
            search: self.search,
            search_logits: self.search_logits,
            search_extract: self.search_extract,
            search_topk: self.search_topk,
            reorder: self.reorder,
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

/// Resolved search window for one generation call.
#[derive(Debug, Clone, Copy)]
struct GenerationWindow {
    max_steps: usize,
    unlimited: bool,
}

impl GenerationWindow {
    fn resolve(limit: OutputLimit, source_tokens: usize, max_positions: usize) -> Self {
        // Two decoder positions are consumed by the mandatory start and target
        // language tokens, so the model's positional table can host at most
        // `max_positions - 2` generated tokens.
        let model_budget = max_positions.saturating_sub(2).max(1);
        match limit {
            OutputLimit::Auto => Self {
                max_steps: default_generation_budget(source_tokens).min(model_budget),
                unlimited: false,
            },
            OutputLimit::Tokens(tokens) => Self {
                max_steps: tokens.clamp(1, model_budget),
                unlimited: false,
            },
            OutputLimit::UntilEos => Self {
                max_steps: model_budget,
                unlimited: true,
            },
        }
    }

    fn allows(self, steps: usize) -> bool {
        steps < self.max_steps
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GenerateOptions {
    pub output_limit: OutputLimit,
    pub num_beams: usize,
    pub early_stopping: EarlyStopping,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            output_limit: OutputLimit::Tokens(256),
            num_beams: 4,
            early_stopping: EarlyStopping::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GenerationProgress {
    pub generated: usize,
    /// Finite ceiling, or `0` when the caller selected no Fluent limit.
    pub max: usize,
    pub preview: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_candidates_are_sorted_by_cumulative_score() {
        let rows = LogitsRows::from_log_probs(&[vec![0.0, -1.0], vec![-2.0, -0.5]]);
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
        let rows = LogitsRows::from_log_probs(&[vec![0.0, -1.0, -2.0]]);
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
        let rows = LogitsRows::from_log_probs(&[
            vec![-0.1, -0.2, -0.3, -10.0],
            vec![-5.0, -5.1, -5.2, -0.5],
        ]);
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
        let rows = LogitsRows::from_log_probs(&[
            vec![-0.01, -0.02, -0.03, -0.005],
            vec![-8.0, -8.1, -8.2, -0.5],
            vec![-9.0, -9.1, -9.2, -0.6],
            vec![-10.0, -10.1, -10.2, -0.7],
        ]);
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

    // -- logits path -------------------------------------------------------

    fn reference_log_probs(data: &[f32], rows: usize, vocab: usize) -> Vec<f32> {
        let tensor = Tensor::from_vec(data.to_vec(), (rows, vocab), &Device::Cpu).unwrap();
        candle_nn::ops::log_softmax(&tensor, 1)
            .unwrap()
            .flatten_all()
            .unwrap()
            .to_vec1::<f32>()
            .unwrap()
    }

    #[test]
    fn manual_log_softmax_matches_candle_bit_for_bit() {
        let (rows, vocab) = (4usize, 4096usize);
        let mut state = 0x0bad_f00du32;
        let data: Vec<f32> = (0..rows * vocab)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                ((state >> 8) as f32 / 8_388_608.0 - 1.0) * 37.0
            })
            .collect();
        let reference = reference_log_probs(&data, rows, vocab);
        let tensor = Tensor::from_vec(data, (rows, vocab), &Device::Cpu).unwrap();
        let mut timers = GenerationTimers::default();
        let prepared = prepare_logits(&tensor, 1, &mut timers).unwrap();

        for row in 0..rows {
            for token in 0..vocab {
                assert_log_prob_matches(
                    prepared.log_prob(row, token),
                    reference[row * vocab + token],
                    &format!("row {row} token {token}"),
                );
            }
        }
    }

    #[test]
    fn manual_log_softmax_matches_candle_for_adversarial_rows() {
        let vocab = 512usize;
        // Very large and very small magnitudes.
        let mut special = vec![0.0f32; vocab];
        special[0] = f32::INFINITY;
        special[1] = f32::NEG_INFINITY;
        special[2] = f32::NAN;
        special[3] = f32::MAX;
        special[4] = f32::MIN;
        let rows: Vec<Vec<f32>> = vec![
            (0..vocab).map(|i| i as f32 * 1e35).collect(),
            (0..vocab).map(|i| -(i as f32) * 1e35).collect(),
            // Repeated and nearly equal values.
            vec![1.0; vocab],
            (0..vocab)
                .map(|i| 1.0 + (i % 2) as f32 * f32::EPSILON)
                .collect(),
            // Infinities and NaN, matching f32 semantics instead of panicking.
            special,
            vec![f32::NAN; vocab],
        ];

        let flat: Vec<f32> = rows.concat();
        let reference = reference_log_probs(&flat, rows.len(), vocab);
        let tensor = Tensor::from_vec(flat, (rows.len(), vocab), &Device::Cpu).unwrap();
        let mut timers = GenerationTimers::default();
        let prepared = prepare_logits(&tensor, 1, &mut timers).unwrap();

        let mut compared = 0usize;
        for (row, values) in rows.iter().enumerate() {
            let expected_row = &reference[row * vocab..(row + 1) * vocab];
            for (token, _) in values.iter().enumerate() {
                // Both paths may legitimately produce NaN; require identical
                // bits so the behaviour is pinned, including NaN payloads.
                assert_log_prob_matches(
                    prepared.log_prob(row, token),
                    expected_row[token],
                    &format!("row {row} token {token}"),
                );
                compared += 1;
            }
        }
        assert_eq!(compared, rows.len() * vocab);
    }

    #[test]
    fn prepare_logits_repeats_the_single_prefill_row() {
        let vocab = 300usize;
        let data: Vec<f32> = (0..vocab).map(|index| index as f32 * 0.01).collect();
        let tensor = Tensor::from_vec(data.clone(), (1, vocab), &Device::Cpu).unwrap();
        let mut timers = GenerationTimers::default();
        let prepared = prepare_logits(&tensor, 4, &mut timers).unwrap();
        assert_eq!(prepared.rows(), 4);
        let reference = reference_log_probs(&data, 1, vocab);
        for row in 0..4 {
            for (token, &expected) in reference.iter().enumerate() {
                assert_log_prob_matches(
                    prepared.log_prob(row, token),
                    expected,
                    &format!("row {row} token {token}"),
                );
            }
        }
    }

    #[test]
    fn optimized_top_candidates_match_the_candle_reference_path() {
        let (rows, vocab, per_beam) = (4usize, 2048usize, 8usize);
        let mut state = 0xfeed_faceu32;
        let data: Vec<f32> = (0..rows * vocab)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                ((state >> 8) as f32 / 8_388_608.0 - 1.0) * 12.0
            })
            .collect();
        let tensor = Tensor::from_vec(data, (rows, vocab), &Device::Cpu).unwrap();
        let mut timers = GenerationTimers::default();
        let prepared = prepare_logits(&tensor, 1, &mut timers).unwrap();

        let log_probs = reference_log_probs(
            &tensor.flatten_all().unwrap().to_vec1::<f32>().unwrap(),
            rows,
            vocab,
        );
        let reference_rows: Vec<Vec<f32>> = log_probs.chunks(vocab).map(<[f32]>::to_vec).collect();
        let reference = LogitsRows::from_log_probs(&reference_rows);
        let beam_scores = [0.0f64, -0.3, -1.7, -2.5];
        let optimized = top_candidates_per_beam(&prepared, &beam_scores, per_beam);
        let expected = top_candidates_per_beam(&reference, &beam_scores, per_beam);

        assert_eq!(optimized.len(), expected.len());
        for (a, e) in optimized.iter().zip(expected.iter()) {
            assert_eq!(a.token, e.token);
            assert_eq!(a.beam, e.beam);
            if cfg!(target_arch = "aarch64") {
                assert!((a.score - e.score).abs() <= 2e-5f64.max(e.score.abs() * 2e-5));
            } else {
                assert_eq!(a.score.to_bits(), e.score.to_bits());
            }
        }
    }

    // -- fused attention ---------------------------------------------------

    /// Candle uses a sequential CPU sum on default x86-64 builds, so the
    /// manual log-softmax is bit-identical there. On aarch64 Candle uses NEON
    /// reductions; the same math differs by a few ULPs and is compared with a
    /// tight tolerance instead. Non-finite values must still match exactly.
    fn assert_log_prob_matches(actual: f32, expected: f32, context: &str) {
        if actual.is_nan() || expected.is_nan() || actual.is_infinite() || expected.is_infinite() {
            assert_eq!(actual.to_bits(), expected.to_bits(), "{context}");
        } else if cfg!(target_arch = "aarch64") {
            let tolerance = 2e-5f32.max(expected.abs() * 2e-5);
            assert!(
                (actual - expected).abs() <= tolerance,
                "{context}: actual {actual} expected {expected}"
            );
        } else {
            assert_eq!(actual.to_bits(), expected.to_bits(), "{context}");
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

        // A single shared kv beam is the cross-attention case: every query
        // beam reads slot 0.
        let slots = vec![0usize; beams];
        let actual = fused_attention(&q, &k, &v, beams, &slots, heads, head_dim, seq, seq, scale);
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

        let slots: Vec<usize> = (0..beams).collect();
        let actual = fused_attention(&q, &k, &v, beams, &slots, heads, head_dim, seq, seq, scale);
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

        let actual = fused_attention(&q, &k, &v, 1, &[0], heads, head_dim, stride, seq, scale);
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
        test_cache_with_max(max_beams, cap, cap, heads, head_dim)
    }

    fn test_cache_with_max(
        max_beams: usize,
        cap: usize,
        max_cap: usize,
        heads: usize,
        head_dim: usize,
    ) -> CpuDecoderCache {
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
            max_cap,
            active_beams: 1,
            max_beams,
            heads,
            head_dim,
            d_model: heads * head_dim,
            phys: (0..max_beams).collect(),
        }
    }

    fn position_slice(cache: &CpuDecoderCache, beam: usize, head: usize, pos: usize) -> Vec<f32> {
        let slot = cache.phys[beam];
        let start = ((slot * cache.heads + head) * cache.cap + pos) * cache.head_dim;
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

    /// Reference beam bookkeeping: one compact `[heads][len][head_dim]` buffer
    /// per logical beam, copied on every reorder. Slow but obviously correct.
    struct ReferenceCache {
        beams: Vec<(Vec<f32>, Vec<f32>)>,
        heads: usize,
        head_dim: usize,
    }

    impl ReferenceCache {
        fn new(heads: usize, head_dim: usize) -> Self {
            Self {
                beams: vec![(Vec::new(), Vec::new())],
                heads,
                head_dim,
            }
        }

        fn append(&mut self, k: &[f32], v: &[f32]) {
            let width = self.heads * self.head_dim;
            for (beam, rows) in self.beams.iter_mut().enumerate() {
                rows.0
                    .extend_from_slice(&k[beam * width..(beam + 1) * width]);
                rows.1
                    .extend_from_slice(&v[beam * width..(beam + 1) * width]);
            }
        }

        fn repeat(&mut self, beams: usize) {
            while self.beams.len() < beams {
                self.beams.push(self.beams[0].clone());
            }
        }

        fn reorder(&mut self, parents: &[usize]) {
            self.beams = parents
                .iter()
                .map(|&parent| self.beams[parent].clone())
                .collect();
        }

        fn rows(&self, beam: usize) -> (Vec<f32>, Vec<f32>) {
            self.beams[beam].clone()
        }
    }

    /// Rows in the compact `[position][head][head_dim]` layout used by the
    /// reference cache.
    fn cache_rows(cache: &CpuDecoderCache, beam: usize) -> (Vec<f32>, Vec<f32>) {
        let slot = cache.phys[beam];
        let mut k = Vec::new();
        let mut v = Vec::new();
        for position in 0..cache.len {
            for head in 0..cache.heads {
                let start = ((slot * cache.heads + head) * cache.cap + position) * cache.head_dim;
                k.extend_from_slice(&cache.layers[0].k[start..start + cache.head_dim]);
                v.extend_from_slice(&cache.layers[0].v[start..start + cache.head_dim]);
            }
        }
        (k, v)
    }

    #[test]
    fn cpu_cache_physical_slots_match_reference_bookkeeping() {
        let (heads, head_dim, cap, beams) = (2, 2, 8, 4);
        let mut cache = test_cache(beams, cap, heads, head_dim);
        let mut reference = ReferenceCache::new(heads, head_dim);
        let mut state = 0x1234_5678u32;
        let mut numbers = |count: usize| {
            (0..count)
                .map(|_| {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    (state >> 8) as f32 / 8_388_608.0 - 1.0
                })
                .collect::<Vec<_>>()
        };

        // Prefill: one beam, three positions.
        for _ in 0..3 {
            let (k, v) = (numbers(heads * head_dim), numbers(heads * head_dim));
            cache.append(0, &k, &v);
            reference.append(&k, &v);
            cache.advance();
        }
        cache.repeat(beams);
        reference.repeat(beams);
        assert_eq!(cache.phys[..beams], [0, 1, 2, 3]);

        // Two steps with duplicate parents, checking the rows after each
        // reorder and after the following append.
        for parents in [vec![2, 0, 2, 2], vec![1, 1, 3, 0], vec![0, 0, 0, 0]] {
            cache.reorder(&parents);
            reference.reorder(&parents);
            for beam in 0..beams {
                assert_eq!(
                    cache_rows(&cache, beam),
                    reference.rows(beam),
                    "beam {beam}"
                );
            }
            let mut unique = cache.phys[..beams].to_vec();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), beams, "physical slots must be distinct");

            let k: Vec<f32> = numbers(beams * heads * head_dim);
            let v: Vec<f32> = numbers(beams * heads * head_dim);
            cache.append(0, &k, &v);
            reference.append(&k, &v);
            cache.advance();
            for beam in 0..beams {
                assert_eq!(
                    cache_rows(&cache, beam),
                    reference.rows(beam),
                    "beam {beam}"
                );
            }
        }
    }

    #[test]
    fn cpu_cache_attention_uses_the_physical_slot() {
        // After reordering [1, 0], logical beam 0 must attend to the rows that
        // beam 1 had accumulated, not to its own old rows.
        let (heads, head_dim, cap) = (2, 2, 6);
        let mut cache = test_cache(2, cap, heads, head_dim);
        cache.append(0, &[1.0, 0.0, 0.0, 1.0], &[1.0; 4]);
        cache.advance();
        cache.repeat(2);

        // Distinct second token per logical beam: beam 0 writes (0,2)/(2,0),
        // beam 1 writes (5,6)/(7,8).
        cache.append(0, &[0.0, 2.0, 2.0, 0.0, 5.0, 6.0, 7.0, 8.0], &[2.0; 8]);
        cache.reorder(&[1, 0]);

        let q: Vec<f32> = vec![1.0; 2 * heads * head_dim];
        let actual = cache.self_attention(0, &q, 1.0);

        // Logical beam 0 now owns the rows that beam 1 had accumulated:
        // position 0 (shared prefill) followed by beam 1's second token.
        // Logical beam 1 owns beam 0's old rows: prefill plus (0,2)/(2,0).
        let slot_one_k: Vec<f32> = vec![1.0, 0.0, 5.0, 6.0, 0.0, 1.0, 7.0, 8.0];
        let slot_zero_k: Vec<f32> = vec![1.0, 0.0, 0.0, 2.0, 0.0, 1.0, 2.0, 0.0];
        let v: Vec<f32> = vec![1.0, 1.0, 2.0, 2.0, 1.0, 1.0, 2.0, 2.0];
        let beam_rows = heads * head_dim;
        let beam_zero = fused_attention(
            &q[..beam_rows],
            &slot_one_k,
            &v,
            1,
            &[0],
            heads,
            head_dim,
            2,
            2,
            1.0,
        );
        let beam_one = fused_attention(
            &q[beam_rows..],
            &slot_zero_k,
            &v,
            1,
            &[0],
            heads,
            head_dim,
            2,
            2,
            1.0,
        );
        let expected: Vec<f32> = beam_zero.into_iter().chain(beam_one).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn generation_window_allows_more_than_512_tokens_only_when_unlimited() {
        let auto = GenerationWindow::resolve(OutputLimit::Auto, 100, 1024);
        assert!(!auto.unlimited);
        assert_eq!(auto.max_steps, 216);

        let finite = GenerationWindow::resolve(OutputLimit::Tokens(512), 100, 1024);
        assert!(!finite.unlimited);
        assert_eq!(finite.max_steps, 512);

        let longer = GenerationWindow::resolve(OutputLimit::Tokens(600), 100, 1024);
        assert!(!longer.unlimited);
        assert_eq!(longer.max_steps, 600);

        let unlimited = GenerationWindow::resolve(OutputLimit::UntilEos, 100, 1024);
        assert!(unlimited.unlimited);
        assert_eq!(unlimited.max_steps, 1022);

        // Simulate the production loop condition: 600 tokens are allowed
        // before an EOS arrives, which is beyond the old 512 ceiling.
        let mut generated = 0usize;
        while unlimited.allows(generated) && generated < 600 {
            generated += 1;
        }
        assert_eq!(generated, 600);
    }

    #[test]
    fn finite_generation_is_clamped_to_model_capacity() {
        let clamped = GenerationWindow::resolve(OutputLimit::Tokens(5_000), 100, 1024);
        assert!(!clamped.unlimited);
        assert_eq!(clamped.max_steps, 1022);

        let minimum = GenerationWindow::resolve(OutputLimit::Tokens(0), 100, 1024);
        assert_eq!(minimum.max_steps, 1);
    }

    #[test]
    fn cpu_cache_grows_beyond_512_positions_and_preserves_data() {
        let (heads, head_dim) = (2usize, 2usize);
        let mut cache = test_cache_with_max(1, 4, 1024, heads, head_dim);
        for position in 0..600usize {
            cache.ensure_next_position().unwrap();
            let base = position as f32 * 10.0;
            let k: Vec<f32> = (0..heads * head_dim).map(|i| base + i as f32).collect();
            cache.append(0, &k, &vec![0.0; heads * head_dim]);
            cache.advance();
        }
        assert_eq!(cache.len, 600);
        assert!(cache.cap > 512, "cache stayed at {}", cache.cap);

        for position in [0usize, 1, 127, 128, 255, 256, 511, 512, 599] {
            let base = position as f32 * 10.0;
            assert_eq!(
                position_slice(&cache, 0, 0, position),
                vec![base, base + 1.0],
                "position {position} was not preserved"
            );
            assert_eq!(
                position_slice(&cache, 0, 1, position),
                vec![base + 2.0, base + 3.0],
                "head 1 position {position} was not preserved"
            );
        }
    }

    #[test]
    fn cpu_cache_capacity_overflow_is_an_error() {
        let (heads, head_dim) = (1usize, 1usize);
        let mut cache = test_cache_with_max(1, 2, 4, heads, head_dim);
        for _ in 0..4 {
            cache.ensure_next_position().unwrap();
            cache.append(0, &[1.0], &[1.0]);
            cache.advance();
        }
        let error = cache
            .ensure_next_position()
            .expect_err("capacity must be enforced");
        assert!(matches!(error, Error::PositionLimit(4)));
    }

    /// Load a real model with `FLUENT_TEST_MODEL` to prove the production
    /// search loop can generate beyond 512 tokens, and that unlimited mode
    /// stays cancellable. Disabling EOS is test-only; normal generation still
    /// terminates on the tokenizer's real EOS id.
    #[test]
    fn real_model_generates_beyond_512_and_unlimited_cancels() {
        let Some(dir) = std::env::var_os("FLUENT_TEST_MODEL").map(std::path::PathBuf::from) else {
            eprintln!("skipping: set FLUENT_TEST_MODEL to run the long-generation test");
            return;
        };
        if !dir.is_dir() {
            eprintln!("skipping: FLUENT_TEST_MODEL is not a directory");
            return;
        }
        let mut tokenizer = NllbTokenizer::from_dir(&dir).expect("tokenizer");
        let (source_ids, _) = tokenizer
            .encode_source(
                "The library opens at eight. Visitors can borrow up to ten books for three weeks.",
                "eng_Latn",
                512,
            )
            .expect("encode source");
        tokenizer.disable_stop_eos_for_test();
        let model = Nllb::load(&dir, DType::F32, &Device::Cpu).expect("load model");
        let cancel = AtomicBool::new(false);

        let finite = model
            .generate(
                &tokenizer,
                &source_ids,
                "fra_Latn",
                GenerateOptions {
                    output_limit: OutputLimit::Tokens(600),
                    num_beams: 1,
                    early_stopping: EarlyStopping::default(),
                },
                &mut |_| {},
                &cancel,
            )
            .expect("finite long generation");
        assert!(
            finite.len() > 512,
            "expected more than 512 tokens, got {}",
            finite.len()
        );
        println!("long generation produced {} tokens", finite.len());

        let cancel = AtomicBool::new(false);
        let mut seen = 0usize;
        let error = model
            .generate(
                &tokenizer,
                &source_ids,
                "fra_Latn",
                GenerateOptions {
                    output_limit: OutputLimit::UntilEos,
                    num_beams: 1,
                    early_stopping: EarlyStopping::default(),
                },
                &mut |generation| {
                    seen = seen.max(generation.generated);
                    if generation.generated > 0 {
                        assert_eq!(
                            generation.max, 0,
                            "unlimited progress must not report a fake ceiling"
                        );
                    }
                    if generation.generated >= 520 {
                        cancel.store(true, AtomicOrdering::Relaxed);
                    }
                },
                &cancel,
            )
            .expect_err("unlimited generation must stop when cancelled");
        assert!(matches!(error, Error::Cancelled));
        assert!(
            seen > 512,
            "cancellation arrived before 512 tokens ({seen})"
        );
        println!("unlimited generation cancelled after {seen} tokens");
    }
}
