use whatlang::{Detector, Lang, Script};

use crate::languages::find_language;

#[derive(Debug, Clone)]
pub struct Detection {
    pub code: &'static str,
    pub name: &'static str,
    pub confidence: f32,
    pub iso3: String,
    pub reliable: bool,
}

/// Detects the language of a text with `whatlang`, then maps the
/// ISO 639-3 code to the FLORES-200 code expected by NLLB.
pub fn detect(text: &str) -> Option<Detection> {
    let trimmed = text.trim();
    if trimmed.chars().filter(|c| c.is_alphabetic()).count() < 3 {
        return None;
    }

    let info = Detector::new().detect(trimmed)?;
    let iso3 = info.lang().code().to_string();
    let script = info.script();
    let code = map_language(info.lang(), script).unwrap_or("eng_Latn");
    let language = find_language(code);
    let confidence = info.confidence() as f32;

    Some(Detection {
        code,
        name: language.map(|l| l.name).unwrap_or(code),
        confidence,
        iso3,
        reliable: confidence >= 0.20 && trimmed.chars().count() >= 6,
    })
}

fn map_language(lang: Lang, script: Script) -> Option<&'static str> {
    let iso3 = lang.code();

    // Codes whose NLLB variant does not match the ISO prefix.
    let special = match iso3 {
        "cmn" | "zho" => Some("zho_Hans"),
        "yue" => Some("yue_Hant"),
        "ara" => Some("arb_Arab"),
        "fas" | "pes" => Some("pes_Arab"),
        "prs" => Some("prs_Arab"),
        "kur" => Some(if script == Script::Arabic {
            "ckb_Arab"
        } else {
            "kmr_Latn"
        }),
        "nob" | "nor" => Some("nob_Latn"),
        "nno" => Some("nno_Latn"),
        "zsm" | "msa" | "may" => Some("zsm_Latn"),
        "fil" => Some("tgl_Latn"),
        "ori" => Some("ory_Orya"),
        "sqi" | "alb" => Some("als_Latn"),
        "hye" | "arm" => Some("hye_Armn"),
        "kat" | "geo" => Some("kat_Geor"),
        "mya" | "bur" => Some("mya_Mymr"),
        "mkd" | "mac" => Some("mkd_Cyrl"),
        "srp" => Some("srp_Cyrl"),
        _ => None,
    };
    if let Some(code) = special {
        return Some(code);
    }

    // Generic prefix match: fra -> fra_Latn, deu -> deu_Latn…
    let prefix = format!("{iso3}_");
    crate::languages::languages()
        .iter()
        .find(|language| language.code.starts_with(&prefix))
        .map(|language| language.code)
        .or_else(|| crate::languages::find_by_iso3(iso3, None).map(|language| language.code))
}
