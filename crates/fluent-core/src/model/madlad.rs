//! MADLAD-400-3B-MT (T5) backend.
//!
//! The official model is distributed as a large f32 safetensors checkpoint,
//! which does not fit many CPU workstations. The same Apache-2.0 repository
//! also ships a q4_k GGUF file that Candle's quantized T5 implementation can
//! load directly. This module implements that GGUF route only; it never
//! downloads weights and never falls back to NLLB.

use std::path::{Path, PathBuf};

use candle_core::{DType, Device, Tensor};
use candle_transformers::models::quantized_t5;
use candle_transformers::quantized_var_builder;
use serde::Deserialize;
use tokenizers::Tokenizer as HfTokenizer;

use crate::model::nllb::GenerationProgress;
use crate::{default_generation_budget, Error, OutputLimit, Result};

use super::registry::MADLAD_LANGUAGES;

/// A verified FLORES-200 code and its MADLAD target prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MadladLanguage {
    pub flores: &'static str,
    pub prefix: &'static str,
}

#[derive(Debug, Deserialize)]
struct MadladConfig {
    architectures: Vec<String>,
    d_model: usize,
    d_ff: usize,
    num_layers: usize,
    num_decoder_layers: Option<usize>,
    num_heads: usize,
    vocab_size: usize,
    n_positions: usize,
    pad_token_id: usize,
    eos_token_id: usize,
    decoder_start_token_id: Option<usize>,
    #[serde(default)]
    tie_word_embeddings: bool,
}

pub fn language_prefix(code: &str) -> Option<&'static str> {
    MADLAD_LANGUAGES
        .iter()
        .find(|language| language.flores == code)
        .map(|language| language.prefix)
}

pub fn supports_language(code: &str) -> bool {
    language_prefix(code).is_some()
}

pub fn supported_languages() -> &'static [MadladLanguage] {
    MADLAD_LANGUAGES
}

pub fn find_gguf(dir: &Path) -> Option<PathBuf> {
    [
        "model-q4k.gguf",
        "model.gguf",
        "model-q3k.gguf",
        "model-q2k.gguf",
    ]
    .iter()
    .map(|name| dir.join(name))
    .find(|path| path.is_file())
}

/// SentencePiece/T5 tokenizer wrapper.
///
/// The model repository's `tokenizer.json` is generated from the official
/// `spiece.model`; loading it avoids adding a separate SentencePiece runtime.
pub struct MadladTokenizer {
    inner: HfTokenizer,
    eos_id: u32,
    pad_id: u32,
}

impl MadladTokenizer {
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let path = dir.join("tokenizer.json");
        if !path.is_file() {
            return Err(Error::MissingModelFile {
                dir: dir.to_path_buf(),
                hint: "missing tokenizer.json (generate it from the official spiece.model)"
                    .to_string(),
            });
        }
        let inner = HfTokenizer::from_file(&path)
            .map_err(|error| Error::Tokenizer(format!("{} : {error}", path.display())))?;
        let eos_id = inner.token_to_id("</s>").ok_or_else(|| {
            Error::Tokenizer("the </s> token is missing from the MADLAD tokenizer".to_string())
        })?;
        let pad_id = inner.token_to_id("<pad>").unwrap_or(0);

        // Every advertised prefix must be a single tokenizer token. If the
        // tokenizer changes, fail at load instead of producing broken prompts.
        for language in supported_languages() {
            if inner.token_to_id(language.prefix).is_none() {
                return Err(Error::Tokenizer(format!(
                    "MADLAD tokenizer is missing target prefix {}",
                    language.prefix
                )));
            }
        }

        Ok(Self {
            inner,
            eos_id,
            pad_id,
        })
    }

    pub fn eos_id(&self) -> u32 {
        self.eos_id
    }

    pub fn pad_id(&self) -> u32 {
        self.pad_id
    }

    pub fn token_to_id(&self, token: &str) -> Option<u32> {
        self.inner.token_to_id(token)
    }

    /// Encodes `text` with the MADLAD target-language prefix and EOS.
    ///
    /// Returns `(ids, truncated)`, matching the NLLB engine contract.
    pub fn encode_source(
        &self,
        text: &str,
        target_language: &str,
        max_tokens: usize,
    ) -> Result<(Vec<u32>, bool)> {
        let prefix = language_prefix(target_language).ok_or_else(|| {
            Error::UnknownLanguage(format!(
                "{target_language} (not in the verified MADLAD language map)"
            ))
        })?;
        // T5's Metaspace tokenizer prepends a space token before the
        // language prefix. Keep that exact tokenizer behaviour: encoding the
        // prefix and text separately omits it and produces degenerate output.
        let prepared = format!("{prefix} {text}");
        let encoding = self
            .inner
            .encode(prepared, false)
            .map_err(|error| Error::Tokenizer(error.to_string()))?;
        let raw = encoding.get_ids();
        let budget = max_tokens.saturating_sub(1).max(1);
        let truncated = raw.len() > budget;
        let mut ids = raw[..raw.len().min(budget)].to_vec();
        ids.push(self.eos_id);
        Ok((ids, truncated))
    }

    pub fn decode(&self, ids: &[u32]) -> Result<String> {
        let filtered: Vec<u32> = ids
            .iter()
            .copied()
            .filter(|id| {
                *id != self.eos_id
                    && *id != self.pad_id
                    && !supported_languages()
                        .iter()
                        .any(|language| self.token_to_id(language.prefix) == Some(*id))
            })
            .collect();
        let text = self
            .inner
            .decode(&filtered, true)
            .map_err(|error| Error::Tokenizer(error.to_string()))?;
        Ok(clean_text(&text))
    }
}

/// Loaded quantized T5/MADLAD model.
pub struct Madlad {
    model: quantized_t5::T5ForConditionalGeneration,
    config: MadladConfig,
    device: Device,
}

impl Madlad {
    pub fn load(
        dir: &Path,
        cancel: &std::sync::atomic::AtomicBool,
        progress: &mut dyn FnMut(usize),
    ) -> Result<Self> {
        if !dir.is_dir() {
            return Err(Error::ModelNotFound(dir.to_path_buf()));
        }
        let config_path = dir.join("config.json");
        if !config_path.is_file() {
            return Err(Error::MissingModelFile {
                dir: dir.to_path_buf(),
                hint: "missing config.json".to_string(),
            });
        }
        let raw = std::fs::read_to_string(&config_path)?;
        let config: MadladConfig = serde_json::from_str(&raw)?;
        if !config
            .architectures
            .iter()
            .any(|architecture| architecture == "T5ForConditionalGeneration")
        {
            return Err(Error::Config(format!(
                "{} is not a T5ForConditionalGeneration config",
                config_path.display()
            )));
        }
        if config.d_model == 0
            || config.d_ff == 0
            || config.num_heads == 0
            || config.d_model % config.num_heads != 0
            || config.num_layers == 0
            || config.num_decoder_layers.unwrap_or(config.num_layers) == 0
            || config.vocab_size == 0
            || config.n_positions == 0
        {
            return Err(Error::Config(
                "MADLAD config contains invalid model dimensions".to_string(),
            ));
        }
        if config.pad_token_id >= config.vocab_size || config.eos_token_id >= config.vocab_size {
            return Err(Error::Config(
                "MADLAD pad/eos token ids are outside the vocabulary".to_string(),
            ));
        }
        let gguf = find_gguf(dir).ok_or_else(|| Error::MissingModelFile {
            dir: dir.to_path_buf(),
            hint: "missing model-q4k.gguf (see docs/MADLAD.md)".to_string(),
        })?;

        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        progress(1);
        let vb = quantized_var_builder::VarBuilder::from_gguf(&gguf, &Device::Cpu)
            .map_err(|error| Error::Other(format!("could not read {}: {error}", gguf.display())))?;
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        progress(2);
        let candle_config: quantized_t5::Config = serde_json::from_str(&raw)?;
        let model = quantized_t5::T5ForConditionalGeneration::load(vb, &candle_config)
            .map_err(|error| Error::Other(format!("could not load MADLAD weights: {error}")))?;
        progress(3);
        Ok(Self {
            model,
            config,
            device: Device::Cpu,
        })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn max_source_tokens(&self) -> usize {
        self.config.n_positions
    }

    pub fn config_label(&self) -> String {
        let decoder_layers = self
            .config
            .num_decoder_layers
            .unwrap_or(self.config.num_layers);
        format!(
            "T5 · {}/{} layers · d_model {} · {} heads · q4_k{}",
            self.config.num_layers,
            decoder_layers,
            self.config.d_model,
            self.config.num_heads,
            if self.config.tie_word_embeddings {
                " · tied"
            } else {
                ""
            }
        )
    }

    /// Deterministic greedy generation. T5/MADLAD does not use NLLB's beam
    /// search implementation; the GUI disables beam search for this backend.
    pub fn generate(
        &mut self,
        tokenizer: &MadladTokenizer,
        source_ids: &[u32],
        limit: OutputLimit,
        progress: &mut dyn FnMut(GenerationProgress),
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Vec<u32>> {
        use std::sync::atomic::Ordering;

        let max_tokens = match limit {
            OutputLimit::Auto => default_generation_budget(source_ids.len()),
            OutputLimit::Tokens(tokens) => tokens.max(1),
            OutputLimit::UntilEos => usize::MAX,
        };
        let limit_label = if max_tokens == usize::MAX {
            0
        } else {
            max_tokens
        };

        self.model.clear_kv_cache();
        let input = Tensor::new(source_ids, &Device::Cpu)?.unsqueeze(0)?;
        let encoder_output = self.model.encode(&input)?;
        let mut decoder_input = self.config.decoder_start_token_id.unwrap_or(0) as u32;
        let mut tokens: Vec<u32> = Vec::new();

        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            if max_tokens != usize::MAX && tokens.len() >= max_tokens {
                break;
            }
            let input = Tensor::new(&[decoder_input], &Device::Cpu)?.unsqueeze(0)?;
            let logits = self.model.decode(&input, &encoder_output)?;
            let row = logits.squeeze(0)?.to_dtype(DType::F32)?.to_vec1::<f32>()?;
            let Some(next) = argmax(&row) else {
                return Err(Error::Other(
                    "MADLAD produced invalid logits (no finite token)".to_string(),
                ));
            };
            if next == tokenizer.eos_id() {
                break;
            }
            tokens.push(next);
            decoder_input = next;
            let preview = tokenizer.decode(&tokens).unwrap_or_default();
            progress(GenerationProgress {
                generated: tokens.len(),
                max: limit_label,
                preview,
            });
        }

        if tokens.is_empty() {
            return Err(Error::Other(
                "MADLAD produced no translation tokens".to_string(),
            ));
        }
        Ok(tokens)
    }
}

fn argmax(values: &[f32]) -> Option<u32> {
    let mut best: Option<(u32, f32)> = None;
    for (index, &value) in values.iter().enumerate() {
        if !value.is_finite() {
            continue;
        }
        match best {
            None => best = Some((index as u32, value)),
            Some((best_index, best_value))
                if value > best_value || (value == best_value && (index as u32) < best_index) =>
            {
                best = Some((index as u32, value));
            }
            _ => {}
        }
    }
    best.map(|(index, _)| index)
}

fn clean_text(text: &str) -> String {
    let trimmed = text.trim();
    let mut output = String::with_capacity(trimmed.len());
    let mut previous_was_space = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !previous_was_space {
                output.push(' ');
            }
            previous_was_space = true;
        } else {
            output.push(ch);
            previous_was_space = false;
        }
    }
    output.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn model_dir() -> Option<PathBuf> {
        std::env::var_os("FLUENT_MADLAD_MODEL")
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
    }

    #[test]
    fn every_advertised_language_has_a_unique_prefix() {
        let mut seen = std::collections::HashSet::new();
        for language in supported_languages() {
            assert!(
                language.prefix.starts_with("<2"),
                "bad prefix for {}: {}",
                language.flores,
                language.prefix
            );
            assert!(
                seen.insert(language.prefix),
                "duplicate MADLAD prefix {}",
                language.prefix
            );
        }
        for required in [
            "eng_Latn", "fra_Latn", "deu_Latn", "spa_Latn", "jpn_Jpan", "arb_Arab", "swh_Latn",
        ] {
            assert!(supports_language(required), "missing {required}");
        }
    }

    #[test]
    fn tokenizer_prefix_and_eos_are_real_tokens() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set FLUENT_MADLAD_MODEL to run tokenizer checks");
            return;
        };
        let tokenizer = MadladTokenizer::from_dir(&dir).expect("tokenizer");
        let prefix = tokenizer.token_to_id("<2fr>").expect("<2fr> token");
        let (ids, truncated) = tokenizer
            .encode_source("The library opens at eight.", "fra_Latn", 64)
            .expect("encode");
        assert!(!truncated);
        assert!(
            ids.contains(&prefix),
            "target prefix missing from tokenized input: {ids:?}"
        );
        assert_eq!(*ids.last().unwrap(), tokenizer.eos_id());
        assert!(ids.len() > 2);
    }

    #[test]
    fn real_model_translates_required_pairs() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set FLUENT_MADLAD_MODEL to run MADLAD inference");
            return;
        };
        let tokenizer = MadladTokenizer::from_dir(&dir).expect("tokenizer");
        let cancel = AtomicBool::new(false);
        let mut model = Madlad::load(&dir, &cancel, &mut |_| {}).expect("load MADLAD");
        for (source, target) in [
            ("eng_Latn", "fra_Latn"),
            ("fra_Latn", "eng_Latn"),
            ("eng_Latn", "deu_Latn"),
            ("eng_Latn", "spa_Latn"),
            ("eng_Latn", "jpn_Jpan"),
            ("eng_Latn", "arb_Arab"),
            ("eng_Latn", "swh_Latn"),
        ] {
            let (ids, _) = tokenizer
                .encode_source("The library opens at eight.", target, 128)
                .expect("encode");
            let output = model
                .generate(
                    &tokenizer,
                    &ids,
                    OutputLimit::Tokens(64),
                    &mut |_| {},
                    &cancel,
                )
                .unwrap_or_else(|error| panic!("{source}->{target}: {error}"));
            let text = tokenizer.decode(&output).unwrap_or_default();
            assert!(!text.trim().is_empty(), "{source}->{target} was empty");
            println!("{source}->{target}: {text}");
        }
    }
}
