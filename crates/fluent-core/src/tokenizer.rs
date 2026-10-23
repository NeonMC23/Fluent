use std::path::Path;

use tokenizers::Tokenizer as HfTokenizer;

use crate::{Error, Result};

/// NLLB tokenizer backed by Hugging Face's `tokenizer.json`.
///
/// The file's post-processor is deliberately bypassed (it appends `</s>` and
/// `<unk>`): the exact sequence is built here, just like Transformers'
/// `NllbTokenizer`.
pub struct NllbTokenizer {
    inner: HfTokenizer,
    eos_id: u32,
    pad_id: u32,
    bos_id: u32,
    unk_id: u32,
}

impl NllbTokenizer {
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let path = dir.join("tokenizer.json");
        if !path.is_file() {
            return Err(Error::MissingModelFile {
                dir: dir.to_path_buf(),
                hint: "missing tokenizer.json".to_string(),
            });
        }
        let inner = HfTokenizer::from_file(&path)
            .map_err(|error| Error::Tokenizer(format!("{} : {error}", path.display())))?;

        let eos_id = inner.token_to_id("</s>").ok_or_else(|| {
            Error::Tokenizer("the </s> token is missing from the tokenizer".to_string())
        })?;
        let pad_id = inner.token_to_id("<pad>").unwrap_or(1);
        let bos_id = inner.token_to_id("<s>").unwrap_or(0);
        let unk_id = inner.token_to_id("<unk>").unwrap_or(3);

        Ok(Self {
            inner,
            eos_id,
            pad_id,
            bos_id,
            unk_id,
        })
    }

    pub fn eos_id(&self) -> u32 {
        self.eos_id
    }

    pub fn pad_id(&self) -> u32 {
        self.pad_id
    }

    pub fn bos_id(&self) -> u32 {
        self.bos_id
    }

    pub fn unk_id(&self) -> u32 {
        self.unk_id
    }

    pub fn token_to_id(&self, token: &str) -> Option<u32> {
        self.inner.token_to_id(token)
    }

    pub fn lang_id(&self, code: &str) -> Result<u32> {
        self.token_to_id(code).ok_or_else(|| {
            Error::UnknownLanguage(format!("{code} (missing from the NLLB tokenizer)"))
        })
    }

    /// Encodes a source sentence the way NLLB expects:
    /// `[source_language] … tokens … [</s>]`.
    ///
    /// Also returns a flag indicating whether the text was truncated.
    pub fn encode_source(
        &self,
        text: &str,
        source_language: &str,
        max_tokens: usize,
    ) -> Result<(Vec<u32>, bool)> {
        let lang_id = self.lang_id(source_language)?;
        let encoding = self
            .inner
            .encode(text, false)
            .map_err(|error| Error::Tokenizer(error.to_string()))?;
        let raw_ids = encoding.get_ids();

        let budget = max_tokens.saturating_sub(2).max(1);
        let truncated = raw_ids.len() > budget;
        let mut ids = Vec::with_capacity(raw_ids.len().min(budget) + 2);
        ids.push(lang_id);
        ids.extend_from_slice(&raw_ids[..raw_ids.len().min(budget)]);
        ids.push(self.eos_id);
        Ok((ids, truncated))
    }

    pub fn decode(&self, ids: &[u32]) -> Result<String> {
        let mut cleaned: Vec<u32> = ids
            .iter()
            .copied()
            .filter(|id| {
                *id != self.eos_id && *id != self.pad_id && *id != self.bos_id && *id != self.unk_id
            })
            .collect();
        // Drop any leading language token.
        while cleaned.first().is_some_and(|id| *id >= 256_000) {
            cleaned.remove(0);
        }
        let text = self
            .inner
            .decode(&cleaned, true)
            .map_err(|error| Error::Tokenizer(error.to_string()))?;
        Ok(clean_text(&text))
    }
}

fn clean_text(text: &str) -> String {
    let trimmed = text.trim();
    // NLLB sometimes inserts a space before punctuation.
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
