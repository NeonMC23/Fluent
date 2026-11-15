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
use crate::model::madlad::{Madlad, MadladTokenizer};
use crate::model::nllb::{GenerateOptions, Nllb};
use crate::model::registry::{model_spec, Architecture};
use crate::model::Checkpoint;
use crate::tokenizer::NllbTokenizer;
use crate::{Error, Mode, OutputLimit, Result};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Numerical precision used for the weights.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Precision {
    /// f32 for the 1.3B model on CPU, f16 for the 3.3B model or a GPU.
    #[default]
    Auto,
    F32,
    F16,
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DeviceChoice {
    /// First available GPU, otherwise CPU.
    #[default]
    Auto,
    Cpu,
    Cuda(usize),
    Metal(usize),
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
    /// Generated-token policy. `Auto` preserves the input-derived budget.
    pub output_limit: OutputLimit,
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
            output_limit: OutputLimit::Auto,
            num_beams: 4,
        }
    }

    /// Builds the configuration from `config.json`.
    pub fn from_project(project: &ProjectConfig, mode: Mode) -> Self {
        let mut config = Self::new(mode, project.model_path(mode).to_path_buf());
        config.max_length = project.max_length;
        config.output_limit = project.output_limit;
        config.num_beams = project.num_beams;
        config
    }
}

// Keep the historical `fluent_core::engine::default_generation_budget` path.
pub use crate::default_generation_budget;

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
    /// Number of encoded source tokens, including any target-language prefix
    /// and EOS token used by the backend.
    pub source_tokens: usize,
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
///
/// This is the conservative form: it ignores the CPU Q8 weight path, so it
/// never underestimates on a GPU or an unusual build. Use
/// [`estimate_memory_for_choice`] when the target device is known.
pub fn estimate_memory(model_dir: &Path, precision: Precision) -> Result<MemoryEstimate> {
    estimate_memory_impl(model_dir, precision, false)
}

/// Like [`estimate_memory`], but accounts for the CPU Q8 weight path.
///
/// Quantized weights are about `9/32` of the f32 footprint, which lets the
/// 1.3B model run on machines that the unquantized estimate would reject.
pub fn estimate_memory_for_choice(
    model_dir: &Path,
    precision: Precision,
    device: DeviceChoice,
) -> Result<MemoryEstimate> {
    let config = crate::model::nllb::NllbConfig::from_dir(model_dir)?;
    let on_cpu = match device {
        DeviceChoice::Cpu => true,
        DeviceChoice::Auto => !has_accelerator(),
        DeviceChoice::Cuda(_) | DeviceChoice::Metal(_) => false,
    };
    // CPU auto mode uses the Q8 path for every model size. On this CPU the
    // Q8 3.3B path is substantially faster than f16 and uses less memory.
    let precision = match (precision, on_cpu) {
        (Precision::Auto, true) => Precision::F32,
        _ => resolve_precision(precision, config.d_model),
    };
    estimate_memory_impl(model_dir, precision, on_cpu && precision == Precision::F32)
}

/// Estimates memory for the selected model family.
pub fn estimate_memory_for_mode(
    mode: Mode,
    model_dir: &Path,
    precision: Precision,
    device: DeviceChoice,
) -> Result<MemoryEstimate> {
    match model_spec(mode).architecture {
        Architecture::Nllb200 => estimate_memory_for_choice(model_dir, precision, device),
        Architecture::T5 => estimate_madlad_memory(model_dir),
    }
}

fn estimate_madlad_memory(model_dir: &Path) -> Result<MemoryEstimate> {
    let gguf =
        crate::model::madlad::find_gguf(model_dir).ok_or_else(|| Error::MissingModelFile {
            dir: model_dir.to_path_buf(),
            hint: "missing model-q4k.gguf (see docs/MADLAD.md)".to_string(),
        })?;
    let on_disk = std::fs::metadata(&gguf)?.len();
    let weights_mb = (on_disk as f64 / (1024.0 * 1024.0)).ceil() as u64;
    // Quantized weights are stored close to their on-disk size; activations,
    // the tokenizer and the growing KV cache need a separate margin.
    let overhead_mb = (weights_mb / 3).max(768);
    Ok(MemoryEstimate {
        weights_mb,
        overhead_mb,
        needed_mb: weights_mb + overhead_mb,
        info: memory_info(),
    })
}

fn resolve_precision(precision: Precision, d_model: usize) -> Precision {
    match precision {
        Precision::Auto if d_model >= 2048 => Precision::F16,
        Precision::Auto => Precision::F32,
        other => other,
    }
}

fn has_accelerator() -> bool {
    cfg!(any(feature = "cuda", feature = "metal"))
}

/// Fraction of the on-disk f32 size used by the in-memory weights.
fn weight_factor(precision: Precision, quantized: bool) -> f64 {
    if quantized {
        // One i8 per value plus one f32 scale per 32-value block.
        9.0 / 32.0
    } else if precision == Precision::F16 {
        0.5
    } else {
        1.0
    }
}

fn estimate_memory_impl(
    model_dir: &Path,
    precision: Precision,
    quantized: bool,
) -> Result<MemoryEstimate> {
    let checkpoint = Checkpoint::discover(model_dir)?;
    let on_disk = checkpoint.on_disk_bytes();
    let config = crate::model::nllb::NllbConfig::from_dir(model_dir)?;
    let precision = resolve_precision(precision, config.d_model);
    let factor = weight_factor(precision, quantized);
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

enum LoadedBackend {
    Nllb {
        model: Box<Nllb>,
        tokenizer: Box<NllbTokenizer>,
    },
    Madlad {
        model: Box<Madlad>,
        tokenizer: Box<MadladTokenizer>,
    },
}

struct Loaded {
    backend: LoadedBackend,
    device: Device,
    precision_label: String,
    load_time: Duration,
}

impl Loaded {
    fn max_source_tokens(&self) -> Option<usize> {
        match &self.backend {
            LoadedBackend::Nllb { .. } => None,
            LoadedBackend::Madlad { model, .. } => Some(model.max_source_tokens()),
        }
    }

    fn encode_source(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
        max_tokens: usize,
    ) -> Result<(Vec<u32>, bool)> {
        match &self.backend {
            LoadedBackend::Nllb { tokenizer, .. } => {
                tokenizer.encode_source(text, source_language, max_tokens)
            }
            LoadedBackend::Madlad { tokenizer, .. } => {
                tokenizer.encode_source(text, target_language, max_tokens)
            }
        }
    }

    fn generate(
        &mut self,
        source_ids: &[u32],
        target_language: &str,
        output_limit: OutputLimit,
        num_beams: usize,
        progress: &mut dyn FnMut(crate::model::nllb::GenerationProgress),
        cancel: &AtomicBool,
    ) -> Result<Vec<u32>> {
        match &mut self.backend {
            LoadedBackend::Nllb { model, tokenizer } => model.generate(
                tokenizer,
                source_ids,
                target_language,
                GenerateOptions {
                    output_limit,
                    num_beams: num_beams.max(1),
                    early_stopping: Default::default(),
                },
                progress,
                cancel,
            ),
            LoadedBackend::Madlad { model, tokenizer } => {
                model.generate(tokenizer, source_ids, output_limit, progress, cancel)
            }
        }
    }

    fn decode(&self, ids: &[u32]) -> Result<String> {
        match &self.backend {
            LoadedBackend::Nllb { tokenizer, .. } => tokenizer.decode(ids),
            LoadedBackend::Madlad { tokenizer, .. } => tokenizer.decode(ids),
        }
    }
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
        self.loaded = None;
        match model_spec(self.config.mode).architecture {
            Architecture::Nllb200 => self.load_nllb(progress, cancel),
            Architecture::T5 => self.load_madlad(progress, cancel),
        }
    }

    fn load_nllb(&mut self, progress: &mut dyn FnMut(Progress), cancel: &AtomicBool) -> Result<()> {
        let started = Instant::now();

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

        let estimate = estimate_memory_impl(
            &self.config.model_dir,
            self.config.precision,
            crate::model::quant::quantization_enabled(&device, dtype),
        )?;
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
            backend: LoadedBackend::Nllb {
                model: Box::new(model),
                tokenizer: Box::new(tokenizer),
            },
            device,
            precision_label: dtype_label(dtype),
            load_time: started.elapsed(),
        });
        Ok(())
    }

    fn load_madlad(
        &mut self,
        progress: &mut dyn FnMut(Progress),
        cancel: &AtomicBool,
    ) -> Result<()> {
        let started = Instant::now();
        if !self.config.model_dir.is_dir() {
            return Err(Error::ModelNotFound(self.config.model_dir.clone()));
        }
        // MADLAD's validated path is CPU q4_k GGUF. Do not silently fall back
        // to another device or advertise acceleration that is not implemented.
        match self.config.device {
            DeviceChoice::Auto | DeviceChoice::Cpu => {}
            DeviceChoice::Cuda(_) | DeviceChoice::Metal(_) => {
                return Err(Error::Other(
                    "MADLAD currently supports CPU execution only".to_string(),
                ));
            }
        }

        progress(Progress::new(
            ProgressStage::Preparing,
            Some(0.05),
            "Checking MADLAD files",
        ));
        let estimate = estimate_memory_for_mode(
            self.config.mode,
            &self.config.model_dir,
            self.config.precision,
            DeviceChoice::Cpu,
        )?;
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
            Some(0.12),
            "Loading MADLAD q4_k weights",
        ));
        let mut report = |step: usize| {
            let fraction = match step {
                1 => 0.35,
                2 => 0.55,
                3 => 0.82,
                _ => 0.82,
            };
            progress(Progress::new(
                ProgressStage::LoadingModel,
                Some(fraction),
                match step {
                    1 => "Reading GGUF tensor table",
                    2 => "Validating T5 configuration",
                    _ => "Building T5 decoder",
                },
            ));
        };
        let model = Madlad::load(&self.config.model_dir, cancel, &mut report)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }

        progress(Progress::new(
            ProgressStage::LoadingModel,
            Some(0.92),
            "Loading MADLAD tokenizer",
        ));
        let tokenizer = MadladTokenizer::from_dir(&self.config.model_dir)?;

        progress(Progress::new(
            ProgressStage::Finishing,
            Some(1.0),
            "MADLAD ready",
        ));
        self.loaded = Some(Loaded {
            backend: LoadedBackend::Madlad {
                model: Box::new(model),
                tokenizer: Box::new(tokenizer),
            },
            device: Device::Cpu,
            precision_label: "Q4_K · GGUF".to_string(),
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
                precision: loaded.precision_label.clone(),
                load_time: Some(loaded.load_time),
            },
            None => EngineStatus {
                loaded: false,
                mode: self.config.mode,
                model_dir: self.config.model_dir.clone(),
                device: if self.config.mode == Mode::Madlad {
                    "Processor (CPU)".to_string()
                } else {
                    resolve_device(self.config.device)
                        .map(|device| device_label(&device))
                        .unwrap_or_else(|_| "unavailable".to_string())
                },
                precision: if self.config.mode == Mode::Madlad {
                    "Q4_K · GGUF".to_string()
                } else {
                    match self.config.precision {
                        Precision::Auto => "automatic",
                        Precision::F32 => "f32",
                        Precision::F16 => "f16",
                    }
                    .to_string()
                },
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

        let output_limit = self.config.output_limit;
        let num_beams = self.config.num_beams.max(1);
        let max_length = self.config.max_length;
        let loaded = self
            .loaded
            .as_mut()
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
        let max_source = loaded
            .max_source_tokens()
            .map(|limit| max_length.min(limit))
            .unwrap_or(max_length);
        let (source_ids, truncated) =
            loaded.encode_source(text, &source_language, &request.target_language, max_source)?;

        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }

        progress(Progress::new(
            ProgressStage::Decoding,
            Some(0.0),
            "Generating translation",
        ));
        let tokens = loaded.generate(
            &source_ids,
            &request.target_language,
            output_limit,
            num_beams,
            &mut |generation| {
                let fraction = if generation.max == 0 {
                    None
                } else {
                    Some(generation.generated as f32 / generation.max as f32)
                };
                let detail = if generation.max == 0 {
                    format!("Translating… {} tokens (no limit)", generation.generated)
                } else {
                    format!(
                        "Translating… {}/{} tokens",
                        generation.generated, generation.max
                    )
                };
                let mut update = Progress::new(ProgressStage::Decoding, fraction, detail);
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
        let translated = loaded.decode(&tokens)?;

        Ok(TranslationResult {
            text: translated,
            source_language,
            target_language: request.target_language.clone(),
            detected,
            truncated,
            source_tokens: source_ids.len(),
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

fn resolve_dtype(precision: Precision, device: &Device, _d_model: usize) -> DType {
    match precision {
        Precision::F32 => DType::F32,
        Precision::F16 => DType::F16,
        Precision::Auto => {
            // CPU inference uses f32 blocks with Q8 quantization for both
            // NLLB sizes. F16 CPU matmuls are much slower here and the Q8
            // path also has a smaller resident footprint. GPU auto stays f16.
            if device.is_cpu() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_model_directory_is_reported_without_starting_a_load() {
        let dir = std::env::temp_dir().join(format!(
            "fluent-missing-model-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut engine = Engine::new(EngineConfig::new(Mode::Fast, &dir));
        let cancel = AtomicBool::new(false);
        let error = engine
            .load(&mut |_| {}, &cancel)
            .expect_err("missing model must fail");
        match error {
            Error::ModelNotFound(reported) => assert_eq!(reported, dir),
            other => panic!("unexpected error: {other}"),
        }
        assert!(!engine.is_loaded());
    }

    #[test]
    fn available_memory_adds_free_swap() {
        let info = MemoryInfo {
            available_mb: Some(8_000),
            swap_free_mb: Some(2_000),
            total_mb: Some(16_000),
        };
        assert_eq!(info.available_with_swap_mb(), Some(10_000));
    }

    #[test]
    fn available_memory_tolerates_partial_information() {
        let without_swap = MemoryInfo {
            available_mb: Some(4_000),
            swap_free_mb: None,
            total_mb: None,
        };
        assert_eq!(without_swap.available_with_swap_mb(), Some(4_000));

        let unknown = MemoryInfo::default();
        assert_eq!(unknown.available_with_swap_mb(), None);
    }

    #[test]
    fn memory_estimate_compares_against_available_memory() {
        let estimate = MemoryEstimate {
            weights_mb: 5_000,
            overhead_mb: 1_000,
            needed_mb: 6_000,
            info: MemoryInfo {
                available_mb: Some(7_000),
                swap_free_mb: Some(1_000),
                total_mb: Some(16_000),
            },
        };
        assert!(estimate.fits());

        let too_big = MemoryEstimate {
            needed_mb: 10_000,
            ..estimate
        };
        assert!(!too_big.fits());
    }

    #[test]
    fn generation_budget_scales_with_source_length() {
        assert_eq!(default_generation_budget(0), 16);
        assert_eq!(default_generation_budget(1), 18);
        assert_eq!(default_generation_budget(12), 40);
        assert_eq!(default_generation_budget(100), 216);
        assert_eq!(default_generation_budget(400), 512);
        assert_eq!(default_generation_budget(usize::MAX), 512);
    }

    #[test]
    fn quantized_weights_shrink_the_estimated_footprint() {
        assert!((weight_factor(Precision::F32, true) - 9.0 / 32.0).abs() < 1e-9);
        assert_eq!(weight_factor(Precision::F32, false), 1.0);
        assert_eq!(weight_factor(Precision::F16, false), 0.5);
        // Quantization wins over the requested precision on CPU f32.
        assert_eq!(weight_factor(Precision::F16, true), 9.0 / 32.0);
    }

    #[test]
    fn automatic_precision_follows_the_model_size() {
        assert_eq!(resolve_precision(Precision::Auto, 1024), Precision::F32);
        // The conservative public estimator keeps its size-based rule; the
        // device-aware estimator and actual CPU load select Q8 for both sizes.
        assert_eq!(resolve_precision(Precision::Auto, 2048), Precision::F16);
        assert_eq!(resolve_precision(Precision::F32, 4096), Precision::F32);
        assert_eq!(resolve_precision(Precision::F16, 1024), Precision::F16);
        // CPU auto uses the Q8 path at both model sizes.
        assert_eq!(
            resolve_dtype(Precision::Auto, &Device::Cpu, 1024),
            DType::F32
        );
        assert_eq!(
            resolve_dtype(Precision::Auto, &Device::Cpu, 2048),
            DType::F32
        );
    }

    #[test]
    fn source_and_generation_limits_are_independent() {
        let project = ProjectConfig {
            max_length: 64,
            output_limit: OutputLimit::Tokens(200),
            ..ProjectConfig::default()
        };
        let config = EngineConfig::from_project(&project, Mode::Fast);
        assert_eq!(config.max_length, 64);
        assert_eq!(config.output_limit, OutputLimit::Tokens(200));
    }
}
