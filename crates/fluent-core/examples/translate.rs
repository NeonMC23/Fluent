//! Command-line example: translates text with a local NLLB model.
//!
//! ```text
//! cargo run --release -p fluent-core --example translate -- \
//!     "Hello, how are you?" eng_Latn fra_Latn models/nllb-200-1.3b
//! ```

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use fluent_core::{
    DeviceChoice, Engine, EngineConfig, Mode, Precision, ProgressStage, TranslationRequest,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!(
            "usage: translate <text> <source_language|auto> <target_language> <model_dir> \
             [cpu|cuda|metal|auto] [auto|f32|f16]"
        );
        std::process::exit(2);
    }

    let text = args[0].clone();
    let source = args[1].clone();
    let target = args[2].clone();
    let model_dir = PathBuf::from(&args[3]);

    let device = match args.get(4).map(String::as_str) {
        Some("cpu") => DeviceChoice::Cpu,
        Some("cuda") => DeviceChoice::Cuda(0),
        Some("metal") => DeviceChoice::Metal(0),
        _ => DeviceChoice::Auto,
    };
    let precision = match args.get(5).map(String::as_str) {
        Some("f32") => Precision::F32,
        Some("f16") => Precision::F16,
        _ => Precision::Auto,
    };

    let mut config = EngineConfig::new(Mode::Fast, model_dir);
    config.device = device;
    config.precision = precision;
    config.num_beams = std::env::var("FLUENT_BEAMS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(4);
    if let Ok(value) = std::env::var("FLUENT_MAX_NEW") {
        if let Ok(value) = value.parse() {
            config.max_new_tokens = value;
        }
    }

    println!("model       : {}", config.model_dir.display());
    println!("device      : {:?}", config.device);
    println!("precision   : {:?}", config.precision);
    println!("source      : {source}");
    println!("target      : {target}");
    println!("text        : {text}");
    println!();

    let started = Instant::now();
    let mut engine = Engine::new(config);
    let cancel = AtomicBool::new(false);

    engine.load(
        &mut |progress| {
            eprintln!(
                "[{:>6.1}%] {} — {}",
                progress.fraction.unwrap_or(0.0) * 100.0,
                progress.stage.label(),
                progress.message
            );
        },
        &cancel,
    )?;
    eprintln!("loaded in {:.2?}\n", started.elapsed());

    let request = TranslationRequest::new(text, source, target);
    let mut last_stage = None;
    let mut last_bucket = 0i32;
    let result = engine.translate(
        &request,
        &mut |progress| {
            let bucket = (progress.fraction.unwrap_or(0.0) * 10.0).floor() as i32;
            if last_stage != Some(progress.stage) || (bucket != last_bucket && bucket > 0) {
                eprintln!(
                    "[{:>6.1}%] {} — {}",
                    progress.fraction.unwrap_or(0.0) * 100.0,
                    progress.stage.label(),
                    progress.message
                );
                last_stage = Some(progress.stage);
                last_bucket = if bucket > 0 { bucket } else { last_bucket };
            }
        },
        &cancel,
    )?;

    println!("--- translation ---");
    println!("{}", result.text);
    println!("------------------");
    println!("detected language : {:?}", result.detected);
    println!("generated tokens  : {}", result.generated_tokens);
    println!("duration          : {:.2?}", result.duration);
    println!("truncated         : {}", result.truncated);
    if result.text.trim().is_empty() {
        eprintln!("warning: the translation is empty (check weights, positions or generation)");
        std::process::exit(1);
    }
    let _ = ProgressStage::Encoding;
    Ok(())
}
