//! Central model registry and capability metadata.

use crate::model::madlad::{self, MadladLanguage};
use crate::{find_language, languages, Language, Mode};

/// Model families with distinct inference implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    Nllb200,
    T5,
}

impl Architecture {
    pub fn label(self) -> &'static str {
        match self {
            Self::Nllb200 => "NLLB-200",
            Self::T5 => "T5",
        }
    }
}

/// User-visible metadata for one selectable model.
#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub mode: Mode,
    pub name: &'static str,
    pub architecture: Architecture,
    pub parameters: &'static str,
    pub storage: &'static str,
    pub license: &'static str,
    pub default_dir: &'static str,
    pub description: &'static str,
    pub supports_beams: bool,
}

pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        mode: Mode::Fast,
        name: "NLLB-200 1.3B",
        architecture: Architecture::Nllb200,
        parameters: "1.3B",
        storage: "5.2 GB FP32 · Q8 CPU",
        license: "CC-BY-NC-4.0",
        default_dir: "nllb-200-1.3b",
        description: "Fast local translation · recommended on CPU",
        supports_beams: true,
    },
    ModelSpec {
        mode: Mode::Accurate,
        name: "NLLB-200 3.3B",
        architecture: Architecture::Nllb200,
        parameters: "3.3B",
        storage: "17.5 GB FP32 · Q8 CPU",
        license: "CC-BY-NC-4.0",
        default_dir: "nllb-200-3.3b",
        description: "Higher quality · GPU or lots of RAM",
        supports_beams: true,
    },
    ModelSpec {
        mode: Mode::Madlad,
        name: "MADLAD-400 3B MT",
        architecture: Architecture::T5,
        parameters: "3B",
        storage: "1.7 GB q4_k GGUF",
        license: "Apache-2.0",
        default_dir: "madlad400-3b-mt",
        description: "T5-based · 450+ languages · CPU greedy decoding",
        supports_beams: false,
    },
];

pub fn model_spec(mode: Mode) -> &'static ModelSpec {
    MODELS
        .iter()
        .find(|spec| spec.mode == mode)
        .expect("every Mode has a registry entry")
}

pub fn supports_language(mode: Mode, code: &str) -> bool {
    match model_spec(mode).architecture {
        Architecture::Nllb200 => find_language(code).is_some(),
        Architecture::T5 => madlad::supports_language(code),
    }
}

pub fn language_list(mode: Mode) -> Vec<&'static Language> {
    languages()
        .iter()
        .filter(|language| supports_language(mode, language.code))
        .collect()
}

/// Curated MADLAD target languages. Every entry is checked against the
/// tokenizer at model load; NLLB keeps its full 204-language table.
pub const MADLAD_LANGUAGES: &[MadladLanguage] = &[
    MadladLanguage {
        flores: "eng_Latn",
        prefix: "<2en>",
    },
    MadladLanguage {
        flores: "fra_Latn",
        prefix: "<2fr>",
    },
    MadladLanguage {
        flores: "deu_Latn",
        prefix: "<2de>",
    },
    MadladLanguage {
        flores: "spa_Latn",
        prefix: "<2es>",
    },
    MadladLanguage {
        flores: "jpn_Jpan",
        prefix: "<2ja>",
    },
    MadladLanguage {
        flores: "arb_Arab",
        prefix: "<2ar>",
    },
    MadladLanguage {
        flores: "swh_Latn",
        prefix: "<2sw>",
    },
    MadladLanguage {
        flores: "zho_Hans",
        prefix: "<2zh>",
    },
    MadladLanguage {
        flores: "zho_Hant",
        prefix: "<2zh_Hant>",
    },
    MadladLanguage {
        flores: "rus_Cyrl",
        prefix: "<2ru>",
    },
    MadladLanguage {
        flores: "por_Latn",
        prefix: "<2pt>",
    },
    MadladLanguage {
        flores: "ita_Latn",
        prefix: "<2it>",
    },
    MadladLanguage {
        flores: "kor_Hang",
        prefix: "<2ko>",
    },
    MadladLanguage {
        flores: "hin_Deva",
        prefix: "<2hi>",
    },
    MadladLanguage {
        flores: "tur_Latn",
        prefix: "<2tr>",
    },
    MadladLanguage {
        flores: "nld_Latn",
        prefix: "<2nl>",
    },
    MadladLanguage {
        flores: "pol_Latn",
        prefix: "<2pl>",
    },
    MadladLanguage {
        flores: "ukr_Cyrl",
        prefix: "<2uk>",
    },
    MadladLanguage {
        flores: "vie_Latn",
        prefix: "<2vi>",
    },
    MadladLanguage {
        flores: "ind_Latn",
        prefix: "<2id>",
    },
    MadladLanguage {
        flores: "tha_Thai",
        prefix: "<2th>",
    },
    MadladLanguage {
        flores: "ben_Beng",
        prefix: "<2bn>",
    },
    MadladLanguage {
        flores: "tam_Taml",
        prefix: "<2ta>",
    },
    MadladLanguage {
        flores: "urd_Arab",
        prefix: "<2ur>",
    },
    MadladLanguage {
        flores: "pes_Arab",
        prefix: "<2fa>",
    },
    MadladLanguage {
        flores: "heb_Hebr",
        prefix: "<2he>",
    },
    MadladLanguage {
        flores: "ell_Grek",
        prefix: "<2el>",
    },
    MadladLanguage {
        flores: "ces_Latn",
        prefix: "<2cs>",
    },
    MadladLanguage {
        flores: "swe_Latn",
        prefix: "<2sv>",
    },
    MadladLanguage {
        flores: "dan_Latn",
        prefix: "<2da>",
    },
    MadladLanguage {
        flores: "fin_Latn",
        prefix: "<2fi>",
    },
    MadladLanguage {
        flores: "nob_Latn",
        prefix: "<2no>",
    },
    MadladLanguage {
        flores: "ron_Latn",
        prefix: "<2ro>",
    },
    MadladLanguage {
        flores: "hun_Latn",
        prefix: "<2hu>",
    },
    MadladLanguage {
        flores: "bul_Cyrl",
        prefix: "<2bg>",
    },
    MadladLanguage {
        flores: "srp_Cyrl",
        prefix: "<2sr>",
    },
    MadladLanguage {
        flores: "hrv_Latn",
        prefix: "<2hr>",
    },
    MadladLanguage {
        flores: "slk_Latn",
        prefix: "<2sk>",
    },
    MadladLanguage {
        flores: "slv_Latn",
        prefix: "<2sl>",
    },
    MadladLanguage {
        flores: "lit_Latn",
        prefix: "<2lt>",
    },
    MadladLanguage {
        flores: "lvs_Latn",
        prefix: "<2lv>",
    },
    MadladLanguage {
        flores: "est_Latn",
        prefix: "<2et>",
    },
    MadladLanguage {
        flores: "cat_Latn",
        prefix: "<2ca>",
    },
    MadladLanguage {
        flores: "glg_Latn",
        prefix: "<2gl>",
    },
    MadladLanguage {
        flores: "eus_Latn",
        prefix: "<2eu>",
    },
    MadladLanguage {
        flores: "zsm_Latn",
        prefix: "<2ms>",
    },
    MadladLanguage {
        flores: "tgl_Latn",
        prefix: "<2fil>",
    },
    MadladLanguage {
        flores: "amh_Ethi",
        prefix: "<2am>",
    },
    MadladLanguage {
        flores: "zul_Latn",
        prefix: "<2zu>",
    },
    MadladLanguage {
        flores: "afr_Latn",
        prefix: "<2af>",
    },
    MadladLanguage {
        flores: "hau_Latn",
        prefix: "<2ha>",
    },
    MadladLanguage {
        flores: "yor_Latn",
        prefix: "<2yo>",
    },
    MadladLanguage {
        flores: "ibo_Latn",
        prefix: "<2ig>",
    },
    MadladLanguage {
        flores: "som_Latn",
        prefix: "<2so>",
    },
    MadladLanguage {
        flores: "khm_Khmr",
        prefix: "<2km>",
    },
    MadladLanguage {
        flores: "lao_Laoo",
        prefix: "<2lo>",
    },
    MadladLanguage {
        flores: "mya_Mymr",
        prefix: "<2my>",
    },
    MadladLanguage {
        flores: "npi_Deva",
        prefix: "<2ne>",
    },
    MadladLanguage {
        flores: "sin_Sinh",
        prefix: "<2si>",
    },
    MadladLanguage {
        flores: "kaz_Cyrl",
        prefix: "<2kk>",
    },
    MadladLanguage {
        flores: "uzn_Latn",
        prefix: "<2uz>",
    },
    MadladLanguage {
        flores: "azj_Latn",
        prefix: "<2az>",
    },
    MadladLanguage {
        flores: "hye_Armn",
        prefix: "<2hy>",
    },
    MadladLanguage {
        flores: "kat_Geor",
        prefix: "<2ka>",
    },
    MadladLanguage {
        flores: "mkd_Cyrl",
        prefix: "<2mk>",
    },
    MadladLanguage {
        flores: "als_Latn",
        prefix: "<2sq>",
    },
    MadladLanguage {
        flores: "bos_Latn",
        prefix: "<2bs>",
    },
    MadladLanguage {
        flores: "isl_Latn",
        prefix: "<2is>",
    },
    MadladLanguage {
        flores: "gle_Latn",
        prefix: "<2ga>",
    },
    MadladLanguage {
        flores: "cym_Latn",
        prefix: "<2cy>",
    },
    MadladLanguage {
        flores: "mlt_Latn",
        prefix: "<2mt>",
    },
    MadladLanguage {
        flores: "ltz_Latn",
        prefix: "<2lb>",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_the_three_models() {
        assert_eq!(MODELS.len(), 3);
        assert_eq!(model_spec(Mode::Fast).architecture, Architecture::Nllb200);
        assert_eq!(
            model_spec(Mode::Accurate).architecture,
            Architecture::Nllb200
        );
        assert_eq!(model_spec(Mode::Madlad).architecture, Architecture::T5);
        assert!(!model_spec(Mode::Madlad).supports_beams);
        assert!(model_spec(Mode::Fast).supports_beams);
    }

    #[test]
    fn madlad_required_languages_are_advertised() {
        for language in [
            "eng_Latn", "fra_Latn", "deu_Latn", "spa_Latn", "jpn_Jpan", "arb_Arab", "swh_Latn",
        ] {
            assert!(supports_language(Mode::Madlad, language), "{language}");
            assert!(supports_language(Mode::Fast, language), "{language}");
        }
        assert!(!supports_language(Mode::Madlad, "not_a_language"));
    }

    #[test]
    fn nllb_keeps_its_full_language_table() {
        assert_eq!(language_list(Mode::Fast).len(), languages().len());
        assert!(language_list(Mode::Madlad).len() >= 50);
        assert!(language_list(Mode::Madlad).len() < languages().len());
    }
}
