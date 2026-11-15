//! Real SMaLL-100 engine integration, gated by `FLUENT_SMALL100_MODEL`.
//!
//! ```text
//! FLUENT_SMALL100_MODEL="$HOME/.local/share/fluent/models/small100" \
//!     cargo test --release -p fluent-core --test small100 -- --nocapture
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use fluent_core::model::small100::{Small100Tokenizer, LANGUAGE_ID_BASE};
use fluent_core::{
    DeviceChoice, Engine, EngineConfig, Mode, OutputLimit, Precision, TranslationRequest,
};

fn model_dir() -> Option<PathBuf> {
    std::env::var_os("FLUENT_SMALL100_MODEL")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

fn rss_mb(key: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = text.lines().find(|line| line.starts_with(key))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

#[test]
fn tokenizer_uses_target_language_and_eos() {
    let Some(dir) = model_dir() else {
        eprintln!("skipping: set FLUENT_SMALL100_MODEL to run SMaLL-100 tests");
        return;
    };
    let tokenizer = Small100Tokenizer::from_dir(&dir).expect("tokenizer");
    assert_eq!(tokenizer.eos_id(), 2);
    assert_eq!(tokenizer.pad_id(), 1);
    assert_eq!(tokenizer.language_count(), 98);

    let (ids, truncated) = tokenizer
        .encode_source("Life is like a box of chocolates.", "fra_Latn", 512)
        .expect("encode");
    // Differential pin against Python `sentencepiece` + the official
    // `vocab.json` mapping. A change here means segmentation or id mapping
    // regressed.
    assert_eq!(
        ids,
        vec![128_028, 21_457, 117, 14_906, 8, 37_089, 432, 110_309, 10_550, 5, 2]
    );
    assert_eq!(ids[0], tokenizer.lang_id("fra_Latn").unwrap());
    assert!(ids[0] >= LANGUAGE_ID_BASE);
    assert_eq!(*ids.last().unwrap(), 2);
    assert!(!truncated);
    assert!(tokenizer.lang_id("bre_Latn").is_err());

    let decoded = tokenizer.decode(&ids[1..ids.len() - 1]).expect("decode");
    assert!(decoded.contains("chocolates"), "{decoded}");
}

#[test]
fn small100_loads_translates_repeats_and_unloads() {
    let Some(dir) = model_dir() else {
        eprintln!("skipping: set FLUENT_SMALL100_MODEL to run SMaLL-100 tests");
        return;
    };
    let mut config = EngineConfig::new(Mode::Small, &dir);
    config.device = DeviceChoice::Cpu;
    config.precision = Precision::Auto;
    config.max_length = 512;
    config.output_limit = OutputLimit::Tokens(96);
    config.num_beams = 2;

    let cancel = AtomicBool::new(false);
    let before = rss_mb("VmRSS:");
    let started = Instant::now();
    let mut engine = Engine::new(config);
    engine.load(&mut |_| {}, &cancel).expect("load engine");
    let status = engine.status();
    println!(
        "load {:.2?} · rss {:?} → {:?} MB · peak {:?} MB · {} · {}",
        started.elapsed(),
        before,
        rss_mb("VmRSS:"),
        rss_mb("VmHWM:"),
        status.device,
        status.precision
    );
    assert!(status.loaded);
    assert_eq!(status.mode, Mode::Small);

    let cases = [
        ("Life is like a box of chocolates.", "eng_Latn", "fra_Latn"),
        ("Life is like a box of chocolates.", "eng_Latn", "deu_Latn"),
        ("Life is like a box of chocolates.", "eng_Latn", "spa_Latn"),
        ("Life is like a box of chocolates.", "eng_Latn", "por_Latn"),
        ("Life is like a box of chocolates.", "eng_Latn", "ita_Latn"),
        ("Life is like a box of chocolates.", "eng_Latn", "zho_Hans"),
        ("生活就像一盒巧克力。", "zho_Hans", "eng_Latn"),
        (
            "La bibliothèque ouvre à huit heures.",
            "fra_Latn",
            "eng_Latn",
        ),
    ];
    for (text, source, target) in cases {
        let started = Instant::now();
        let request = TranslationRequest::new(text, source, target);
        let result = engine
            .translate(&request, &mut |_| {}, &cancel)
            .unwrap_or_else(|error| panic!("{source}->{target}: {error}"));
        println!(
            "{source}->{target} {:.0} ms · {} tokens · {:?}",
            started.elapsed().as_secs_f64() * 1000.0,
            result.generated_tokens,
            result.text
        );
        assert!(
            !result.text.trim().is_empty(),
            "{source}->{target} was empty"
        );
        if source != target {
            assert_ne!(
                result.text.trim(),
                text.trim(),
                "{source}->{target} copied the source"
            );
        }
    }

    // Deterministic beam search: repeating the same request twice must match.
    let request =
        TranslationRequest::new("Life is like a box of chocolates.", "eng_Latn", "fra_Latn");
    let first = engine.translate(&request, &mut |_| {}, &cancel).unwrap();
    let second = engine.translate(&request, &mut |_| {}, &cancel).unwrap();
    assert_eq!(first.text, second.text);
    assert_eq!(first.generated_tokens, second.generated_tokens);
    println!("deterministic: {:?}", first.text);

    engine.unload();
    assert!(!engine.status().loaded);
    engine.load(&mut |_| {}, &cancel).expect("reload");
    let again = engine.translate(&request, &mut |_| {}, &cancel).unwrap();
    assert_eq!(again.text, first.text);
    engine.unload();
}

#[test]
fn small100_cancels_during_generation() {
    let Some(dir) = model_dir() else {
        eprintln!("skipping: set FLUENT_SMALL100_MODEL to run SMaLL-100 tests");
        return;
    };
    let mut config = EngineConfig::new(Mode::Small, &dir);
    config.device = DeviceChoice::Cpu;
    config.precision = Precision::Auto;
    config.max_length = 512;
    config.output_limit = OutputLimit::UntilEos;
    config.num_beams = 1;
    let mut engine = Engine::new(config);
    let cancel = Arc::new(AtomicBool::new(false));
    engine
        .load(&mut |_| {}, &cancel.clone())
        .expect("load engine");

    let trigger = Arc::clone(&cancel);
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(90));
        trigger.store(true, Ordering::SeqCst);
    });
    let request = TranslationRequest::new(
        "This is a deliberately long paragraph used to exercise cancellation while the decoder is still running.",
        "eng_Latn",
        "fra_Latn",
    );
    let started = Instant::now();
    let result = engine.translate(&request, &mut |_| {}, &cancel);
    assert!(result.is_err(), "cancellation should stop generation");
    println!("cancelled after {:.2?}", started.elapsed());
    assert!(started.elapsed() < Duration::from_secs(30));
}
