use std::sync::OnceLock;

use crate::Language;

const RAW: &str = include_str!("languages.tsv");

fn parse() -> Vec<Language> {
    let mut out = Vec::with_capacity(220);
    for line in RAW.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split('\t');
        let code = parts.next().unwrap_or("").trim();
        let name = parts.next().unwrap_or("").trim();
        let native = parts.next().unwrap_or(name).trim();
        let popular = parts.next().unwrap_or("0").trim() == "1";
        let iso3 = parts.next().unwrap_or("").trim();
        if code.is_empty() || name.is_empty() {
            continue;
        }
        // The file is static: the strings live as long as the program.
        out.push(Language {
            code: Box::leak(code.to_string().into_boxed_str()),
            name: Box::leak(name.to_string().into_boxed_str()),
            native: Box::leak(native.to_string().into_boxed_str()),
            popular,
            iso3: Box::leak(iso3.to_string().into_boxed_str()),
        });
    }
    out
}

pub fn languages() -> &'static [Language] {
    static LANGUAGES: OnceLock<Vec<Language>> = OnceLock::new();
    LANGUAGES.get_or_init(parse).as_slice()
}

pub fn find_language(code: &str) -> Option<&'static Language> {
    languages().iter().find(|language| language.code == code)
}

pub fn popular_languages() -> impl Iterator<Item = &'static Language> {
    languages().iter().filter(|language| language.popular)
}

/// Finds an NLLB language from an ISO 639-3 code, preferring
/// the script when several variants exist.
pub fn find_by_iso3(iso3: &str, script: Option<&str>) -> Option<&'static Language> {
    let iso3 = iso3.to_ascii_lowercase();
    let candidates: Vec<&'static Language> = languages()
        .iter()
        .filter(|language| language.iso3.eq_ignore_ascii_case(&iso3))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    if let Some(script) = script {
        if let Some(found) = candidates
            .iter()
            .copied()
            .find(|language| language.code.ends_with(script))
        {
            return Some(found);
        }
    }
    candidates.first().copied()
}

pub fn is_supported(code: &str) -> bool {
    find_language(code).is_some()
}
