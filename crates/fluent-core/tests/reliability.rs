//! End-to-end engine lifecycle, cancellation and repeatability checks.
//!
//! These tests need the NLLB weights, which are not redistributed with the
//! repository. Point `FLUENT_TEST_MODEL` at a model directory to run them:
//!
//! ```text
//! FLUENT_TEST_MODEL='/path/to/nllb-200-1.3b' \
//!     cargo test -p fluent-core --test reliability -- --nocapture
//! ```
//!
//! Without the variable they skip instead of failing, so the normal workspace
//! test run stays self-contained.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use fluent_core::{
    Engine, EngineConfig, Mode, Precision, Progress, TranslationRequest, TranslationResult,
};

fn model_dir() -> Option<PathBuf> {
    let path = std::env::var_os("FLUENT_TEST_MODEL")?;
    let path = PathBuf::from(path);
    path.is_dir().then_some(path)
}

fn engine(dir: &PathBuf) -> Engine {
    let mut config = EngineConfig::new(Mode::Accurate, dir);
    config.precision = Precision::Auto;
    // Short, deterministic generations keep the suite fast while still
    // exercising the encoder, prefill, decode and cache paths.
    config.output_limit = fluent_core::OutputLimit::Tokens(24);
    config.num_beams = 2;
    Engine::new(config)
}

fn translate(engine: &mut Engine, cancel: &AtomicBool) -> fluent_core::Result<TranslationResult> {
    let mut noop = |_progress: Progress| {};
    engine.translate(
        &TranslationRequest::new(
            "The library opens at eight and closes at seven.",
            "eng_Latn",
            "fra_Latn",
        ),
        &mut noop,
        cancel,
    )
}

fn rss_mb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

#[test]
fn lifecycle_load_translate_unload_reload() {
    let Some(dir) = model_dir() else {
        eprintln!("skipping: set FLUENT_TEST_MODEL to run the lifecycle test");
        return;
    };
    let cancel = AtomicBool::new(false);
    let mut engine = engine(&dir);

    engine.load(&mut |_| {}, &cancel).expect("initial load");
    assert!(engine.is_loaded());
    let loaded_rss = rss_mb();
    let first = translate(&mut engine, &cancel).expect("first translation");

    engine.unload();
    assert!(!engine.is_loaded(), "unload must release the model state");
    let unloaded_rss = rss_mb();

    // `translate` transparently reloads; the second output must match the
    // first because the same weights and settings are used.
    let second = translate(&mut engine, &cancel).expect("reloaded translation");
    assert!(engine.is_loaded());
    assert_eq!(first.text, second.text, "reload changed the translation");

    println!(
        "lifecycle: rss loaded={loaded_rss:?} MB unloaded={unloaded_rss:?} MB -> output {:?}",
        first.text
    );
}

#[test]
fn repeated_translations_are_deterministic_and_stable() {
    let Some(dir) = model_dir() else {
        eprintln!("skipping: set FLUENT_TEST_MODEL to run the repeatability test");
        return;
    };
    let cancel = AtomicBool::new(false);
    let mut engine = engine(&dir);
    let first = translate(&mut engine, &cancel).expect("first translation");
    let baseline_rss = rss_mb();

    for round in 0..5 {
        let result = translate(&mut engine, &cancel).expect("repeat translation");
        assert_eq!(
            result.text, first.text,
            "translation changed on repetition {round}"
        );
    }

    let final_rss = rss_mb();
    if let (Some(before), Some(after)) = (baseline_rss, final_rss) {
        // The allocator may keep freed buffers, but a leak in the KV cache or
        // beam state would grow linearly with the number of translations.
        assert!(
            after <= before + 256,
            "RSS grew from {before} MB to {after} MB over five translations"
        );
    }
    println!("repeats: rss {baseline_rss:?} MB -> {final_rss:?} MB");
}

#[test]
fn cancellation_is_observed_and_bounded() {
    let Some(dir) = model_dir() else {
        eprintln!("skipping: set FLUENT_TEST_MODEL to run the cancellation test");
        return;
    };
    let mut engine = engine(&dir);
    let loaded = AtomicBool::new(false);
    engine.load(&mut |_| {}, &loaded).expect("load");

    // Already-cancelled calls must not start the encoder.
    let cancelled = AtomicBool::new(true);
    let started = Instant::now();
    let error = translate(&mut engine, &cancelled).expect_err("cancelled call must fail");
    assert!(matches!(error, fluent_core::Error::Cancelled));
    println!(
        "cancellation: pre-cancelled call returned in {:.1} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );

    // Cancel while the encoder is running. A long source keeps the encoder
    // busy long enough for the flag to be observed between layers.
    let flag = Arc::new(AtomicBool::new(false));
    let trigger = flag.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        trigger.store(true, Ordering::Relaxed);
    });
    let long_text = "When the first trains arrived in the valley, few people believed that the \
                     small farming community would become an industrial centre within a single \
                     generation. The river had always provided water for the orchards, and the \
                     surrounding hills were covered with forests that supplied timber for the new \
                     factories. Engineers from the capital surveyed the land in 1857 and proposed a \
                     network of tracks that would connect the mines to the harbour.";
    let mut noop = |_progress: Progress| {};
    let started = Instant::now();
    let error = engine
        .translate(
            &TranslationRequest::new(long_text, "eng_Latn", "fra_Latn"),
            &mut noop,
            &flag,
        )
        .expect_err("cancelled encoder must fail");
    let latency = started.elapsed();
    canceller.join().expect("canceller thread");
    assert!(matches!(error, fluent_core::Error::Cancelled));
    assert!(
        latency < Duration::from_secs(10),
        "cancellation took {latency:?}"
    );
    println!(
        "cancellation: encoder cancellation returned in {:.0} ms",
        latency.as_secs_f64() * 1000.0
    );

    // The engine must remain usable after a cancelled generation.
    let ok = translate(&mut engine, &loaded).expect("translation after cancellation");
    assert!(!ok.text.is_empty());
}
