//! Reproducible local-chat benchmark.
//!
//! ```text
//! cargo run --release -p fluent-core --example chat_eval -- \
//!   --model "$HOME/.local/share/fluent/models/qwen2.5-1.5b-instruct" \
//!   --id qwen2.5-1.5b-instruct --out /tmp/chat.json
//! ```

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use fluent_core::{ChatEngine, ChatMessage, ChatOptions};
use serde_json::{json, Value};

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

fn run_case(
    engine: &mut ChatEngine,
    id: &str,
    prompt: &str,
    messages: Vec<ChatMessage>,
    options: ChatOptions,
    cancel: &AtomicBool,
) -> Value {
    let started = Instant::now();
    let mut streamed = String::new();
    let mut deltas = 0usize;
    let result = engine.reply(
        &messages,
        options,
        &mut |delta| {
            streamed.push_str(delta);
            deltas += 1;
        },
        cancel,
    );
    let wall = started.elapsed();
    match result {
        Ok(reply) => {
            println!(
                "[{id}] {} tok · ttft {} · {:.2} tok/s · {}",
                reply.generated_tokens,
                reply
                    .first_token
                    .map(|value| format!("{value:.2?}"))
                    .unwrap_or_else(|| "—".to_string()),
                reply.generated_tokens as f64 / reply.duration.as_secs_f64().max(0.001),
                reply.text.replace('\n', " ")
            );
            json!({
                "id": id,
                "prompt": prompt,
                "status": "ok",
                "output": reply.text,
                "streamed_equal": streamed == reply.text,
                "deltas": deltas,
                "prompt_tokens": reply.prompt_tokens,
                "generated_tokens": reply.generated_tokens,
                "ttft_ms": reply.first_token.map(|d| d.as_secs_f64() * 1000.0),
                "duration_ms": wall.as_secs_f64() * 1000.0,
                "tokens_per_second": reply.generated_tokens as f64 / wall.as_secs_f64().max(0.001),
                "context_truncated": reply.context_truncated,
                "hit_token_limit": reply.hit_token_limit,
            })
        }
        Err(error) => {
            println!("[{id}] ERROR {error}");
            json!({"id": id, "prompt": prompt, "status": "error", "error": error.to_string()})
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = PathBuf::new();
    let mut id = "chat".to_string();
    let mut out = PathBuf::from("/tmp/fluent-chat-eval.json");
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--model" => model = PathBuf::from(iter.next().ok_or("--model needs a value")?),
            "--id" => id = iter.next().ok_or("--id needs a value")?,
            "--out" => out = PathBuf::from(iter.next().ok_or("--out needs a value")?),
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    if model.as_os_str().is_empty() {
        return Err("--model is required".into());
    }
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let mut engine = ChatEngine::load(&model, "Qwen2.5 chat", &cancel)?;
    let load = started.elapsed();
    println!("loaded {:.2?} · rss {:?} MB", load, rss_mb("VmRSS:"));

    let mut cases = Vec::new();
    let english = ChatMessage::user("What is the capital of France? Answer in one sentence.");
    cases.push(run_case(
        &mut engine,
        "en-factual",
        "What is the capital of France? Answer in one sentence.",
        vec![english.clone()],
        ChatOptions::default(),
        &cancel,
    ));
    cases.push(run_case(
        &mut engine,
        "en-explain",
        "Explain why the sky appears blue in two sentences.",
        vec![ChatMessage::user(
            "Explain why the sky appears blue in two sentences.",
        )],
        ChatOptions::default(),
        &cancel,
    ));
    cases.push(run_case(
        &mut engine,
        "fr-explain",
        "Explique en une phrase pourquoi le ciel est bleu.",
        vec![ChatMessage::user(
            "Explique en une phrase pourquoi le ciel est bleu.",
        )],
        ChatOptions::default(),
        &cancel,
    ));
    let tips = ChatMessage::user("Give me three practical tips for learning a new language.");
    cases.push(run_case(
        &mut engine,
        "en-long",
        "Give me three practical tips for learning a new language.",
        vec![tips.clone()],
        ChatOptions::default(),
        &cancel,
    ));
    // Follow-up using the previous answer.
    let previous = cases
        .last()
        .and_then(|value| value["output"].as_str())
        .unwrap_or_default()
        .to_string();
    cases.push(run_case(
        &mut engine,
        "en-followup",
        "Which of those tips is easiest to start today?",
        vec![
            tips,
            ChatMessage::assistant(previous),
            ChatMessage::user("Which of those tips is easiest to start today?"),
        ],
        ChatOptions::default(),
        &cancel,
    ));
    // Repeated identical request for stability.
    cases.push(run_case(
        &mut engine,
        "en-repeat",
        "What is the capital of France? Answer in one sentence.",
        vec![english],
        ChatOptions::default(),
        &cancel,
    ));
    // Small context budget.
    cases.push(run_case(
        &mut engine,
        "context-truncated",
        "Tell me a long story about a library.",
        vec![ChatMessage::user(
            "Tell me a long story about a library, with several characters and a mystery.",
        )],
        ChatOptions {
            max_context: 64,
            max_new_tokens: 32,
            ..Default::default()
        },
        &cancel,
    ));

    let report = json!({
        "schema": "fluent-chat-eval-v1",
        "model_id": id,
        "model_dir": model.display().to_string(),
        "environment": {
            "cpu": cpu_model(),
            "threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
            "gpu": "not used (CPU backend)",
        },
        "load_ms": load.as_secs_f64() * 1000.0,
        "rss_after_load_mb": rss_mb("VmRSS:"),
        "peak_rss_mb": rss_mb("VmHWM:"),
        "results": cases,
    });
    std::fs::write(&out, serde_json::to_string_pretty(&report)?)?;
    println!("wrote {}", out.display());
    Ok(())
}
