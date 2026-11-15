//! Versioned remote model catalogue.
//!
//! The catalogue is a strictly validated JSON document served from the
//! official Fluent repository. It mirrors the acquisition metadata compiled
//! into [`super::registry`] so users can refresh model information without
//! updating the application.
//!
//! Security boundaries:
//! - HTTPS only, with a small body limit.
//! - Unknown schema versions, unknown backends, unsafe file names, non-HTTPS
//!   URLs and files outside the pinned Hugging Face revision fail closed.
//! - Entries must agree with the built-in registry (id, backend, revision,
//!   licence and exact file manifest). The catalogue can therefore never point
//!   Fluent at arbitrary weights or a backend the binary does not implement.
//!
//! Adding a genuinely new model still requires a Fluent update that implements
//! its backend and registers it locally. This is deliberate: a remote document
//! must never be able to introduce executable behaviour.

use std::collections::HashSet;
use std::io::Read;
use std::time::Duration;

use serde::Deserialize;

use crate::model::registry::{model_spec_by_id, Architecture, ModelSpec};
use crate::{Error, Result};

/// Only this schema version is accepted.
pub const SCHEMA_VERSION: u32 = 1;
/// Hard limit for the downloaded document.
pub const MAX_BYTES: u64 = 256 * 1024;
/// Official endpoint.
pub const DEFAULT_URL: &str =
    "https://raw.githubusercontent.com/NeonMC23/Fluent/main/catalogue/models-v1.json";
/// Environment override, intended for testing and self-hosted mirrors.
pub const URL_ENV: &str = "FLUENT_CATALOGUE_URL";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalogue {
    pub schema_version: u32,
    #[serde(default)]
    pub generated_at: String,
    /// Oldest Fluent version that understands this document.
    #[serde(default = "default_minimum_version")]
    pub minimum_app_version: String,
    pub models: Vec<CatalogueModel>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogueModel {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    /// One of `nllb200`, `t5`, `small100`.
    pub backend: String,
    pub revision: String,
    pub license: String,
    #[serde(default)]
    pub license_url: Option<String>,
    #[serde(default)]
    pub model_page: Option<String>,
    #[serde(default)]
    pub deprecated: bool,
    /// Sum of the required file sizes.
    pub total_size: u64,
    /// Fluent/NLLB language codes supported by this entry.
    pub languages: Vec<String>,
    pub files: Vec<CatalogueFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogueFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub url: String,
}

/// A catalogue entry paired with the compiled model it describes.
#[derive(Debug, Clone)]
pub struct CatalogueMatch {
    pub spec: &'static ModelSpec,
    pub entry: CatalogueModel,
}

fn default_minimum_version() -> String {
    "0.2.0".to_string()
}

impl Catalogue {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() as u64 > MAX_BYTES {
            return Err(Error::Catalogue(format!(
                "document is larger than {} KiB",
                MAX_BYTES / 1024
            )));
        }
        let catalogue: Self = serde_json::from_slice(bytes)
            .map_err(|error| Error::Catalogue(format!("invalid JSON: {error}")))?;
        catalogue.validate()?;
        Ok(catalogue)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(Error::Catalogue(format!(
                "unsupported schema version {} (expected {SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        if !version_at_least(env!("CARGO_PKG_VERSION"), &self.minimum_app_version) {
            return Err(Error::Catalogue(format!(
                "this catalogue requires Fluent {} or newer (running {})",
                self.minimum_app_version,
                env!("CARGO_PKG_VERSION")
            )));
        }
        if self.models.is_empty() {
            return Err(Error::Catalogue("the catalogue has no models".to_string()));
        }
        if self.models.len() > super::registry::MODELS.len() {
            return Err(Error::Catalogue(format!(
                "the catalogue lists {} models; at most {} are supported",
                self.models.len(),
                super::registry::MODELS.len()
            )));
        }
        let mut ids = HashSet::new();
        for model in &self.models {
            model.validate()?;
            if !ids.insert(model.id.as_str()) {
                return Err(Error::Catalogue(format!(
                    "duplicate model id {:?}",
                    model.id
                )));
            }
        }
        // Fail closed if the document disagrees with the models compiled into
        // this build. This is what prevents a remote document from
        // substituting weights or introducing an unknown backend.
        self.matches()?;
        Ok(())
    }

    /// Maps catalogue entries to the models compiled into this build.
    ///
    /// Returns one match per local model that is present in the catalogue.
    pub fn matches(&self) -> Result<Vec<CatalogueMatch>> {
        let mut matches = Vec::new();
        for entry in &self.models {
            let spec = model_spec_by_id(&entry.id).ok_or_else(|| {
                Error::Catalogue(format!(
                    "model {:?} is not supported by this Fluent build",
                    entry.id
                ))
            })?;
            entry.matches_spec(spec)?;
            matches.push(CatalogueMatch {
                spec,
                entry: entry.clone(),
            });
        }
        Ok(matches)
    }

    pub fn contains(&self, mode: crate::Mode) -> bool {
        let id = super::registry::model_spec(mode).id;
        self.models.iter().any(|model| model.id == id)
    }
}

impl CatalogueModel {
    fn validate(&self) -> Result<()> {
        validate_id(&self.id)?;
        validate_text("display_name", &self.display_name, 128)?;
        if self.description.len() > 1024 || self.description.chars().any(|c| c.is_control()) {
            return Err(Error::Catalogue(format!(
                "model {:?} has an invalid description",
                self.id
            )));
        }
        validate_text("license", &self.license, 128)?;
        Architecture::from_catalogue_id(&self.backend).ok_or_else(|| {
            Error::Catalogue(format!(
                "model {:?} uses unsupported backend {:?}",
                self.id, self.backend
            ))
        })?;
        if self.revision.len() != 40 || !self.revision.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Catalogue(format!(
                "model {:?} has an invalid revision",
                self.id
            )));
        }
        if self.languages.is_empty() || self.languages.len() > 204 {
            return Err(Error::Catalogue(format!(
                "model {:?} has an invalid language list",
                self.id
            )));
        }
        let mut seen = HashSet::new();
        for code in &self.languages {
            if crate::find_language(code).is_none() {
                return Err(Error::Catalogue(format!(
                    "model {:?} lists unknown language {code:?}",
                    self.id
                )));
            }
            if !seen.insert(code.as_str()) {
                return Err(Error::Catalogue(format!(
                    "model {:?} repeats language {code:?}",
                    self.id
                )));
            }
        }
        if let Some(url) = &self.license_url {
            validate_https_url(url)?;
        }
        if let Some(url) = &self.model_page {
            validate_https_url(url)?;
        }
        if self.files.is_empty() || self.files.len() > 64 {
            return Err(Error::Catalogue(format!(
                "model {:?} has an invalid file manifest",
                self.id
            )));
        }
        let total: u64 = self.files.iter().map(|file| file.size).sum();
        if self.total_size != total {
            return Err(Error::Catalogue(format!(
                "model {:?} total_size does not match its files",
                self.id
            )));
        }
        let mut names = HashSet::new();
        for file in &self.files {
            file.validate()?;
            if !names.insert(file.name.as_str()) {
                return Err(Error::Catalogue(format!(
                    "model {:?} repeats file {:?}",
                    self.id, file.name
                )));
            }
        }
        Ok(())
    }

    /// Rejects entries that disagree with the compiled registry.
    fn matches_spec(&self, spec: &ModelSpec) -> Result<()> {
        if self.backend != spec.architecture.catalogue_id() {
            return Err(Error::Catalogue(format!(
                "model {:?} claims backend {:?}, but this build uses {:?}",
                self.id,
                self.backend,
                spec.architecture.catalogue_id()
            )));
        }
        if self.revision != spec.revision {
            return Err(Error::Catalogue(format!(
                "model {:?} revision does not match this Fluent build",
                self.id
            )));
        }
        if !self.license.eq_ignore_ascii_case(spec.license) {
            return Err(Error::Catalogue(format!(
                "model {:?} licence does not match this Fluent build",
                self.id
            )));
        }
        if self.files.len() != spec.files.len() {
            return Err(Error::Catalogue(format!(
                "model {:?} file count does not match this Fluent build",
                self.id
            )));
        }
        for file in &self.files {
            let expected = spec
                .files
                .iter()
                .find(|candidate| candidate.name == file.name)
                .ok_or_else(|| {
                    Error::Catalogue(format!(
                        "model {:?} lists unexpected file {:?}",
                        self.id, file.name
                    ))
                })?;
            if file.size != expected.size || !file.sha256.eq_ignore_ascii_case(expected.sha256) {
                return Err(Error::Catalogue(format!(
                    "model {:?} file {:?} does not match this Fluent build",
                    self.id, file.name
                )));
            }
            let expected_url = format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                spec.repo, spec.revision, expected.name
            );
            if file.url != expected_url {
                return Err(Error::Catalogue(format!(
                    "model {:?} file {:?} has an unexpected download URL",
                    self.id, file.name
                )));
            }
        }
        for code in &self.languages {
            if !spec.mode.supports_language(code) {
                return Err(Error::Catalogue(format!(
                    "model {:?} lists language {code:?} that the backend does not support",
                    self.id
                )));
            }
        }
        Ok(())
    }
}

fn version_at_least(current: &str, required: &str) -> bool {
    fn parts(value: &str) -> Vec<u64> {
        value
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    }
    let current = parts(current);
    let required = parts(required);
    for index in 0..3 {
        let a = current.get(index).copied().unwrap_or(0);
        let b = required.get(index).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    true
}

fn validate_id(id: &str) -> Result<()> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        && !id.starts_with('.')
        && !id.ends_with('.');
    if valid {
        Ok(())
    } else {
        Err(Error::Catalogue(format!("invalid model id {id:?}")))
    }
}

fn validate_text(field: &str, value: &str, max: usize) -> Result<()> {
    if value.is_empty() || value.len() > max || value.chars().any(|c| c.is_control()) {
        Err(Error::Catalogue(format!("invalid {field}")))
    } else {
        Ok(())
    }
}

impl CatalogueFile {
    fn validate(&self) -> Result<()> {
        let valid_name = !self.name.is_empty()
            && self.name.len() <= 128
            && !self.name.contains('/')
            && !self.name.contains('\\')
            && self.name != "."
            && self.name != ".."
            && self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if !valid_name {
            return Err(Error::Catalogue(format!(
                "unsafe file name {:?}",
                self.name
            )));
        }
        if self.size == 0 || self.size > 64 * 1024 * 1024 * 1024 {
            return Err(Error::Catalogue(format!(
                "file {:?} has an invalid size",
                self.name
            )));
        }
        let valid_hash = self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !valid_hash {
            return Err(Error::Catalogue(format!(
                "file {:?} has an invalid SHA-256",
                self.name
            )));
        }
        validate_https_url(&self.url)?;
        Ok(())
    }
}

fn validate_https_url(url: &str) -> Result<()> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(Error::Catalogue(format!("refusing a non-HTTPS URL: {url}")));
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .split('@')
        .next_back()
        .unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    if host.is_empty()
        || host.eq_ignore_ascii_case("localhost")
        || host.parse::<std::net::IpAddr>().is_ok()
    {
        return Err(Error::Catalogue(format!("unsafe URL host: {url}")));
    }
    Ok(())
}

/// Fetches and validates the catalogue from `url` (or the environment
/// override, or the official endpoint).
pub fn fetch(url: Option<&str>) -> Result<Catalogue> {
    let url = url
        .map(str::to_string)
        .or_else(|| {
            std::env::var(URL_ENV)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| DEFAULT_URL.to_string());
    validate_https_url(&url)?;

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(15)))
        .timeout_per_read(Some(Duration::from_secs(20)))
        .build()
        .into();
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|error| Error::Catalogue(format!("catalogue request failed: {error}")))?;
    if response.status().as_u16() != 200 {
        return Err(Error::Catalogue(format!(
            "catalogue request returned HTTP {}",
            response.status().as_u16()
        )));
    }
    let mut body = Vec::new();
    response
        .body_mut()
        .with_config()
        .limit(MAX_BYTES + 1)
        .reader()
        .read_to_end(&mut body)
        .map_err(|error| Error::Catalogue(format!("catalogue download failed: {error}")))?;
    Catalogue::parse(&body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::registry::MODELS;

    fn valid_entry(spec: &ModelSpec) -> CatalogueModel {
        CatalogueModel {
            id: spec.id.to_string(),
            display_name: spec.name.to_string(),
            description: spec.description.to_string(),
            backend: spec.architecture.catalogue_id().to_string(),
            revision: spec.revision.to_string(),
            license: spec.license.to_string(),
            license_url: None,
            model_page: None,
            deprecated: false,
            total_size: spec.files.iter().map(|file| file.size).sum(),
            languages: spec
                .mode
                .supports_language("eng_Latn")
                .then(|| "eng_Latn".to_string())
                .into_iter()
                .collect(),
            files: spec
                .files
                .iter()
                .map(|file| CatalogueFile {
                    name: file.name.to_string(),
                    size: file.size,
                    sha256: file.sha256.to_string(),
                    url: format!(
                        "https://huggingface.co/{}/resolve/{}/{}",
                        spec.repo, spec.revision, file.name
                    ),
                })
                .collect(),
        }
    }

    fn valid_catalogue() -> Catalogue {
        Catalogue {
            schema_version: SCHEMA_VERSION,
            generated_at: "2026-11-15T00:00:00Z".to_string(),
            minimum_app_version: "0.2.0".to_string(),
            models: MODELS.iter().map(valid_entry).collect(),
        }
    }

    #[test]
    fn the_committed_catalogue_matches_this_build() {
        let catalogue = Catalogue::parse(include_bytes!("../../../../catalogue/models-v1.json"))
            .expect("committed catalogue is valid");
        let matches = catalogue.matches().expect("catalogue matches the registry");
        assert_eq!(matches.len(), MODELS.len());
    }

    #[test]
    fn a_matching_catalogue_validates_and_maps_every_model() {
        let catalogue = valid_catalogue();
        catalogue.validate().unwrap();
        let matches = catalogue.matches().unwrap();
        assert_eq!(matches.len(), MODELS.len());
    }

    #[test]
    fn newer_required_versions_fail_closed() {
        let mut catalogue = valid_catalogue();
        catalogue.minimum_app_version = "99.0.0".to_string();
        assert!(catalogue.validate().is_err());
        catalogue.minimum_app_version = env!("CARGO_PKG_VERSION").to_string();
        catalogue.validate().unwrap();
    }

    #[test]
    fn total_size_must_match_the_file_manifest() {
        let mut catalogue = valid_catalogue();
        catalogue.models[0].total_size += 1;
        assert!(catalogue.validate().is_err());
    }

    #[test]
    fn unknown_schema_versions_fail_closed() {
        let mut catalogue = valid_catalogue();
        catalogue.schema_version = 2;
        assert!(catalogue.validate().is_err());
    }

    #[test]
    fn unknown_backends_fail_closed() {
        let mut catalogue = valid_catalogue();
        catalogue.models[0].backend = "python".to_string();
        assert!(catalogue.validate().is_err());
    }

    #[test]
    fn unsafe_file_names_and_non_https_urls_are_rejected() {
        let mut catalogue = valid_catalogue();
        catalogue.models[0].files[0].name = "../escape".to_string();
        assert!(catalogue.validate().is_err());

        let mut catalogue = valid_catalogue();
        catalogue.models[0].files[0].url =
            "http://huggingface.co/facebook/nllb-200-1.3B/resolve/x/config.json".to_string();
        assert!(catalogue.validate().is_err());
    }

    #[test]
    fn hashes_must_be_lowercase_hex() {
        let mut catalogue = valid_catalogue();
        catalogue.models[0].files[0].sha256 = "ABC".to_string();
        assert!(catalogue.validate().is_err());
    }

    #[test]
    fn entries_that_disagree_with_the_build_are_rejected() {
        let mut catalogue = valid_catalogue();
        catalogue.models[0].files[0].size += 1;
        assert!(catalogue.matches().is_err());

        let mut catalogue = valid_catalogue();
        catalogue.models[0].revision = "0".repeat(40);
        assert!(catalogue.matches().is_err());

        let mut catalogue = valid_catalogue();
        catalogue.models[0].files[0].url = format!(
            "https://huggingface.co/facebook/nllb-200-1.3B/resolve/{}/config.json",
            "0".repeat(40)
        );
        assert!(catalogue.matches().is_err());
    }

    #[test]
    fn unknown_models_fail_closed() {
        let mut catalogue = valid_catalogue();
        let mut entry = valid_entry(&MODELS[0]);
        entry.id = "mystery-model".to_string();
        catalogue.models.push(entry);
        assert!(catalogue.matches().is_err());
    }

    #[test]
    fn oversized_documents_are_rejected_before_parsing() {
        let bytes = vec![b' '; MAX_BYTES as usize + 1];
        assert!(Catalogue::parse(&bytes).is_err());
    }

    #[test]
    fn network_failures_are_reported_cleanly() {
        let error = fetch(Some("http://example.com/models.json")).unwrap_err();
        assert!(matches!(error, Error::Catalogue(_)));
    }
}
