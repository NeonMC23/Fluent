//! Fluent Core — local NLLB-200 translation engine in pure Rust.
//!
//! The engine loads Meta's NLLB-200 checkpoints directly from their PyTorch
//! (`.bin`) or safetensors files, without Python, ONNX conversion or a
//! background server.

pub mod chat;
pub mod config;
pub mod detect;
pub mod engine;
pub mod languages;
pub mod model;
pub mod tokenizer;

use std::fmt;
use std::path::PathBuf;

pub use chat::{ChatEngine, ChatMessage, ChatOptions, ChatReply, ChatRole};
pub use config::ProjectConfig;
pub use detect::Detection;
pub use engine::{
    estimate_memory, estimate_memory_for_choice, estimate_memory_for_mode, memory_info,
    DeviceChoice, Engine, EngineConfig, EngineStatus, MemoryEstimate, MemoryInfo, Precision,
    Progress, ProgressStage, TranslationRequest, TranslationResult,
};
pub use languages::{find_language, languages, popular_languages};
pub use model::catalogue::{
    fetch as fetch_catalogue, Catalogue, CatalogueFile, CatalogueMatch, CatalogueModel,
};
pub use model::chat_registry::{
    chat_model_spec, default_chat_model, ChatModelFile, ChatModelSpec, CHAT_MODELS,
};
pub use model::model_manager::{
    available_bytes, chat_model_dir, directory_size, inspect_chat_model, inspect_model,
    install_chat_model, install_model, installed_chat_models, model_dir, remove_chat_model,
    remove_model, verify_chat_model, verify_model, InstallState, ManagedFile, ManagedModel,
    ModelInstallation, TaskProgress, TaskStage,
};
pub use model::registry::model_spec_by_id;
pub use model::registry::{
    language_list, model_spec, resolve_language_selection, Architecture, LanguageSelection,
    ModelFile, ModelSpec, MODELS,
};
pub use tokenizer::NllbTokenizer;

/// Requested output-length policy.
///
/// `Auto` preserves the input-derived budget used by earlier releases.
/// `Tokens(n)` is a finite ceiling. `UntilEos` removes the Fluent token
/// ceiling and runs until the model emits EOS, the caller cancels, or the
/// model's positional capacity is reached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutputLimit {
    #[default]
    Auto,
    Tokens(usize),
    UntilEos,
}

impl OutputLimit {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Tokens(_) => "Custom limit",
            Self::UntilEos => "No limit",
        }
    }
}

/// Length-aware default budget for generated tokens.
///
/// A translation is rarely longer than twice its source, so this keeps short
/// inputs far below the 512-token ceiling while still leaving room for
/// expansion, identifiers and punctuation. `Auto` preserves this behaviour.
pub fn default_generation_budget(source_tokens: usize) -> usize {
    source_tokens
        .saturating_mul(2)
        .saturating_add(16)
        .clamp(16, 512)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("translation cancelled")]
    Cancelled,

    #[error("empty text")]
    EmptyText,

    #[error("the target language cannot be “auto”")]
    AutoTarget,

    #[error("unknown language: {0}")]
    UnknownLanguage(String),

    #[error("model not found: {0}")]
    ModelNotFound(PathBuf),

    #[error("model file missing in {dir}: {hint}")]
    MissingModelFile { dir: PathBuf, hint: String },

    #[error("not enough memory to load the model (about {needed_mb} MB needed, {available_mb} MB available)")]
    OutOfMemory { needed_mb: u64, available_mb: u64 },

    #[error("not enough memory to extend the translation cache to {0} tokens")]
    CacheAllocation(usize),

    #[error("the model reached its {0}-token positional capacity before producing EOS")]
    PositionLimit(usize),

    #[error("tokenizer: {0}")]
    Tokenizer(String),

    #[error("configuration: {0}")]
    Config(String),

    #[error("model catalogue: {0}")]
    Catalogue(String),

    #[error("engine error: {0}")]
    Candle(#[from] candle_core::Error),

    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A language as exposed by the UI and the NLLB tokenizer.
#[derive(Debug, Clone, Copy)]
pub struct Language {
    pub code: &'static str,
    pub name: &'static str,
    pub native: &'static str,
    pub popular: bool,
    pub iso3: &'static str,
}

impl Language {
    pub fn label(&self) -> String {
        format!("{} ({})", self.name, self.code)
    }

    pub fn search_haystack(&self) -> String {
        format!("{} {} {} {}", self.name, self.native, self.code, self.iso3).to_lowercase()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Fast,
    Accurate,
    Madlad,
    Small,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Accurate => "accurate",
            Self::Madlad => "madlad",
            Self::Small => "small",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "NLLB-200 1.3B",
            Self::Accurate => "NLLB-200 3.3B",
            Self::Madlad => "MADLAD-400 3B",
            Self::Small => "SMaLL-100",
        }
    }

    pub fn model_label(self) -> &'static str {
        match self {
            Self::Fast => "NLLB-200 1.3B",
            Self::Accurate => "NLLB-200 3.3B",
            Self::Madlad => "MADLAD-400 3B",
            Self::Small => "SMaLL-100",
        }
    }

    pub fn supports_language(self, code: &str) -> bool {
        crate::model::registry::supports_language(self, code)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Mode {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fast" | "1.3b" => Ok(Self::Fast),
            "accurate" | "3.3b" => Ok(Self::Accurate),
            "madlad" | "madlad400" | "madlad-400-3b-mt" | "3b" => Ok(Self::Madlad),
            "small" | "small100" | "small-100" | "330m" => Ok(Self::Small),
            other => Err(Error::Config(format!("unknown translation mode: {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_parsing_accepts_labels_and_model_sizes() {
        assert_eq!("fast".parse::<Mode>().unwrap(), Mode::Fast);
        assert_eq!("FAST".parse::<Mode>().unwrap(), Mode::Fast);
        assert_eq!("1.3B".parse::<Mode>().unwrap(), Mode::Fast);
        assert_eq!("accurate".parse::<Mode>().unwrap(), Mode::Accurate);
        assert_eq!(" 3.3b ".parse::<Mode>().unwrap(), Mode::Accurate);
        assert_eq!("madlad".parse::<Mode>().unwrap(), Mode::Madlad);
        assert_eq!("MADLAD400".parse::<Mode>().unwrap(), Mode::Madlad);
        assert_eq!("small".parse::<Mode>().unwrap(), Mode::Small);
        assert_eq!("SMALL100".parse::<Mode>().unwrap(), Mode::Small);
        assert!("medium".parse::<Mode>().is_err());
    }

    #[test]
    fn mode_labels_are_stable() {
        assert_eq!(Mode::Fast.as_str(), "fast");
        assert_eq!(Mode::Accurate.as_str(), "accurate");
        assert_eq!(Mode::Fast.label(), "NLLB-200 1.3B");
        assert_eq!(Mode::Accurate.model_label(), "NLLB-200 3.3B");
        assert_eq!(Mode::Madlad.model_label(), "MADLAD-400 3B");
        assert_eq!(Mode::Small.model_label(), "SMaLL-100");
    }
}
