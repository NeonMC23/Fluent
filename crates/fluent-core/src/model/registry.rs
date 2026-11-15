//! Central model registry and capability metadata.

use crate::model::madlad::{self, MadladLanguage};
use crate::model::small100;
use crate::{find_language, languages, Language, Mode};

/// Model families with distinct inference implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    Nllb200,
    T5,
    Small100,
}

impl Architecture {
    /// Identifier used by the remote catalogue.
    pub fn catalogue_id(self) -> &'static str {
        match self {
            Self::Nllb200 => "nllb200",
            Self::T5 => "t5",
            Self::Small100 => "small100",
        }
    }

    pub fn from_catalogue_id(id: &str) -> Option<Self> {
        match id {
            "nllb200" => Some(Self::Nllb200),
            "t5" => Some(Self::T5),
            "small100" => Some(Self::Small100),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Nllb200 => "NLLB-200",
            Self::T5 => "T5",
            Self::Small100 => "SMaLL-100",
        }
    }
}

/// One required model file, pinned by size and SHA-256.
#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// User-visible and acquisition metadata for one selectable model.
#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub mode: Mode,
    /// Stable identifier used by the remote catalogue and settings.
    pub id: &'static str,
    pub name: &'static str,
    pub architecture: Architecture,
    pub parameters: &'static str,
    pub storage: &'static str,
    pub license: &'static str,
    pub default_dir: &'static str,
    pub description: &'static str,
    pub supports_beams: bool,
    pub repo: &'static str,
    pub revision: &'static str,
    pub files: &'static [ModelFile],
}

impl ModelSpec {
    pub fn download_bytes(self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }
}

const NLLB13_FILES: &[ModelFile] = &[
    ModelFile {
        name: "config.json",
        size: 808,
        sha256: "6918d76e923d6b372a14b8d1d837f27b809bd009943c4ebe943f8e34b2226233",
    },
    ModelFile {
        name: "tokenizer.json",
        size: 17_331_176,
        sha256: "e316b82de11d0f951f370943b3c438311629547285129b0b81dadabd01bca665",
    },
    ModelFile {
        name: "tokenizer_config.json",
        size: 564,
        sha256: "d1aa8c3697d3e35674f97b5b7e9c99d22b010f528e80140257d97316be90d044",
    },
    ModelFile {
        name: "special_tokens_map.json",
        size: 3548,
        sha256: "992bd4ed610d644d6823081937bcc91bb8878dd556cea4ae5327f2480361330e",
    },
    ModelFile {
        name: "sentencepiece.bpe.model",
        size: 4_852_054,
        sha256: "14bb8dfb35c0ffdea7bc01e56cea38b9e3d5efcdcb9c251d6b40538e1aab555a",
    },
    ModelFile {
        name: "pytorch_model.bin",
        size: 5_482_882_236,
        sha256: "8a134f26de3677e0b7c47d3e98c199fdb4c85a8c6577448d861af4ff24e2ad3f",
    },
];

const NLLB33_FILES: &[ModelFile] = &[
    ModelFile {
        name: "config.json",
        size: 808,
        sha256: "24027fbba45b65c168cc22f5afa3561476691e0e9a06286e1430ffff74e418eb",
    },
    ModelFile {
        name: "tokenizer.json",
        size: 17_331_176,
        sha256: "e316b82de11d0f951f370943b3c438311629547285129b0b81dadabd01bca665",
    },
    ModelFile {
        name: "tokenizer_config.json",
        size: 564,
        sha256: "d1aa8c3697d3e35674f97b5b7e9c99d22b010f528e80140257d97316be90d044",
    },
    ModelFile {
        name: "special_tokens_map.json",
        size: 3548,
        sha256: "992bd4ed610d644d6823081937bcc91bb8878dd556cea4ae5327f2480361330e",
    },
    ModelFile {
        name: "sentencepiece.bpe.model",
        size: 4_852_054,
        sha256: "14bb8dfb35c0ffdea7bc01e56cea38b9e3d5efcdcb9c251d6b40538e1aab555a",
    },
    ModelFile {
        name: "pytorch_model.bin.index.json",
        size: 90_035,
        sha256: "c1290129b7a4156bfeb180501a37729dafddec3233d58e52ab0465eb03bf5ead",
    },
    ModelFile {
        name: "pytorch_model-00001-of-00003.bin",
        size: 6_933_375_959,
        sha256: "06aec913c891950ff0c59bd018c1b46dc95be6e106c36df87d13175418b3972c",
    },
    ModelFile {
        name: "pytorch_model-00002-of-00003.bin",
        size: 8_545_249_147,
        sha256: "1c5a44acb0287e905a62a506cf3e5af37956607f4c667d891ec069aa276be0be",
    },
    ModelFile {
        name: "pytorch_model-00003-of-00003.bin",
        size: 2_098_840_299,
        sha256: "2cf744009996251567d7972f4436d65cf94380c5ddd8f524cb878e090b27bb50",
    },
];

const MADLAD_FILES: &[ModelFile] = &[
    ModelFile {
        name: "config.json",
        size: 749,
        sha256: "cad399cab799b99409a6ec2d90d72552257c2bb752861261d2016691e0643e7c",
    },
    ModelFile {
        name: "generation_config.json",
        size: 142,
        sha256: "0849b38987568ccfe4ebefc22bbda1cec4bee01c345e93bfab207d4692b0a1d5",
    },
    ModelFile {
        name: "tokenizer_config.json",
        size: 830,
        sha256: "641fc660745306dfb935f666a68f8bc10a44c39241cfb357be518fda8c09662d",
    },
    ModelFile {
        name: "special_tokens_map.json",
        size: 414,
        sha256: "7f79f1d5063d56c4b980eec0692f3c7429bdef335071d34e566bd00fd4b5e3e0",
    },
    ModelFile {
        name: "added_tokens.json",
        size: 4,
        sha256: "d914176fd50bd7f565700006a31aa97b79d3ad17cee20c8e5ff2061d5cb74817",
    },
    ModelFile {
        name: "tokenizer.json",
        size: 16_629_031,
        sha256: "a2799ccc696b752ba00c34f58726bfe253a04921ceb6cfc620400f560474790b",
    },
    ModelFile {
        name: "spiece.model",
        size: 4_427_844,
        sha256: "ef11ac9a22c7503492f56d48dce53be20e339b63605983e9f27d2cd0e0f3922c",
    },
    ModelFile {
        name: "model-q4k.gguf",
        size: 1_654_597_280,
        sha256: "ea6e5531a3e95213c7f0635988d119e078a655c09306e47851e15d4c0c3f9c37",
    },
];

const SMALL100_FILES: &[ModelFile] = &[
    ModelFile {
        name: "config.json",
        size: 890,
        sha256: "26fd3989cea6037d432c480f5181d05c088a613394d0361edc6605a6a1058715",
    },
    ModelFile {
        name: "tokenizer_config.json",
        size: 1_867,
        sha256: "d5a67f279887133e8eb8b9749fd1b6b5831bb1a040fcd6ac820296041f303ba2",
    },
    ModelFile {
        name: "special_tokens_map.json",
        size: 1_559,
        sha256: "009ea667e0ca903c10dac22cf7ae3a3a0b173ff33f8c64154fddd8c043805622",
    },
    ModelFile {
        name: "vocab.json",
        size: 3_708_092,
        sha256: "b6e77e474aeea8f441363aca7614317c06381f3eacfe10fb9856d5081d1074cc",
    },
    ModelFile {
        name: "sentencepiece.bpe.model",
        size: 2_423_393,
        sha256: "d8f7c76ed2a5e0822be39f0a4f95a55eb19c78f4593ce609e2edbc2aea4d380a",
    },
    ModelFile {
        name: "model.safetensors",
        size: 1_330_973_772,
        sha256: "dd3b845a36ea4ed90437fd0b9b477e30c21f144d3658679fd5c945e3c96b0fbc",
    },
];

pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        mode: Mode::Fast,
        id: "nllb-200-1.3b",
        name: "NLLB-200 1.3B",
        architecture: Architecture::Nllb200,
        parameters: "1.3B",
        storage: "5.2 GB FP32 · Q8 CPU",
        license: "CC-BY-NC-4.0",
        default_dir: "nllb-200-1.3b",
        description: "Fast local translation · recommended on CPU",
        supports_beams: true,
        repo: "facebook/nllb-200-1.3B",
        revision: "b0de46b488af0cf31749cd8da5ed3171e11b2309",
        files: NLLB13_FILES,
    },
    ModelSpec {
        mode: Mode::Accurate,
        id: "nllb-200-3.3b",
        name: "NLLB-200 3.3B",
        architecture: Architecture::Nllb200,
        parameters: "3.3B",
        storage: "17.5 GB FP32 · Q8 CPU",
        license: "CC-BY-NC-4.0",
        default_dir: "nllb-200-3.3b",
        description: "Higher quality · GPU or lots of RAM",
        supports_beams: true,
        repo: "facebook/nllb-200-3.3B",
        revision: "1a07f7d195896b2114afcb79b7b57ab512e7b43e",
        files: NLLB33_FILES,
    },
    ModelSpec {
        mode: Mode::Madlad,
        id: "madlad400-3b-mt",
        name: "MADLAD-400 3B MT",
        architecture: Architecture::T5,
        parameters: "3B",
        storage: "1.7 GB q4_k GGUF",
        license: "Apache-2.0",
        default_dir: "madlad400-3b-mt",
        description: "T5-based · 450+ languages · CPU greedy decoding",
        supports_beams: false,
        repo: "google/madlad400-3b-mt",
        revision: "fa184c675da0b5c9e1c8694fccd4e12e2d422094",
        files: MADLAD_FILES,
    },
    ModelSpec {
        mode: Mode::Small,
        id: "small100",
        name: "SMaLL-100",
        architecture: Architecture::Small100,
        parameters: "330M",
        storage: "1.4 GB FP32 · Q8 CPU",
        license: "MIT",
        default_dir: "small100",
        description: "Lightweight · 98 mapped languages · fast CPU translation",
        supports_beams: true,
        repo: "alirezamsh/small100",
        revision: "8ab680e26a596d2e3d2d2d17ae0f68df1037328c",
        files: SMALL100_FILES,
    },
];

pub fn model_spec_by_id(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|spec| spec.id == id)
}

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
        Architecture::Small100 => small100::supports_language(code),
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
    fn registry_has_the_four_models() {
        assert_eq!(MODELS.len(), 4);
        assert_eq!(model_spec(Mode::Fast).architecture, Architecture::Nllb200);
        assert_eq!(
            model_spec(Mode::Accurate).architecture,
            Architecture::Nllb200
        );
        assert_eq!(model_spec(Mode::Madlad).architecture, Architecture::T5);
        assert_eq!(model_spec(Mode::Small).architecture, Architecture::Small100);
        assert!(!model_spec(Mode::Madlad).supports_beams);
        assert!(model_spec(Mode::Fast).supports_beams);
        assert!(model_spec(Mode::Small).supports_beams);
    }

    #[test]
    fn model_ids_and_directories_are_unique() {
        let mut ids = std::collections::HashSet::new();
        let mut dirs = std::collections::HashSet::new();
        for spec in MODELS {
            assert!(ids.insert(spec.id), "duplicate id {}", spec.id);
            assert!(
                dirs.insert(spec.default_dir),
                "duplicate dir {}",
                spec.default_dir
            );
            assert!(!spec.files.is_empty(), "{} has no files", spec.id);
            assert!(spec.download_bytes() > 0);
            assert_eq!(model_spec_by_id(spec.id).map(|s| s.mode), Some(spec.mode));
        }
        assert!(model_spec_by_id("unknown").is_none());
    }

    #[test]
    fn small100_advertises_the_required_languages() {
        for language in [
            "eng_Latn", "fra_Latn", "deu_Latn", "spa_Latn", "por_Latn", "ita_Latn", "zho_Hans",
        ] {
            assert!(supports_language(Mode::Small, language), "{language}");
        }
        assert_eq!(language_list(Mode::Small).len(), 98);
        assert!(!supports_language(Mode::Small, "zho_Hant"));
    }

    #[test]
    fn catalogue_backend_ids_round_trip() {
        for architecture in [
            Architecture::Nllb200,
            Architecture::T5,
            Architecture::Small100,
        ] {
            assert_eq!(
                Architecture::from_catalogue_id(architecture.catalogue_id()),
                Some(architecture)
            );
        }
        assert_eq!(Architecture::from_catalogue_id("python"), None);
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
