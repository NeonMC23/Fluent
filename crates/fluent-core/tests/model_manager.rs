//! Optional real-model installation verification.
//!
//! ```text
//! FLUENT_TEST_MODELS_ROOT="$HOME/.local/share/fluent/models" \
//!   cargo test --release -p fluent-core --test model_manager -- --nocapture
//! ```

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use fluent_core::{
    chat_model_spec, model_dir, verify_chat_model, verify_model, InstallState, Mode,
};

#[test]
fn existing_models_verify_when_requested() {
    let Some(root) = std::env::var_os("FLUENT_TEST_MODELS_ROOT").map(PathBuf::from) else {
        eprintln!("skipping: set FLUENT_TEST_MODELS_ROOT to verify installed models");
        return;
    };
    let cancel = AtomicBool::new(false);
    let mut checked = 0;
    for mode in [Mode::Fast, Mode::Accurate, Mode::Madlad, Mode::Small] {
        let dir = model_dir(&root, mode);
        if !dir.is_dir() {
            eprintln!("skipping {}: directory not found", dir.display());
            continue;
        }
        let report = verify_model(mode, &dir, &mut |_| {}, &cancel).expect("verification");
        println!(
            "{}: {:?}, {} bytes, verified={}, missing={}, bad={}",
            mode,
            report.state,
            report.installed_bytes,
            report.verified,
            report.missing_files().len(),
            report.bad_hashes().len()
        );
        assert_eq!(report.state, InstallState::Installed, "{}", mode);
        assert_eq!(report.bad_hashes().len(), 0, "{}", mode);
        checked += 1;
    }
    assert!(
        checked > 0,
        "no model directories found under {}",
        root.display()
    );
}

#[test]
fn downloaded_chat_model_verifies_when_requested() {
    let Some(dir) = std::env::var_os("FLUENT_TEST_CHAT_MODEL").map(PathBuf::from) else {
        eprintln!("skipping: set FLUENT_TEST_CHAT_MODEL to verify a chat model");
        return;
    };
    let id = std::env::var("FLUENT_TEST_CHAT_MODEL_ID")
        .unwrap_or_else(|_| "qwen2.5-0.5b-instruct".to_string());
    let spec = chat_model_spec(&id).expect("known chat model id");
    let cancel = AtomicBool::new(false);
    let report =
        verify_chat_model(spec, &dir, &mut |_| {}, &cancel).expect("chat verification");
    println!(
        "{}: {:?}, {} bytes, verified={}, missing={}, bad={}",
        spec.id,
        report.state,
        report.installed_bytes,
        report.verified,
        report.missing_files().len(),
        report.bad_hashes().len()
    );
    assert_eq!(report.state, InstallState::Installed);
    assert_eq!(report.bad_hashes().len(), 0);
}
