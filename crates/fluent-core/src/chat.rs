//! Local chat backend.
//!
//! Chat is a separate capability from translation. It uses a quantized
//! instruction-tuned causal language model (Qwen2.5-Instruct GGUF) and a
//! separate model directory, worker and GUI view. Translation models are never
//! exposed here.

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::models::quantized_qwen2::ModelWeights;
use serde::{Deserialize, Serialize};
use tokenizers::Tokenizer;

use crate::{Error, Result};

/// Default hidden system instruction. It is not shown as a chat message.
pub const DEFAULT_SYSTEM_PROMPT: &str = "You are Fluent, a helpful local assistant. Answer clearly and concisely. If you are unsure, say so.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChatOptions {
    /// Total prompt + generated token budget.
    pub max_context: usize,
    pub max_new_tokens: usize,
    /// Applied to tokens seen in the recent window. 1.0 disables it.
    pub repetition_penalty: f32,
    pub repetition_window: usize,
}

impl Default for ChatOptions {
    fn default() -> Self {
        Self {
            max_context: 4096,
            max_new_tokens: 512,
            repetition_penalty: 1.1,
            repetition_window: 96,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatReply {
    pub text: String,
    pub prompt_tokens: usize,
    pub generated_tokens: usize,
    pub first_token: Option<Duration>,
    pub duration: Duration,
    /// Older turns or the start of the prompt were omitted to fit the context.
    pub context_truncated: bool,
    /// Generation stopped at the configured output ceiling rather than EOS.
    pub hit_token_limit: bool,
}

pub struct ChatEngine {
    model: ModelWeights,
    tokenizer: Tokenizer,
    im_start: u32,
    im_end: u32,
    end_of_text: u32,
    device: Device,
    model_label: String,
}

impl ChatEngine {
    /// Loads a chat model from `dir`. The directory must contain the GGUF and
    /// the tokenizer files from the chat model manifest.
    pub fn load(dir: &Path, model_label: impl Into<String>, cancel: &AtomicBool) -> Result<Self> {
        if !dir.is_dir() {
            return Err(Error::ModelNotFound(dir.to_path_buf()));
        }
        let gguf_path = find_gguf(dir).ok_or_else(|| Error::MissingModelFile {
            dir: dir.to_path_buf(),
            hint: "missing a chat model .gguf file".to_string(),
        })?;
        let tokenizer_path = dir.join("tokenizer.json");
        if !tokenizer_path.is_file() {
            return Err(Error::MissingModelFile {
                dir: dir.to_path_buf(),
                hint: "missing tokenizer.json".to_string(),
            });
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }

        let device = Device::Cpu;
        let mut file = File::open(&gguf_path)?;
        let content = gguf_file::Content::read(&mut file)
            .map_err(|error| Error::Other(format!("chat GGUF: {error}")))?;
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let model = ModelWeights::from_gguf(content, &mut file, &device)
            .map_err(|error| Error::Other(format!("chat model: {error}")))?;
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|error| Error::Tokenizer(format!("{}: {error}", tokenizer_path.display())))?;
        let im_start = tokenizer.token_to_id("<|im_start|>").ok_or_else(|| {
            Error::Tokenizer("chat tokenizer is missing <|im_start|>".to_string())
        })?;
        let im_end = tokenizer
            .token_to_id("<|im_end|>")
            .ok_or_else(|| Error::Tokenizer("chat tokenizer is missing <|im_end|>".to_string()))?;
        let end_of_text = tokenizer.token_to_id("<|endoftext|>").ok_or_else(|| {
            Error::Tokenizer("chat tokenizer is missing <|endoftext|>".to_string())
        })?;

        Ok(Self {
            model,
            tokenizer,
            im_start,
            im_end,
            end_of_text,
            device,
            model_label: model_label.into(),
        })
    }

    pub fn model_label(&self) -> &str {
        &self.model_label
    }

    /// Generates an assistant reply.
    ///
    /// The caller owns the visible conversation; `messages` should contain the
    /// user/assistant turns and optionally a leading system message. Streaming
    /// deltas are delivered through `on_delta`. On cancellation the partial
    /// text is returned in the error path via `on_delta`, and the engine state
    /// is reset for the next request.
    pub fn reply(
        &mut self,
        messages: &[ChatMessage],
        options: ChatOptions,
        on_delta: &mut dyn FnMut(&str),
        cancel: &AtomicBool,
    ) -> Result<ChatReply> {
        let started = Instant::now();
        let max_new = options.max_new_tokens.max(1);
        let max_prompt = options.max_context.saturating_sub(max_new).max(1);
        let (prompt_ids, context_truncated) = self.encode_with_budget(messages, max_prompt)?;
        self.generate(
            prompt_ids,
            started,
            max_new,
            options,
            context_truncated,
            on_delta,
            cancel,
        )
    }

    /// Encodes a conversation within a prompt budget.
    ///
    /// The oldest turns are dropped first. If a single message is still too
    /// long, its beginning (where the instruction usually is) is kept and the
    /// assistant suffix is appended, so the model is never asked to continue a
    /// truncated assistant turn.
    fn encode_with_budget(
        &self,
        messages: &[ChatMessage],
        max_prompt: usize,
    ) -> Result<(Vec<u32>, bool)> {
        let mut kept = messages.to_vec();
        let mut truncated = false;
        loop {
            let prompt = build_prompt(&kept);
            let encoding = self
                .tokenizer
                .encode(prompt.as_str(), false)
                .map_err(|error| Error::Tokenizer(error.to_string()))?;
            if encoding.get_ids().len() <= max_prompt {
                return Ok((encoding.get_ids().to_vec(), truncated));
            }
            let start =
                usize::from(kept.first().map(|message| message.role) == Some(ChatRole::System));
            let remaining = kept.len().saturating_sub(start);
            if remaining > 1 {
                let pair = kept.get(start).map(|message| message.role) == Some(ChatRole::User)
                    && kept.get(start + 1).map(|message| message.role) == Some(ChatRole::Assistant);
                let count = if pair { 2 } else { 1 }.min(remaining);
                kept.drain(start..start + count);
                truncated = true;
                continue;
            }
            // A single oversized message: keep the beginning, then append the
            // assistant suffix so generation starts in the right role.
            let suffix = self
                .tokenizer
                .encode("<|im_start|>assistant\n", false)
                .map_err(|error| Error::Tokenizer(error.to_string()))?;
            let keep = max_prompt.saturating_sub(suffix.get_ids().len());
            let mut ids = encoding.get_ids().to_vec();
            ids.truncate(keep);
            ids.extend_from_slice(suffix.get_ids());
            ids.truncate(max_prompt);
            return Ok((ids, true));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn generate(
        &mut self,
        prompt_ids: Vec<u32>,
        started: Instant,
        max_new: usize,
        options: ChatOptions,
        context_truncated: bool,
        on_delta: &mut dyn FnMut(&str),
        cancel: &AtomicBool,
    ) -> Result<ChatReply> {
        let prompt_tokens = prompt_ids.len();
        if prompt_tokens == 0 {
            return Err(Error::EmptyText);
        }
        let input = Tensor::new(prompt_ids.as_slice(), &self.device)?.unsqueeze(0)?;
        let logits = self
            .model
            .forward(&input, 0)
            .map_err(|error| Error::Other(format!("chat prefill: {error}")))?;
        let mut next = self.sample(&logits, &[], options)?;
        let mut generated: Vec<u32> = Vec::with_capacity(max_new);
        let mut emitted = String::new();
        let mut first_token = None;

        for position in prompt_tokens..prompt_tokens.saturating_add(max_new) {
            if cancel.load(Ordering::Relaxed) {
                // Keep the partial answer visible to the caller.
                let text = self
                    .tokenizer
                    .decode(&generated, true)
                    .unwrap_or_else(|_| emitted.clone());
                if text.len() > emitted.len() {
                    on_delta(&text[emitted.len()..]);
                }
                return Err(Error::Cancelled);
            }
            if next == self.im_end || next == self.end_of_text {
                break;
            }
            if generated.is_empty() {
                first_token = Some(started.elapsed());
            }
            generated.push(next);
            let text = self
                .tokenizer
                .decode(&generated, true)
                .map_err(|error| Error::Tokenizer(error.to_string()))?;
            if text.len() > emitted.len() && text.starts_with(&emitted) {
                on_delta(&text[emitted.len()..]);
                emitted = text;
            } else if !text.starts_with(&emitted) {
                // A multi-byte token was only partially decoded; wait for the
                // next token and emit the corrected suffix then.
                if text.len() > emitted.len() {
                    on_delta(&text[emitted.len()..]);
                    emitted = text;
                }
            }

            let input = Tensor::new(&[next], &self.device)?.unsqueeze(0)?;
            let logits = self
                .model
                .forward(&input, position)
                .map_err(|error| Error::Other(format!("chat decode: {error}")))?;
            next = self.sample(&logits, &generated, options)?;
        }

        let text = self
            .tokenizer
            .decode(&generated, true)
            .map_err(|error| Error::Tokenizer(error.to_string()))?;
        if text.len() > emitted.len() {
            on_delta(&text[emitted.len()..]);
        }
        Ok(ChatReply {
            text,
            prompt_tokens,
            generated_tokens: generated.len(),
            first_token,
            duration: started.elapsed(),
            context_truncated,
            hit_token_limit: generated.len() >= max_new,
        })
    }

    fn sample(&self, logits: &Tensor, recent: &[u32], options: ChatOptions) -> Result<u32> {
        let values = logits
            .flatten_all()
            .map_err(|error| Error::Other(format!("chat logits: {error}")))?
            .to_vec1::<f32>()
            .map_err(|error| Error::Other(format!("chat logits: {error}")))?;
        if values.is_empty() {
            return Err(Error::Other("chat model returned empty logits".to_string()));
        }
        let penalty = options.repetition_penalty;
        let mut adjusted = values;
        if penalty > 1.0 {
            let start = recent.len().saturating_sub(options.repetition_window);
            let mut seen = std::collections::HashSet::new();
            for &token in &recent[start..] {
                if !seen.insert(token) {
                    continue;
                }
                let index = token as usize;
                if let Some(value) = adjusted.get_mut(index) {
                    *value = if *value > 0.0 {
                        *value / penalty
                    } else {
                        *value * penalty
                    };
                }
            }
        }
        let (index, _) = adjusted
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .ok_or_else(|| Error::Other("chat sampler failed".to_string()))?;
        Ok(index as u32)
    }

    /// Token id for `<|im_start|>`, used by tests and diagnostics.
    pub fn im_start(&self) -> u32 {
        self.im_start
    }
}

fn find_gguf(dir: &Path) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut candidates: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// Builds the Qwen2.5 chat prompt. A leading system message overrides the
/// default instruction.
pub fn build_prompt(messages: &[ChatMessage]) -> String {
    let mut prompt = String::new();
    let (system, rest) = match messages.first() {
        Some(first) if first.role == ChatRole::System => (first.content.as_str(), &messages[1..]),
        _ => (DEFAULT_SYSTEM_PROMPT, messages),
    };
    prompt.push_str("<|im_start|>system\n");
    prompt.push_str(if system.trim().is_empty() {
        DEFAULT_SYSTEM_PROMPT
    } else {
        system
    });
    prompt.push_str("<|im_end|>\n");
    for message in rest {
        let role = match message.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        };
        prompt.push_str("<|im_start|>");
        prompt.push_str(role);
        prompt.push('\n');
        prompt.push_str(&message.content);
        prompt.push_str("<|im_end|>\n");
    }
    prompt.push_str("<|im_start|>assistant\n");
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contains_system_and_turns() {
        let messages = vec![
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi!"),
            ChatMessage::user("How are you?"),
        ];
        let prompt = build_prompt(&messages);
        assert!(prompt.starts_with("<|im_start|>system\n"));
        assert!(prompt.contains(DEFAULT_SYSTEM_PROMPT));
        assert!(prompt.contains("<|im_start|>user\nHello<|im_end|>"));
        assert!(prompt.contains("<|im_start|>assistant\nHi!<|im_end|>"));
        assert!(prompt.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn leading_system_message_overrides_default() {
        let messages = vec![
            ChatMessage {
                role: ChatRole::System,
                content: "Be brief.".to_string(),
            },
            ChatMessage::user("Hello"),
        ];
        let prompt = build_prompt(&messages);
        assert!(prompt.contains("Be brief."));
        assert!(!prompt.contains(DEFAULT_SYSTEM_PROMPT));
    }
}
