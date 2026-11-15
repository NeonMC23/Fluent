//! Real MADLAD engine integration, gated by `FLUENT_MADLAD_MODEL`.
//!
//! ```text
//! FLUENT_MADLAD_MODEL="$HOME/.local/share/fluent/models/madlad400-3b-mt" \
//!     cargo test --release -p fluent-core --test madlad -- --nocapture
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use fluent_core::{
    DeviceChoice, Engine, EngineConfig, Mode, OutputLimit, Precision, ProgressStage,
    TranslationRequest,
};

fn model_dir() -> Option<PathBuf> {
    std::env::var_os("FLUENT_MADLAD_MODEL")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

fn memory_mb(key: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with(key))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

fn rss_mb() -> Option<u64> {
    memory_mb("VmRSS:")
}

fn peak_rss_mb() -> Option<u64> {
    memory_mb("VmHWM:")
}

#[test]
fn madlad_loads_translates_limits_and_unloads() {
    let Some(dir) = model_dir() else {
        eprintln!("skipping: set FLUENT_MADLAD_MODEL to run MADLAD engine tests");
        return;
    };
    let mut config = EngineConfig::new(Mode::Madlad, &dir);
    config.device = DeviceChoice::Cpu;
    config.precision = Precision::Auto;
    config.max_length = 256;
    config.output_limit = OutputLimit::Tokens(48);
    config.num_beams = 1;

    let cancel = AtomicBool::new(false);
    let before_rss = rss_mb();
    let load_started = Instant::now();
    let mut engine = Engine::new(config);
    engine
        .load(&mut |_| {}, &cancel)
        .expect("load MADLAD engine");
    let load_time = load_started.elapsed();
    let status = engine.status();
    println!(
        "load: {:.2?} · rss before {:?} MB · after {:?} MB · peak {:?} MB · device {} · precision {}",
        load_time,
        before_rss,
        rss_mb(),
        peak_rss_mb(),
        status.device,
        status.precision
    );
    assert_eq!(status.mode, Mode::Madlad);
    assert!(status.loaded);

    for (source, target) in [
        ("eng_Latn", "fra_Latn"),
        ("fra_Latn", "eng_Latn"),
        ("eng_Latn", "deu_Latn"),
        ("eng_Latn", "spa_Latn"),
        ("eng_Latn", "jpn_Jpan"),
        ("eng_Latn", "arb_Arab"),
        ("eng_Latn", "swh_Latn"),
    ] {
        let started = Instant::now();
        let mut first_token = None;
        let result = engine
            .translate(
                &TranslationRequest::new(
                    "The library opens at eight. Visitors can borrow up to ten books for three weeks.",
                    source,
                    target,
                ),
                &mut |progress| {
                    if progress.stage == ProgressStage::Decoding
                        && progress.fraction.unwrap_or(0.0) > 0.0
                        && first_token.is_none()
                    {
                        first_token = Some(started.elapsed());
                    }
                },
                &cancel,
            )
            .unwrap_or_else(|error| panic!("{source}->{target}: {error}"));
        assert!(!result.text.trim().is_empty(), "{source}->{target} empty");
        assert!(result.generated_tokens > 0);
        println!(
            "{source}->{target}: {:?} · {} tokens · first {:?} · total {:.2?}",
            result.text,
            result.generated_tokens,
            first_token,
            started.elapsed()
        );
    }

    engine.config_mut().output_limit = OutputLimit::Tokens(8);
    let limited = engine
        .translate(
            &TranslationRequest::new("Hello world", "eng_Latn", "fra_Latn"),
            &mut |_| {},
            &cancel,
        )
        .expect("finite limit");
    assert!(
        limited.generated_tokens <= 8,
        "finite limit exceeded: {}",
        limited.generated_tokens
    );

    engine.config_mut().output_limit = OutputLimit::UntilEos;
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let trigger = cancel_flag.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(700));
        trigger.store(true, Ordering::Relaxed);
    });
    let error = engine
        .translate(
            &TranslationRequest::new(
                "The library opens at eight and closes at seven. Visitors can borrow books.",
                "eng_Latn",
                "fra_Latn",
            ),
            &mut |_| {},
            &cancel_flag,
        )
        .expect_err("long generation should observe cancellation");
    canceller.join().ok();
    assert!(
        matches!(error, fluent_core::Error::Cancelled),
        "unexpected cancellation result: {error}"
    );

    engine.unload();
    assert!(!engine.is_loaded());
    println!("unload: rss {:?} MB", rss_mb());
}
