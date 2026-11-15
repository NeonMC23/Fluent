//! SMaLL-100 tokenizer and language mapping.
//!
//! SMaLL-100 is an M2M-100-style encoder/decoder with a 12-layer encoder and a
//! shallow 3-layer decoder. It uses the standard M2M-100 SentencePiece model
//! for segmentation but a separate `vocab.json` for token ids, plus 100
//! `__xx__` language tokens at the end of the vocabulary.
//!
//! Unlike NLLB/M2M-100, the **target** language token is prepended to the
//! encoder input (`[__tgt__] source </s>`) and the decoder starts at `</s>`
//! without a forced language token. The generation code path therefore needs
//! [`super::nllb::GenerateOptions::force_target_token`] disabled.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::{Error, Result};

/// Number of trailing made-up vocabulary entries in the SMaLL-100 checkpoint.
pub const MADEUP_WORDS: usize = 8;

/// Beginning of the language-token id range in the model vocabulary.
pub const LANGUAGE_ID_BASE: u32 = 128_004;

/// Fluent/NLLB language code -> M2M-100 short code.
///
/// Only languages that exist in both Fluent's NLLB table and the SMaLL-100
/// vocabulary are mapped. SMaLL-100 does not include Breton or Western
/// Frisian, which are present in neither list.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("afr_Latn", "af"),
    ("amh_Ethi", "am"),
    ("arb_Arab", "ar"),
    ("ast_Latn", "ast"),
    ("azj_Latn", "az"),
    ("bak_Cyrl", "ba"),
    ("bel_Cyrl", "be"),
    ("bul_Cyrl", "bg"),
    ("ben_Beng", "bn"),
    ("bos_Latn", "bs"),
    ("cat_Latn", "ca"),
    ("ceb_Latn", "ceb"),
    ("ces_Latn", "cs"),
    ("cym_Latn", "cy"),
    ("dan_Latn", "da"),
    ("deu_Latn", "de"),
    ("ell_Grek", "el"),
    ("eng_Latn", "en"),
    ("spa_Latn", "es"),
    ("est_Latn", "et"),
    ("pes_Arab", "fa"),
    ("fuv_Latn", "ff"),
    ("fin_Latn", "fi"),
    ("fra_Latn", "fr"),
    ("gle_Latn", "ga"),
    ("gla_Latn", "gd"),
    ("glg_Latn", "gl"),
    ("guj_Gujr", "gu"),
    ("hau_Latn", "ha"),
    ("heb_Hebr", "he"),
    ("hin_Deva", "hi"),
    ("hrv_Latn", "hr"),
    ("hat_Latn", "ht"),
    ("hun_Latn", "hu"),
    ("hye_Armn", "hy"),
    ("ind_Latn", "id"),
    ("ibo_Latn", "ig"),
    ("ilo_Latn", "ilo"),
    ("isl_Latn", "is"),
    ("ita_Latn", "it"),
    ("jpn_Jpan", "ja"),
    ("jav_Latn", "jv"),
    ("kat_Geor", "ka"),
    ("kaz_Cyrl", "kk"),
    ("khm_Khmr", "km"),
    ("kan_Knda", "kn"),
    ("kor_Hang", "ko"),
    ("ltz_Latn", "lb"),
    ("lug_Latn", "lg"),
    ("lin_Latn", "ln"),
    ("lao_Laoo", "lo"),
    ("lit_Latn", "lt"),
    ("lvs_Latn", "lv"),
    ("plt_Latn", "mg"),
    ("mkd_Cyrl", "mk"),
    ("mal_Mlym", "ml"),
    ("khk_Cyrl", "mn"),
    ("mar_Deva", "mr"),
    ("zsm_Latn", "ms"),
    ("mya_Mymr", "my"),
    ("npi_Deva", "ne"),
    ("nld_Latn", "nl"),
    ("nob_Latn", "no"),
    ("nso_Latn", "ns"),
    ("oci_Latn", "oc"),
    ("ory_Orya", "or"),
    ("pan_Guru", "pa"),
    ("pol_Latn", "pl"),
    ("pbt_Arab", "ps"),
    ("por_Latn", "pt"),
    ("ron_Latn", "ro"),
    ("rus_Cyrl", "ru"),
    ("snd_Arab", "sd"),
    ("sin_Sinh", "si"),
    ("slk_Latn", "sk"),
    ("slv_Latn", "sl"),
    ("som_Latn", "so"),
    ("als_Latn", "sq"),
    ("srp_Cyrl", "sr"),
    ("ssw_Latn", "ss"),
    ("sun_Latn", "su"),
    ("swe_Latn", "sv"),
    ("swh_Latn", "sw"),
    ("tam_Taml", "ta"),
    ("tha_Thai", "th"),
    ("tgl_Latn", "tl"),
    ("tsn_Latn", "tn"),
    ("tur_Latn", "tr"),
    ("ukr_Cyrl", "uk"),
    ("urd_Arab", "ur"),
    ("uzn_Latn", "uz"),
    ("vie_Latn", "vi"),
    ("wol_Latn", "wo"),
    ("xho_Latn", "xh"),
    ("ydd_Hebr", "yi"),
    ("yor_Latn", "yo"),
    ("zho_Hans", "zh"),
    ("zul_Latn", "zu"),
];

pub fn nllb_to_m2m(code: &str) -> Option<&'static str> {
    LANGUAGES
        .iter()
        .find(|(nllb, _)| *nllb == code)
        .map(|(_, m2m)| *m2m)
}

pub fn supports_language(code: &str) -> bool {
    nllb_to_m2m(code).is_some() && crate::find_language(code).is_some()
}

pub struct Small100Tokenizer {
    sp: sentencepiece_rust::SentencePieceProcessor,
    vocab: HashMap<String, u32>,
    id_to_piece: Vec<String>,
    lang_ids: HashMap<&'static str, u32>,
    language_base: u32,
    eos_id: u32,
    pad_id: u32,
    bos_id: u32,
    unk_id: u32,
}

#[derive(Debug, Deserialize)]
struct TokenizerConfig {
    #[serde(default)]
    additional_special_tokens: Vec<String>,
}

impl Small100Tokenizer {
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let sp_path = dir.join("sentencepiece.bpe.model");
        let vocab_path = dir.join("vocab.json");
        if !sp_path.is_file() {
            return Err(Error::MissingModelFile {
                dir: dir.to_path_buf(),
                hint: "missing sentencepiece.bpe.model".to_string(),
            });
        }
        if !vocab_path.is_file() {
            return Err(Error::MissingModelFile {
                dir: dir.to_path_buf(),
                hint: "missing vocab.json".to_string(),
            });
        }

        let sp = sentencepiece_rust::SentencePieceProcessor::open(&sp_path)
            .map_err(|error| Error::Tokenizer(format!("{}: {error}", sp_path.display())))?;

        let raw = std::fs::read_to_string(&vocab_path)?;
        let vocab: HashMap<String, u32> = serde_json::from_str(&raw)
            .map_err(|error| Error::Tokenizer(format!("{}: {error}", vocab_path.display())))?;
        if vocab.len() > u32::MAX as usize {
            return Err(Error::Tokenizer("vocabulary is too large".to_string()));
        }
        let mut id_to_piece = vec![String::new(); vocab.len()];
        for (piece, id) in &vocab {
            let index = *id as usize;
            if index >= id_to_piece.len() {
                return Err(Error::Tokenizer(format!(
                    "vocabulary id {id} for {piece:?} is out of range"
                )));
            }
            id_to_piece[index] = piece.clone();
        }

        let eos_id = *vocab
            .get("</s>")
            .ok_or_else(|| Error::Tokenizer("vocab.json is missing </s>".to_string()))?;
        let pad_id = *vocab.get("<pad>").unwrap_or(&1);
        let bos_id = *vocab.get("<s>").unwrap_or(&0);
        let unk_id = *vocab.get("<unk>").unwrap_or(&3);

        let language_base = vocab.len() as u32;
        if language_base != LANGUAGE_ID_BASE {
            return Err(Error::Tokenizer(format!(
                "unexpected vocabulary size {language_base}; the pinned SMaLL-100 tokenizer expects {LANGUAGE_ID_BASE}"
            )));
        }
        let lang_ids = load_language_ids(dir, language_base)?;
        if lang_ids.len() != LANGUAGES.len() {
            return Err(Error::Tokenizer(format!(
                "tokenizer_config.json exposes {} of the {} mapped SMaLL-100 languages",
                lang_ids.len(),
                LANGUAGES.len()
            )));
        }

        Ok(Self {
            sp,
            vocab,
            id_to_piece,
            lang_ids,
            language_base,
            eos_id,
            pad_id,
            bos_id,
            unk_id,
        })
    }

    pub fn eos_id(&self) -> u32 {
        self.eos_id
    }

    pub fn pad_id(&self) -> u32 {
        self.pad_id
    }

    pub fn bos_id(&self) -> u32 {
        self.bos_id
    }

    pub fn unk_id(&self) -> u32 {
        self.unk_id
    }

    /// Maps a Fluent/NLLB language code to the SMaLL-100 language token id.
    pub fn lang_id(&self, nllb_code: &str) -> Result<u32> {
        let m2m = nllb_to_m2m(nllb_code).ok_or_else(|| {
            Error::UnknownLanguage(format!(
                "{nllb_code} (not mapped by the SMaLL-100 tokenizer)"
            ))
        })?;
        self.lang_ids.get(m2m).copied().ok_or_else(|| {
            Error::UnknownLanguage(format!(
                "{nllb_code} (missing __{m2m}__ from the tokenizer)"
            ))
        })
    }

    /// Encodes `[target_language] … tokens … </s>`.
    ///
    /// SMaLL-100 conditions on the **target** language token in the encoder
    /// input, not the source language.
    pub fn encode_source(
        &self,
        text: &str,
        target_language: &str,
        max_tokens: usize,
    ) -> Result<(Vec<u32>, bool)> {
        let lang_id = self.lang_id(target_language)?;
        let pieces = self
            .sp
            .encode_pieces(text)
            .map_err(|error| Error::Tokenizer(error.to_string()))?;

        let budget = max_tokens.saturating_sub(2).max(1);
        let truncated = pieces.len() > budget;
        let mut ids = Vec::with_capacity(pieces.len().min(budget) + 2);
        ids.push(lang_id);
        for piece in pieces.iter().take(budget) {
            ids.push(*self.vocab.get(piece).unwrap_or(&self.unk_id));
        }
        ids.push(self.eos_id);
        Ok((ids, truncated))
    }

    pub fn decode(&self, ids: &[u32]) -> Result<String> {
        let mut pieces: Vec<&str> = Vec::with_capacity(ids.len());
        for id in ids {
            if *id == self.eos_id || *id == self.pad_id || *id == self.bos_id || *id == self.unk_id
            {
                continue;
            }
            let Some(piece) = self.id_to_piece.get(*id as usize) else {
                continue;
            };
            // Language tokens and the trailing made-up entries have no text.
            if *id >= self.language_base || piece.is_empty() {
                continue;
            }
            pieces.push(piece);
        }
        Ok(decode_pieces(&pieces))
    }

    /// Number of language tokens declared by the tokenizer configuration.
    pub fn language_count(&self) -> usize {
        self.lang_ids.len()
    }
}

fn load_language_ids(dir: &Path, base: u32) -> Result<HashMap<&'static str, u32>> {
    let path = dir.join("tokenizer_config.json");
    let raw = std::fs::read_to_string(&path)?;
    let config: TokenizerConfig = serde_json::from_str(&raw)
        .map_err(|error| Error::Tokenizer(format!("{}: {error}", path.display())))?;

    let mut ids = HashMap::with_capacity(config.additional_special_tokens.len());
    for (index, token) in config.additional_special_tokens.iter().enumerate() {
        let Some(code) = token
            .strip_prefix("__")
            .and_then(|rest| rest.strip_suffix("__"))
        else {
            continue;
        };
        // `LANGUAGES` stores static strings; only accept codes we know.
        let Some((_, m2m)) = LANGUAGES.iter().find(|(_, m2m)| *m2m == code) else {
            continue;
        };
        let id = base + index as u32;
        if id >= base + 100 {
            return Err(Error::Tokenizer(format!(
                "language token {token:?} is outside the expected range"
            )));
        }
        ids.insert(*m2m, id);
    }
    Ok(ids)
}

/// Decodes SentencePiece pieces the way `SentencePieceProcessor::DecodePieces`
/// does for a model without byte fallback.
fn decode_pieces(pieces: &[&str]) -> String {
    let mut output = String::new();
    for piece in pieces {
        if piece.len() >= 2 && piece.starts_with('<') && piece.ends_with('>') {
            // Control pieces and made-up tokens carry no surface text.
            continue;
        }
        let mut first = true;
        for part in piece.split('\u{2581}') {
            if !first {
                output.push(' ');
            }
            first = false;
            output.push_str(part);
        }
    }
    let trimmed = output.trim_start_matches(' ');
    clean_text(trimmed)
}

fn clean_text(text: &str) -> String {
    let trimmed = text.trim();
    let mut output = String::with_capacity(trimmed.len());
    let mut previous_was_space = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !previous_was_space {
                output.push(' ');
            }
            previous_was_space = true;
        } else {
            output.push(ch);
            previous_was_space = false;
        }
    }
    output.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mapping_entry_exists_in_the_language_table() {
        let mut codes = std::collections::HashSet::new();
        for (nllb, m2m) in LANGUAGES {
            assert!(
                crate::find_language(nllb).is_some(),
                "unknown Fluent language code {nllb}"
            );
            assert!(codes.insert(*m2m), "duplicate M2M-100 code {m2m}");
        }
        // SMaLL-100 has 100 language tokens; Breton and Frisian are the two
        // missing from Fluent's NLLB table.
        assert_eq!(LANGUAGES.len(), 98);
    }

    #[test]
    fn required_languages_are_mapped() {
        for code in [
            "eng_Latn", "fra_Latn", "deu_Latn", "spa_Latn", "por_Latn", "ita_Latn", "zho_Hans",
        ] {
            assert!(supports_language(code), "{code}");
        }
        assert!(!supports_language("bre_Latn"));
    }

    #[test]
    fn pieces_decode_like_sentencepiece() {
        assert_eq!(
            decode_pieces(&["\u{2581}Hello", "\u{2581}world"]),
            "Hello world"
        );
        assert_eq!(decode_pieces(&["\u{2581}", "生活", "。"]), "生活。");
        assert_eq!(decode_pieces(&["\u{2581}a", "\u{2581}b"]), "a b");
    }
}
