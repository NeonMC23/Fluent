//! Product-level translation benchmark for Fluent's NLLB and MADLAD backends.
//!
//! This runs the same `Engine` path as the GUI, loads one model per process,
//! warms up once, and records per-case latency, first-token latency, token
//! counts, peak RSS and environment metadata as JSON.
//!
//! Example:
//! ```text
//! cargo run --release -p fluent-core --example eval -- \
//!   --mode madlad \
//!   --model "$HOME/.local/share/fluent/models/madlad400-3b-mt" \
//!   --corpus benchmarks/flores-5x10.json \
//!   --limit 128 --reps 1 --out /tmp/madlad.json
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use fluent_core::{
    DeviceChoice, Engine, EngineConfig, Mode, OutputLimit, Precision, ProgressStage,
    TranslationRequest,
};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
struct Corpus {
    name: String,
    #[serde(default)]
    license: String,
    #[serde(default)]
    source: String,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    #[serde(default)]
    category: String,
    source_lang: String,
    target_lang: String,
    source: String,
    #[serde(default)]
    reference: String,
}

struct Args {
    mode: Mode,
    model: PathBuf,
    corpus: PathBuf,
    out: PathBuf,
    reps: usize,
    warmup: bool,
    limit: usize,
    beams: usize,
    max_length: usize,
    device: DeviceChoice,
    precision: Precision,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        mode: Mode::Fast,
        model: PathBuf::new(),
        corpus: PathBuf::from("benchmarks/flores-5x10.json"),
        out: PathBuf::from("/tmp/fluent-eval.json"),
        reps: 1,
        warmup: true,
        limit: 128,
        beams: 4,
        max_length: 512,
        device: DeviceChoice::Cpu,
        precision: Precision::Auto,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        let mut value = || iter.next().ok_or_else(|| format!("{arg} requires a value"));
        match arg.as_str() {
            "--mode" => args.mode = value()?.parse().map_err(|e| format!("{e}"))?,
            "--model" => args.model = PathBuf::from(value()?),
            "--corpus" => args.corpus = PathBuf::from(value()?),
            "--out" => args.out = PathBuf::from(value()?),
            "--reps" => args.reps = value()?.parse().map_err(|e| format!("{e}"))?,
            "--warmup" => args.warmup = value()?.parse::<u8>().map_err(|e| format!("{e}"))? != 0,
            "--limit" => args.limit = value()?.parse().map_err(|e| format!("{e}"))?,
            "--beams" => args.beams = value()?.parse().map_err(|e| format!("{e}"))?,
            "--max-length" => args.max_length = value()?.parse().map_err(|e| format!("{e}"))?,
            "--device" => {
                args.device = match value()?.as_str() {
                    "cpu" => DeviceChoice::Cpu,
                    "cuda" => DeviceChoice::Cuda(0),
                    "metal" => DeviceChoice::Metal(0),
                    _ => DeviceChoice::Auto,
                }
            }
            "--precision" => {
                args.precision = match value()?.as_str() {
                    "f32" => Precision::F32,
                    "f16" => Precision::F16,
                    _ => Precision::Auto,
                }
            }
            "-h" | "--help" => {
                println!(
                    "usage: eval --mode fast|accurate|madlad --model DIR [--corpus FILE] \
                     [--out FILE] [--reps N] [--limit N] [--beams N] [--max-length N]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if args.model.as_os_str().is_empty() {
        return Err("--model is required".to_string());
    }
    Ok(args)
}

fn read_meminfo() -> (Option<u64>, Option<u64>, Option<u64>) {
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
        return (None, None, None);
    };
    let mut total = None;
    let mut available = None;
    let mut swap = None;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap_or("");
        let value: Option<u64> = parts.next().and_then(|v| v.parse().ok());
        match key {
            "MemTotal:" => total = value.map(|v| v / 1024),
            "MemAvailable:" => available = value.map(|v| v / 1024),
            "SwapFree:" => swap = value.map(|v| v / 1024),
            _ => {}
        }
    }
    (total, available, swap)
}

fn rss_mb(key: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = text.lines().find(|line| line.starts_with(key))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("model name"))
                .and_then(|line| line.split_once(':'))
                .map(|(_, value)| value.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn os_label() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("PRETTY_NAME="))
                .map(|line| {
                    line.trim_start_matches("PRETTY_NAME=")
                        .trim_matches('"')
                        .to_string()
                })
        })
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}

fn git_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn file_info(dir: &Path) -> Value {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if name.ends_with(".bin")
                || name.ends_with(".safetensors")
                || name.ends_with(".gguf")
                || name == "model.safetensors.index.json"
                || name == "pytorch_model.bin.index.json"
            {
                files.push(json!({"name": name, "bytes": size}));
            }
        }
    }
    files.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Value::Array(files)
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        Some((values[mid - 1] + values[mid]) / 2.0)
    } else {
        Some(values[mid])
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let corpus: Corpus = serde_json::from_str(&std::fs::read_to_string(&args.corpus)?)?;
    let (total_ram, available_ram, swap_free) = read_meminfo();
    let commit = git_commit();

    let mut config = EngineConfig::new(args.mode, &args.model);
    config.device = args.device;
    config.precision = args.precision;
    config.max_length = args.max_length;
    config.output_limit = OutputLimit::Tokens(args.limit);
    // MADLAD has no beam search; record the effective greedy width instead of
    // copying an inapplicable CLI value into the report.
    let effective_beams = if args.mode == Mode::Madlad {
        1
    } else {
        args.beams
    };
    config.num_beams = effective_beams;

    println!(
        "mode={} model={} corpus={} cases={} limit={} beams={}",
        args.mode.as_str(),
        args.model.display(),
        corpus.name,
        corpus.cases.len(),
        args.limit,
        effective_beams
    );
    let load_started = Instant::now();
    let mut engine = Engine::new(config);
    let cancel = AtomicBool::new(false);
    engine.load(&mut |_| {}, &cancel)?;
    let load_ms = load_started.elapsed().as_secs_f64() * 1000.0;
    let status = engine.status();
    let rss_after_load = rss_mb("VmRSS:");
    println!(
        "loaded in {:.1} ms · rss {:?} MB · {} · {}",
        load_ms, rss_after_load, status.device, status.precision
    );

    let mut results = Vec::new();
    for (index, case) in corpus.cases.iter().enumerate() {
        if !args.mode.supports_language(&case.source_lang)
            || !args.mode.supports_language(&case.target_lang)
        {
            results.push(json!({
                "id": case.id,
                "category": case.category,
                "source_lang": case.source_lang,
                "target_lang": case.target_lang,
                "status": "unsupported",
            }));
            continue;
        }

        let request = TranslationRequest::new(&case.source, &case.source_lang, &case.target_lang);
        if args.warmup && index == 0 {
            let _ = engine.translate(&request, &mut |_| {}, &cancel)?;
        }

        let mut samples = Vec::new();
        let mut output = String::new();
        for _ in 0..args.reps.max(1) {
            let started = Instant::now();
            let mut ttft = None;
            let result = engine.translate(
                &request,
                &mut |progress| {
                    if progress.stage == ProgressStage::Decoding
                        && progress.fraction.unwrap_or(0.0) > 0.0
                        && ttft.is_none()
                    {
                        ttft = Some(started.elapsed());
                    }
                },
                &cancel,
            )?;
            let elapsed = started.elapsed();
            output = result.text.clone();
            samples.push(json!({
                "duration_ms": elapsed.as_secs_f64() * 1000.0,
                "reported_ms": result.duration.as_secs_f64() * 1000.0,
                "ttft_ms": ttft.map(|d: Duration| d.as_secs_f64() * 1000.0),
                "source_tokens": result.source_tokens,
                "generated_tokens": result.generated_tokens,
                "characters": result.text.chars().count(),
                "truncated_source": result.truncated,
                "text": result.text,
            }));
        }

        let mut durations: Vec<f64> = samples
            .iter()
            .filter_map(|s| s["duration_ms"].as_f64())
            .collect();
        let mut ttfts: Vec<f64> = samples
            .iter()
            .filter_map(|s| s["ttft_ms"].as_f64())
            .collect();
        let generated = samples
            .first()
            .and_then(|s| s["generated_tokens"].as_u64())
            .unwrap_or(0);
        let source_tokens = samples
            .first()
            .and_then(|s| s["source_tokens"].as_u64())
            .unwrap_or(0);
        let median_ms = median(&mut durations);
        results.push(json!({
            "id": case.id,
            "category": case.category,
            "source_lang": case.source_lang,
            "target_lang": case.target_lang,
            "status": "ok",
            "source": case.source,
            "reference": case.reference,
            "output": output,
            "source_tokens": source_tokens,
            "generated_tokens": generated,
            "output_chars": output.chars().count(),
            "median_ms": median_ms,
            "median_ttft_ms": median(&mut ttfts),
            "tokens_per_second": median_ms.map(|ms| generated as f64 / (ms / 1000.0)),
            "samples": samples,
        }));
        println!(
            "[{}/{}] {} {}→{} · {} tok · {:.0} ms",
            index + 1,
            corpus.cases.len(),
            case.id,
            case.source_lang,
            case.target_lang,
            generated,
            median_ms.unwrap_or(0.0)
        );
    }

    let peak_rss = rss_mb("VmHWM:");
    let report = json!({
        "schema": "fluent-eval-v1",
        "git_commit": commit,
        "generated_at_utc": chrono_like_now(),
        "corpus": {
            "name": corpus.name,
            "license": corpus.license,
            "source": corpus.source,
            "cases": corpus.cases.len(),
        },
        "environment": {
            "os": os_label(),
            "arch": std::env::consts::ARCH,
            "cpu": cpu_model(),
            "threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
            "ram_total_mb": total_ram,
            "ram_available_mb": available_ram,
            "swap_free_mb": swap_free,
            "gpu": "not used (CPU backend)",
        },
        "model": {
            "mode": args.mode.as_str(),
            "label": args.mode.model_label(),
            "dir": args.model.display().to_string(),
            "files": file_info(&args.model),
            "device": status.device,
            "precision": status.precision,
        },
        "settings": {
            "output_limit": args.limit,
            "num_beams": effective_beams,
            "greedy_backend": args.mode == Mode::Madlad,
            "max_source_tokens": args.max_length,
            "reps": args.reps,
            "warmup": args.warmup,
        },
        "load_ms": load_ms,
        "rss_after_load_mb": rss_after_load,
        "peak_rss_mb": peak_rss,
        "results": results,
    });
    std::fs::write(&args.out, serde_json::to_string_pretty(&report)?)?;
    println!("wrote {}", args.out.display());
    Ok(())
}

fn chrono_like_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("unix:{}", now.as_secs())
}
