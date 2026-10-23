//! High-level translation engine.
//!
//! [`Engine`] owns the tokenizer and the loaded NLLB model. It is deliberately
//! synchronous: the GUI runs it on a worker thread and can cancel a running
//! translation through an [`AtomicBool`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use candle_core::{DType, Device};

use crate::config::ProjectConfig;
use crate::detect::{detect, Detection};
use crate::model::nllb::{GenerateOptions, Nllb};
use crate::model::Checkpoint;
use crate::tokenizer::NllbTokenizer;
use crate::{Error, Mode, Result};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Numerical precision used for the weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Precision {
    /// f32 for the 1.3B model on CPU, f16 for the 3.3B model or a GPU.
    Auto,
    F32,
    F16,
}

impl Default for Precision {
    fn default() -> Self {
        Self::Auto
    }
}

impl Precision {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::F32 => "f32 (maximum precision)",
            Self::F16 => "f16 (half the memory)",
        }
    }
}

/// Device on which the model runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceChoice {
    /// First available GPU, otherwise CPU.
    Auto,
    Cpu,
    Cuda(usize),
    Metal(usize),
}

impl Default for DeviceChoice {
    fn default() -> Self {
        Self::Auto
    }
}

impl DeviceChoice {
    pub fn label(self) -> String {
        match self {
            Self::Auto => "Automatic".to_string(),
            Self::Cpu => "Processor (CPU)".to_string(),
            Self::Cuda(index) => format!("NVIDIA GPU #{index} (CUDA)"),
            Self::Metal(index) => format!("Apple GPU #{index} (Metal)"),
        }
    }
}

/// Everything needed to load and run a model.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub mode: Mode,
    pub model_dir: PathBuf,
    pub device: DeviceChoice,
    pub precision: Precision,
    /// Maximum length of the source sequence (tokens).
    pub max_length: usize,
    /// Maximum number of generated tokens.
    pub max_new_tokens: usize,
    /// Beam search width.
    pub num_beams: usize,
}

impl EngineConfig {
    pub fn new(mode: Mode, model_dir: impl Into<PathBuf>) -> Self {
        Self {
            mode,
            model_dir: model_dir.into(),
            device: DeviceChoice::Auto,
            precision: Precision::Auto,
            max_length: 512,
            max_new_tokens: 512,
            num_beams: 4,
        }
    }

    /// Builds the configuration from `config.json`.
    pub fn from_project(project: &ProjectConfig, mode: Mode) -> Self {
        let mut config = Self::new(mode, project.model_path(mode).to_path_buf());
        config.max_length = project.max_length;
        config.max_new_tokens = project.max_length;
        config.num_beams = project.num_beams;
        config
    }
}

// ---------------------------------------------------------------------------
// Progress and results
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressStage {
    Preparing,
    LoadingModel,
    Encoding,
    Decoding,
    Finishing,
}

impl ProgressStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing",
            Self::LoadingModel => "Loading model",
            Self::Encoding => "Analyzing text",
            Self::Decoding => "Translating",
            Self::Finishing => "Finishing",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub stage: ProgressStage,
    /// Between 0 and 1, when known.
    pub fraction: Option<f32>,
    pub message: String,
    /// Preview of the translation in progress (during decoding).
    pub preview: Option<String>,
}

impl Progress {
    pub fn new(stage: ProgressStage, fraction: Option<f32>, message: impl Into<String>) -> Self {
        Self {
            stage,
            fraction,
            message: message.into(),
            preview: None,
        }
    }
}

/// Snapshot of the engine state, suitable for display in the UI.
#[derive(Debug, Clone)]
pub struct EngineStatus {
    pub loaded: bool,
    pub mode: Mode,
    pub model_dir: PathBuf,
    pub device: String,
    pub precision: String,
    pub load_time: Option<Duration>,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TranslationRequest {
    pub text: String,
    /// FLORES-200 code (`fra_Latn`) or `auto`.
    pub source_language: String,
    pub target_language: String,
}

impl TranslationRequest {
    pub fn new(
        text: impl Into<String>,
        source_language: impl Into<String>,
        target_language: impl Into<String>,
    ) -> Self {
        Self {
            text: text.into(),
            source_language: source_language.into(),
            target_language: target_language.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TranslationResult {
    pub text: String,
    pub source_language: String,
    pub target_language: String,
    pub detected: Option<Detection>,
    /// `true` if the source text was truncated to `max_length` tokens.
    pub truncated: bool,
    pub generated_tokens: usize,
    pub duration: Duration,
    pub mode: Mode,
    pub device: String,
}

// ---------------------------------------------------------------------------
// Memory helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryInfo {
    /// Available RAM, in MB.
    pub available_mb: Option<u64>,
    /// Free swap memory, in MB.
    pub swap_free_mb: Option<u64>,
    /// Total RAM, in MB.
    pub total_mb: Option<u64>,
}

impl MemoryInfo {
    pub fn available_with_swap_mb(&self) -> Option<u64> {
        match (self.available_mb, self.swap_free_mb) {
            (Some(available), Some(swap)) => Some(available + swap),
            (Some(available), None) => Some(available),
            (None, Some(swap)) => Some(swap),
            (None, None) => None,
        }
    }
}

/// Reads `/proc/meminfo` on Linux; returns `None` on other systems.
pub fn memory_info() -> MemoryInfo {
    #[cfg(target_os = "linux")]
    {
        let Ok(raw) = std::fs::read_to_string("/proc/meminfo") else {
            return MemoryInfo::default();
        };
        let mut info = MemoryInfo::default();
        for line in raw.lines() {
            let Some((key, rest)) = line.split_once(':') else {
                continue;
            };
            let value = rest
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<u64>().ok())
                .map(|kb| kb / 1024);
            match key {
                "MemAvailable" => info.available_mb = value,
                "MemTotal" => info.total_mb = value,
                "SwapFree" => info.swap_free_mb = value,
                _ => {}
            }
        }
        info
    }
    #[cfg(not(target_os = "linux"))]
    {
        MemoryInfo::default()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MemoryEstimate {
    /// Model weights once loaded, in MB.
    pub weights_mb: u64,
    /// Headroom for activations, the KV cache and the tokenizer.
    pub overhead_mb: u64,
    pub needed_mb: u64,
    pub info: MemoryInfo,
}

impl MemoryEstimate {
    pub fn fits(&self) -> bool {
        match self.info.available_with_swap_mb() {
            Some(available) => self.needed_mb <= available,
            None => true,
        }
    }
}

/// Estimates the memory needed to load a model.
pub fn estimate_memory(model_dir: &Path, precision: Precision) -> Result<MemoryEstimate> {
    let checkpoint = Checkpoint::discover(model_dir)?;
    let on_disk = checkpoint.on_disk_bytes();
    let config = crate::model::nllb::NllbConfig::from_dir(model_dir)?;
    let precision = match precision {
        Precision::Auto => {
            if config.d_model >= 2048 {
                Precision::F16
            } else {
                Precision::F32
            }
        }
        other => other,
    };
    let factor = match precision {
        Precision::F16 => 0.5,
        _ => 1.0,
    };
    let weights_mb = (on_disk as f64 * factor / (1024.0 * 1024.0)).ceil() as u64;
    let overhead_mb = (weights_mb / 10).max(768);
    let needed_mb = weights_mb + overhead_mb;
    Ok(MemoryEstimate {
        weights_mb,
        overhead_mb,
        needed_mb,
        info: memory_info(),
    })
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

struct Loaded {
    model: Nllb,
    tokenizer: NllbTokenizer,
    device: Device,
    dtype: DType,
    load_time: Duration,
}

pub struct Engine {
    config: EngineConfig,
    loaded: Option<Loaded>,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            loaded: None,
        }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut EngineConfig {
        &mut self.config
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded.is_some()
    }

    /// Loads the model if needed (no-op when already loaded).
    pub fn ensure_loaded(
        &mut self,
        progress: &mut dyn FnMut(Progress),
        cancel: &AtomicBool,
    ) -> Result<()> {
        if self.loaded.is_some() {
            return Ok(());
        }
        self.load(progress, cancel)
    }

    pub fn load(&mut self, progress: &mut dyn FnMut(Progress), cancel: &AtomicBool) -> Result<()> {
        let started = Instant::now();
        self.loaded = None;

        progress(Progress::new(
            ProgressStage::Preparing,
            Some(0.02),
            "Reading configuration",
        ));
        if !self.config.model_dir.is_dir() {
            return Err(Error::ModelNotFound(self.config.model_dir.clone()));
        }
        let nllb_config = crate::model::nllb::NllbConfig::from_dir(&self.config.model_dir)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }

        progress(Progress::new(
            ProgressStage::Preparing,
            Some(0.05),
            "Selecting device",
        ));
        let device = resolve_device(self.config.device)?;
        let dtype = resolve_dtype(self.config.precision, &device, nllb_config.d_model);

        let estimate = estimate_memory(&self.config.model_dir, self.config.precision)?;
        if !estimate.fits() {
            return Err(Error::OutOfMemory {
                needed_mb: estimate.needed_mb,
                available_mb: estimate
                    .info
                    .available_with_swap_mb()
                    .unwrap_or(estimate.needed_mb),
            });
        }

        progress(Progress::new(
            ProgressStage::LoadingModel,
            Some(0.08),
            format!(
                "Loading model {} on {}",
                self.config.mode.model_label(),
                device_label(&device)
            ),
        ));

        let total_steps = (nllb_config.encoder_layers + nllb_config.decoder_layers + 2).max(1);
        let mut last_report = Instant::now();
        let mut report = |step: usize| {
            // Avoid flooding the UI with messages.
            if step == total_steps || last_report.elapsed() >= Duration::from_millis(120) {
                let fraction = 0.08 + 0.80 * (step as f32 / total_steps as f32);
                progress(Progress::new(
                    ProgressStage::LoadingModel,
                    Some(fraction.min(0.88)),
                    format!("Loading weights ({step}/{total_steps})"),
                ));
                last_report = Instant::now();
            }
        };

        let model =
            Nllb::load_with_progress(&self.config.model_dir, dtype, &device, cancel, &mut report)?;

        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }

        progress(Progress::new(
            ProgressStage::LoadingModel,
            Some(0.92),
            "Loading tokenizer",
        ));
        let tokenizer = NllbTokenizer::from_dir(&self.config.model_dir)?;

        progress(Progress::new(
            ProgressStage::Finishing,
            Some(1.0),
            "Model ready",
        ));

        self.loaded = Some(Loaded {
            model,
            tokenizer,
            device,
            dtype,
            load_time: started.elapsed(),
        });
        Ok(())
    }

    /// Immediately frees the memory used by the weights.
    pub fn unload(&mut self) {
        self.loaded = None;
    }

    pub fn status(&self) -> EngineStatus {
        match &self.loaded {
            Some(loaded) => EngineStatus {
                loaded: true,
                mode: self.config.mode,
                model_dir: self.config.model_dir.clone(),
                device: device_label(&loaded.device),
                precision: dtype_label(loaded.dtype),
                load_time: Some(loaded.load_time),
            },
            None => EngineStatus {
                loaded: false,
                mode: self.config.mode,
                model_dir: self.config.model_dir.clone(),
                device: resolve_device(self.config.device)
                    .map(|device| device_label(&device))
                    .unwrap_or_else(|_| "unavailable".to_string()),
                precision: match self.config.precision {
                    Precision::Auto => "automatic",
                    Precision::F32 => "f32",
                    Precision::F16 => "f16",
                }
                .to_string(),
                load_time: None,
            },
        }
    }

    pub fn translate(
        &mut self,
        request: &TranslationRequest,
        progress: &mut dyn FnMut(Progress),
        cancel: &AtomicBool,
    ) -> Result<TranslationResult> {
        let started = Instant::now();
        let text = request.text.trim();
        if text.is_empty() {
            return Err(Error::EmptyText);
        }
        if request.target_language == "auto" {
            return Err(Error::AutoTarget);
        }

        self.ensure_loaded(progress, cancel)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }

        let loaded = self
            .loaded
            .as_ref()
            .ok_or_else(|| Error::Other("the model is not loaded".to_string()))?;

        let (source_language, detected) = if request.source_language == "auto" {
            match detect(text) {
                Some(detection) => (detection.code.to_string(), Some(detection)),
                None => ("eng_Latn".to_string(), None),
            }
        } else {
            (request.source_language.clone(), None)
        };

        progress(Progress::new(
            ProgressStage::Encoding,
            None,
            format!("Encoding text ({source_language})"),
        ));
        let (source_ids, truncated) =
            loaded
                .tokenizer
                .encode_source(text, &source_language, self.config.max_length)?;

        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }

        progress(Progress::new(
            ProgressStage::Decoding,
            Some(0.0),
            "Generating translation",
        ));
        let options = GenerateOptions {
            max_new_tokens: self.config.max_new_tokens.max(1),
            num_beams: self.config.num_beams.max(1),
        };
        let tokens = loaded.model.generate(
            &loaded.tokenizer,
            &source_ids,
            &request.target_language,
            options,
            &mut |generation| {
                let fraction = if generation.max == 0 {
                    None
                } else {
                    Some(generation.generated as f32 / generation.max as f32)
                };
                let mut update = Progress::new(
                    ProgressStage::Decoding,
                    fraction,
                    format!(
                        "Translating… {}/{} tokens",
                        generation.generated, generation.max
                    ),
                );
                update.preview = Some(generation.preview);
                progress(update);
            },
            cancel,
        )?;

        progress(Progress::new(
            ProgressStage::Finishing,
            Some(1.0),
            "Finishing translation",
        ));
        let translated = loaded.tokenizer.decode(&tokens)?;

        Ok(TranslationResult {
            text: translated,
            source_language,
            target_language: request.target_language.clone(),
            detected,
            truncated,
            generated_tokens: tokens.len(),
            duration: started.elapsed(),
            mode: self.config.mode,
            device: device_label(&loaded.device),
        })
    }
}

// ---------------------------------------------------------------------------
// Device / dtype resolution
// ---------------------------------------------------------------------------

fn resolve_device(choice: DeviceChoice) -> Result<Device> {
    match choice {
        DeviceChoice::Cpu => Ok(Device::Cpu),
        DeviceChoice::Cuda(index) => new_cuda(index),
        DeviceChoice::Metal(index) => new_metal(index),
        DeviceChoice::Auto => {
            #[cfg(feature = "cuda")]
            {
                if let Ok(device) = Device::new_cuda(0) {
                    return Ok(device);
                }
            }
            #[cfg(feature = "metal")]
            {
                if let Ok(device) = Device::new_metal(0) {
                    return Ok(device);
                }
            }
            Ok(Device::Cpu)
        }
    }
}

fn new_cuda(index: usize) -> Result<Device> {
    #[cfg(feature = "cuda")]
    {
        Device::new_cuda(index).map_err(|error| Error::Other(format!("CUDA unavailable: {error}")))
    }
    #[cfg(not(feature = "cuda"))]
    {
        let _ = index;
        Err(Error::Other(
            "this Fluent build does not support CUDA; \
             rebuild with --features cuda"
                .to_string(),
        ))
    }
}

fn new_metal(index: usize) -> Result<Device> {
    #[cfg(feature = "metal")]
    {
        Device::new_metal(index)
            .map_err(|error| Error::Other(format!("Metal unavailable: {error}")))
    }
    #[cfg(not(feature = "metal"))]
    {
        let _ = index;
        Err(Error::Other(
            "this Fluent build does not support Metal; \
             rebuild with --features metal"
                .to_string(),
        ))
    }
}

fn resolve_dtype(precision: Precision, device: &Device, d_model: usize) -> DType {
    match precision {
        Precision::F32 => DType::F32,
        Precision::F16 => DType::F16,
        Precision::Auto => {
            if device.is_cpu() && d_model < 2048 {
                DType::F32
            } else {
                DType::F16
            }
        }
    }
}

pub fn device_label(device: &Device) -> String {
    match device.location() {
        candle_core::DeviceLocation::Cpu => "CPU".to_string(),
        candle_core::DeviceLocation::Cuda { gpu_id } => format!("CUDA #{gpu_id}"),
        candle_core::DeviceLocation::Metal { gpu_id } => format!("Metal #{gpu_id}"),
    }
}

fn dtype_label(dtype: DType) -> String {
    match dtype {
        DType::F16 => "f16",
        DType::BF16 => "bf16",
        DType::F32 => "f32",
        DType::F64 => "f64",
        other => return format!("{other:?}").to_lowercase(),
    }
    .to_string()
}
