use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, Mode, Result};

#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub config_path: Option<PathBuf>,
    pub base_dir: PathBuf,
    pub default_source_language: String,
    pub default_target_language: String,
    pub default_mode: Mode,
    pub theme: String,
    pub fast_model: PathBuf,
    pub accurate_model: PathBuf,
    pub max_length: usize,
    pub num_beams: usize,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            config_path: None,
            fast_model: base_dir.join("models/nllb-200-1.3b"),
            accurate_model: base_dir.join("models/nllb-200-3.3b"),
            base_dir,
            default_source_language: "auto".to_string(),
            default_target_language: "eng_Latn".to_string(),
            default_mode: Mode::Fast,
            theme: "system".to_string(),
            max_length: 512,
            num_beams: 4,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    default_source_language: Option<String>,
    default_target_language: Option<String>,
    default_mode: Option<String>,
    theme: Option<String>,
    models: Option<RawModels>,
    generation: Option<RawGeneration>,
}

#[derive(Debug, Deserialize, Default)]
struct RawModels {
    fast: Option<String>,
    accurate: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RawGeneration {
    max_length: Option<usize>,
    num_beams: Option<usize>,
}

impl ProjectConfig {
    /// Loads `config.json`, searching in order:
    /// `FLUENT_CONFIG`, the executable directory, its parent, the current directory.
    pub fn load() -> Self {
        for candidate in config_candidates() {
            if candidate.is_file() {
                match Self::load_from(&candidate) {
                    Ok(config) => return config,
                    Err(error) => {
                        eprintln!(
                            "Fluent: could not read {} ({error}), using defaults.",
                            candidate.display()
                        );
                    }
                }
            }
        }
        Self::default()
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let raw_text = std::fs::read_to_string(path)?;
        let raw: RawConfig = serde_json::from_str(&raw_text)?;
        let base_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        let mut config = Self {
            config_path: Some(path.to_path_buf()),
            base_dir: base_dir.clone(),
            ..Self::default()
        };

        if let Some(value) = raw.default_source_language {
            config.default_source_language = value;
        }
        if let Some(value) = raw.default_target_language {
            config.default_target_language = value;
        }
        if let Some(value) = raw.default_mode {
            if let Ok(mode) = value.parse() {
                config.default_mode = mode;
            }
        }
        if let Some(value) = raw.theme {
            config.theme = value;
        }
        if let Some(models) = raw.models {
            if let Some(fast) = models.fast {
                config.fast_model = resolve_model_path(&base_dir, &fast);
            }
            if let Some(accurate) = models.accurate {
                config.accurate_model = resolve_model_path(&base_dir, &accurate);
            }
        }
        if let Some(generation) = raw.generation {
            if let Some(max_length) = generation.max_length {
                config.max_length = max_length.clamp(16, 1024);
            }
            if let Some(num_beams) = generation.num_beams {
                config.num_beams = num_beams.clamp(1, 8);
            }
        }

        if !config.fast_model.is_dir() {
            let fallback = base_dir.join("models/nllb-200-1.3b");
            if fallback.is_dir() {
                config.fast_model = fallback;
            }
        }
        if !config.accurate_model.is_dir() {
            let fallback = base_dir.join("models/nllb-200-3.3b");
            if fallback.is_dir() {
                config.accurate_model = fallback;
            }
        }

        Ok(config)
    }

    pub fn model_path(&self, mode: Mode) -> &Path {
        match mode {
            Mode::Fast => &self.fast_model,
            Mode::Accurate => &self.accurate_model,
        }
    }

    pub fn validate(&self) -> Result<()> {
        for mode in [Mode::Fast, Mode::Accurate] {
            let path = self.model_path(mode);
            if !path.exists() {
                return Err(Error::ModelNotFound(path.to_path_buf()));
            }
        }
        Ok(())
    }
}

fn config_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(value) = std::env::var("FLUENT_CONFIG") {
        if !value.trim().is_empty() {
            candidates.push(PathBuf::from(value));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("config.json"));
            if let Some(parent) = dir.parent() {
                candidates.push(parent.join("config.json"));
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("config.json"));
    }
    candidates
}

fn resolve_model_path(base_dir: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}
