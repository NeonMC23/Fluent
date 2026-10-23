//! Fluent Core — local NLLB-200 translation engine in pure Rust.
//!
//! The engine loads Meta's NLLB-200 checkpoints directly from their PyTorch
//! (`.bin`) or safetensors files, without Python, ONNX conversion or a
//! background server.

pub mod config;
pub mod detect;
pub mod engine;
pub mod languages;
pub mod model;
pub mod tokenizer;

use std::fmt;
use std::path::PathBuf;

pub use config::ProjectConfig;
pub use detect::Detection;
pub use engine::{
    estimate_memory, memory_info, DeviceChoice, Engine, EngineConfig, EngineStatus, MemoryEstimate,
    MemoryInfo, Precision, Progress, ProgressStage, TranslationRequest, TranslationResult,
};
pub use languages::{find_language, languages, popular_languages};
pub use tokenizer::NllbTokenizer;

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

    #[error("tokenizer: {0}")]
    Tokenizer(String),

    #[error("configuration: {0}")]
    Config(String),

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
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Accurate => "accurate",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Accurate => "Accurate",
        }
    }

    pub fn model_label(self) -> &'static str {
        match self {
            Self::Fast => "NLLB-200 1.3B",
            Self::Accurate => "NLLB-200 3.3B",
        }
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
            other => Err(Error::Config(format!("unknown translation mode: {other}"))),
        }
    }
}
