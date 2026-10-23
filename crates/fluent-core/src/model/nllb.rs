use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

use candle_core::{DType, Device, Tensor};
use candle_nn::{layer_norm, linear, LayerNorm, Linear, Module, VarBuilder};
use serde::Deserialize;

use crate::tokenizer::NllbTokenizer;
use crate::{Error, Result};

use super::checkpoint::Checkpoint;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    Relu,
    Gelu,
    Silu,
}

impl Default for Activation {
    fn default() -> Self {
        Self::Relu
    }
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
    q_proj: Linear,
    k_proj: Linear,
    v_proj: Linear,
    out_proj: Linear,
    num_heads: usize,
    head_dim: usize,
    d_model: usize,
    scaling: f64,
}

impl Attention {
    fn load(vb: VarBuilder, d_model: usize, num_heads: usize) -> Result<Self> {
        let head_dim = d_model / num_heads;
        Ok(Self {
            q_proj: linear(d_model, d_model, vb.pp("q_proj"))?,
            k_proj: linear(d_model, d_model, vb.pp("k_proj"))?,
            v_proj: linear(d_model, d_model, vb.pp("v_proj"))?,
            out_proj: linear(d_model, d_model, vb.pp("out_proj"))?,
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
        Ok(self.out_proj.forward(&out)?)
    }

    fn attend_cross(&self, q: &Tensor, k: &Tensor, v: &Tensor) -> Result<Tensor> {
        let q = (q * self.scaling)?;
        // k/v have a batch of 1: broadcast over the beams.
        let scores = q.broadcast_matmul(&k.transpose(2, 3)?)?;
        let probs = candle_nn::ops::softmax_last_dim(&scores)?;
        let out = probs.broadcast_matmul(v)?;
        let out = self.merge_heads(out)?;
        Ok(self.out_proj.forward(&out)?)
    }
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

struct EncoderLayer {
    self_attn: Attention,
    self_attn_layer_norm: LayerNorm,
    fc1: Linear,
    fc2: Linear,
    final_layer_norm: LayerNorm,
    activation: Activation,
}

impl EncoderLayer {
    fn load(vb: VarBuilder, cfg: &NllbConfig) -> Result<Self> {
        Ok(Self {
            self_attn: Attention::load(
                vb.pp("self_attn"),
                cfg.d_model,
                cfg.encoder_attention_heads,
            )?,
            self_attn_layer_norm: layer_norm(cfg.d_model, 1e-5, vb.pp("self_attn_layer_norm"))?,
            fc1: linear(cfg.d_model, cfg.encoder_ffn_dim, vb.pp("fc1"))?,
            fc2: linear(cfg.encoder_ffn_dim, cfg.d_model, vb.pp("fc2"))?,
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
    ) -> Result<Self> {
        let mut layers = Vec::with_capacity(cfg.encoder_layers);
        for index in 0..cfg.encoder_layers {
            if cancel.load(AtomicOrdering::Relaxed) {
                return Err(Error::Cancelled);
            }
            layers.push(EncoderLayer::load(vb.pp(format!("layers.{index}")), cfg)?);
            *loaded += 1;
            progress(*loaded);
        }
        Ok(Self {
            layers,
            layer_norm: layer_norm(cfg.d_model, 1e-5, vb.pp("layer_norm"))?,
        })
    }

    fn forward(&self, x: Tensor) -> Result<Tensor> {
        let mut x = x;
        for layer in &self.layers {
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
    fc1: Linear,
    fc2: Linear,
    final_layer_norm: LayerNorm,
    activation: Activation,
}

impl DecoderLayer {
    fn load(vb: VarBuilder, cfg: &NllbConfig) -> Result<Self> {
        Ok(Self {
            self_attn: Attention::load(
                vb.pp("self_attn"),
                cfg.d_model,
                cfg.decoder_attention_heads,
            )?,
            self_attn_layer_norm: layer_norm(cfg.d_model, 1e-5, vb.pp("self_attn_layer_norm"))?,
            encoder_attn: Attention::load(
                vb.pp("encoder_attn"),
                cfg.d_model,
                cfg.decoder_attention_heads,
            )?,
            encoder_attn_layer_norm: layer_norm(
                cfg.d_model,
                1e-5,
                vb.pp("encoder_attn_layer_norm"),
            )?,
            fc1: linear(cfg.d_model, cfg.decoder_ffn_dim, vb.pp("fc1"))?,
            fc2: linear(cfg.decoder_ffn_dim, cfg.d_model, vb.pp("fc2"))?,
            final_layer_norm: layer_norm(cfg.d_model, 1e-5, vb.pp("final_layer_norm"))?,
            activation: cfg.activation_function,
        })
    }

    fn forward(&self, x: &Tensor, index: usize, cache: &mut DecoderCache) -> Result<Tensor> {
        let residual = x;
        let h = self.self_attn_layer_norm.forward(x)?;
        let (q, k_new, v_new) = self.self_attn.project_qkv(&h)?;

        let (k_all, v_all) = match &cache.self_kv[index] {
            Some((k, v)) => (Tensor::cat(&[k, &k_new], 2)?, Tensor::cat(&[v, &v_new], 2)?),
            None => (k_new, v_new),
        };
        cache.self_kv[index] = Some((k_all.clone(), v_all.clone()));

        let h = self.self_attn.attend(&q, &k_all, &v_all)?;
        let x = (residual + h)?;

        let residual = &x;
        let h = self.encoder_attn_layer_norm.forward(&x)?;
        let q = self.encoder_attn.project_q(&h)?;
        let (k, v) = &cache.cross_kv[index];
        let h = self.encoder_attn.attend_cross(&q, k, v)?;
        let x = (residual + h)?;

        let residual = &x;
        let h = self.final_layer_norm.forward(&x)?;
        let h = self.fc1.forward(&h)?;
        let h = self.activation.apply(&h)?;
        let h = self.fc2.forward(&h)?;
        Ok((residual + h)?)
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
    ) -> Result<Self> {
        let mut layers = Vec::with_capacity(cfg.decoder_layers);
        for index in 0..cfg.decoder_layers {
            if cancel.load(AtomicOrdering::Relaxed) {
                return Err(Error::Cancelled);
            }
            layers.push(DecoderLayer::load(vb.pp(format!("layers.{index}")), cfg)?);
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

    fn forward(&self, mut x: Tensor, cache: &mut DecoderCache) -> Result<Tensor> {
        for (index, layer) in self.layers.iter().enumerate() {
            x = layer.forward(&x, index, cache)?;
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
// Model
// ---------------------------------------------------------------------------

pub struct Nllb {
    config: NllbConfig,
    shared: Tensor,
    pos_table: Tensor,
    encoder: Encoder,
    decoder: Decoder,
    lm_head: Linear,
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

        let mut loaded = 1;
        progress(loaded);
        let encoder = Encoder::load(
            vb.pp("model").pp("encoder"),
            &config,
            &mut loaded,
            progress,
            cancel,
        )?;
        let decoder = Decoder::load(
            vb.pp("model").pp("decoder"),
            &config,
            &mut loaded,
            progress,
            cancel,
        )?;
        let lm_head = Linear::new(shared.clone(), None);
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

    pub fn config(&self) -> &NllbConfig {
        &self.config
    }

    fn embed(&self, ids: &Tensor, position_start: usize) -> Result<Tensor> {
        let (batch, seq) = ids.dims2()?;
        // `Tensor::embedding` expects 1-D ids.
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
        let x = self.embed(input_ids, 0)?;
        self.encoder.forward(x)
    }

    fn decode_step(&self, input_ids: &Tensor, cache: &mut DecoderCache) -> Result<Tensor> {
        let position_start = cache.seq_len();
        let x = self.embed(input_ids, position_start)?;
        let x = self.decoder.forward(x, cache)?;
        let last = x.narrow(1, x.dim(1)? - 1, 1)?;
        let logits = self.lm_head.forward(&last)?;
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
        let device = self.shared.device();
        let input_ids = Tensor::new(source_ids, device)?.unsqueeze(0)?;
        let encoder_hidden = self.encode(&input_ids)?;
        let cross_kv = self.decoder.prepare_cross(&encoder_hidden)?;
        let mut cache = DecoderCache::new(cross_kv);

        // Step 0: start token (eos), logits ignored.
        let eos = Tensor::new(&[tokenizer.eos_id()], device)?.unsqueeze(0)?;
        let _ = self.decode_step(&eos, &mut cache)?;
        if cancel.load(AtomicOrdering::Relaxed) {
            return Err(Error::Cancelled);
        }

        // Step 1: the target language is forced as the first generated token.
        let target_id = tokenizer.lang_id(target_language)?;
        let target = Tensor::new(&[target_id], device)?.unsqueeze(0)?;
        let first_logits = self.decode_step(&target, &mut cache)?;

        let beams = options.num_beams.max(1);
        let mut cache = cache.repeat(beams)?;
        let mut scores = vec![0f64; beams];
        let mut sequences: Vec<Vec<u32>> = vec![Vec::new(); beams];
        let mut rows = logits_rows(&first_logits, beams)?;

        let max_new = options.max_new_tokens.max(1);
        let eos_id = tokenizer.eos_id();
        let mut completed: Vec<(Vec<u32>, f64)> = Vec::new();
        let mut steps = 0usize;

        while steps < max_new {
            if cancel.load(AtomicOrdering::Relaxed) {
                return Err(Error::Cancelled);
            }

            let candidates = top_candidates(&rows, &scores, beams * 4 + 4);
            let mut active: Vec<Candidate> = Vec::with_capacity(beams);
            for candidate in candidates {
                if candidate.token == eos_id {
                    let length = (sequences[candidate.beam].len() + 1).max(1);
                    completed.push((
                        sequences[candidate.beam].clone(),
                        candidate.score / length as f64,
                    ));
                } else if active.len() < beams {
                    active.push(candidate);
                }
            }

            if active.is_empty() {
                break;
            }

            let parents: Vec<u32> = active.iter().map(|c| c.beam as u32).collect();
            let parents_tensor = Tensor::new(parents.as_slice(), device)?;
            cache = cache.reorder(&parents_tensor)?;

            let tokens: Vec<u32> = active.iter().map(|c| c.token).collect();
            let input_ids = Tensor::new(tokens.as_slice(), device)?.unsqueeze(1)?;
            let logits = self.decode_step(&input_ids, &mut cache)?;
            rows = logits_rows(&logits, 1)?;

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
                break;
            }

            // Early beam search stopping, like Transformers'
            // `BeamHypotheses.is_done`: once enough hypotheses are finished
            // and the best running hypothesis can no longer overtake them
            // (score normalized by length).
            if completed.len() >= beams {
                let worst_completed = completed
                    .iter()
                    .map(|(_, score)| *score)
                    .fold(f64::INFINITY, f64::min);
                let best_active =
                    scores.iter().copied().fold(f64::NEG_INFINITY, f64::max) / (steps as f64 + 1.0);
                if worst_completed >= best_active {
                    break;
                }
            }
        }

        if completed.is_empty() {
            return Err(Error::Other(
                "the model produced no translation tokens".to_string(),
            ));
        }

        let best = completed
            .into_iter()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(sequence, _)| sequence)
            .unwrap_or_default();
        Ok(best)
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
    if config.pad_token_id as usize + 1 <= rows && dim > 0 {
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

#[derive(Debug, Clone, Copy)]
pub struct GenerateOptions {
    pub max_new_tokens: usize,
    pub num_beams: usize,
}

#[derive(Debug, Clone)]
pub struct GenerationProgress {
    pub generated: usize,
    pub max: usize,
    pub preview: String,
}
