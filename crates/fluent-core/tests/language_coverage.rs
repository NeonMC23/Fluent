//! Language-capability regression tests.
//!
//! The static registry tests run everywhere. The tokenizer checks are gated by
//! `FLUENT_TEST_MODELS_ROOT` so CI does not need multi-gigabyte checkpoints.
//!
//! ```text
//! FLUENT_TEST_MODELS_ROOT="$HOME/.local/share/fluent/models" \
//!     cargo test --release -p fluent-core --test language_coverage -- --nocapture
//! ```

use std::collections::HashSet;
use std::path::PathBuf;

use fluent_core::model::madlad::MadladTokenizer;
use fluent_core::model::small100::{self, Small100Tokenizer};
use fluent_core::{language_list, model_dir, resolve_language_selection, Mode, NllbTokenizer};

fn models_root() -> Option<PathBuf> {
    std::env::var_os("FLUENT_TEST_MODELS_ROOT")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

#[test]
fn advertised_language_lists_are_unique_and_have_display_names() {
    for mode in [Mode::Fast, Mode::Accurate, Mode::Madlad, Mode::Small] {
        let list = language_list(mode);
        assert!(!list.is_empty(), "{mode} has no languages");
        let mut codes = HashSet::new();
        for language in &list {
            assert!(
                codes.insert(language.code),
                "{mode}: duplicate {}",
                language.code
            );
            assert!(
                !language.name.is_empty(),
                "{mode}: {} has no name",
                language.code
            );
            assert!(
                !language.native.is_empty(),
                "{mode}: {} has no native name",
                language.code
            );
        }
    }
    assert_eq!(language_list(Mode::Fast).len(), 202);
    assert_eq!(language_list(Mode::Accurate).len(), 202);
    assert_eq!(language_list(Mode::Madlad).len(), 72);
    assert_eq!(language_list(Mode::Small).len(), 98);
}

#[test]
fn chinese_variants_match_each_backend() {
    // NLLB and MADLAD advertise both scripts; SMaLL-100 only maps Simplified.
    for mode in [Mode::Fast, Mode::Accurate, Mode::Madlad] {
        assert!(mode.supports_language("zho_Hans"), "{mode}");
        assert!(mode.supports_language("zho_Hant"), "{mode}");
    }
    assert!(Mode::Small.supports_language("zho_Hans"));
    assert!(!Mode::Small.supports_language("zho_Hant"));

    let selected = resolve_language_selection(Mode::Small, "eng_Latn", "zho_Hant");
    assert_eq!(selected.target, "eng_Latn");
    assert!(selected.adjusted);
    let selected = resolve_language_selection(Mode::Madlad, "eng_Latn", "zho_Hant");
    assert_eq!(selected.target, "zho_Hant");
    assert!(!selected.adjusted);
}

#[test]
fn every_advertised_small100_code_maps_to_a_language_token() {
    // Static mapping check: every Fluent code has an M2M-100 short code and a
    // unique target token.
    let mut codes = HashSet::new();
    for (nllb, m2m) in small100::LANGUAGES {
        assert!(codes.insert(*nllb), "duplicate Fluent code {nllb}");
        assert!(codes.insert(*m2m), "duplicate M2M code {m2m}");
    }
    for code in language_list(Mode::Small) {
        assert!(small100::nllb_to_m2m(code.code).is_some(), "{code:?}");
    }
}

#[test]
fn real_tokenizers_cover_every_advertised_language() {
    let Some(root) = models_root() else {
        eprintln!("skipping: set FLUENT_TEST_MODELS_ROOT to check installed tokenizers");
        return;
    };

    let nllb_dir = model_dir(&root, Mode::Fast);
    if nllb_dir.is_dir() {
        let tokenizer = NllbTokenizer::from_dir(&nllb_dir).expect("NLLB tokenizer");
        let mut checked = 0;
        for language in language_list(Mode::Fast) {
            tokenizer
                .lang_id(language.code)
                .unwrap_or_else(|error| panic!("NLLB missing {}: {error}", language.code));
            checked += 1;
        }
        println!("NLLB tokenizer covers {checked} advertised languages");
        assert_eq!(checked, 202);
    } else {
        eprintln!("skipping NLLB: {} not found", nllb_dir.display());
    }

    let small_dir = model_dir(&root, Mode::Small);
    if small_dir.is_dir() {
        let tokenizer = Small100Tokenizer::from_dir(&small_dir).expect("SMaLL tokenizer");
        let mut checked = 0;
        for language in language_list(Mode::Small) {
            tokenizer
                .lang_id(language.code)
                .unwrap_or_else(|error| panic!("SMaLL missing {}: {error}", language.code));
            checked += 1;
        }
        assert_eq!(checked, 98);
        assert_eq!(tokenizer.language_count(), 98);
        assert!(tokenizer.lang_id("zho_Hant").is_err());
        println!("SMaLL-100 tokenizer covers {checked} advertised languages");
    } else {
        eprintln!("skipping SMaLL-100: {} not found", small_dir.display());
    }

    let madlad_dir = model_dir(&root, Mode::Madlad);
    if madlad_dir.is_dir() {
        let tokenizer = MadladTokenizer::from_dir(&madlad_dir).expect("MADLAD tokenizer");
        let mut checked = 0;
        for language in language_list(Mode::Madlad) {
            let prefix = fluent_core::model::madlad::language_prefix(language.code)
                .unwrap_or_else(|| panic!("MADLAD missing mapping for {}", language.code));
            assert!(
                tokenizer.token_to_id(prefix).is_some(),
                "MADLAD tokenizer is missing {prefix} for {}",
                language.code
            );
            checked += 1;
        }
        assert_eq!(checked, 72);
        println!("MADLAD tokenizer covers {checked} advertised languages");
    } else {
        eprintln!("skipping MADLAD: {} not found", madlad_dir.display());
    }
}
